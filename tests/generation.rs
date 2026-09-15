//! Consumer-level contracts exercised through real site builds and the CLI.
//!
//! Temporary copies of the shipped example connect configuration, translation
//! identity, rendering, asset versioning, and output ownership. Byte snapshots
//! detect nondeterminism and partial replacement; targeted unit suites isolate
//! recovery states and I/O boundaries that complete builds cannot reliably reach.
//! Platform-specific cases use real filesystem behavior rather than mocked success.

mod common;

use common::tempdir;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use walkdir::WalkDir;

/// Copy authored example inputs, excluding old output, and add translated subroutes.
fn fixture() -> TempDir {
    let temp = tempdir();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/minimal");
    for entry in WalkDir::new(&source).into_iter().map(Result::unwrap) {
        let relative = entry.path().strip_prefix(&source).unwrap();
        if matches!(relative.components().next(), Some(part) if part.as_os_str() == "dist") {
            continue;
        }
        let destination = temp.path().join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&destination).unwrap();
        } else {
            fs::copy(entry.path(), &destination).unwrap();
        }
    }
    // Extra routes belong to regression fixtures, not the shipped two-page poem.
    for (language, slug) in [("en", "about"), ("de", "ueber")] {
        let pages = temp.path().join("content").join(language).join("pages");
        let index = fs::read_to_string(pages.join("index.yaml")).unwrap();
        assert!(index.contains("slug: \"\""));
        fs::write(
            pages.join("about.yaml"),
            index.replace("slug: \"\"", &format!("slug: {slug}")),
        )
        .unwrap();
    }
    temp
}

/// Capture relative file names and bytes, ignoring timestamps and traversal order.
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

/// Mutate a known fixture spelling; fail if drift would leave the intended risk untested.
fn replace(root: &Path, file: &str, before: &str, after: &str) {
    let path = root.join(file);
    let source = fs::read_to_string(&path).unwrap();
    assert!(
        source.contains(before),
        "fixture must exercise the intended case"
    );
    fs::write(path, source.replace(before, after)).unwrap();
}

/// Assert pre-installation rejection preserves all output bytes and leaves no recovery work.
fn assert_rejected_without_replacement(root: &Path, previous: &BTreeMap<PathBuf, Vec<u8>>) {
    assert!(regen::build(root).is_err(), "invalid fixture was accepted");
    assert_eq!(*previous, snapshot(&root.join("dist")));
    assert!(!root.join(".regen-stage").exists());
    assert!(!root.join(".regen-previous").exists());
}

#[test]
fn cli_builds_from_default_and_explicit_sites_and_preserves_output_on_failure() {
    let site = fixture();
    let caller = tempdir();
    let binary = env!("CARGO_BIN_EXE_regen");
    let built = Command::new(binary)
        .arg("build")
        .current_dir(site.path())
        .output()
        .unwrap();
    assert!(built.status.success(), "{:?}", built);
    let output = site.path().join("dist");
    assert!(output.join("index.html").is_file());
    assert!(output.join("de/ueber/index.html").is_file());
    let first = snapshot(&output);

    let rebuilt = Command::new(binary)
        .args(["build", "--site"])
        .arg(site.path())
        .current_dir(caller.path())
        .output()
        .unwrap();
    assert!(rebuilt.status.success(), "{:?}", rebuilt);
    assert_eq!(first, snapshot(&output));
    assert!(!caller.path().join("dist").exists());

    fs::write(site.path().join("regen.toml"), "[site").unwrap();
    let failed = Command::new(binary)
        .args(["build", "--site"])
        .arg(site.path())
        .current_dir(caller.path())
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert_eq!(first, snapshot(&output));
    assert!(!site.path().join(".regen-stage").exists());
}

#[test]
fn language_subtags_produce_localized_routes_and_repeated_variants_are_rejected() {
    let site = fixture();
    for (old, new) in [
        ("en", "zh-cmn-gan-nan-hans-cn-fonipa"),
        ("de", "de-latn-de-1901"),
    ] {
        replace(
            site.path(),
            "regen.toml",
            &format!("\"{old}\""),
            &format!("\"{new}\""),
        );
        fs::rename(
            site.path().join("content").join(old),
            site.path().join("content").join(new),
        )
        .unwrap();
    }
    regen::build(site.path()).unwrap();
    let output = site.path().join("dist");
    assert!(output.join("about/index.html").is_file());
    assert!(output.join("de-latn-de-1901/ueber/index.html").is_file());
    assert!(!output.join("zh-cmn-gan-nan-hans-cn-fonipa").exists());
    let sitemap = fs::read_to_string(output.join("sitemap.xml")).unwrap();
    assert!(sitemap.contains("hreflang=\"zh-cmn-gan-nan-hans-cn-fonipa\""));
    assert!(sitemap.contains("https://example.com/de-latn-de-1901/ueber/"));
    let first = snapshot(&output);

    replace(
        site.path(),
        "regen.toml",
        "de-latn-de-1901",
        "de-latn-de-1901-1901",
    );
    fs::rename(
        site.path().join("content/de-latn-de-1901"),
        site.path().join("content/de-latn-de-1901-1901"),
    )
    .unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&output));
}

