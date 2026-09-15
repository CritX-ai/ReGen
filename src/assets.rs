//! Optimized assets in a content-addressed directory.
//!
//! Keep relative asset paths together under one digest so CSS references survive
//! versioning. Only CSS is transformed; other bytes are streamed unchanged.
//! This module writes into a caller-owned stage and never installs live output.

use crate::files::{files, output_file, read_text, reject_symlinks};
use anyhow::{Context, Result, ensure};
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
pub(crate) fn prepare(root: &Path, stage: &Path) -> Result<Assets> {
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
        if source
            .extension()
            .is_some_and(|extension| extension == "css")
        {
            let css = optimize_css(&read_text(source)?)
                .with_context(|| format!("cannot optimize {name}"))?;
            digest.update((css.len() as u64).to_le_bytes());
            digest.update(css.as_bytes());
            output.write_all(css.as_bytes())?;
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

fn optimize_css(source: &str) -> Result<String> {
    // No Bundler/FileProvider: @import and url() are preserved, never fetched/read.
    let mut stylesheet = StyleSheet::parse(source, ParserOptions::default()).map_err(css_error)?;
    stylesheet
        .minify(MinifyOptions::default())
        .map_err(css_error)
        .and_then(|()| {
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
fn css_error(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("CSS processing error: {error}")
}

#[cfg(test)]
#[path = "../tests/unit/assets.rs"]
mod tests;
