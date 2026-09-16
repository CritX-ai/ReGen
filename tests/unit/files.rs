//! Filesystem policy and transaction recovery states isolated from rendering.
//!
//! Direct state changes model lost stages, blocked promotion, and failed rollback
//! without timing-dependent races. These cases defend preservation of owned and
//! intervening data, not support for concurrent site mutation. Permission probes
//! account for privileged runners that can bypass Unix mode bits.

use super::{TEXT_LIMIT, Transaction, output_file, read_text, read_text_stream};
use crate::test_support::tempdir;
use std::fs;
use std::path::Path;

/// Establish recognized output with sentinel bytes for preservation assertions.
fn owned_output(root: &Path) {
    fs::create_dir(root.join("dist")).unwrap();
    fs::write(
        root.join("dist/regen-manifest.json"),
        br#"{"generator":"ReGen","format":1}"#,
    )
    .unwrap();
    fs::write(root.join("dist/keep.txt"), "previous site").unwrap();
}

/// Apply a temporary permission boundary and restore it even during unwinding.
#[cfg(unix)]
fn with_mode<T>(path: &Path, mode: u32, action: impl FnOnce() -> T) -> T {
    use std::os::unix::fs::PermissionsExt;

    struct Restore<'a> {
        path: &'a Path,
        permissions: fs::Permissions,
    }

    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            if self.path.exists() {
                fs::set_permissions(self.path, self.permissions.clone()).unwrap();
            }
        }
    }

    let _restore = Restore {
        path,
        permissions: fs::metadata(path).unwrap().permissions(),
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    action()
}

#[test]
fn text_limit_accepts_the_boundary_and_rejects_one_more_byte() {
    let temp = tempdir();
    let path = temp.path().join("bounded.txt");
    let file = fs::File::create(&path).unwrap();
    file.set_len(TEXT_LIMIT).unwrap();
    let text = read_text(&path).unwrap();
    assert_eq!(text.len() as u64, TEXT_LIMIT);
    assert!(text.bytes().all(|byte| byte == 0));
    drop(text);
    file.set_len(TEXT_LIMIT + 1).unwrap();
    assert!(read_text(&path).is_err());
    assert_eq!(file.metadata().unwrap().len(), TEXT_LIMIT + 1);
}

#[test]
fn missing_and_nonregular_text_sources_are_not_treated_as_empty_documents() {
    let temp = tempdir();
    let missing = temp.path().join("missing.txt");
    let error = read_text(&missing).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(format!("{error:#}").contains(missing.to_str().unwrap()));
    assert!(!missing.exists());

    let directory = temp.path().join("document");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("keep.txt"), b"not a text document").unwrap();
    assert!(read_text(&directory).is_err());
    assert_eq!(
        fs::read(directory.join("keep.txt")).unwrap(),
        b"not a text document"
    );
}

#[test]
fn output_paths_cannot_escape_the_destination_or_truncate_existing_files() {
    let temp = tempdir();
    let root = temp.path().join("output");
    fs::create_dir(&root).unwrap();
    let outside = temp.path().join("keep.txt");
    fs::write(&outside, "outside").unwrap();
    assert!(output_file(&root, "../keep.txt").is_err());
    assert!(output_file(&root, "").is_err());
    fs::write(root.join("keep.txt"), "inside").unwrap();
    assert!(output_file(&root, "keep.txt").is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "outside");
    assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "inside");
}

#[test]
fn a_lost_stage_cannot_install_an_incomplete_first_build() {
    let temp = tempdir();
    let transaction = Transaction::begin(temp.path(), "dist").unwrap();
    fs::remove_dir(transaction.stage()).unwrap();
    assert!(transaction.commit().is_err());
    assert!(!temp.path().join("dist").exists());
    assert!(!temp.path().join(".regen-previous").exists());
    assert!(!temp.path().join(".regen-stage").exists());
}

#[test]
fn failed_promotion_restores_the_previous_site() {
    let temp = tempdir();
    owned_output(temp.path());
    let manifest = fs::read(temp.path().join("dist/regen-manifest.json")).unwrap();
    let transaction = Transaction::begin(temp.path(), "dist").unwrap();
    fs::write(transaction.stage().join("new.txt"), "incomplete").unwrap();
    fs::remove_dir_all(transaction.stage()).unwrap();

    assert!(transaction.commit().is_err());
    assert_eq!(
        fs::read_to_string(temp.path().join("dist/keep.txt")).unwrap(),
        "previous site"
    );
    assert_eq!(
        fs::read(temp.path().join("dist/regen-manifest.json")).unwrap(),
        manifest
    );
    assert!(!temp.path().join("dist/new.txt").exists());
    assert!(!temp.path().join(".regen-previous").exists());
    assert!(!temp.path().join(".regen-stage").exists());
}