#[test]
fn invalid_language_configuration_preserves_the_previous_site() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let first = snapshot(&site.path().join("dist"));
    let config = fs::read_to_string(site.path().join("regen.toml")).unwrap();
    for invalid in [
        format!(
            "languages = []\n{}",
            config.split("[[languages]]").next().unwrap()
        ),
        config.replace("default_language = \"en\"", "default_language = \"fr\""),
        config.replace("code = \"de\"", "code = \"en\""),
        config.replace("code = \"de\"", "code = \"assets\""),
        config.replace("code = \"de\"", "code = \"en_US\""),
        config.replace("code = \"de\"", "code = \"e\""),
        config.replace("code = \"de\"", "code = \"en-u-ca\""),
        config.replace("code = \"de\"", "code = \"zh-cmn-gan-nan-wuu\""),
        config.replace("code = \"de\"", "code = \"qaaa-cmn\""),
        format!("unknown = true\n{config}"),
        config.replace(
            "name = \"Deutsch\"",
            "name = \"Deutsch\"\ndirection = \"vertical\"",
        ),
    ] {
        fs::write(site.path().join("regen.toml"), invalid).unwrap();
        assert!(regen::build(site.path()).is_err());
        assert_eq!(first, snapshot(&site.path().join("dist")));
        assert!(!site.path().join(".regen-stage").exists());
    }
}

#[test]
fn nested_translation_ids_must_match_even_when_page_counts_match() {
    // Source paths pair translations; moving both sources must not change their URLs.
    let site = fixture();
    for language in ["en", "de"] {
        let pages = site.path().join("content").join(language).join("pages");
        fs::create_dir(pages.join("guide")).unwrap();
        fs::rename(pages.join("about.yaml"), pages.join("guide/about.yaml")).unwrap();
    }
    regen::build(site.path()).unwrap();
    let output = site.path().join("dist");
    assert!(output.join("about/index.html").is_file());
    assert!(output.join("de/ueber/index.html").is_file());
    assert!(!output.join("guide").exists());
    let first = snapshot(&output);
    let translated = site.path().join("content/de/pages/guide");
    fs::rename(translated.join("about.yaml"), translated.join("other.yaml")).unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&output));

    fs::rename(translated.join("other.yaml"), translated.join("about.yaml")).unwrap();
    replace(
        site.path(),
        "content/de/pages/guide/about.yaml",
        "slug: ueber",
        "slug: \"\"",
    );
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&output));
}

#[test]
fn empty_unknown_locales_and_missing_site_content_are_not_silently_ignored() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let first = snapshot(&site.path().join("dist"));
    fs::create_dir(site.path().join("content/fr")).unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&site.path().join("dist")));

    fs::remove_dir(site.path().join("content/fr")).unwrap();
    fs::remove_file(site.path().join("content/de/site.yaml")).unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&site.path().join("dist")));
}

#[test]
fn missing_optional_asset_trees_build_and_invalid_css_preserves_that_build() {
    let site = fixture();
    fs::remove_dir_all(site.path().join("assets")).unwrap();
    fs::remove_dir_all(site.path().join("public")).unwrap();
    let built = regen::build(site.path()).unwrap();
    assert_eq!(built.assets, 0);
    let output = site.path().join("dist");
    assert!(output.join("index.html").is_file());
    assert!(output.join("de/ueber/index.html").is_file());
    let first = snapshot(&output);

    fs::create_dir(site.path().join("assets")).unwrap();
    fs::write(site.path().join("assets/broken.css"), "}").unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&output));
    assert!(!site.path().join(".regen-stage").exists());
}

#[test]
fn interrupted_backup_requires_manual_recovery_and_unknown_ownership_is_preserved() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let output = site.path().join("dist");
    let previous = site.path().join(".regen-previous");
    let first = snapshot(&output);
    fs::rename(&output, &previous).unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&previous));
    assert!(!output.exists());
    assert!(!site.path().join(".regen-stage").exists());

    fs::rename(&previous, &output).unwrap();
    regen::build(site.path()).unwrap();
    assert_eq!(first, snapshot(&output));
    assert!(!previous.exists());

    fs::write(
        output.join("regen-manifest.json"),
        r#"{"generator":"ReGen","format":2}"#,
    )
    .unwrap();
    let unrecognized = snapshot(&output);
    assert!(regen::build(site.path()).is_err());
    assert_eq!(unrecognized, snapshot(&output));
    assert!(!site.path().join(".regen-stage").exists());
}

#[test]
fn repeated_builds_are_byte_identical_and_localized_routes_match() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let output = site.path().join("dist");
    let first = snapshot(&output);
    regen::build(site.path()).unwrap();
    assert_eq!(first, snapshot(&output));
    for route in [
        "index.html",
        "about/index.html",
        "de/index.html",
        "de/ueber/index.html",
    ] {
        assert!(
            output.join(route).is_file(),
            "missing localized route {route}"
        );
    }
    let about = fs::read_to_string(output.join("about/index.html")).unwrap();
    assert!(about.contains("https://example.com/de/ueber/"));
    assert!(about.contains("https://example.com/about/"));
    let sitemap = fs::read_to_string(output.join("sitemap.xml")).unwrap();
    assert!(sitemap.contains("https://example.com/de/ueber/"));
    assert!(!site.path().join(".regen-stage").exists());
}

