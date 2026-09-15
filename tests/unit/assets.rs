//! Asset-stream and staging failure contracts below the full-build boundary.
//!
//! Real handles, bounded writers, and occupied directories exercise incomplete I/O
//! and failed renames. Digest assertions defend the URL's byte identity, not merely
//! successful copying; a failed stage remains the transaction owner's responsibility.

use super::copy_asset;
use crate::test_support::tempdir;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::Path;

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
    let first = super::prepare(root.path(), first_stage.path()).unwrap();
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
    let second = super::prepare(root.path(), second_stage.path()).unwrap();
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
    let error = super::prepare(root.path(), stage.path()).err().unwrap();
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
    let error = super::prepare(root.path(), stage.path()).err().unwrap();
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
