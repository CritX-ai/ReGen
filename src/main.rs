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
    /// Build a complete optimized site in SITE/dist with recoverable output replacement.
    Build {
        /// Directory containing regen.toml, templates/ and content/.
        #[arg(long, default_value = ".")]
        site: PathBuf,
    },
}

fn main() {
    let Cli {
        command: Command::Build { site },
    } = Cli::parse();
    match regen::build(&site) {
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
