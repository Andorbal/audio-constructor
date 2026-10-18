//! `audioc blocks` and `audioc block-update`: the block library from the
//! command line (ARCHITECTURE §12).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ac_core::{Block, BlockSource, update_def};
use ac_io::Library;
use anyhow::{Result, anyhow};
use clap::Args;
use serde_json::json;

#[derive(Debug, Args)]
pub struct BlocksArgs {
    /// Also list the user and imported blocks in this library folder.
    #[arg(long, value_name = "DIR")]
    library: Option<PathBuf>,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
}

fn library(dir: Option<&Path>) -> Library {
    match dir {
        Some(d) => Library::open(d),
        None => Library::shipped_only(),
    }
}

pub fn list(args: BlocksArgs) -> Result<ExitCode> {
    let lib = library(args.library.as_deref());
    for issue in &lib.issues {
        eprintln!("warning: {issue}");
    }
    let blocks = lib.search("");
    if args.json {
        let v: Vec<_> = blocks
            .iter()
            .map(|b| {
                let d = &b.block.def;
                json!({
                    "source": b.source.to_string(),
                    "name": d.name,
                    "version": d.version,
                    "tags": d.tags,
                    "description": d.description,
                    "inputs": d.inputs.iter().map(|i| &i.name).collect::<Vec<_>>(),
                    "outputs": d.outputs.iter().map(|o| &o.name).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        for b in blocks {
            let d = &b.block.def;
            println!(
                "{:<32} v{:<3} {}{}",
                b.source.to_string(),
                d.version,
                d.name,
                if d.tags.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", d.tags.join(", "))
                }
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Projects to update.
    projects: Vec<PathBuf>,
    /// Also use the user and imported blocks in this library folder.
    #[arg(long, value_name = "DIR")]
    library: Option<PathBuf>,
    /// Refresh every copy that differs from the library, even at the same version.
    #[arg(long)]
    force: bool,
    /// Report what would change without writing.
    #[arg(long)]
    check: bool,
}

/// Bring the projects' copies of library blocks up to date.
pub fn update(args: UpdateArgs) -> Result<ExitCode> {
    let lib = library(args.library.as_deref());
    let mut stale = 0;
    for path in &args.projects {
        let mut project = ac_io::load_project(path).map_err(|e| anyhow!(e))?;
        let mut changed = Vec::new();
        let ids: Vec<_> = project.group_defs.keys().cloned().collect();
        for id in ids {
            let Some(source) = project.group_defs[&id]
                .source
                .as_deref()
                .and_then(BlockSource::parse)
            else {
                continue;
            };
            let Some(lb) = lib.get(&source) else {
                continue;
            };
            let current = Block::from_project(&project, &id);
            let newer = lb.block.def.version > project.group_defs[&id].version;
            let differs = current.as_ref() != Some(&lb.block);
            if newer || (args.force && differs) {
                let from = project.group_defs[&id].version;
                let report = update_def(&mut project, &id, &lb.block, &source);
                changed.push(format!(
                    "{source} v{from} → v{}{}",
                    lb.block.def.version,
                    if report.dropped_wires.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " ({} wires to removed sockets dropped)",
                            report.dropped_wires.len()
                        )
                    }
                ));
            }
        }
        if changed.is_empty() {
            continue;
        }
        stale += 1;
        for c in &changed {
            println!("{}: {c}", path.display());
        }
        if !args.check {
            ac_io::save_project(path, &project).map_err(|e| anyhow!(e))?;
        }
    }
    Ok(if args.check && stale > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
