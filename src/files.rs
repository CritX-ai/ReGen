//! Filesystem policy and recoverable installation of generated output.
//!
//! Centralize portable names, regular-file inventories, bounded text reads, and
//! exclusive output creation so every input/output path obeys the same rules.
//! Transactions protect existing sites, not against hostile concurrent mutation:
//! callers must keep the site tree stable between inspection and filesystem access.

use anyhow::{Context, Result, bail, ensure};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Per-file UTF-8 input bound; binary assets are streamed without this text limit.
const TEXT_LIMIT: u64 = 8 * 1024 * 1024;

/// A portable URL/file alphabet prevents aliases on case-insensitive filesystems.
pub(crate) fn portable_path(path: &str) -> Result<()> {
    ensure!(!path.is_empty(), "path must not be empty");
    for part in path.split('/') {
        ensure!(
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b)),
            "non-portable path: {path}; use lowercase ASCII letters, digits, '-', '_' and '.' in normal path segments"
        );
        let stem = part.split('.').next().unwrap_or_default();
        ensure!(
            !matches!(stem, "con" | "prn" | "aux" | "nul")
                && !(stem.len() == 4
                    && (stem.starts_with("com") || stem.starts_with("lpt"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "reserved Windows path: {path}"
        );
    }
    Ok(())
}

/// Reject symlinks in every component, returning `None` at the first missing one.
///
/// Returned metadata describes the final component only when the full path exists.
/// This inspection is not a race-free capability for later opens or renames.
pub(crate) fn reject_symlinks(path: &Path) -> Result<Option<fs::Metadata>> {
    let mut prefix = PathBuf::new();
    let mut inspected = None;
    for part in path.components() {
        prefix.push(part);
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) => {
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "symlink not permitted: {}",
                    prefix.display()
                );
                inspected = Some(metadata);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot inspect {}", prefix.display()));
            }
        }
    }
    Ok(inspected)
}

/// Collect regular files in deterministic traversal order after validating the tree.
///
/// `optional` permits an absent root, never an existing root of the wrong type.
pub(crate) fn files(root: &Path, optional: bool) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    visit_tree(root, optional, |entry| {
        if entry.file_type().is_file() {
            paths.push(entry.into_path());
        }
        Ok(())
    })?;
    Ok(paths)
}

/// Visit validated regular files and directories in deterministic relative order.
///
/// Excludes the root itself and visits parents before children, including empty
/// directories. Invalid names, symlinks, special files, and traversal errors abort
/// the inventory rather than silently omitting sources.
pub(crate) fn visit_tree(
    root: &Path,
    optional: bool,
    mut visitor: impl FnMut(walkdir::DirEntry) -> Result<()>,
) -> Result<()> {
    let metadata = reject_symlinks(root)?;
    if optional && metadata.is_none() {
        return Ok(());
    }
    ensure!(
        metadata.is_some_and(|metadata| metadata.is_dir()),
        "required directory missing: {}",
        root.display()
    );
    for entry in WalkDir::new(root).follow_links(false).sort_by_file_name() {
        let entry = entry.with_context(|| format!("cannot walk {}", root.display()))?;
        if entry.path() == root {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("WalkDir yields descendants");
        let relative = relative
            .to_str()
            .context("paths must be UTF-8")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        portable_path(&relative)?;
        let kind = entry.file_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "only regular files and directories permitted: {}",
            entry.path().display()
        );
        visitor(entry)?;
    }
    Ok(())
}

/// Read a regular UTF-8 file within the text limit, rejecting symlinked ancestors.
pub(crate) fn read_text(path: &Path) -> Result<String> {
    let metadata = reject_symlinks(path)?
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
        .with_context(|| format!("cannot inspect {}", path.display()))?;
    ensure!(metadata.is_file(), "not a regular file: {}", path.display());
    ensure!(
        metadata.len() <= TEXT_LIMIT,
        "text input exceeds 8 MiB: {}",
        path.display()
    );
    read_text_stream(File::open(path)?, path)
}

/// Enforce the limit on bytes actually read, even if a file grows after inspection.
fn read_text_stream(input: impl Read, path: &Path) -> Result<String> {
    let mut text = String::new();
    input
        .take(TEXT_LIMIT + 1)
        .read_to_string(&mut text)
        .with_context(|| format!("cannot read UTF-8 text {}", path.display()))?;
    ensure!(
        text.len() as u64 <= TEXT_LIMIT,
        "text input exceeds 8 MiB: {}",
        path.display()
    );
    Ok(text)
}

