//! `audioc`: the Audio Constructor command line.

mod docs;
mod inspect;
mod render;
mod util;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Render, run, and inspect Audio Constructor projects.
#[derive(Debug, Parser)]
#[command(name = "audioc", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render a project offline to a WAV file.
    Render(render::RenderArgs),
    /// Compile a project and print problems, disabled nodes, and latency.
    Validate {
        project: PathBuf,
        /// Sample rate to compile at (default: the project's).
        #[arg(long)]
        sample_rate: Option<u32>,
    },
    /// Print a project's settings, nodes, wires, and groups.
    Inspect { project: PathBuf },
    /// List every node kind, or generate the node reference.
    Nodes {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Write Markdown reference pages into this directory.
        #[arg(long, value_name = "DIR")]
        docs: Option<PathBuf>,
    },
    /// Rewrite project files in canonical form.
    Fmt {
        files: Vec<PathBuf>,
        /// Report files that aren't canonical instead of rewriting them.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Render(args) => render::run(args),
        Command::Validate {
            project,
            sample_rate,
        } => inspect::validate(&project, sample_rate),
        Command::Inspect { project } => inspect::inspect(&project),
        Command::Nodes { json, docs } => docs::nodes(json, docs.as_deref()),
        Command::Fmt { files, check } => inspect::fmt(&files, check),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