#[test]
fn project_prefix_changes_public_urls_but_not_output_locations() {
    let site = fixture();
    replace(
        site.path(),
        "regen.toml",
        "https://example.com",
        "https://example.com/ReGen/",
    );
    replace(
        site.path(),
        "templates/page.html",
        "{% endblock %}",
        "<p>{{ site_root }}|{% for item in navigation %}{{ item.id }}={{ item.path }},{{ item.url }};{% endfor %}</p>{% endblock %}",
    );
    regen::build(site.path()).unwrap();
    let output = site.path().join("dist");
    assert!(output.join("index.html").is_file());
    assert!(output.join("de/ueber/index.html").is_file());
    assert!(!output.join("ReGen").exists());
    let html = fs::read_to_string(output.join("about/index.html")).unwrap();
    assert!(html.contains("https://example.com/ReGen/about/"));
    assert!(html.contains("https://example.com/ReGen/de/ueber/"));
    assert!(html.contains("/ReGen/assets/"));
    assert!(!html.contains("/ReGen/ReGen/"));
    assert!(html.contains("/ReGen/|about=/ReGen/about/,https://example.com/ReGen/about/;index=/ReGen/,https://example.com/ReGen/;"));
    let german = fs::read_to_string(output.join("de/ueber/index.html")).unwrap();
    assert!(german.contains("/ReGen/|about=/ReGen/de/ueber/,https://example.com/ReGen/de/ueber/;index=/ReGen/de/,https://example.com/ReGen/de/;"));
    let sitemap = fs::read_to_string(output.join("sitemap.xml")).unwrap();
    assert!(sitemap.contains("https://example.com/ReGen/de/ueber/"));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("regen-manifest.json")).unwrap()).unwrap();
    assert!(
        manifest["files"]
            .as_object()
            .unwrap()
            .keys()
            .all(|name| !name.starts_with("ReGen/"))
    );
    let first = snapshot(&output);
    // A project prefix without a trailing slash must identify the same site,
    // not lose its last character or introduce different public URLs.
    replace(
        site.path(),
        "regen.toml",
        "https://example.com/ReGen/",
        "https://example.com/ReGen",
    );
    regen::build(site.path()).unwrap();
    assert_eq!(first, snapshot(&output));
}

#[test]
fn unsafe_url_prefixes_are_rejected_before_normalization_or_output() {
    for base_url in [
        "https://example.com/ReGen/../outside/",
        "https://example.com/ReGen/%2e%2e/outside/",
        "https://example.com//ReGen/",
        "https://example.com/ReGen/?query=1",
        "https://example.com/ReGen/#fragment",
        "https://user:password@example.com/ReGen/",
        "example.com",
        "ftp://example.com",
        "https://example.com ",
        "https:///prefix",
        "https://[invalid",
        "https://example.com//",
        "https://example.com/./prefix",
    ] {
        let site = fixture();
        replace(site.path(), "regen.toml", "https://example.com", base_url);
        assert!(
            regen::build(site.path()).is_err(),
            "accepted unsafe base URL {base_url}"
        );
        assert!(!site.path().join("dist").exists());
    }
}

#[test]
fn arbitrary_content_object_iteration_is_reproducible() {
    // Free-form YAML objects must not reintroduce unordered iteration inside Tera.
    let site = fixture();
    replace(
        site.path(),
        "templates/page.html",
        "{% endblock %}",
        "{% for key, value in site %}<p>{{ key }}: {{ value }}</p>{% endfor %}{% endblock %}",
    );
    regen::build(site.path()).unwrap();
    let first = snapshot(&site.path().join("dist"));
    for _ in 0..4 {
        regen::build(site.path()).unwrap();
        assert_eq!(first, snapshot(&site.path().join("dist")));
    }
}

#[test]
fn grouped_collection_iteration_is_reproducible() {
    // Stable group keys are insufficient unless members also retain authored order.
    let site = fixture();
    fs::write(site.path().join("templates/page.html"),
        r#"{% extends "base.html" %}{% block content %}{% set entries = [{"group":"b","value":"first"},{"group":"a","value":"second"},{"group":"b","value":"third"},{"group":"c","value":"fourth"}] %}<p>{% for group, items in entries | group_by(attribute="group") %}{{ group }}:{% for item in items %}{{ item.value }},{% endfor %};{% endfor %}</p>{% endblock %}"#).unwrap();
    regen::build(site.path()).unwrap();
    let html = fs::read_to_string(site.path().join("dist/index.html")).unwrap();
    assert!(html.contains("a:second,;b:first,third,;c:fourth,;"));
    let first = snapshot(&site.path().join("dist"));
    for _ in 0..4 {
        regen::build(site.path()).unwrap();
        assert_eq!(first, snapshot(&site.path().join("dist")));
    }
}

#[test]
fn clean_build_removes_deleted_pages_and_versions_changed_assets() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let first = snapshot(&site.path().join("dist/assets"));
    fs::remove_file(site.path().join("content/en/pages/about.yaml")).unwrap();
    fs::remove_file(site.path().join("content/de/pages/about.yaml")).unwrap();
    let css = site.path().join("assets/site.css");
    fs::write(
        &css,
        format!(
            "{}\nbody {{ color: red; }}",
            fs::read_to_string(&css).unwrap()
        ),
    )
    .unwrap();
    regen::build(site.path()).unwrap();
    assert!(!site.path().join("dist/about/index.html").exists());
    assert!(!site.path().join("dist/de/ueber/index.html").exists());
    assert!(
        first
            .keys()
            .all(|path| !site.path().join("dist/assets").join(path).exists())
    );
    let css_output = snapshot(&site.path().join("dist/assets"));
    let css_bytes = css_output
        .iter()
        .find(|(path, _)| path.extension().unwrap() == "css")
        .unwrap()
        .1;
    assert!(String::from_utf8_lossy(css_bytes).contains("red"));
    assert!(css_bytes.len() < fs::read(css).unwrap().len());
}

