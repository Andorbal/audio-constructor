//! `audioc blocks`, `block-update`, `pack`, and `unpack`: the block library
//! and project packs from the command line (ARCHITECTURE §12).

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
            let differs = !current.is_some_and(|c| c.same_definition(&lb.block));
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

#[derive(Debug, Args)]
pub struct PackArgs {
    /// The project to pack.
    project: PathBuf,
    /// The pack to write (default: next to the project, `<name>.acpack`).
    #[arg(short, long)]
    output: Option<PathBuf>,
}

/// `audioc pack`: a project and its audio files in one file to share.
pub fn pack(args: PackArgs) -> Result<ExitCode> {
    let registry = ac_nodes::registry();
    let out = args
        .output
        .unwrap_or_else(|| args.project.with_extension(ac_io::PACK_EXTENSION));
    let r = ac_io::write_pack(&args.project, &out, &registry).map_err(|e| anyhow!(e))?;
    for m in &r.missing {
        eprintln!("warning: {m} is missing; the pack doesn't include it");
    }
    for (was, now) in &r.moved {
        println!("copied {was} into the pack as {now}");
    }
    println!("packed {} files → {}", r.files, out.display());
    Ok(if r.missing.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

#[derive(Debug, Args)]
pub struct UnpackArgs {
    /// The pack to unpack.
    pack: PathBuf,
    /// The folder to unpack into (default: a new folder next to the pack).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Overwrite files that are already there.
    #[arg(long)]
    force: bool,
}

/// `audioc unpack`: the project and audio files from a pack.
pub fn unpack(args: UnpackArgs) -> Result<ExitCode> {
    let project = match &args.output {
        Some(dir) => {
            let bytes =
                std::fs::read(&args.pack).map_err(|e| anyhow!("{}: {e}", args.pack.display()))?;
            ac_io::unpack(&bytes, dir, args.force).map_err(|e| anyhow!(e))?
        }
        None => ac_io::unpack_beside(&args.pack).map_err(|e| anyhow!(e))?,
    };
    println!("unpacked → {}", project.display());
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// A project holding the group, or a block file to export again (to
    /// give it a preview, or a new name).
    file: PathBuf,
    /// The group definition to export, by id or name (not needed for a
    /// block file, or a project with one group).
    group: Option<String>,
    /// The block file to write (default: `<name>.acblock` here).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Don't render a preview beside it.
    #[arg(long)]
    no_preview: bool,
}

/// The definition `want` names in `project` (its id, or its name ignoring
/// case), or the only one there is.
fn pick_group(project: &ac_core::Project, want: Option<&str>) -> Result<ac_core::GroupId> {
    let defs = &project.group_defs;
    let names = || {
        defs.iter()
            .map(|(id, d)| format!("{} ({})", id.0, d.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match want {
        Some(w) => defs
            .keys()
            .find(|id| id.0 == w)
            .or_else(|| {
                defs.iter()
                    .find(|(_, d)| d.name.eq_ignore_ascii_case(w.trim()))
                    .map(|(id, _)| id)
            })
            .cloned()
            .ok_or_else(|| {
                if defs.is_empty() {
                    anyhow!("the project has no groups")
                } else {
                    anyhow!("the project has no group `{w}`; it has: {}", names())
                }
            }),
        None => match defs.len() {
            0 => Err(anyhow!("the project has no groups")),
            1 => defs
                .keys()
                .next()
                .cloned()
                .ok_or_else(|| anyhow!("the project has no groups")),
            _ => Err(anyhow!("say which group to export: {}", names())),
        },
    }
}

/// `audioc block-export`: a group (or a block file) as a block file to
/// share, with a preview rendered beside it.
pub fn export(args: ExportArgs) -> Result<ExitCode> {
    let is_block = args
        .file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ac_core::BLOCK_EXTENSION));
    let block = if is_block {
        if args.group.is_some() {
            return Err(anyhow!("a block file holds one block; leave out the group"));
        }
        let text = std::fs::read_to_string(&args.file)
            .map_err(|e| anyhow!("{}: {e}", args.file.display()))?;
        Block::from_json(&text).map_err(|e| anyhow!("{}: {e}", args.file.display()))?
    } else {
        let project = ac_io::load_project(&args.file).map_err(|e| anyhow!(e))?;
        let id = pick_group(&project, args.group.as_deref())?;
        Block::from_project(&project, &id).ok_or_else(|| anyhow!("no group `{}`", id.0))?
    };
    let out = args.output.unwrap_or_else(|| {
        PathBuf::from(format!(
            "{}.{}",
            ac_core::slug(&block.def.name),
            ac_core::BLOCK_EXTENSION
        ))
    });
    let registry = (!args.no_preview).then(|| std::sync::Arc::new(ac_nodes::registry()));
    let name = block.def.name.clone();
    let e = ac_io::export_block(block, &out, registry).map_err(|e| anyhow!(e))?;
    println!("exported {name} → {}", e.block.display());
    if let (Some(path), Some(p)) = (&e.preview, &e.rendered) {
        let channels = if p.channels.len() == 1 {
            "mono"
        } else {
            "stereo"
        };
        println!(
            "preview → {} ({:.1} s, {channels}, {} Hz)",
            path.display(),
            p.seconds(),
            p.sample_rate
        );
        for f in &p.feeds {
            println!("  {f}");
        }
        if p.reduced_db > 0.0 {
            println!("  turned down {:.1} dB to stay under −1 dBFS", p.reduced_db);
        }
        for w in &p.warnings {
            eprintln!("warning: {w}");
        }
    }
    if let Some(why) = &e.no_preview {
        eprintln!("warning: no preview: {why}");
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Args)]
pub struct IndexArgs {
    /// The folder of `.acblock` files (and their previews) to index.
    dir: PathBuf,
    /// The library's name (default: the existing index's, or the folder's).
    #[arg(long)]
    name: Option<String>,
    /// A sentence or two about the library.
    #[arg(long)]
    description: Option<String>,
    /// Report whether `index.json` is up to date instead of writing it.
    #[arg(long)]
    check: bool,
}

/// `audioc library-index`: write the `index.json` that shares a folder of
/// blocks from any web host.
pub fn index(args: IndexArgs) -> Result<ExitCode> {
    let registry = ac_nodes::registry();
    let b = ac_io::index::build_index(
        &args.dir,
        &registry,
        args.name.as_deref(),
        args.description.as_deref(),
    )
    .map_err(|e| anyhow!(e))?;
    for p in &b.problems {
        eprintln!("error: {p} (left out)");
    }
    for w in &b.warnings {
        eprintln!("warning: {w}");
    }
    let path = args.dir.join(ac_io::index::INDEX_FILE);
    let text = b.index.to_json();
    let previews = b
        .index
        .blocks
        .iter()
        .filter(|e| e.preview.is_some())
        .count();
    if args.check {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        if current == text && b.problems.is_empty() {
            println!("{} is up to date", path.display());
            return Ok(ExitCode::SUCCESS);
        }
        println!(
            "{} {}",
            path.display(),
            if current.is_empty() {
                "is missing"
            } else if current == text {
                "lists every block that loads, but some don't"
            } else {
                "is out of date; run without --check to rewrite it"
            }
        );
        return Ok(ExitCode::FAILURE);
    }
    let written = ac_io::index::write_index(&args.dir, &b.index).map_err(|e| anyhow!(e))?;
    println!(
        "indexed {} block{} ({previews} with a preview) → {}",
        b.index.blocks.len(),
        if b.index.blocks.len() == 1 { "" } else { "s" },
        written.display()
    );
    Ok(if b.problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// A link to an `.acblock`, an `.acpack`, or a library's `index.json`
    /// (https, or http on this computer).
    url: String,
    /// The library folder to import blocks into (default: the one the
    /// editor uses).
    #[arg(long, value_name = "DIR")]
    library: Option<PathBuf>,
    /// From a library index, only these blocks (by id; repeat for more).
    #[arg(long, value_name = "ID")]
    only: Vec<String>,
    /// List a library index's blocks instead of importing them.
    #[arg(long)]
    list: bool,
    /// Where a pack is saved and unpacked (default: here).
    #[arg(short, long, value_name = "DIR")]
    output: Option<PathBuf>,
}

/// `audioc block-import`: blocks, packs, and libraries from a link.
pub fn import(args: ImportArgs) -> Result<ExitCode> {
    use ac_io::remote::{Fetched, fetch_link};
    let fetched = fetch_link(&args.url).map_err(|e| anyhow!(e))?;
    let open_library = |dir: &Option<PathBuf>| -> Result<Library> {
        let dir = dir
            .clone()
            .or_else(ac_io::library::default_dir)
            .ok_or_else(|| anyhow!("this system has no data folder; give one with --library"))?;
        Ok(Library::open(&dir))
    };
    match fetched {
        Fetched::Block(b) => {
            let mut lib = open_library(&args.library)?;
            let line = import_one(&mut lib, *b)?;
            println!("{line}");
        }
        Fetched::Library(remote) => {
            for s in &remote.skipped {
                eprintln!("warning: the index's {s}; skipped");
            }
            let entries: Vec<_> = remote
                .index
                .blocks
                .iter()
                .filter(|e| args.only.is_empty() || args.only.iter().any(|o| o == &e.id))
                .collect();
            for o in &args.only {
                if remote.index.get(o).is_none() {
                    eprintln!("warning: {} has no block `{o}`", remote.index.name);
                }
            }
            if args.list {
                println!(
                    "{} ({} blocks)",
                    remote.index.name,
                    remote.index.blocks.len()
                );
                if !remote.index.description.is_empty() {
                    println!("{}", remote.index.description);
                }
                for e in entries {
                    println!(
                        "{:<28} v{:<3} {}{}{}",
                        e.id,
                        e.version,
                        e.name,
                        if e.author.is_empty() {
                            String::new()
                        } else {
                            format!(" · by {}", e.author)
                        },
                        if e.preview.is_some() {
                            " · preview"
                        } else {
                            ""
                        }
                    );
                }
                return Ok(ExitCode::SUCCESS);
            }
            let mut lib = open_library(&args.library)?;
            let mut failed = 0;
            for e in entries {
                match remote.fetch_block(e).map_err(|e| anyhow!(e)) {
                    Ok(b) => println!("{}", import_one(&mut lib, b)?),
                    Err(why) => {
                        failed += 1;
                        eprintln!("error: {why}");
                    }
                }
            }
            if failed > 0 {
                return Ok(ExitCode::from(2));
            }
        }
        Fetched::Pack { bytes, name } => {
            let dir = args.output.clone().unwrap_or_else(|| PathBuf::from("."));
            std::fs::create_dir_all(&dir).map_err(|e| anyhow!("{}: {e}", dir.display()))?;
            let pack = free_path(&dir, &name, ac_io::PACK_EXTENSION);
            std::fs::write(&pack, &bytes).map_err(|e| anyhow!("{}: {e}", pack.display()))?;
            println!("downloaded → {}", pack.display());
            let project = ac_io::unpack_beside(&pack).map_err(|e| anyhow!(e))?;
            println!("unpacked → {}", project.display());
            // The pack's blocks join the library as when the editor opens one.
            if args.library.is_some() || ac_io::library::default_dir().is_some() {
                let mut lib = open_library(&args.library)?;
                let mut p = ac_io::load_project(&project).map_err(|e| anyhow!(e))?;
                let adopted = lib.adopt(&mut p);
                if adopted > 0 {
                    ac_io::save_project(&project, &p).map_err(|e| anyhow!(e))?;
                    println!(
                        "added {adopted} block{} from it to the library",
                        if adopted == 1 { "" } else { "s" }
                    );
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Import one downloaded block; the line to print.
fn import_one(lib: &mut Library, b: ac_io::remote::RemoteBlock) -> Result<String> {
    let name = b.block.def.name.clone();
    let known = lib
        .blocks()
        .iter()
        .any(|x| x.block.same_definition(&b.block));
    let source = lib
        .import_with_preview(b.block, &b.id_hint, b.preview.as_deref())
        .map_err(|e| anyhow!("couldn't import {name}: {e}"))?;
    let at = lib
        .get(&source)
        .and_then(|x| x.path.clone())
        .map(|p| format!(" ({})", p.display()))
        .unwrap_or_default();
    Ok(if known {
        format!("{name} is already in the library as {source}")
    } else {
        let preview = if lib.get(&source).and_then(|x| x.preview_path()).is_some() {
            ", with its preview"
        } else {
            ""
        };
        format!("imported {name} → {source}{at}{preview}")
    })
}

/// `dir/name.ext`, or `dir/name-2.ext`… if that's taken.
fn free_path(dir: &Path, name: &str, ext: &str) -> PathBuf {
    let mut path = dir.join(format!("{name}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{name}-{n}.{ext}"));
        n += 1;
    }
    path
}
