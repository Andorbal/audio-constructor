//! `audioc`: the Audio Constructor command line.

mod bench;
mod blocks;
mod devices;
mod docs;
mod inspect;
mod live;
mod render;
mod util;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::{Parser, Subcommand};

// Debug builds abort on any allocation inside the real-time callbacks
// (everything wrapped in `assert_no_alloc`), so `audioc run` doubles as a
// real-time hygiene check.
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

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
    /// Run a project live on the audio devices until Ctrl+C.
    Run(live::RunArgs),
    /// List audio hosts and devices, and MIDI inputs.
    Devices,
    /// Measure round-trip latency through a loopback cable.
    Latency(live::LatencyArgs),
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
    /// Render stress projects and report the real-time ratio per node and overall.
    Bench(bench::BenchArgs),
    /// List the block library.
    Blocks(blocks::BlocksArgs),
    /// Update projects' copies of library blocks to the library's versions.
    BlockUpdate(blocks::UpdateArgs),
    /// Bundle a project and its audio files into one `.acpack` to share.
    Pack(blocks::PackArgs),
    /// Unpack an `.acpack` into a folder.
    Unpack(blocks::UnpackArgs),
    /// Rewrite project and block files in canonical form.
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
        Command::Run(args) => {
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            if let Err(e) = ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed)) {
                eprintln!("warning: Ctrl+C won't stop cleanly ({e}); use --duration");
            }
            live::run(args, &stop)
        }
        Command::Devices => devices::devices(),
        Command::Latency(args) => live::latency(args),
        Command::Validate {
            project,
            sample_rate,
        } => inspect::validate(&project, sample_rate),
        Command::Inspect { project } => inspect::inspect(&project),
        Command::Nodes { json, docs } => docs::nodes(json, docs.as_deref()),
        Command::Fmt { files, check } => inspect::fmt(&files, check),
        Command::Bench(args) => bench::run(args),
        Command::Blocks(args) => blocks::list(args),
        Command::BlockUpdate(args) => blocks::update(args),
        Command::Pack(args) => blocks::pack(args),
        Command::Unpack(args) => blocks::unpack(args),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