#[test]
fn missing_translation_fails_without_changing_previous_output() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let first = snapshot(&site.path().join("dist"));
    fs::remove_file(site.path().join("content/de/pages/about.yaml")).unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&site.path().join("dist")));
}

#[test]
fn render_error_and_output_collision_leave_the_previous_site_intact() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let first = snapshot(&site.path().join("dist"));
    replace(
        site.path(),
        "content/de/pages/about.yaml",
        "template: page.html",
        "template: missing.html",
    );
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&site.path().join("dist")));
    assert!(!site.path().join(".regen-stage").exists());
    replace(
        site.path(),
        "content/de/pages/about.yaml",
        "template: missing.html",
        "template: page.html",
    );
    fs::write(
        site.path().join("public/index.html"),
        "must not replace generated homepage",
    )
    .unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(first, snapshot(&site.path().join("dist")));
}

#[test]
fn unowned_output_and_interrupted_staging_are_never_deleted() {
    let site = fixture();
    fs::create_dir(site.path().join("dist")).unwrap();
    fs::write(site.path().join("dist/keep.txt"), "not generated").unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(
        fs::read_to_string(site.path().join("dist/keep.txt")).unwrap(),
        "not generated"
    );
    fs::create_dir(site.path().join(".regen-stage")).unwrap();
    fs::write(
        site.path().join(".regen-stage/keep.txt"),
        "interrupted build",
    )
    .unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(
        fs::read_to_string(site.path().join(".regen-stage/keep.txt")).unwrap(),
        "interrupted build"
    );
}

#[test]
fn unoptimized_code_comments_and_binary_assets_are_preserved() {
    // Optimization boundaries must preserve attribution and code semantics; the
    // manifest must describe installed bytes, including unchanged and empty files.
    let site = fixture();
    let javascript = b"/*! keep license */\nconst crossFileGlobal = 42;\n";
    fs::write(site.path().join("assets/shared.js"), javascript).unwrap();
    let binary: Vec<u8> = (0..=255).cycle().take(65536 + 257).collect();
    fs::write(site.path().join("assets/payload.bin"), &binary).unwrap();
    let inline_js = "/*! inline license */\nwindow.shared = function () { return 2 + 3; };";
    let inline_css = "/*! inline attribution */\n.keep { color: rgb(1, 2, 3); }";
    replace(
        site.path(),
        "templates/page.html",
        "{% endblock %}",
        &format!(
            "<!-- page attribution --><script>{inline_js}</script><style>{inline_css}</style>{{% endblock %}}"
        ),
    );
    fs::write(site.path().join("public/raw.css"), inline_css).unwrap();
    fs::write(site.path().join("public/empty.bin"), []).unwrap();
    regen::build(site.path()).unwrap();
    let outputs = snapshot(&site.path().join("dist/assets"));
    let copied = outputs
        .iter()
        .find(|(path, _)| path.file_name().unwrap() == "shared.js")
        .unwrap()
        .1;
    assert_eq!(copied, javascript);
    let copied_binary = outputs
        .iter()
        .find(|(path, _)| path.file_name().unwrap() == "payload.bin")
        .unwrap()
        .1;
    assert_eq!(copied_binary, &binary);
    let output = site.path().join("dist");
    let html = fs::read_to_string(output.join("index.html")).unwrap();
    assert!(html.contains(inline_js));
    assert!(html.contains(inline_css));
    assert!(html.contains("<!-- page attribution -->"));
    assert_eq!(
        fs::read(output.join("raw.css")).unwrap(),
        inline_css.as_bytes()
    );
    let mut files = snapshot(&output);
    let manifest: serde_json::Value =
        serde_json::from_slice(&files.remove(Path::new("regen-manifest.json")).unwrap()).unwrap();
    let expected: serde_json::Map<String, serde_json::Value> = files
        .into_iter()
        .map(|(path, bytes)| {
            (
                path.to_str()
                    .unwrap()
                    .replace(std::path::MAIN_SEPARATOR, "/"),
                serde_json::json!({
                    "sha256": format!("{:x}", Sha256::digest(&bytes)),
                    "bytes": bytes.len(),
                }),
            )
        })
        .collect();
    assert_eq!(manifest["files"], serde_json::Value::Object(expected));
}

#[test]
fn ambiguous_yaml_and_unsafe_routes_are_rejected() {
    let site = fixture();
    replace(
        site.path(),
        "content/en/pages/about.yaml",
        "slug: about",
        "slug: about\nslug: duplicate",
    );
    assert!(regen::build(site.path()).is_err());
    replace(
        site.path(),
        "content/en/pages/about.yaml",
        "slug: about\nslug: duplicate",
        "slug: ../escape",
    );
    assert!(regen::build(site.path()).is_err());
    assert!(!site.path().join("escape").exists());
    replace(
        site.path(),
        "content/en/pages/about.yaml",
        "slug: ../escape",
        "slug: de",
    );
    assert!(regen::build(site.path()).is_err());
}

