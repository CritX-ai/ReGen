//! CLI boundary for the library's build pipeline.
//!
//! Argument parsing and human-facing success/error reporting belong here; site
//! validation and output recovery remain shared with library callers.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "regen",
    version,
    about = "Reproducible multilingual static sites from Tera and YAML"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a site using an inherited profile and recoverable output replacement.
    Build {
        /// Directory containing regen.toml, templates/ and content/.
        #[arg(long, default_value = ".")]
        site: PathBuf,
        /// Build profile; overrides build.profile in regen.toml.
        #[arg(long)]
        profile: Option<String>,
        /// Write review/ using the selected profile (use --profile release for final review).
        #[arg(long)]
        review: bool,
        /// Override rendered HTML minification (off by default).
        #[arg(long, action = clap::ArgAction::Set, value_name = "BOOL")]
        minify_html: Option<bool>,
        /// Override CSS compact printing (enabled for release when compiled).
        #[arg(long, action = clap::ArgAction::Set, value_name = "BOOL")]
        minify_css: Option<bool>,
        /// Override JavaScript minification (off by default).
        #[arg(long, action = clap::ArgAction::Set, value_name = "BOOL")]
        minify_js: Option<bool>,
        /// Override the asset minifier gate; does not enable CSS/JS minification.
        #[arg(long, action = clap::ArgAction::Set, value_name = "BOOL")]
        minify_assets: Option<bool>,
        /// Check minification: true rejects differences, warn reports them, false skips checks.
        #[arg(long, value_name = "true|false|warn")]
        regression_checks: Option<regen::RegressionCheckMode>,
    },
}

fn main() {
    let Cli {
        command:
            Command::Build {
                site,
                profile,
                review,
                minify_html,
                minify_css,
                minify_js,
                minify_assets,
                regression_checks,
            },
    } = Cli::parse();
    let options = regen::BuildOptions {
        profile: profile.as_deref(),
        review,
        minify_html,
        minify_css,
        minify_js,
        minify_assets,
        regression_checks,
    };
    match regen::build_with_options(&site, &options) {
        Ok(summary) => println!(
            "Built {} pages in {} languages and {} assets into {}",
            summary.pages,
            summary.languages,
            summary.assets,
            summary.output.display()
        ),
        Err(error) => {
            eprintln!("error: {error:#}");
            std::process::exit(1);
        }
    }
}
