//! Optimized assets in a content-addressed directory.
//!
//! Keep relative asset paths together under one digest so CSS references survive
//! versioning. CSS and JavaScript optimization is optional; other bytes are copied.
//! This module writes into a caller-owned stage and never installs live output.

#[cfg(feature = "minify-css")]
use crate::config::CssOptions;
#[cfg(feature = "minify-js")]
use crate::config::JsOptions;
use crate::config::{Minify, RegressionCheckMode};
#[cfg(any(feature = "minify-css", feature = "minify-js"))]
use crate::files::read_text;
use crate::files::{files, output_file, reject_symlinks};
#[cfg(any(feature = "minify-css", feature = "minify-js"))]
use anyhow::Context;
use anyhow::{Result, ensure};
#[cfg(feature = "minify-css")]
use lightningcss::stylesheet::{MinifyOptions, ParserOptions, PrinterOptions, StyleSheet};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

/// Template URL suffix and source count for the prepared asset tree.
pub(crate) struct Assets {
    /// Root-relative `/assets/<digest>` path, before the deployment prefix.
    pub(crate) base: String,
    /// Number of source files, including CSS and unchanged binary assets.
    pub(crate) count: usize,
}

/// Prepare optional `assets/`, hashing optimized bytes rather than source spelling.
///
/// Returns errors for invalid inventory paths, CSS processing, changed copy lengths,
/// or filesystem failures. The caller must discard the stage on failure.
pub(crate) fn prepare(
    root: &Path,
    stage: &Path,
    minify: &Minify,
    minify_assets: bool,
    regression_checks: RegressionCheckMode,
) -> Result<Assets> {
    let input = root.join("assets");
    let sources = files(&input, true)?;
    fs::create_dir(stage.join("assets"))?;
    let mut digest = Sha256::new();
    // Version the digest encoding and frame names/content lengths so distinct
    // inventories cannot alias through ambiguous byte concatenation.
    digest.update(b"regen-assets-v1\0");
    let mut buffer = [0_u8; 65536];
    for source in &sources {
        let name = source
            .strip_prefix(&input)
            .expect("files returns descendants")
            .to_str()
            .expect("files validates UTF-8 paths")
            .replace(std::path::MAIN_SEPARATOR, "/");
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        let mut output = output_file(stage, &format!("assets/{name}"))?;
        #[cfg(not(any(feature = "minify-css", feature = "minify-js")))]
        let _ = (minify, minify_assets, regression_checks);
        let optimized: Option<String> = match source.extension().and_then(|value| value.to_str()) {
            #[cfg(feature = "minify-css")]
            Some("css") if minify_assets && minify.css => {
                let text = read_text(source)?;
                let optimized = optimize_css(&text, &minify.css_options)
                    .with_context(|| format!("cannot optimize {name}"))?;
                if regression_checks != RegressionCheckMode::Off {
                    regression_checks.check(
                        check_regression(
                            &text,
                            &optimized,
                            minify.css_options.optimize,
                            |source| optimize_css(source, &CssOptions::conservative()),
                        ),
                        "CSS",
                        format_args!("{name}"),
                        "processing changed the non-optimizing representation",
                    )?;
                }
                Some(optimized)
            }
            #[cfg(feature = "minify-js")]
            Some(extension @ ("js" | "mjs")) if minify_assets && minify.js => {
                let text = read_text(source)?;
                let module = extension == "mjs";
                let optimized = optimize_js(&text, module, &minify.js_options)
                    .with_context(|| format!("cannot optimize {name}"))?;
                if regression_checks != RegressionCheckMode::Off {
                    let conservative = JsOptions {
                        source_type: minify.js_options.source_type,
                        ..JsOptions::conservative()
                    };
                    regression_checks.check(
                        check_regression(&text, &optimized, minify.js_options.compress, |source| {
                            optimize_js(source, module, &conservative)
                        }),
                        "JavaScript",
                        format_args!("{name}"),
                        "processing changed the non-optimizing representation",
                    )?;
                }
                Some(optimized)
            }
            _ => None,
        };
        if let Some(optimized) = optimized {
            digest.update((optimized.len() as u64).to_le_bytes());
            digest.update(optimized.as_bytes());
            output.write_all(optimized.as_bytes())?;
        } else {
            reject_symlinks(source)?;
            let mut input = File::open(source)?;
            let expected = input.metadata()?.len();
            copy_asset(
                &mut input,
                &mut output,
                expected,
                &mut digest,
                &mut buffer,
                source,
            )?;
        }
    }
    let hash = format!("{:x}", digest.finalize());
    let temporary = stage.join("asset-cache");
    fs::rename(stage.join("assets"), &temporary)?;
    fs::create_dir(stage.join("assets"))?;
    fs::rename(temporary, stage.join("assets").join(&hash))?;
    Ok(Assets {
        base: format!("/assets/{hash}"),
        count: sources.len(),
    })
}

