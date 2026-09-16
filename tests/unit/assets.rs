//! Asset-stream and staging failure contracts below the full-build boundary.
//!
//! Real handles, bounded writers, and occupied directories exercise incomplete I/O
//! and failed renames. Digest assertions defend the URL's byte identity, not merely
//! successful copying; a failed stage remains the transaction owner's responsibility.

use super::copy_asset;
#[cfg(feature = "minify-css")]
use crate::config::CssOptions;
use crate::config::RegressionCheckMode;
#[cfg(feature = "minify-js")]
use crate::config::{JsOptions, JsSourceType};
use crate::test_support::tempdir;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::Path;

const MINIFY: crate::config::Minify = crate::config::Minify {
    html: cfg!(feature = "minify-html"),
    css: cfg!(feature = "minify-css"),
    js: cfg!(feature = "minify-js"),
    html_options: crate::config::HtmlOptions::conservative(),
    css_options: crate::config::CssOptions::conservative(),
    js_options: crate::config::JsOptions::conservative(),
};

#[cfg(feature = "minify-css")]
#[test]
fn css_regression_guard_rejects_invalid_source_and_invalid_minifier_output() {
    let check = |source, output| {
        super::check_regression(source, output, true, |text| {
            super::optimize_css(text, &CssOptions::conservative())
        })
    };
    assert!(check("a { color: red; }", "a{color:#ff0000}").unwrap());
    assert!(check("}", "a{}").is_err());
    let error = RegressionCheckMode::Warn
        .check(
            check("a{}", "}"),
            "CSS",
            format_args!("broken.css"),
            "processing changed the non-optimizing representation",
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("broken.css"));
}

#[cfg(feature = "minify-js")]
#[test]
fn javascript_regression_guard_rejects_invalid_source_and_invalid_minifier_output() {
    let check = |source, output| {
        super::check_regression(source, output, true, |text| {
            super::optimize_js(text, false, &JsOptions::conservative())
        })
    };
    assert!(check("const answer = 1;", "const answer=1").unwrap());
    assert!(check("let x; let x;", "let x;").is_err());
    let error = RegressionCheckMode::Warn
        .check(
            check("const expression = /valid/;", "const expression = /(/;"),
            "JavaScript",
            format_args!("broken.js"),
            "processing changed the non-optimizing representation",
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("broken.js"));
}

#[test]
fn asset_copy_hashes_the_complete_length_prefixed_stream_across_buffer_boundaries() {
    // Span the fixed buffer boundary with a pattern that exposes lost or repeated chunks.
    let source: Vec<_> = (0..65539).map(|index| (index % 251) as u8).collect();
    let temp = tempdir();
    let path = temp.path().join("asset.bin");
    let destination = temp.path().join("copied.bin");
    fs::write(&path, &source).unwrap();
    let mut input = fs::File::open(&path).unwrap();
    let mut output = fs::File::create(&destination).unwrap();
    let mut digest = Sha256::new();
    copy_asset(
        &mut input,
        &mut output,
        source.len() as u64,
        &mut digest,
        &mut [0; 65536],
        &path,
    )
    .unwrap();
    drop(output);
    assert_eq!(fs::read(destination).unwrap(), source);
    let mut expected = Sha256::new();
    expected.update((source.len() as u64).to_le_bytes());
    expected.update(&source);
    assert_eq!(digest.finalize(), expected.finalize());
}

#[test]
fn assets_that_shrink_or_grow_after_inspection_cannot_publish_a_stale_length_hash() {
    for changed in [b"s".as_slice(), b"source grew".as_slice()] {
        let temp = tempdir();
        let path = temp.path().join("asset.bin");
        fs::write(&path, b"source").unwrap();
        let mut input = fs::File::open(&path).unwrap();
        let inspected = input.metadata().unwrap().len();
        fs::write(&path, changed).unwrap();
        let destination = temp.path().join("copied.bin");
        let mut output = fs::File::create(&destination).unwrap();
        assert!(
            copy_asset(
                &mut input,
                &mut output,
                inspected,
                &mut Sha256::new(),
                &mut [0; 65536],
                &path,
            )
            .is_err()
        );
        drop(output);
        assert_eq!(fs::read(destination).unwrap(), changed);
        assert_eq!(fs::read(&path).unwrap(), changed);
    }
}