#[test]
fn successful_replacement_removes_stale_output_and_only_its_own_recovery_state() {
    let temp = tempdir();
    // Install real owned output first: no backup exists until a later replacement.
    let first = Transaction::begin(temp.path(), "dist").unwrap();
    super::write_output(
        first.stage(),
        "regen-manifest.json",
        br#"{"generator":"ReGen","format":1}"#,
    )
    .unwrap();
    super::write_output(first.stage(), "keep.txt", b"previous site").unwrap();
    let installed = first.commit().unwrap();
    assert_eq!(
        fs::read(installed.join("keep.txt")).unwrap(),
        b"previous site"
    );
    assert!(!temp.path().join(".regen-previous").exists());
    assert!(!temp.path().join(".regen-stage").exists());
    fs::write(temp.path().join("source.txt"), b"source remains").unwrap();
    let transaction = Transaction::begin(temp.path(), "dist").unwrap();
    super::write_output(transaction.stage(), "nested/new.txt", b"complete new site").unwrap();
    let output = transaction.commit().unwrap();

    assert_eq!(output, temp.path().join("dist"));
    assert_eq!(
        fs::read(output.join("nested/new.txt")).unwrap(),
        b"complete new site"
    );
    assert!(!output.join("keep.txt").exists());
    assert!(!output.join("regen-manifest.json").exists());
    assert!(!temp.path().join(".regen-stage").exists());
    assert!(!temp.path().join(".regen-previous").exists());
    assert_eq!(
        fs::read(temp.path().join("source.txt")).unwrap(),
        b"source remains"
    );
}

#[test]
fn a_backup_appearing_before_commit_is_preserved_for_manual_recovery() {
    let temp = tempdir();
    owned_output(temp.path());
    let transaction = Transaction::begin(temp.path(), "dist").unwrap();
    fs::write(transaction.stage().join("new.txt"), "new site").unwrap();
    fs::create_dir(temp.path().join(".regen-previous")).unwrap();
    fs::write(
        temp.path().join(".regen-previous/keep.txt"),
        "recovery copy",
    )
    .unwrap();

    assert!(transaction.commit().is_err());
    assert_eq!(
        fs::read_to_string(temp.path().join("dist/keep.txt")).unwrap(),
        "previous site"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join(".regen-previous/keep.txt")).unwrap(),
        "recovery copy"
    );
    assert!(!temp.path().join("dist/new.txt").exists());
    assert!(!temp.path().join(".regen-stage").exists());
}

#[test]
fn failed_backup_cleanup_keeps_the_installed_site_and_recoverable_backup() {
    let temp = tempdir();
    owned_output(temp.path());
    let transaction = Transaction::begin(temp.path(), "dist").unwrap();
    fs::write(transaction.stage().join("new.txt"), "new site").unwrap();
    // A directory replaced by a file after admission can be renamed aside,
    // but must not be silently deleted by directory-only backup cleanup.
    fs::remove_dir_all(temp.path().join("dist")).unwrap();
    fs::write(temp.path().join("dist"), "changed after admission").unwrap();

    assert!(transaction.commit().is_err());
    assert_eq!(
        fs::read_to_string(temp.path().join("dist/new.txt")).unwrap(),
        "new site"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join(".regen-previous")).unwrap(),
        "changed after admission"
    );
    assert!(!temp.path().join(".regen-stage").exists());
}

#[cfg(unix)]
#[test]
fn non_directory_ancestors_report_the_filesystem_error_without_mutation() {
    let temp = tempdir();
    let file = temp.path().join("file");
    fs::write(&file, "not a directory").unwrap();
    let error = super::reject_symlinks(&file.join("child")).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotADirectory
    );
    assert_eq!(fs::read_to_string(file).unwrap(), "not a directory");
}

#[cfg(unix)]
#[test]
fn commit_rechecks_output_and_backup_symlinks_before_promotion() {
    use std::os::unix::fs::symlink;

    for name in ["dist", ".regen-previous"] {
        let temp = tempdir();
        let outside = tempdir();
        fs::write(outside.path().join("keep.txt"), "external").unwrap();
        let transaction = Transaction::begin(temp.path(), "dist").unwrap();
        fs::write(transaction.stage().join("new.txt"), "new site").unwrap();
        symlink(outside.path(), temp.path().join(name)).unwrap();

        assert!(transaction.commit().is_err());
        assert_eq!(
            fs::read_to_string(outside.path().join("keep.txt")).unwrap(),
            "external"
        );
        assert!(!outside.path().join("new.txt").exists());
        assert!(
            fs::symlink_metadata(temp.path().join(name))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!temp.path().join(".regen-stage").exists());
    }
}