#[test]
fn reserved_root_slugs_do_not_restrict_localized_pages() {
    // A localized /de/assets/ page does not occupy the root asset namespace.
    let site = fixture();
    replace(
        site.path(),
        "content/de/pages/about.yaml",
        "slug: ueber",
        "slug: assets",
    );
    regen::build(site.path()).unwrap();
    let output = site.path().join("dist");
    assert!(output.join("de/assets/index.html").is_file());
    assert!(output.join("assets").is_dir());
    let previous = snapshot(&output);
    replace(
        site.path(),
        "content/en/pages/about.yaml",
        "slug: about",
        "slug: assets",
    );
    assert_rejected_without_replacement(site.path(), &previous);
}

#[test]
fn asset_file_names_cannot_alias_windows_devices_or_case_variants() {
    let site = fixture();
    for name in ["con.txt", "com1.txt", "lpt9.txt"] {
        let path = site.path().join("assets").join(name);
        fs::write(&path, "reserved").unwrap();
        assert!(regen::build(site.path()).is_err());
        fs::remove_file(path).unwrap();
    }
    fs::write(site.path().join("assets/Upper.css"), "body {}").unwrap();
    assert!(regen::build(site.path()).is_err());
    fs::remove_file(site.path().join("assets/Upper.css")).unwrap();
    fs::write(site.path().join("assets/com0.txt"), "portable").unwrap();
    fs::write(site.path().join("assets/lpt10.txt"), "portable").unwrap();
    regen::build(site.path()).unwrap();
    let assets = snapshot(&site.path().join("dist/assets"));
    assert!(
        assets
            .keys()
            .any(|path| path.file_name().unwrap() == "com0.txt")
    );
    assert!(
        assets
            .keys()
            .any(|path| path.file_name().unwrap() == "lpt10.txt")
    );
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_read_or_replace_external_files() {
    use std::os::unix::fs::symlink;
    let site = fixture();
    let external = tempdir();
    let outside = external.path().join("keep.txt");
    fs::write(&outside, "outside site").unwrap();
    symlink(&outside, site.path().join("assets/leak.txt")).unwrap();
    assert!(regen::build(site.path()).is_err());
    fs::remove_file(site.path().join("assets/leak.txt")).unwrap();
    symlink(external.path(), site.path().join("dist")).unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "outside site");
}

#[test]
fn numeric_regions_and_long_primary_codes_generate_localized_routes() {
    // Syntax support must not become a registry lookup; URL canonicalization still
    // removes the ordinary HTTP default port without altering locale routing.
    let site = fixture();
    for (old, new) in [("en", "qaaa"), ("de", "es-419")] {
        replace(
            site.path(),
            "regen.toml",
            &format!("\"{old}\""),
            &format!("\"{new}\""),
        );
        fs::rename(
            site.path().join("content").join(old),
            site.path().join("content").join(new),
        )
        .unwrap();
    }
    replace(
        site.path(),
        "regen.toml",
        "https://example.com",
        "HTTP://EXAMPLE.COM:80/",
    );
    regen::build(site.path()).unwrap();
    let output = site.path().join("dist");
    assert!(output.join("about/index.html").is_file());
    assert!(output.join("es-419/ueber/index.html").is_file());
    assert!(!output.join("qaaa").exists());
    let sitemap = fs::read_to_string(output.join("sitemap.xml")).unwrap();
    assert!(sitemap.contains("hreflang=\"qaaa\""));
    assert!(sitemap.contains("http://example.com/es-419/ueber/"));
    assert!(!sitemap.contains(":80"));
}

#[test]
fn malformed_content_layouts_preserve_the_installed_site() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    let content = site.path().join("content");

    fs::write(content.join("stray.yaml"), "unassigned: content").unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::remove_file(content.join("stray.yaml")).unwrap();

    fs::create_dir(content.join("en/drafts")).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::remove_dir(content.join("en/drafts")).unwrap();

    // The validated inventory must retain child types, not just allowed names.
    fs::rename(
        content.join("en/site.yaml"),
        site.path().join("saved-site.yaml"),
    )
    .unwrap();
    fs::create_dir(content.join("en/site.yaml")).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::remove_dir(content.join("en/site.yaml")).unwrap();
    fs::rename(
        site.path().join("saved-site.yaml"),
        content.join("en/site.yaml"),
    )
    .unwrap();

    fs::rename(content.join("de/pages"), site.path().join("saved-pages")).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::write(content.join("de/pages"), "not a page directory").unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::remove_file(content.join("de/pages")).unwrap();
    fs::rename(site.path().join("saved-pages"), content.join("de/pages")).unwrap();

    fs::rename(content.join("de"), site.path().join("saved-locale")).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
}

