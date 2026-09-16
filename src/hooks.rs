//! Trusted host hooks run as literal argv, with the primary build outcome fixed.

use std::fmt::{self, Write as _};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, ensure};

use crate::config::{HookWhen, ResolvedBuild};

/// Stop on the first required failure; optional failures only warn.
pub(crate) fn run_pre(settings: &ResolvedBuild, root: &Path, output: &Path) -> Result<()> {
    for (index, hook) in settings.hooks.pre.iter().enumerate() {
        let result = execute(&hook.command, settings, root, output, "pre", None)
            .with_context(|| format!("pre hook {} ({:?}) failed", index + 1, hook.command[0]));
        if let Err(error) = result {
            if !hook.allow_failure {
                return Err(error);
            }
            eprintln!("warning: {error:#}");
        }
    }
    Ok(())
}

/// Run every eligible hook against the original build result, even after failures.
///
/// Required failures are returned to the transaction owner; successful installation
/// cannot be undone by this runner. The first required failure retains its cause,
/// while subsequent failures are attached as additional diagnostics.
pub(crate) fn run_post(
    settings: &ResolvedBuild,
    root: &Path,
    output: &Path,
    build_error: Option<&anyhow::Error>,
) -> Result<()> {
    let failed = build_error.is_some();
    let status = if failed { "failure" } else { "success" };
    let mut details = None;
    let mut failure: Option<anyhow::Error> = None;
    for (index, hook) in settings.hooks.post.iter().enumerate() {
        if !match hook.when {
            HookWhen::Always => true,
            HookWhen::Success => !failed,
            HookWhen::Failure => failed,
        } {
            continue;
        }
        let error_details = if hook.error_details {
            build_error.map(|error| details.get_or_insert_with(|| ErrorDetails::new(error)))
        } else {
            None
        };
        let result = execute(
            &hook.command,
            settings,
            root,
            output,
            status,
            error_details.as_deref(),
        )
        .with_context(|| format!("post hook {} ({:?}) failed", index + 1, hook.command[0]));
        if let Err(error) = result {
            if hook.allow_failure {
                eprintln!("warning: {error:#}");
            } else {
                failure = Some(match failure {
                    Some(first) => {
                        first.context(format!("additional post-hook failure: {error:#}"))
                    }
                    None => error,
                });
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn execute(
    argv: &[String],
    settings: &ResolvedBuild,
    root: &Path,
    output: &Path,
    status: &str,
    details: Option<&ErrorDetails>,
) -> Result<()> {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env("REGEN_SITE", root)
        .env("REGEN_OUTPUT", output)
        .env("REGEN_PROFILE", &settings.profile)
        .env("REGEN_STATUS", status)
        .env_remove("REGEN_ERROR")
        .env_remove("REGEN_ERROR_TRUNCATED");
    if let Some(details) = details {
        command.env("REGEN_ERROR", &details.text).env(
            "REGEN_ERROR_TRUNCATED",
            if details.truncated { "1" } else { "0" },
        );
    }
    let status = command.status().context("cannot execute hook process")?;
    ensure!(status.success(), "hook process ended with {status}");
    Ok(())
}

/// Bound formatting itself, not just its final environment value. Returning a
/// formatting error stops walking an arbitrarily long chain once the limit is hit.
struct ErrorDetails {
    text: String,
    truncated: bool,
}

impl ErrorDetails {
    fn new(error: &anyhow::Error) -> Self {
        let mut details = Self {
            text: String::new(),
            truncated: false,
        };
        if write!(&mut details, "{error:#}").is_err() {
            details.truncated = true;
        }
        details
    }

    fn append(&mut self, text: &str) -> fmt::Result {
        const LIMIT: usize = 8192;
        let remaining = LIMIT - self.text.len();
        if text.len() <= remaining {
            self.text.push_str(text);
            return Ok(());
        }
        let mut boundary = remaining;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        self.text.push_str(&text[..boundary]);
        self.truncated = true;
        Err(fmt::Error)
    }
}

impl fmt::Write for ErrorDetails {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        // OS environment values cannot contain NUL. Escape it before applying
        // the byte bound so malformed content cannot disable failure reporting.
        let mut parts = text.split('\0');
        self.append(parts.next().expect("split always yields a first part"))?;
        for part in parts {
            self.append("\\0")?;
            self.append(part)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/hooks.rs"]
mod tests;