#[test]
fn text_streams_stop_after_one_lookahead_byte_even_when_the_source_keeps_growing() {
    use std::io::Seek;

    let temp = tempdir();
    let path = temp.path().join("growing.txt");
    let writer = fs::File::create(&path).unwrap();
    writer.set_len(TEXT_LIMIT).unwrap();
    let mut input = fs::File::open(&path).unwrap();
    assert_eq!(input.metadata().unwrap().len(), TEXT_LIMIT);
    // Grow only after inspection, so the stream bound must catch what metadata cannot.
    writer.set_len(TEXT_LIMIT + 2).unwrap();
    assert!(read_text_stream(input.try_clone().unwrap(), &path).is_err());
    // Cloned File handles share a cursor: one byte distinguishes the boundary,
    // while draining the final byte would exceed the promised read bound.
    assert_eq!(input.stream_position().unwrap(), TEXT_LIMIT + 1);
    assert_eq!(writer.metadata().unwrap().len(), TEXT_LIMIT + 2);
}

#[test]
fn text_stream_read_failures_preserve_the_io_cause_and_source() {
    // A real unreadable handle must not become an empty successful input.
    let temp = tempdir();
    let path = temp.path().join("write-only.txt");
    fs::write(&path, "source remains intact").unwrap();
    let input = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let error = read_text_stream(input, &path).unwrap_err();
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(fs::read_to_string(&path).unwrap(), "source remains intact");
}