#[test]
fn page_extensions_templates_and_homepages_are_validated_before_replacement() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    let pages = site.path().join("content/en/pages");

    fs::rename(pages.join("about.yaml"), pages.join("about.txt")).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::rename(pages.join("about.txt"), pages.join("about.yaml")).unwrap();
    fs::rename(pages.join("about.yaml"), pages.join("about..yaml")).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::rename(pages.join("about..yaml"), pages.join("about.yaml")).unwrap();

    let about = fs::read_to_string(pages.join("about.yaml")).unwrap();
    for template in ["page.txt", "../page.html"] {
        fs::write(
            pages.join("about.yaml"),
            about.replace("template: page.html", &format!("template: {template}")),
        )
        .unwrap();
        assert_rejected_without_replacement(site.path(), &previous);
    }
    fs::write(pages.join("about.yaml"), about).unwrap();

    replace(
        site.path(),
        "content/en/pages/index.yaml",
        "slug: \"\"",
        "slug: home",
    );
    assert_rejected_without_replacement(site.path(), &previous);
}

#[test]
fn yaml_schema_and_tag_errors_do_not_replace_output_or_read_external_content() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    let page = site.path().join("content/en/pages/about.yaml");
    let original = fs::read_to_string(&page).unwrap();
    fs::write(&page, format!("{original}\nunknown: value\n")).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::write(&page, &original).unwrap();

    let outside = tempdir();
    let secret = outside.path().join("secret.yaml");
    fs::write(&secret, "secret: must not be included\n").unwrap();
    fs::write(
        site.path().join("content/en/site.yaml"),
        format!("secret: !include {}\n", secret.display()),
    )
    .unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    assert_eq!(
        fs::read_to_string(secret).unwrap(),
        "secret: must not be included\n"
    );
}

#[test]
fn invalid_template_inputs_fail_before_installation() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    let templates = site.path().join("templates");

    fs::write(templates.join("notes.txt"), "not a template").unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::remove_file(templates.join("notes.txt")).unwrap();

    fs::write(templates.join("page.html"), "{% if %}").unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::write(templates.join("page.html"), [0xff]).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
}

#[test]
fn missing_required_trees_and_non_directory_optional_trees_are_errors() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    for name in ["content", "templates", "assets", "public"] {
        let source = site.path().join(name);
        let saved = site.path().join(format!("saved-{name}"));
        fs::rename(&source, &saved).unwrap();
        if matches!(name, "assets" | "public") {
            fs::write(&source, "not a directory").unwrap();
        }
        assert_rejected_without_replacement(site.path(), &previous);
        if source.is_file() {
            fs::remove_file(&source).unwrap();
        }
        fs::rename(saved, source).unwrap();
    }
}

#[test]
fn missing_nonregular_and_invalid_utf8_configuration_preserve_output() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    let config = site.path().join("regen.toml");
    fs::remove_file(&config).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::create_dir(&config).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::remove_dir(&config).unwrap();
    fs::write(&config, [0xff]).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
}

#[test]
fn invalid_utf8_content_and_css_are_not_copied_as_binary_assets() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    let page = site.path().join("content/en/pages/about.yaml");
    let original = fs::read(&page).unwrap();
    fs::write(&page, [0xff]).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::write(page, original).unwrap();
    fs::write(site.path().join("assets/site.css"), [0xff]).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
}

#[test]
fn generated_namespaces_and_parent_file_collisions_preserve_output() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    for name in [
        "assets/injected.txt",
        "sitemap.xml",
        "regen-manifest.json",
        "about",
    ] {
        let path = site.path().join("public").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "must remain only in the input tree").unwrap();
        assert_rejected_without_replacement(site.path(), &previous);
        fs::remove_file(&path).unwrap();
        if name.starts_with("assets/") {
            fs::remove_dir(path.parent().unwrap()).unwrap();
        }
    }
}

#[test]
fn malformed_owned_markers_and_non_directory_output_are_never_replaced() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let output = site.path().join("dist");
    for marker in [
        r#"{"generator":"ReGen","format":1"#,
        r#"{"generator":"Other","format":1}"#,
    ] {
        fs::write(output.join("regen-manifest.json"), marker).unwrap();
        let previous = snapshot(&output);
        assert_rejected_without_replacement(site.path(), &previous);
    }

    fs::remove_dir_all(&output).unwrap();
    fs::write(&output, "user-owned file").unwrap();
    assert!(regen::build(site.path()).is_err());
    assert_eq!(fs::read_to_string(&output).unwrap(), "user-owned file");
    assert!(!site.path().join(".regen-stage").exists());
}

#[test]
fn grouping_empty_nullable_boolean_and_integer_keys_preserves_order_and_types() {
    // Numeric keys must sort numerically, while null groups are omitted rather than
    // converted into a printable key or confused with false.
    let site = fixture();
    fs::write(
        site.path().join("templates/page.html"),
        r#"{% set empty = [] %}<p>{% for key, items in empty | group_by(attribute="group") %}unexpected{% else %}empty{% endfor %}</p>
{% set entries = [{"group":true,"value":"yes"},{"group":false,"value":"no"},{"group":none,"value":"skip"}] %}
<p>{% for key, items in entries | group_by(attribute="group") %}{{ key }}:{% for item in items %}{{ item.value }},{% endfor %};{% endfor %}</p>
{% set numbers = [{"meta":{"group":10},"value":"ten"},{"meta":{"group":-2},"value":"negative"},{"meta":{"group":10},"value":"again"}] %}
<p>{% for key, items in numbers | group_by(attribute="meta.group") %}{{ key }}:{% for item in items %}{{ item.value }},{% endfor %};{% endfor %}</p>"#,
    )
    .unwrap();
    regen::build(site.path()).unwrap();
    let html = fs::read_to_string(site.path().join("dist/index.html")).unwrap();
    assert!(html.contains("<p>empty</p>"));
    assert!(html.contains("false:no,;true:yes,;"));
    assert!(html.contains("-2:negative,;10:ten,again,;"));
    assert!(!html.contains("skip"));
}