/// Hash exactly the bytes written, rejecting a stale inspected source length.
///
/// Length checking detects truncation/growth, not same-length concurrent mutation;
/// keeping source files stable remains the caller's responsibility.
fn copy_asset(
    input: &mut impl Read,
    output: &mut impl Write,
    expected: u64,
    digest: &mut Sha256,
    buffer: &mut [u8; 65536],
    source: &Path,
) -> Result<()> {
    digest.update(expected.to_le_bytes());
    let mut copied = 0_u64;
    loop {
        let count = input.read(buffer)?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        digest.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    ensure!(
        copied == expected,
        "asset changed during build: {}",
        source.display()
    );
    Ok(())
}

/// Validate both sides through the native non-optimizing printer before comparison.
#[cfg(any(feature = "minify-css", feature = "minify-js"))]
fn check_regression(
    source: &str,
    output: &str,
    transformed: bool,
    canonicalize: impl Fn(&str) -> Result<String>,
) -> Result<bool> {
    let baseline = if transformed {
        std::borrow::Cow::Owned(canonicalize(source)?)
    } else {
        std::borrow::Cow::Borrowed(output)
    };
    let checked = canonicalize(output)?;
    Ok(baseline == checked)
}

#[cfg(feature = "minify-css")]
fn optimize_css(source: &str, options: &CssOptions) -> Result<String> {
    // No Bundler/FileProvider: @import and url() are preserved, never fetched/read.
    let mut stylesheet = StyleSheet::parse(source, ParserOptions::default()).map_err(css_error)?;
    let optimized = if options.optimize {
        stylesheet.minify(MinifyOptions {
            unused_symbols: options.unused_symbols.iter().cloned().collect(),
            ..MinifyOptions::default()
        })
    } else {
        Ok(())
    };
    optimized.context("CSS processing error").and_then(|()| {
        stylesheet
            .to_css(PrinterOptions {
                minify: true,
                ..PrinterOptions::default()
            })
            .map(|output| output.code)
            .map_err(css_error)
    })
}

/// Own the borrowed upstream diagnostic at the CSS-to-build error boundary.
#[cfg(feature = "minify-css")]
fn css_error(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("CSS processing error: {error}")
}

#[cfg(feature = "minify-js")]
fn optimize_js(source: &str, module: bool, options: &JsOptions) -> Result<String> {
    use oxc_allocator::Allocator;
    use oxc_ast::ast::{CommentContent, CommentPosition};
    use oxc_codegen::{Codegen, CodegenOptions, CommentOptions, LegalComment};
    use oxc_minifier::{
        CompressOptions, CompressOptionsKeepNames, CompressOptionsUnused, Compressor,
    };
    use oxc_parser::{ParseOptions, Parser};
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let source_type =
        if module || matches!(options.source_type, crate::config::JsSourceType::Module) {
            SourceType::mjs()
        } else {
            SourceType::cjs()
        };
    let parsed = Parser::new(&allocator, source, source_type)
        .with_options(ParseOptions {
            parse_regular_expression: true,
            ..Default::default()
        })
        .parse();
    if let Some(error) = parsed.errors.first() {
        anyhow::bail!("JavaScript processing error: {error}");
    }
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&program);
    if let Some(error) = semantic.errors.first() {
        anyhow::bail!("JavaScript processing error: {error}");
    }
    let scoping = semantic.semantic.into_scoping();
    if options.compress {
        // Other scripts can use these bindings. Only explicit removal may discard
        // them; never mangle names or assume property reads/global access are pure.
        let mut compress = CompressOptions {
            drop_debugger: options.drop_debugger,
            drop_console: options.drop_console,
            join_vars: options.join_vars,
            sequences: options.sequences,
            unused: if options.remove_unused {
                CompressOptionsUnused::Remove
            } else {
                CompressOptionsUnused::Keep
            },
            keep_names: if options.keep_names {
                CompressOptionsKeepNames::all_true()
            } else {
                CompressOptionsKeepNames::all_false()
            },
            ..CompressOptions::safest()
        };
        compress.treeshake.annotations = options.pure_annotations;
        Compressor::new(&allocator).build_with_scoping(&mut program, scoping, compress);
    }
    // Oxc's legal-comment predicate excludes trailing comments even at EOF.
    // Classify notices by content, not placement, before collecting them.
    for comment in &mut program.comments {
        if matches!(
            comment.content,
            CommentContent::Legal | CommentContent::JsdocLegal
        ) {
            comment.position = CommentPosition::Leading;
        }
    }
    Ok(Codegen::new()
        .with_options(CodegenOptions {
            minify: true,
            // EOF preservation includes legal comments after the last statement,
            // which the upstream inline placement policy otherwise drops.
            comments: CommentOptions {
                legal: LegalComment::Eof,
                normal: false,
                jsdoc: false,
                ..CommentOptions::default()
            },
            ..CodegenOptions::default()
        })
        .build(&program)
        .code)
}

#[cfg(test)]
#[path = "../tests/unit/assets.rs"]
mod tests;
