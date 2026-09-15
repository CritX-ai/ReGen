/// Resolve the OS-owned temp parent, never fixture inputs or intentional symlinks.
pub fn tempdir() -> tempfile::TempDir {
    let parent = std::env::temp_dir()
        .canonicalize()
        .expect("canonicalize the system temporary directory");
    tempfile::tempdir_in(parent).expect("create a temporary fixture directory")
}
