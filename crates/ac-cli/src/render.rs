//! `audioc render`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use ac_engine::{RenderOptions, Severity, render};
use ac_io::{BitDepth, read_wav, write_wav};
use anyhow::{Result, anyhow};
use clap::Args;

use crate::util::{load, parse_set, parse_time};

#[derive(Debug, Args)]
pub struct RenderArgs {
    /// The project to render.
    project: PathBuf,
    /// Output WAV file.
    #[arg(short, long)]
    output: PathBuf,
    /// Start time (seconds or m:ss).
    #[arg(long, default_value = "0")]
    from: String,
    /// End time (default: the project length).
    #[arg(long)]
    to: Option<String>,
    /// Sample rate (default: the project's).
    #[arg(long)]
    sample_rate: Option<u32>,
    /// Block size (default: the project's render block size).
    #[arg(long)]
    block_size: Option<usize>,
    /// 16, 24, or 32f.
    #[arg(long, default_value = "24")]
    bit_depth: String,
    /// Override a parameter: node.socket=value (repeatable).
    #[arg(long = "set", value_name = "NODE.SOCKET=VALUE")]
    set: Vec<String>,
    /// Feed this WAV file to AudioIn nodes as if from a device.
    #[arg(long)]
    input: Option<PathBuf>,
    /// Print nothing but errors.
    #[arg(short, long)]
    quiet: bool,
}

pub fn run(args: RenderArgs) -> Result<ExitCode> {
    let registry = Arc::new(ac_nodes::registry());
    let (project, resources) = load(&args.project, &registry)?;
    let depth = BitDepth::parse(&args.bit_depth)
        .ok_or_else(|| anyhow!("--bit-depth must be 16, 24, or 32f"))?;
    let set = args
        .set
        .iter()
        .map(|s| parse_set(s))
        .collect::<Result<Vec<_>>>()?;
    let input = match &args.input {
        Some(p) => Some(read_wav(p).map_err(|e| anyhow!(e))?),
        None => None,
    };
    let opts = RenderOptions {
        sample_rate: args.sample_rate,
        block_size: args.block_size,
        from: parse_time(&args.from)?,
        to: args.to.as_deref().map(parse_time).transpose()?,
        input,
        set,
        ..RenderOptions::default()
    };
    let t0 = Instant::now();
    let out = render(&project, registry, resources, &opts);
    let elapsed = t0.elapsed().as_secs_f64();
    for d in &out.report.diagnostics {
        if d.severity >= Severity::Warning || !args.quiet {
            eprintln!("{d}");
        }
    }
    write_wav(&args.output, &out.channels, out.sample_rate, depth).map_err(|e| anyhow!(e))?;
    if !args.quiet {
        let secs = out.frames() as f64 / f64::from(out.sample_rate);
        println!(
            "rendered {:.3} s × {} ch at {} Hz ({}) in {:.3} s ({:.0}× real time) → {}",
            secs,
            out.channels.len(),
            out.sample_rate,
            depth,
            elapsed,
            if elapsed > 0.0 { secs / elapsed } else { 0.0 },
            args.output.display()
        );
        for (c, ch) in out.channels.iter().enumerate() {
            println!(
                "  ch{c}: peak {:.4} ({:.1} dBFS), rms {:.4}",
                ac_core::testing::peak(ch),
                ac_dsp::util::gain_to_db(ac_core::testing::peak(ch)),
                ac_core::testing::rms(ch)
            );
        }
    }
    Ok(if out.report.has_errors() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    })
}
