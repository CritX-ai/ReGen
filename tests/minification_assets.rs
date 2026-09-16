//! Native minifier controls and atomic asset failures through the public build API.

#![cfg(any(feature = "minify-css", feature = "minify-js"))]

mod common;

use common::tempdir;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use walkdir::WalkDir;

fn configure(root: &Path, language: &str, options: &str) {
    fs::write(
        root.join("regen.toml"),
        format!(
            "[site]\ntitle = \"Asset controls\"\nbase_url = \"https://example.com\"\n\
             default_language = \"en\"\n[[languages]]\ncode = \"en\"\nname = \"English\"\n\
             [build.minify]\nhtml = false\n{language} = true\n\
             [build.minify.{language}_options]\n{options}\n"
        ),
    )
    .unwrap();
}

fn fixture(language: &str, source: &str) -> TempDir {
    let site = tempdir();
    for directory in ["content/en/pages", "templates", "assets"] {
        fs::create_dir_all(site.path().join(directory)).unwrap();
    }
    fs::write(site.path().join("content/en/site.yaml"), "{}\n").unwrap();
    fs::write(
        site.path().join("content/en/pages/index.yaml"),
        "title: Fixture\ndescription: Asset controls\ntemplate: page.html\nslug: \"\"\n",
    )
    .unwrap();
    fs::write(
        site.path().join("templates/page.html"),
        "<!doctype html><title>Fixture</title><main>Asset controls</main>",
    )
    .unwrap();
    fs::write(site.path().join(format!("assets/site.{language}")), source).unwrap();
    configure(site.path(), language, "");
    site
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    WalkDir::new(root)
        .into_iter()
        .map(Result::unwrap)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            (
                entry.path().strip_prefix(root).unwrap().to_path_buf(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

fn emitted_asset(root: &Path, name: &str) -> String {
    let asset = WalkDir::new(root.join("dist/assets"))
        .into_iter()
        .map(Result::unwrap)
        .find(|entry| entry.file_type().is_file() && entry.file_name() == name)
        .unwrap();
    fs::read_to_string(asset.path()).unwrap()
}

fn assert_failed_rebuild_preserves_output(root: &Path) {
    let previous = snapshot(&root.join("dist"));
    assert!(regen::build(root).is_err());
    assert_eq!(previous, snapshot(&root.join("dist")));
    assert!(!root.join(".regen-stage").exists());
    assert!(!root.join(".regen-previous").exists());
}

#[cfg(feature = "minify-css")]
#[test]
fn css_configuration_removes_only_listed_symbols_and_rejects_bad_rebuilds_atomically() {
    let site = fixture(
        "css",
        ".kept { color: red; color: blue; } .dead { opacity: 0; }\
         #unlisted { opacity: 1; }\
         @keyframes unlisted-motion { from { opacity: 0; } to { opacity: 1; } }",
    );
    regen::build_with_options(
        site.path(),
        &regen::BuildOptions {
            regression_checks: Some(regen::RegressionCheckMode::Enforce),
            ..Default::default()
        },
    )
    .unwrap();
    let compact = emitted_asset(site.path(), "site.css");
    assert!(compact.contains("color:red;color:#00f"));
    assert!(compact.contains(".dead"));

    configure(
        site.path(),
        "css",
        "optimize = true\nunused_symbols = [\"dead\"]",
    );
    assert_failed_rebuild_preserves_output(site.path());
    regen::build_with_options(
        site.path(),
        &regen::BuildOptions {
            regression_checks: Some(regen::RegressionCheckMode::Off),
            ..Default::default()
        },
    )
    .unwrap();
    let optimized = emitted_asset(site.path(), "site.css");
    assert!(optimized.contains(".kept"));
    assert!(!optimized.contains("color:red"));
    assert!(!optimized.contains(".dead"));
    assert!(optimized.contains("#unlisted"));
    assert!(optimized.contains("unlisted-motion"));

    fs::write(site.path().join("assets/site.css"), "}").unwrap();
    assert_failed_rebuild_preserves_output(site.path());
}

#[cfg(feature = "minify-js")]
#[test]
fn javascript_configuration_controls_compression_and_rejects_bad_rebuilds_atomically() {
    let site = fixture(
        "js",
        "/*! leading notice */ var publicAnswer = 6 * 7; debugger;\
         console.log(argumentEffect()); effectful(); //! @license trailing notice\n",
    );
    regen::build_with_options(
        site.path(),
        &regen::BuildOptions {
            regression_checks: Some(regen::RegressionCheckMode::Enforce),
            ..Default::default()
        },
    )
    .unwrap();
    let compact = emitted_asset(site.path(), "site.js");
    assert!(compact.contains("6*7"));
    assert!(compact.contains("debugger"));
    assert!(compact.contains("console.log(argumentEffect())"));

    configure(
        site.path(),
        "js",
        "compress = true\ndrop_debugger = true\ndrop_console = true",
    );
    assert_failed_rebuild_preserves_output(site.path());
    regen::build_with_options(
        site.path(),
        &regen::BuildOptions {
            regression_checks: Some(regen::RegressionCheckMode::Off),
            ..Default::default()
        },
    )
    .unwrap();
    let compressed = emitted_asset(site.path(), "site.js");
    assert!(compressed.contains("publicAnswer=42"));
    assert!(!compressed.contains("debugger"));
    assert!(!compressed.contains("console.log"));
    assert!(!compressed.contains("argumentEffect"));
    assert!(compressed.contains("effectful()"));
    for output in [&compact, &compressed] {
        assert!(output.contains("leading notice"));
        assert!(output.contains("trailing notice"));
    }

    fs::write(site.path().join("assets/site.js"), "export function {").unwrap();
    assert_failed_rebuild_preserves_output(site.path());
}

#[cfg(feature = "minify-js")]
#[test]
fn javascript_early_errors_and_malformed_regex_never_replace_valid_output() {
    let site = fixture("js", "let x; const expression = /valid/u;");
    regen::build(site.path()).unwrap();
    for source in ["let x; let x;", "const expression = /(/;"] {
        fs::write(site.path().join("assets/site.js"), source).unwrap();
        for compress in [false, true] {
            configure(site.path(), "js", &format!("compress = {compress}"));
            assert_failed_rebuild_preserves_output(site.path());
        }
    }
}

fn warning_build(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_regen"))
        .args(["build", "--regression-checks", "warn", "--site"])
        .arg(root)
        .output()
        .unwrap()
}

fn assert_warning_assets(
    language: &str,
    source: &str,
    options: &str,
    retained: &str,
    removed: &str,
    invalid: &str,
) {
    let site = fixture(language, source);
    let root = site.path();
    let name = format!("site.{language}");
    let second_name = format!("second.{language}");
    fs::write(root.join("assets").join(&second_name), source).unwrap();
    let unchanged = warning_build(root);
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert!(
        !String::from_utf8(unchanged.stderr)
            .unwrap()
            .contains("warning:")
    );
    assert!(emitted_asset(root, &name).contains(removed));
    configure(root, language, options);
    let warned = warning_build(root);
    assert!(warned.status.success(), "{warned:?}");
    let stderr = String::from_utf8(warned.stderr).unwrap();
    for asset in [&name, &second_name] {
        assert!(
            stderr
                .lines()
                .any(|line| line.starts_with("warning:") && line.contains(asset)),
            "{stderr}"
        );
        let optimized = emitted_asset(root, asset);
        assert!(optimized.contains(retained), "{optimized}");
        assert!(!optimized.contains(removed), "{optimized}");
    }
    // The policy applies to comparisons, not native parser or UTF-8 failures.
    let previous = snapshot(&root.join("dist"));
    for broken in [invalid.as_bytes(), &[0xff]] {
        fs::write(root.join("assets").join(&name), broken).unwrap();
        let rejected = warning_build(root);
        assert!(!rejected.status.success());
        assert_eq!(previous, snapshot(&root.join("dist")));
        assert!(!root.join(".regen-stage").exists());
        assert!(!root.join(".regen-previous").exists());
    }
}

#[cfg(feature = "minify-css")]
#[test]
fn css_warning_checks_report_each_asset_and_install_optimized_output() {
    assert_warning_assets(
        "css",
        ".kept { color: red; } .dead { opacity: 0; }",
        "optimize = true\nunused_symbols = [\"dead\"]",
        ".kept",
        ".dead",
        "}",
    );
}

#[cfg(feature = "minify-js")]
#[test]
fn javascript_warning_checks_report_each_asset_and_install_optimized_output() {
    assert_warning_assets(
        "js",
        "debugger; effectful();",
        "compress = true\ndrop_debugger = true",
        "effectful()",
        "debugger",
        "export function {",
    );
}
