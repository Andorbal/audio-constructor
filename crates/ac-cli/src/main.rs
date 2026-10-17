//! `audioc`: the Audio Constructor command line.

use clap::Parser;

/// Render, run, and inspect Audio Constructor projects.
#[derive(Debug, Parser)]
#[command(name = "audioc", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