#[test]
fn invalid_grouping_arguments_and_keys_abort_without_replacement() {
    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    for expression in [
        r#"[{"group":"a"}] | group_by"#,
        r#"[{"other":"a"}] | group_by(attribute="group")"#,
        r#"[{"group":1.5}] | group_by(attribute="group")"#,
        // Numeric equality must not let an integer group admit a later float.
        r#"[{"group":1},{"group":1.0}] | group_by(attribute="group")"#,
        r#"[{"group":["a"]}] | group_by(attribute="group")"#,
    ] {
        fs::write(
            site.path().join("templates/page.html"),
            format!("{{{{ {expression} }}}}"),
        )
        .unwrap();
        assert_rejected_without_replacement(site.path(), &previous);
    }
}

#[test]
fn nonexistent_site_roots_do_not_create_output() {
    let parent = tempdir();
    let missing = parent.path().join("missing");
    assert!(regen::build(&missing).is_err());
    assert!(!missing.exists());
    assert_eq!(snapshot(parent.path()), BTreeMap::new());
}

// Linux permits raw filename bytes; macOS rejects this fixture before ReGen runs.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_names_are_rejected_without_replacement() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    let invalid = site
        .path()
        .join("assets")
        .join(OsStr::from_bytes(b"invalid-\xff"));
    fs::write(&invalid, "not portable").unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    fs::remove_file(invalid).unwrap();
}

#[cfg(unix)]
#[test]
fn special_files_are_rejected_without_replacement() {
    use std::os::unix::net::UnixListener;

    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));

    let socket = site.path().join("public/socket");
    let listener = UnixListener::bind(&socket).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    drop(listener);
    fs::remove_file(socket).unwrap();
}

#[cfg(unix)]
#[test]
fn symlinked_roots_and_configuration_do_not_read_external_sites() {
    use std::os::unix::fs::symlink;

    let site = fixture();
    regen::build(site.path()).unwrap();
    let previous = snapshot(&site.path().join("dist"));
    let parent = tempdir();
    let linked = parent.path().join("linked-site");
    symlink(site.path(), &linked).unwrap();
    assert!(regen::build(&linked).is_err());
    assert_eq!(previous, snapshot(&site.path().join("dist")));

    let config = site.path().join("regen.toml");
    let external = parent.path().join("external.toml");
    fs::rename(&config, &external).unwrap();
    let bytes = fs::read(&external).unwrap();
    symlink(&external, &config).unwrap();
    assert_rejected_without_replacement(site.path(), &previous);
    assert_eq!(fs::read(&external).unwrap(), bytes);
}

#[test]
fn xml_sitemap_escapes_permitted_authority_punctuation() {
    // URL parsing alone does not make accepted authority characters safe in XML.
    let site = fixture();
    replace(
        site.path(),
        "regen.toml",
        "https://example.com",
        "https://exa&\\\"m'ple.com",
    );
    regen::build(site.path()).unwrap();
    let sitemap = fs::read_to_string(site.path().join("dist/sitemap.xml")).unwrap();
    let authority = "https://exa&amp;&quot;m&apos;ple.com";
    assert!(sitemap.contains(&format!("<loc>{authority}/</loc>")));
    assert!(sitemap.contains(&format!("href=\"{authority}/de/\"")));
    assert!(!sitemap.contains("https://exa&\"m'ple.com"));
}

#[test]
fn rendered_html_escapes_author_text_in_content_and_quoted_attributes() {
    // Compare with inert HTML after the same minification, not a serializer's
    // incidental entity spelling, to exercise the complete escaping boundary.
    let site = fixture();
    let page = serde_json::json!({
        "title": "<img src=x onerror=alert(1)> &lt;author&gt; 雨",
        "description": "\" autofocus onfocus=\"alert(1)",
        "template": "page.html",
        "slug": ""
    });
    fs::write(
        site.path().join("content/en/pages/index.yaml"),
        serde_json::to_vec(&page).unwrap(),
    )
    .unwrap();
    let template = site.path().join("templates/page.html");
    fs::write(
        &template,
        r#"{% extends "base.html" %}{% block content %}<p title="{{ page.description }}">{{ page.title }}</p>{% endblock %}"#,
    )
    .unwrap();
    regen::build(site.path()).unwrap();
    let rendered = fs::read(site.path().join("dist/index.html")).unwrap();
    fs::write(
        &template,
        r#"{% extends "base.html" %}{% block content %}<p title="&quot; autofocus onfocus=&quot;alert(1)">&lt;img src=x onerror=alert(1)&gt; &amp;lt;author&amp;gt; 雨</p>{% endblock %}"#,
    )
    .unwrap();
    regen::build(site.path()).unwrap();
    assert_eq!(
        rendered,
        fs::read(site.path().join("dist/index.html")).unwrap()
    );
}

