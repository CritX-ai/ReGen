//! Rendering and final-byte inventory contracts independent of complete site fixtures.
//!
//! Exercise escaping contexts, typed grouping, exclusive public copying, and digest
//! failures directly. Real I/O errors must propagate rather than yield plausible
//! HTML, partial copies, or manifests that claim unreadable files were inventoried.

use super::{copy_public, escape_html, file_digest, ordered_group_by, write_manifest, xml_escape};
use std::collections::BTreeMap;
use std::fs::{self, File};
use tera::{Context, Tera, Value};

#[test]
fn html_escaping_treats_existing_entities_as_untrusted_text_and_preserves_unicode() {
    let mut output = Vec::new();
    escape_html("<&>\"' café 東京 𐍈 &amp;", &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "&lt;&amp;&gt;&quot;&#x27; café 東京 𐍈 &amp;amp;"
    );
}

#[test]
fn html_escaping_propagates_a_real_writer_capacity_error() {
    let mut buffer = [0_u8; 3];
    let error = escape_html("&", &mut buffer.as_mut_slice()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WriteZero);
}

#[test]
fn xml_escaping_protects_both_attribute_quotes_and_preserves_unicode() {
    assert_eq!(
        xml_escape("https://example.com/?q=<café>&quote=\"'"),
        "https://example.com/?q=&lt;café&gt;&amp;quote=&quot;&apos;"
    );
}

#[test]
fn grouped_integer_extremes_render_in_numeric_order_without_loss() {
    // Stringifying or narrowing keys would reorder negatives or lose wide integers.
    let mut tera = Tera::new();
    tera.register_filter("group_by", ordered_group_by);
    tera.add_raw_template(
        "groups.html",
        r#"{% for key, items in entries | group_by(attribute="group") %}{{ key }}:{% for item in items %}{{ item.label }},{% endfor %};{% endfor %}"#,
    )
    .unwrap();
    let entries: Vec<_> = [
        (Value::from(i128::MAX), "maximum"),
        (Value::from(u64::MAX), "wide"),
        (Value::from(i128::MIN), "minimum"),
        (Value::from(-1_i64), "negative"),
    ]
    .into_iter()
    .map(|(group, label)| BTreeMap::from([("group", group), ("label", Value::from(label))]))
    .collect();
    let mut context = Context::new();
    context.insert("entries", &entries);
    assert_eq!(
        tera.render("groups.html", &context).unwrap(),
        format!(
            "{}:minimum,;-1:negative,;{}:wide,;{}:maximum,;",
            i128::MIN,
            u64::MAX,
            i128::MAX
        )
    );
}

#[test]
fn public_copy_refuses_to_truncate_an_existing_destination() {
    let source = tempfile::tempdir().unwrap();
    let stage = tempfile::tempdir().unwrap();
    fs::write(source.path().join("keep.txt"), b"new source").unwrap();
    fs::write(stage.path().join("keep.txt"), b"existing output").unwrap();
    let inventory = crate::files::files(source.path(), false).unwrap();
    assert!(copy_public(source.path(), inventory, stage.path()).is_err());
    assert_eq!(
        fs::read(stage.path().join("keep.txt")).unwrap(),
        b"existing output"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn public_copy_propagates_a_real_kernel_read_failure() {
    // /proc/PID/mem is a regular file, but reading its unmapped zero address fails.
    // Exercise the copy phase's I/O boundary without a synthetic failing reader.
    let root = std::path::PathBuf::from(format!("/proc/{}", std::process::id()));
    let source = root.join("mem");
    assert!(fs::symlink_metadata(&source).unwrap().is_file());
    let site = tempfile::tempdir().unwrap();
    let transaction = crate::files::Transaction::begin(site.path()).unwrap();
    let error = copy_public(&root, vec![source], transaction.stage()).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error(),
        Some(5)
    );
    drop(transaction);
    assert!(!site.path().join(".regen-stage").exists());
    assert!(!site.path().join("dist").exists());
}

#[cfg(unix)]
#[test]
fn unreadable_files_cannot_be_copied_or_recorded_in_a_successful_manifest() {
    use std::os::unix::fs::PermissionsExt;

    let source = tempfile::tempdir().unwrap();
    let stage = tempfile::tempdir().unwrap();
    let path = source.path().join("private.txt");
    fs::write(&path, b"private content").unwrap();
    let inventory = crate::files::files(source.path(), false).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
    // Privileged runners may bypass mode bits; assert the observed capability
    // instead of assuming chmod necessarily makes this file unreadable.
    let readable = File::open(&path).is_ok();
    let copied = copy_public(source.path(), inventory, stage.path());
    let manifest = write_manifest(source.path());
    if readable {
        copied.unwrap();
        manifest.unwrap();
        assert_eq!(
            fs::read(stage.path().join("private.txt")).unwrap(),
            b"private content"
        );
        let output: serde_json::Value =
            serde_json::from_slice(&fs::read(source.path().join("regen-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(output["files"]["private.txt"]["bytes"], 15);
    } else {
        for error in [copied.unwrap_err(), manifest.unwrap_err()] {
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::PermissionDenied
            );
        }
        assert!(!stage.path().join("private.txt").exists());
        assert!(!source.path().join("regen-manifest.json").exists());
    }
}

#[test]
fn manifest_requires_an_existing_tree_and_preserves_an_existing_marker() {
    let stage = tempfile::tempdir().unwrap();
    let missing = stage.path().join("missing");
    assert!(write_manifest(&missing).is_err());
    assert!(!missing.exists());
    fs::write(stage.path().join("regen-manifest.json"), b"existing marker").unwrap();
    assert!(write_manifest(stage.path()).is_err());
    assert_eq!(
        fs::read(stage.path().join("regen-manifest.json")).unwrap(),
        b"existing marker"
    );
}

#[test]
fn digest_read_errors_cannot_produce_a_valid_empty_file_digest() {
    let directory = tempfile::tempdir().unwrap();
    let file = File::create(directory.path().join("write-only")).unwrap();
    let error = file_digest(file, &mut [0; 64])
        .err()
        .expect("write-only handle must fail");
    assert!(error.raw_os_error().is_some());
}