#[test]
fn asset_read_failure_preserves_the_io_cause_without_emitting_bytes() {
    // An unreadable source must not be mistaken for a valid empty asset.
    let temp = tempdir();
    let path = temp.path().join("write-only.bin");
    fs::write(&path, b"source").unwrap();
    let mut input = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let destination = temp.path().join("copied.bin");
    let mut output = fs::File::create(&destination).unwrap();
    let error = copy_asset(
        &mut input,
        &mut output,
        6,
        &mut Sha256::new(),
        &mut [0; 65536],
        &path,
    )
    .unwrap_err();
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    drop(output);
    assert_eq!(fs::read(destination).unwrap(), b"");
    assert_eq!(fs::read(&path).unwrap(), b"source");
}

#[test]
fn asset_write_failure_reports_capacity_exhaustion_instead_of_a_valid_hash() {
    // A matching input length cannot make a partially written asset valid.
    let mut output = [0; 3];
    let error = copy_asset(
        &mut Cursor::new(b"asset"),
        &mut output.as_mut_slice(),
        5,
        &mut Sha256::new(),
        &mut [0; 65536],
        Path::new("asset.bin"),
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::WriteZero
    );
    assert_eq!(&output, b"ass");
}

#[test]
fn asset_copy_cannot_report_success_when_the_destination_handle_is_read_only() {
    let temp = tempdir();
    let path = temp.path().join("asset.bin");
    let destination = temp.path().join("copied.bin");
    fs::write(&path, b"asset").unwrap();
    fs::write(&destination, b"existing destination").unwrap();
    let mut input = fs::File::open(&path).unwrap();
    let mut output = fs::File::open(&destination).unwrap();
    let error = copy_asset(
        &mut input,
        &mut output,
        5,
        &mut Sha256::new(),
        &mut [0; 65536],
        &path,
    )
    .unwrap_err();
    assert!(
        error
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error()
            .is_some()
    );
    assert_eq!(fs::read(destination).unwrap(), b"existing destination");
    assert_eq!(fs::read(path).unwrap(), b"asset");
}

#[cfg(feature = "minify-css")]
#[test]
fn equivalent_css_spelling_keeps_the_asset_url_and_relative_binary_references() {
    let root = tempdir();
    let first_stage = tempdir();
    let second_stage = tempdir();
    fs::create_dir_all(root.path().join("assets/css")).unwrap();
    fs::create_dir_all(root.path().join("assets/images")).unwrap();
    let stylesheet = root.path().join("assets/css/site.css");
    fs::write(
        &stylesheet,
        "body { color: #ff0000; background-image: url(../images/pixel.bin); }",
    )
    .unwrap();
    let binary = [0, 255, 13, 10];
    fs::write(root.path().join("assets/images/pixel.bin"), binary).unwrap();
    let first = super::prepare(
        root.path(),
        first_stage.path(),
        &MINIFY,
        true,
        RegressionCheckMode::Off,
    )
    .unwrap();
    let relative = first.base.trim_start_matches('/');
    let optimized = fs::read(first_stage.path().join(relative).join("css/site.css")).unwrap();
    assert_eq!(
        fs::read(first_stage.path().join(relative).join("images/pixel.bin")).unwrap(),
        binary
    );
    assert!(
        std::str::from_utf8(&optimized)
            .unwrap()
            .contains("../images/pixel.bin")
    );
    fs::write(
        &stylesheet,
        "body{color:red;background-image:url(../images/pixel.bin)}",
    )
    .unwrap();
    let second = super::prepare(
        root.path(),
        second_stage.path(),
        &MINIFY,
        true,
        RegressionCheckMode::Off,
    )
    .unwrap();
    assert_eq!(first.base, second.base);
    assert_eq!(first.count, 2);
    assert_eq!(second.count, 2);
    assert_eq!(
        fs::read(second_stage.path().join(relative).join("css/site.css")).unwrap(),
        optimized
    );
}