/// Create a portable output path exclusively, creating required parent directories.
///
/// Existing files and file/directory conflicts are errors, never overwrite targets.
/// The caller owns `root` and must prevent concurrent mutation of the output tree.
pub(crate) fn output_file(root: &Path, relative: &str) -> Result<File> {
    portable_path(relative)?;
    let destination = root.join(relative);
    reject_symlinks(&destination)?;
    fs::create_dir_all(
        destination
            .parent()
            .expect("a portable relative file has a parent"),
    )?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .with_context(|| {
            format!(
                "cannot create output (possible route collision): {}",
                destination.display()
            )
        })
}

/// Write one new output file with the same collision policy as `output_file`.
pub(crate) fn write_output(root: &Path, relative: &str, bytes: &[u8]) -> Result<()> {
    output_file(root, relative)?
        .write_all(bytes)
        .with_context(|| format!("cannot write output {relative}"))
}

/// The exclusive staging directory doubles as a per-site build lock.
/// Nothing automatically deletes a stage or backup left by another invocation.
pub(crate) struct Transaction {
    output: PathBuf,
    stage: PathBuf,
    previous: PathBuf,
    /// Promotion succeeded; dropping must no longer try to discard the stage.
    committed: bool,
}

/// Recognize ReGen output without retaining the potentially large file inventory.
#[derive(serde::Deserialize)]
struct OutputMarker {
    generator: String,
    format: u32,
}

impl Transaction {
    /// Reserve a stage and refuse replacement of unowned or interrupted output.
    ///
    /// Ownership is a recognized manifest marker, not an integrity or authenticity
    /// check. Existing recovery directories require a human decision before reuse.
    pub(crate) fn begin(root: &Path) -> Result<Self> {
        let output = root.join("dist");
        let stage = root.join(".regen-stage");
        let previous = root.join(".regen-previous");
        let has_output = reject_symlinks(&output)?.is_some();
        reject_symlinks(&stage)?;
        ensure!(
            reject_symlinks(&previous)?.is_none(),
            ".regen-previous exists; recover the interrupted build before building (see docs/architecture.md)"
        );
        fs::create_dir(&stage).context("cannot reserve .regen-stage; another build may be active, or an interrupted stage needs manual recovery")?;
        let transaction = Self {
            output,
            stage,
            previous,
            committed: false,
        };
        if has_output {
            files(&transaction.output, false)?;
            let marker_path = transaction.output.join("regen-manifest.json");
            // Stream past the file inventory; large sites must not need a second
            // in-memory copy of their generated manifest just to establish ownership.
            let marker: OutputMarker = serde_json::from_reader(BufReader::new(File::open(&marker_path)
                .context("refusing to replace dist without a ReGen manifest; choose a dedicated site directory")?))?;
            ensure!(
                marker.generator == "ReGen" && marker.format == 1,
                "refusing to replace dist without a recognized ReGen manifest"
            );
        }
        Ok(transaction)
    }

    /// Private output workspace owned by this transaction until promotion.
    pub(crate) fn stage(&self) -> &Path {
        &self.stage
    }

    /// Install the completed stage, restoring the old output if promotion fails.
    ///
    /// Backup cleanup errors occur after installation; callers must not interpret
    /// every error as proof that the previous output is still live.
    pub(crate) fn commit(mut self) -> Result<PathBuf> {
        let has_output = reject_symlinks(&self.output)?.is_some();
        ensure!(
            reject_symlinks(&self.previous)?.is_none(),
            ".regen-previous appeared during generation; output not replaced"
        );
        if has_output {
            fs::rename(&self.output, &self.previous).context("cannot move previous dist aside")?;
        }
        self.promote(has_output)?;
        // Only remove the backup this invocation actually created.
        if has_output {
            fs::remove_dir_all(&self.previous).context(
                "new site installed, but old .regen-previous cleanup failed; recover manually",
            )?;
        }
        Ok(self.output.clone())
    }

    /// Keep failed promotion and recovery together so neither can lose the backup.
    fn promote(&mut self, has_previous: bool) -> Result<()> {
        if let Err(error) = fs::rename(&self.stage, &self.output) {
            if has_previous && let Err(rollback) = fs::rename(&self.previous, &self.output) {
                bail!(
                    "output replacement failed: {error}; rollback failed: {rollback}; recover .regen-previous manually"
                );
            }
            return Err(error).context("cannot promote generated site");
        }
        self.committed = true;
        Ok(())
    }
}

// Drop may remove only this invocation's unpromoted stage, never a recovery backup.
// Cleanup failure is reported without replacing the original build error.
impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.committed
            && self.stage.is_dir()
            && let Err(error) = fs::remove_dir_all(&self.stage)
        {
            eprintln!(
                "warning: cannot remove own failed stage {}: {error}",
                self.stage.display()
            );
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/files.rs"]
mod tests;
