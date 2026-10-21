//! `audioc`: the Audio Constructor command line.

mod bench;
mod blocks;
mod devices;
mod docs;
mod inspect;
mod live;
mod plugins;
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
    /// List audio hosts and devices, MIDI ports, gamepads, and how OSC works.
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
    /// Write a group (or a block file) as a block file to share, with a preview beside it.
    BlockExport(blocks::ExportArgs),
    /// Import blocks, packs, or a whole library from a link.
    BlockImport(blocks::ImportArgs),
    /// Write the `index.json` that shares a folder of blocks from a web host.
    LibraryIndex(blocks::IndexArgs),
    /// Bundle a project and its audio files into one `.acpack` to share.
    Pack(blocks::PackArgs),
    /// Unpack an `.acpack` into a folder.
    Unpack(blocks::UnpackArgs),
    /// Print the version; with `--check`, also ask whether a newer release
    /// is out (sends nothing but the request).
    Version {
        #[arg(long)]
        check: bool,
    },
    /// List the CLAP and VST3 plugins installed here (scanning their folders).
    Plugins(plugins::PluginsArgs),
    /// Describe one plugin bundle as JSON (what `plugins` runs per bundle).
    #[command(hide = true)]
    ScanBundle { path: PathBuf },
    /// Rewrite project and block files in canonical form.
    Fmt {
        files: Vec<PathBuf>,
        /// Report files that aren't canonical instead of rewriting them.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    // A bug that panics leaves a report the editor offers to send.
    if let Some(dir) = ac_io::crash::default_dir() {
        ac_io::crash::install(dir, "audioc");
    }
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
        Command::BlockExport(args) => blocks::export(args),
        Command::BlockImport(args) => blocks::import(args),
        Command::LibraryIndex(args) => blocks::index(args),
        Command::Pack(args) => blocks::pack(args),
        Command::Unpack(args) => blocks::unpack(args),
        Command::Version { check } => version(check),
        Command::Plugins(args) => plugins::list(args),
        Command::ScanBundle { path } => return plugins::scan_bundle(&path),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn version(check: bool) -> anyhow::Result<ExitCode> {
    use ac_io::update::{self, Check, Version};
    println!("audioc {}", env!("CARGO_PKG_VERSION"));
    if check {
        match update::check(&update::feed_url(), Version::current()).map_err(anyhow::Error::msg)? {
            Check::Newer(release) => {
                println!("A newer release is out: {}", release.version);
                println!("Download it from {}", release.page);
            }
            Check::UpToDate(latest) => {
                println!("This is the newest release (the latest is {latest}).");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
