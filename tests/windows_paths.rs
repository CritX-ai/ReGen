//! Real builds must accept both ordinary drive paths and canonical verbatim paths.

#![cfg(windows)]

mod common;

use std::fs;
use std::path::{Component, Path, PathBuf, Prefix};
use walkdir::WalkDir;

#[test]
fn absolute_and_canonical_verbatim_site_roots_build_real_output() {
    let site = common::tempdir();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/minimal");
    for entry in WalkDir::new(&source).into_iter().map(Result::unwrap) {
        let relative = entry.path().strip_prefix(&source).unwrap();
        if matches!(relative.components().next(), Some(part) if part.as_os_str() == "dist") {
            continue;
        }
        let destination = site.path().join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&destination).unwrap();
        } else {
            fs::copy(entry.path(), &destination).unwrap();
        }
    }

    let verbatim = fs::canonicalize(site.path()).unwrap();
    let mut parts = verbatim.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        panic!("canonical Windows fixture must have a path prefix");
    };
    let Prefix::VerbatimDisk(drive) = prefix.kind() else {
        panic!("this regression requires a temporary directory on a local Windows drive");
    };
    // Reuse the real drive and remaining components, preserving non-UTF-8 names.
    let mut absolute = PathBuf::from(format!("{}:", char::from(drive)));
    absolute.extend(parts);
    assert!(absolute.is_absolute());
    assert!(matches!(
        absolute.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
    ));
    fs::write(
        site.path().join("templates/windows.html"),
        "<main>{{ page.title }}</main>",
    )
    .unwrap();

    for (root, content) in [
        (&absolute, "ordinary absolute path"),
        (&verbatim, "canonical verbatim path"),
    ] {
        fs::write(root.join("public/windows-path.txt"), content).unwrap();
        fs::write(
            root.join("content/en/pages/index.yaml"),
            format!("title: {content}\ndescription: Native root paths\ntemplate: windows.html\nslug: \"\"\n"),
        )
        .unwrap();
        let summary = regen::build(root).unwrap();
        assert_eq!(summary.output, verbatim.join("dist"));
        let html = fs::read_to_string(summary.output.join("index.html")).unwrap();
        assert_eq!(html, format!("<main>{content}</main>"));
        assert_eq!(
            fs::read_to_string(summary.output.join("windows-path.txt")).unwrap(),
            content
        );

        // Missing descendants must retain NotFound, not fail on a prefix fragment.
        let missing = root.join("missing/site");
        let error = regen::build(&missing).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
        assert!(!root.join("missing").exists());
        assert_eq!(
            fs::read_to_string(summary.output.join("index.html")).unwrap(),
            html
        );
        assert!(!root.join(".regen-stage").exists());
        assert!(!root.join(".regen-previous").exists());
    }
}