#[test]
fn captured_and_filtered_includes_preserve_markup_and_escape_author_text_once() {
    // Captured HTML is already escaped. Safety-aware filters must preserve its
    // markup without either double-escaping author text or trusting that text.
    let site = fixture();
    let page = serde_json::json!({
        "title": "<img src=x onerror=alert(1)> &lt;author&gt; 雨",
        "description": "Included author text",
        "template": "page.html",
        "slug": ""
    });
    fs::write(
        site.path().join("content/en/pages/index.yaml"),
        serde_json::to_vec(&page).unwrap(),
    )
    .unwrap();
    fs::write(
        site.path().join("templates/fragment.html"),
        "<em>{{ page.title }}</em>",
    )
    .unwrap();
    let template = site.path().join("templates/page.html");
    fs::write(
        &template,
        r#"{% extends "base.html" %}{% block content %}{% set excerpt %}{% include "fragment.html" %}{% endset %}<p>{{ excerpt | trim }}</p><p>{% filter trim %}{% include "fragment.html" %}{% endfilter %}</p>{% endblock %}"#,
    )
    .unwrap();
    regen::build(site.path()).unwrap();
    let rendered = fs::read(site.path().join("dist/index.html")).unwrap();
    fs::write(
        &template,
        r#"{% extends "base.html" %}{% block content %}<p><em>&lt;img src=x onerror=alert(1)&gt; &amp;lt;author&amp;gt; 雨</em></p><p><em>&lt;img src=x onerror=alert(1)&gt; &amp;lt;author&amp;gt; 雨</em></p>{% endblock %}"#,
    )
    .unwrap();
    regen::build(site.path()).unwrap();
    assert_eq!(
        rendered,
        fs::read(site.path().join("dist/index.html")).unwrap()
    );
}

#[test]
fn unique_content_maps_keep_distinct_records_in_authored_order() {
    // Equal-sized maps are not interchangeable: deduplication must compare their
    // contents while keeping the first occurrence, including nested YAML values.
    let site = fixture();
    let page = serde_json::json!({
        "title": "Unique records",
        "description": "Structured content",
        "template": "unique.html",
        "slug": "",
        "data": {
            "entries": [
                {"label": "first", "meta": {"rank": 2}},
                {"label": "second", "meta": {"rank": 1}},
                {"label": "first", "meta": {"rank": 2}},
                {"label": "first", "meta": {"rank": 3}}
            ]
        }
    });
    fs::write(
        site.path().join("content/en/pages/index.yaml"),
        serde_json::to_vec(&page).unwrap(),
    )
    .unwrap();
    fs::write(
        site.path().join("templates/unique.html"),
        r#"{% extends "base.html" %}{% block content %}<p>{% for entry in page.data.entries | unique %}{{ entry.label }}:{{ entry.meta.rank }};{% endfor %}</p>{% endblock %}"#,
    )
    .unwrap();
    regen::build(site.path()).unwrap();
    let rendered = fs::read_to_string(site.path().join("dist/index.html")).unwrap();
    assert!(rendered.contains("first:2;second:1;first:3;"));
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires strace; the Linux coverage job runs this explicitly"]
fn kernel_io_failures_cannot_publish_partial_output() {
    // Real filesystem failures must unwind through the complete CLI, preserving
    // the installed site and source while discarding only this build's stage.
    // Injection is scoped to one child's exact path, never global syscall ordinals.
    // The second asset statx inspects its open descriptor. The fourth assets mkdir
    // recreates the namespace after initial creation and two parent checks.
    // The cache source path matches its final rename into the content-hashed tree.
    for (relative, fault) in [
        (".regen-stage", "statx:error=EIO:when=1"),
        (".regen-previous", "statx:error=EIO:when=1"),
        ("assets/mark.svg", "statx:error=EIO:when=1"),
        ("assets/mark.svg", "statx:error=EIO:when=2"),
        ("assets/mark.svg", "openat:error=EIO"),
        ("assets/mark.svg", "read:error=EIO"),
        (".regen-stage/assets/mark.svg", "openat:error=ENOSPC"),
        (".regen-stage/assets/site.css", "write:error=ENOSPC"),
        (".regen-stage/index.html", "write:error=ENOSPC"),
        (".regen-stage/assets", "mkdir:error=ENOSPC:when=4"),
        (".regen-stage/asset-cache", "rename:error=EIO"),
    ] {
        let site = fixture();
        regen::build(site.path()).unwrap();
        let before = snapshot(site.path());
        let logs = tempdir();
        let trace = logs.path().join("kernel.trace");
        let result = Command::new("strace")
            .args(["-qq", "-o"])
            .arg(&trace)
            .arg("-P")
            .arg(site.path().join(relative))
            .arg("-e")
            .arg(format!("inject={fault}"))
            .arg(env!("CARGO_BIN_EXE_regen"))
            .args(["build", "--site"])
            .arg(site.path())
            .output()
            .expect("install strace to run the Linux kernel-fault scenarios");
        assert!(
            fs::read_to_string(trace).unwrap().contains("(INJECTED)"),
            "fault was not exercised for {relative}: {fault}: {result:?}"
        );
        assert_eq!(
            result.status.code(),
            Some(1),
            "{relative}: {fault}: {result:?}"
        );
        assert_eq!(before, snapshot(site.path()), "{relative}: {fault}");
        assert!(!site.path().join(".regen-stage").exists());
        assert!(!site.path().join(".regen-previous").exists());
    }
}