#[test]
fn failed_rollback_preserves_both_the_recovery_copy_and_intervening_output() {
    // A nonempty intervening directory blocks rollback without a scheduler race.
    let temp = tempdir();
    owned_output(temp.path());
    let mut transaction = Transaction::begin(temp.path(), "dist").unwrap();
    let manifest = fs::read(transaction.output.join("regen-manifest.json")).unwrap();
    fs::rename(&transaction.output, &transaction.previous).unwrap();
    fs::remove_dir(transaction.stage()).unwrap();
    fs::create_dir(&transaction.output).unwrap();
    fs::write(transaction.output.join("intervening.txt"), "other writer").unwrap();

    assert!(transaction.promote(true).is_err());
    drop(transaction);
    assert_eq!(
        fs::read_to_string(temp.path().join(".regen-previous/keep.txt")).unwrap(),
        "previous site"
    );
    assert_eq!(
        fs::read(temp.path().join(".regen-previous/regen-manifest.json")).unwrap(),
        manifest
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("dist/intervening.txt")).unwrap(),
        "other writer"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_inventory_root_cannot_read_outside_the_source_tree() {
    use std::os::unix::fs::symlink;

    // Optional means absent is allowed, not that a linked root may be traversed.
    let temp = tempdir();
    let outside = tempdir();
    fs::write(outside.path().join("private.txt"), "not an asset").unwrap();
    let root = temp.path().join("assets");
    symlink(outside.path(), &root).unwrap();
    assert!(super::files(&root, true).is_err());
    assert_eq!(
        fs::read_to_string(outside.path().join("private.txt")).unwrap(),
        "not an asset"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_ancestors_cannot_redirect_text_reads_or_output_creation() {
    use std::os::unix::fs::symlink;

    let temp = tempdir();
    let outside = tempdir();
    fs::write(outside.path().join("private.txt"), b"external source").unwrap();
    symlink(outside.path(), temp.path().join("linked")).unwrap();
    assert!(read_text(&temp.path().join("linked/private.txt")).is_err());
    assert!(output_file(temp.path(), "linked/new.txt").is_err());
    assert!(!outside.path().join("new.txt").exists());
    assert_eq!(
        fs::read(outside.path().join("private.txt")).unwrap(),
        b"external source"
    );
    assert!(
        fs::symlink_metadata(temp.path().join("linked"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn inventories_reject_nonportable_descendants_instead_of_silently_omitting_them() {
    let temp = tempdir();
    fs::create_dir(temp.path().join(".hidden")).unwrap();
    fs::write(temp.path().join(".hidden/source.txt"), b"required source").unwrap();
    assert!(super::files(temp.path(), false).is_err());
    assert_eq!(
        fs::read(temp.path().join(".hidden/source.txt")).unwrap(),
        b"required source"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn inventory_names_must_be_utf8_without_lossy_aliases() {
    use std::os::unix::ffi::OsStringExt;

    let temp = tempdir();
    let name = std::ffi::OsString::from_vec(b"invalid-\xff.txt".to_vec());
    let path = temp.path().join(name);
    fs::write(&path, b"required source").unwrap();
    assert!(super::files(temp.path(), false).is_err());
    assert_eq!(fs::read(path).unwrap(), b"required source");
}

#[cfg(unix)]
#[test]
fn inventories_reject_symlink_descendants_even_when_the_target_is_a_regular_file() {
    use std::os::unix::fs::symlink;

    let temp = tempdir();
    let outside = tempdir();
    let target = outside.path().join("private.txt");
    fs::write(&target, b"external source").unwrap();
    let link = temp.path().join("linked.txt");
    symlink(&target, &link).unwrap();
    assert!(super::files(temp.path(), false).is_err());
    assert_eq!(fs::read(target).unwrap(), b"external source");
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
}

#[cfg(unix)]
#[test]
fn denied_stage_cleanup_leaves_recoverable_files_without_touching_the_previous_site() {
    let temp = tempdir();
    owned_output(temp.path());
    let transaction = Transaction::begin(temp.path(), "dist").unwrap();
    let stage = transaction.stage().to_path_buf();
    fs::write(stage.join("unfinished.txt"), "recoverable").unwrap();
    with_mode(&stage, 0o555, || {
        let denied = fs::File::create(stage.join("permission-probe")).is_err();
        drop(transaction);
        if denied {
            assert_eq!(
                fs::read_to_string(stage.join("unfinished.txt")).unwrap(),
                "recoverable"
            );
        } else {
            // Privileged callers can bypass directory permissions and must clean up.
            assert!(!stage.exists());
        }
    });
    assert_eq!(
        fs::read_to_string(temp.path().join("dist/keep.txt")).unwrap(),
        "previous site"
    );
}

#[cfg(unix)]
#[test]
fn inspected_but_unreadable_text_reports_permission_denied_without_mutation() {
    // Metadata can be readable even when opening the file is forbidden.
    let temp = tempdir();
    let path = temp.path().join("private.txt");
    fs::write(&path, "private source").unwrap();
    with_mode(&path, 0o000, || {
        let denied = fs::File::open(&path).is_err();
        let result = read_text(&path);
        if denied {
            assert_eq!(
                result
                    .unwrap_err()
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
        } else {
            assert_eq!(result.unwrap(), "private source");
        }
    });
    assert_eq!(fs::read_to_string(&path).unwrap(), "private source");
}

#[cfg(unix)]
#[test]
fn unreadable_subdirectories_fail_the_inventory_instead_of_omitting_sources() {
    let temp = tempdir();
    let nested = temp.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("source.txt"), "required source").unwrap();
    with_mode(&nested, 0o000, || {
        let denied = fs::read_dir(&nested).is_err();
        let result = super::files(temp.path(), false);
        if denied {
            let error = result.unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<walkdir::Error>()
                    .unwrap()
                    .io_error()
                    .unwrap()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
        } else {
            assert_eq!(result.unwrap(), vec![nested.join("source.txt")]);
        }
    });
    assert_eq!(
        fs::read_to_string(nested.join("source.txt")).unwrap(),
        "required source"
    );
}

#[cfg(unix)]
#[test]
fn denied_output_directory_creation_does_not_create_a_partial_destination() {
    let temp = tempdir();
    with_mode(temp.path(), 0o555, || {
        let denied = fs::create_dir(temp.path().join("permission-probe")).is_err();
        let result = output_file(temp.path(), "nested/output.txt");
        if denied {
            assert_eq!(
                result
                    .unwrap_err()
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
            assert!(!temp.path().join("nested").exists());
        } else {
            drop(result.unwrap());
            assert_eq!(
                fs::read(temp.path().join("nested/output.txt")).unwrap(),
                b""
            );
        }
    });
}

#[cfg(unix)]
#[test]
fn denied_backup_rename_preserves_the_live_site_and_does_not_promote_the_stage() {
    let temp = tempdir();
    owned_output(temp.path());
    let transaction = Transaction::begin(temp.path(), "dist").unwrap();
    fs::write(transaction.stage().join("new.txt"), "new site").unwrap();
    with_mode(temp.path(), 0o555, || {
        let denied = fs::File::create(temp.path().join("permission-probe")).is_err();
        let result = transaction.commit();
        if denied {
            assert_eq!(
                result
                    .unwrap_err()
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
            assert_eq!(
                fs::read_to_string(temp.path().join("dist/keep.txt")).unwrap(),
                "previous site"
            );
            assert!(!temp.path().join("dist/new.txt").exists());
            assert!(!temp.path().join(".regen-previous").exists());
        } else {
            assert_eq!(result.unwrap(), temp.path().join("dist"));
            assert_eq!(
                fs::read_to_string(temp.path().join("dist/new.txt")).unwrap(),
                "new site"
            );
            assert!(!temp.path().join(".regen-previous").exists());
        }
    });
}
