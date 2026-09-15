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
    let mut output = Vec::new();
    let mut digest = Sha256::new();
    copy_asset(
        &mut Cursor::new(&source),
        &mut output,
        source.len() as u64,
        &mut digest,
        &mut [0; 65536],
        Path::new("asset.bin"),
    )
    .unwrap();
    assert_eq!(output, source);
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
        let mut output = Vec::new();
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
        assert_eq!(output, changed);
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
    let mut output = Vec::new();
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
    assert!(output.is_empty());
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
