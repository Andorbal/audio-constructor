//! `audioc render`.

use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use ac_engine::{RenderOptions, RenderedFile, Severity, render};
use ac_io::{BitDepth, read_wav, write_midi_file, write_wav};
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
    /// Write what MIDI Out nodes send to this standard MIDI file.
    #[arg(long)]
    midi_out: Option<PathBuf>,
    /// Print nothing but errors.
    #[arg(short, long)]
    quiet: bool,
}

/// Where a File Out node's file goes: its config path, relative to `base`
/// (the main output's folder), with `.wav` added if it has no extension.
/// Paths that could reach outside `base` are refused, so a shared project
/// can't write anywhere else on the machine.
pub fn file_out_path(base: &Path, path: &str) -> Result<PathBuf, String> {
    let rel = Path::new(path.trim());
    if rel.as_os_str().is_empty() {
        return Err("no path".into());
    }
    for c in rel.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return Err(format!("\"{path}\" leaves the output folder")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("\"{path}\" must be relative to the output folder"));
            }
        }
    }
    let mut full = base.join(rel);
    if full.extension().is_none() {
        full.set_extension("wav");
    }
    Ok(full)
}

fn write_file_out(path: &Path, file: &RenderedFile, depth: BitDepth) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| anyhow!("couldn't create {}: {e}", dir.display()))?;
    }
    write_wav(path, &file.channels, file.sample_rate, depth).map_err(|e| anyhow!(e))
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
    let base = args
        .output
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut written = Vec::new();
    for file in &out.files {
        match file_out_path(&base, &file.path) {
            Ok(path) if path == args.output || written.iter().any(|(p, _)| p == &path) => {
                eprintln!(
                    "warning: File Out \"{}\" writes {} again; skipped",
                    file.name,
                    path.display()
                );
            }
            Ok(path) => {
                write_file_out(&path, file, depth)?;
                written.push((path, file));
            }
            Err(why) => eprintln!("warning: File Out \"{}\": {why}; skipped", file.name),
        }
    }
    if let Some(path) = &args.midi_out {
        let start = (opts.from.max(0.0) * f64::from(out.sample_rate)).round() as u64;
        write_midi_file(
            path,
            &out.midi,
            start,
            f64::from(out.sample_rate),
            &project.settings.tempo,
        )
        .map_err(|e| anyhow!("couldn't write {}: {e}", path.display()))?;
    }
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
        for (path, file) in &written {
            let peak = file
                .channels
                .iter()
                .map(|c| ac_core::testing::peak(c))
                .fold(0.0, f32::max);
            println!(
                "  File Out \"{}\": {} ch, peak {:.4} → {}",
                file.name,
                file.channels.len(),
                peak,
                path.display()
            );
        }
        if let Some(path) = &args.midi_out {
            println!("  {} MIDI messages → {}", out.midi.len(), path.display());
        }
    }
    Ok(if out.report.has_errors() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_out_paths_stay_in_the_output_folder() {
        let base = Path::new("out");
        assert_eq!(
            file_out_path(base, "drums").unwrap(),
            Path::new("out/drums.wav")
        );
        assert_eq!(
            file_out_path(base, "stems/bass.wav").unwrap(),
            Path::new("out/stems/bass.wav")
        );
        assert_eq!(
            file_out_path(base, "./x.aiff").unwrap(),
            Path::new("out/./x.aiff")
        );
        assert!(file_out_path(base, "").is_err());
        assert!(file_out_path(base, "../up.wav").is_err());
        assert!(file_out_path(base, "a/../../up.wav").is_err());
        assert!(file_out_path(base, "/etc/x.wav").is_err());
    }
}