#[test]
fn an_existing_asset_directory_is_not_reused_or_overwritten() {
    let root = tempdir();
    let stage = tempdir();
    fs::create_dir(stage.path().join("assets")).unwrap();
    fs::write(stage.path().join("assets/keep.bin"), b"other writer").unwrap();
    let error = super::prepare(
        root.path(),
        stage.path(),
        &MINIFY,
        true,
        RegressionCheckMode::Off,
    )
    .err()
    .unwrap();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        fs::read(stage.path().join("assets/keep.bin")).unwrap(),
        b"other writer"
    );
}

#[test]
fn an_occupied_cache_destination_cannot_destroy_staged_assets_or_recovery_data() {
    let root = tempdir();
    let stage = tempdir();
    fs::create_dir(root.path().join("assets")).unwrap();
    fs::write(root.path().join("assets/source.bin"), b"source").unwrap();
    fs::create_dir(stage.path().join("asset-cache")).unwrap();
    fs::write(stage.path().join("asset-cache/keep.bin"), b"recoverable").unwrap();
    let error = super::prepare(
        root.path(),
        stage.path(),
        &MINIFY,
        true,
        RegressionCheckMode::Off,
    )
    .err()
    .unwrap();
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(
        fs::read(stage.path().join("asset-cache/keep.bin")).unwrap(),
        b"recoverable"
    );
    assert_eq!(
        fs::read(stage.path().join("assets/source.bin")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(root.path().join("assets/source.bin")).unwrap(),
        b"source"
    );
}

#[cfg(feature = "minify-css")]
#[test]
fn compact_css_preserves_duplicate_declarations_and_rule_order_without_optimization() {
    let source = ".card { color: red; color: blue; } \
                  .other { color: green; } .card { margin-top: 1px; }";
    let compact = super::optimize_css(source, &CssOptions::conservative()).unwrap();
    assert!(compact.contains("color:red;color:#00f"));
    assert_eq!(compact.matches(".card{").count(), 2);
    assert!(compact.find(".card{").unwrap() < compact.find(".other{").unwrap());
    assert!(compact.find(".other{").unwrap() < compact.rfind(".card{").unwrap());

    let optimized = super::optimize_css(
        source,
        &CssOptions {
            optimize: true,
            ..CssOptions::conservative()
        },
    )
    .unwrap();
    assert!(!optimized.contains("color:red"));
    assert!(optimized.contains("color:#00f"));
    assert!(optimized.contains("margin-top:1px"));
}

#[cfg(feature = "minify-css")]
#[test]
fn css_liveness_removal_requires_explicit_symbols_and_keeps_unlisted_rules() {
    let source = ".live{color:red}.dead{color:blue}#dead-id{color:green}\
                  @keyframes dead-motion{from{opacity:0}to{opacity:1}}\
                  @keyframes live-motion{from{opacity:0}to{opacity:1}}";
    let mut options = CssOptions {
        optimize: true,
        ..CssOptions::conservative()
    };
    let retained = super::optimize_css(source, &options).unwrap();
    for symbol in [".live", ".dead", "#dead-id", "dead-motion", "live-motion"] {
        assert!(retained.contains(symbol), "lost unlisted symbol {symbol}");
    }
    options.unused_symbols = ["dead", "dead-id", "dead-motion"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let removed = super::optimize_css(source, &options).unwrap();
    for symbol in [".dead", "#dead-id", "dead-motion"] {
        assert!(!removed.contains(symbol), "retained unused symbol {symbol}");
    }
    assert!(removed.contains(".live"));
    assert!(removed.contains("live-motion"));
}

#[cfg(feature = "minify-js")]
#[test]
fn javascript_compaction_does_not_enable_compression_or_drop_public_names() {
    let source = "var publicValue = 6 * 7;\
                  function publicEntry(value) { return arguments.length + value; }\
                  var publicFunction = function NamedFunction() {};\
                  var publicClass = class NamedClass {};";
    let compact = super::optimize_js(source, false, &JsOptions::conservative()).unwrap();
    assert!(compact.contains("6*7"));
    let compressed = super::optimize_js(
        source,
        false,
        &JsOptions {
            compress: true,
            ..JsOptions::conservative()
        },
    )
    .unwrap();
    assert!(compressed.contains("publicValue=42"));
    for output in [&compact, &compressed] {
        for name in [
            "publicValue",
            "publicEntry",
            "arguments.length",
            "publicFunction",
            "NamedFunction",
            "publicClass",
            "NamedClass",
        ] {
            assert!(output.contains(name), "lost public binding/name {name}");
        }
    }
}

#[cfg(feature = "minify-js")]
#[test]
fn debugger_console_and_pure_annotations_are_independent_opt_ins() {
    let source = "debugger; console.log(consoleArgument()); effectful();\
                  object.value; unknownGlobal;\
                  /* @__PURE__ */ annotated(annotationArgument());";
    let baseline = JsOptions {
        compress: true,
        remove_unused: true,
        ..JsOptions::conservative()
    };
    for options in [JsOptions::conservative(), baseline.clone()] {
        let output = super::optimize_js(source, false, &options).unwrap();
        for effect in [
            "debugger",
            "console.log",
            "consoleArgument()",
            "effectful()",
            "object.value",
            "unknownGlobal",
            "annotated(",
            "annotationArgument()",
        ] {
            assert!(output.contains(effect), "lost effect {effect}");
        }
    }
    let no_debugger = super::optimize_js(
        source,
        false,
        &JsOptions {
            drop_debugger: true,
            ..baseline.clone()
        },
    )
    .unwrap();
    assert!(!no_debugger.contains("debugger"));
    assert!(no_debugger.contains("console.log(consoleArgument())"));
    assert!(no_debugger.contains("annotated(annotationArgument())"));

    let no_console = super::optimize_js(
        source,
        false,
        &JsOptions {
            drop_console: true,
            ..baseline.clone()
        },
    )
    .unwrap();
    assert!(!no_console.contains("console.log"));
    // The upstream opt-in removes console arguments too, not merely the logging.
    assert!(!no_console.contains("consoleArgument"));
    assert!(no_console.contains("debugger"));
    assert!(no_console.contains("effectful()"));
    assert!(no_console.contains("annotated(annotationArgument())"));

    let annotated = super::optimize_js(
        source,
        false,
        &JsOptions {
            pure_annotations: true,
            ..baseline
        },
    )
    .unwrap();
    assert!(!annotated.contains("annotated("));
    assert!(annotated.contains("annotationArgument()"));
    assert!(annotated.contains("console.log(consoleArgument())"));
    assert!(annotated.contains("debugger"));
    assert!(annotated.contains("effectful()"));
    assert!(annotated.contains("object.value"));
    assert!(annotated.contains("unknownGlobal"));
}

#[cfg(feature = "minify-js")]
#[test]
fn variable_joining_and_expression_sequences_are_separately_controlled() {
    let source = "var first = readFirst(); var second = readSecond();\
                  emitFirst(); emitSecond();";
    let baseline = JsOptions {
        compress: true,
        ..JsOptions::conservative()
    };
    let separate = super::optimize_js(source, false, &baseline).unwrap();
    assert!(separate.contains("var first=readFirst();var second=readSecond()"));
    assert!(separate.contains("emitFirst();emitSecond()"));

    let joined = super::optimize_js(
        source,
        false,
        &JsOptions {
            join_vars: true,
            ..baseline.clone()
        },
    )
    .unwrap();
    assert!(joined.contains("var first=readFirst(),second=readSecond()"));
    assert!(joined.contains("emitFirst();emitSecond()"));

    let sequenced = super::optimize_js(
        source,
        false,
        &JsOptions {
            sequences: true,
            ..baseline
        },
    )
    .unwrap();
    assert!(sequenced.contains("var first=readFirst();var second=readSecond()"));
    assert!(sequenced.contains("emitFirst(),emitSecond()"));
}

#[cfg(feature = "minify-js")]
#[test]
fn unused_module_bindings_require_opt_in_and_effectful_initializers_survive() {
    let source = "const unusedValue = 42; function unusedFunction() {}\
                  const unusedEffect = effectful(); export const published = 7;";
    let mut options = JsOptions {
        compress: true,
        ..JsOptions::conservative()
    };
    let retained = super::optimize_js(source, true, &options).unwrap();
    for name in ["unusedValue", "unusedFunction", "unusedEffect", "published"] {
        assert!(retained.contains(name));
    }
    options.remove_unused = true;
    let removed = super::optimize_js(source, true, &options).unwrap();
    for name in ["unusedValue", "unusedFunction", "unusedEffect"] {
        assert!(!removed.contains(name));
    }
    assert!(removed.contains("effectful()"));
    assert!(removed.contains("export"));
    assert!(removed.contains("published"));
}

#[cfg(feature = "minify-js")]
#[test]
fn relaxing_name_retention_does_not_mangle_public_bindings() {
    let source = "var publicFunction = function NamedFunction() {};\
                  var publicClass = class NamedClass {};";
    let output = super::optimize_js(
        source,
        false,
        &JsOptions {
            compress: true,
            keep_names: false,
            ..JsOptions::conservative()
        },
    )
    .unwrap();
    assert!(output.contains("publicFunction"));
    assert!(output.contains("publicClass"));
    assert!(!output.contains("NamedFunction"));
    assert!(!output.contains("NamedClass"));
}

#[cfg(feature = "minify-js")]
#[test]
fn legal_notices_survive_all_positions_even_when_their_statements_are_removed() {
    let source = "/*! leading notice */ const unusedValue = 1;\
                  function unusedFunction() { /** @license body notice */ return 1; }\
                  /* @preserve middle notice */ const anotherUnused = 2;\
                  //! @license trailing notice\n";
    for options in [
        JsOptions::conservative(),
        JsOptions {
            compress: true,
            ..JsOptions::conservative()
        },
        JsOptions {
            compress: true,
            remove_unused: true,
            ..JsOptions::conservative()
        },
    ] {
        let output = super::optimize_js(source, true, &options).unwrap();
        for notice in [
            "leading notice",
            "body notice",
            "middle notice",
            "trailing notice",
        ] {
            assert!(output.contains(notice), "lost {notice}");
        }
    }
}

#[cfg(feature = "minify-js")]
#[test]
fn javascript_grammar_is_explicit_and_mjs_always_uses_module_mode() {
    for compress in [false, true] {
        let script_options = JsOptions {
            compress,
            source_type: JsSourceType::Script,
            ..JsOptions::conservative()
        };
        let module_options = JsOptions {
            source_type: JsSourceType::Module,
            ..script_options.clone()
        };
        let classic = "var await = 1; observe(await);";
        let output = super::optimize_js(classic, false, &script_options).unwrap();
        assert!(output.contains("await=1"));
        assert!(output.contains("observe(await)"));
        assert!(super::optimize_js(classic, false, &module_options).is_err());
        assert!(super::optimize_js(classic, true, &script_options).is_err());

        let module = "export const response = await fetch('./data.json');";
        assert!(super::optimize_js(module, false, &script_options).is_err());
        for (is_mjs, options) in [(false, &module_options), (true, &script_options)] {
            let output = super::optimize_js(module, is_mjs, options).unwrap();
            assert!(output.contains("export"));
            assert!(output.contains("response"));
            assert!(output.contains("await"));
            assert!(output.contains("./data.json"));
        }
    }
}

#[cfg(feature = "minify-js")]
#[test]
fn ambiguous_await_keeps_the_selected_grammar_without_parser_fallback() {
    use oxc_allocator::Allocator;
    use oxc_ast::ast::{Expression, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let source = "await\nPromise.resolve();";
    for (source_type, grammar) in [
        (JsSourceType::Script, SourceType::cjs()),
        (JsSourceType::Module, SourceType::mjs()),
    ] {
        let output = super::optimize_js(
            source,
            false,
            &JsOptions {
                source_type,
                ..JsOptions::conservative()
            },
        )
        .unwrap();
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, &output, grammar).parse();
        assert!(parsed.errors.is_empty(), "invalid output: {output}");
        let Statement::ExpressionStatement(first) = &parsed.program.body[0] else {
            panic!("expected an expression: {output}");
        };
        match source_type {
            JsSourceType::Script => {
                assert_eq!(parsed.program.body.len(), 2);
                assert!(
                    matches!(&first.expression, Expression::Identifier(id) if id.name == "await")
                );
            }
            JsSourceType::Module => {
                assert_eq!(parsed.program.body.len(), 1);
                assert!(matches!(&first.expression, Expression::AwaitExpression(_)));
            }
        }
    }
}
