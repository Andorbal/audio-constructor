//! `audioc plugins`, and the plugin host `render`, `run`, and `validate`
//! use for projects with Plugin nodes.
//!
//! Plugins are found by scanning the standard folders, each bundle
//! described in a child process (`audioc scan-bundle <path>`, hidden), so a
//! plugin that crashes while being described can't take `audioc` down
//! (DECISIONS D135). The result is cached where the editor keeps it too.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use ac_core::{PluginDesc, PluginFormat, Project, project_plugins};
use ac_host::{Catalog, PluginHost, Scanner, scan};
use anyhow::Result;
use clap::Args;

#[derive(Debug, Args)]
pub struct PluginsArgs {
    /// Print the catalog as JSON.
    #[arg(long)]
    json: bool,
    /// Describe every bundle again, not just new or changed ones.
    #[arg(long)]
    rescan: bool,
    /// Describe bundles in this process instead of a child process each
    /// (faster, but a crashing plugin takes audioc with it).
    #[arg(long)]
    in_process: bool,
}

/// The scanner that describes one bundle in a child `audioc`.
pub fn scanner() -> Option<Scanner> {
    Some(Scanner {
        program: std::env::current_exe().ok()?,
        args: vec!["scan-bundle".into()],
    })
}

fn cache_path() -> Option<PathBuf> {
    ac_host::default_cache_path()
}

/// Scan the standard folders (reusing the cache for unchanged bundles
/// unless `fresh`), save the cache, and return the catalog.
pub fn rescan(fresh: bool, in_process: bool, verbose: bool) -> Catalog {
    let cache = cache_path();
    let previous = match (&cache, fresh) {
        (Some(p), false) => Catalog::load(p),
        _ => Catalog::default(),
    };
    let scanner = if in_process { None } else { scanner() };
    let catalog = scan::scan(scanner.as_ref(), &previous, |bundle| {
        if verbose {
            eprintln!("describing {}", bundle.display());
        }
    });
    if let Some(p) = &cache
        && let Err(e) = catalog.save(p)
    {
        eprintln!(
            "warning: couldn't save the plugin list to {}: {e}",
            p.display()
        );
    }
    catalog
}

fn kind(d: &PluginDesc) -> &'static str {
    if d.instrument || (d.audio_in.is_empty() && d.midi_in) {
        "instrument"
    } else if d.audio_in.is_empty() && d.audio_out.is_empty() {
        "note effect"
    } else {
        "effect"
    }
}

fn ports(d: &PluginDesc) -> String {
    let side = |name: &str, ports: &[ac_core::PluginPort]| -> Option<String> {
        if ports.is_empty() {
            return None;
        }
        let ch: Vec<String> = ports.iter().map(|p| p.channels.to_string()).collect();
        Some(format!("{name} {}", ch.join("+")))
    };
    let mut parts: Vec<String> = [side("in", &d.audio_in), side("out", &d.audio_out)]
        .into_iter()
        .flatten()
        .collect();
    if d.midi_in {
        parts.push("midi in".into());
    }
    if d.midi_out {
        parts.push("midi out".into());
    }
    parts.push(format!(
        "{} parameter{}",
        d.params.len(),
        if d.params.len() == 1 { "" } else { "s" }
    ));
    parts.join(", ")
}

pub fn list(args: PluginsArgs) -> Result<ExitCode> {
    let catalog = rescan(args.rescan, args.in_process, !args.json);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&catalog)?);
        return Ok(ExitCode::SUCCESS);
    }
    for format in [PluginFormat::Clap, PluginFormat::Vst3] {
        let dirs: Vec<String> = scan::search_paths(format)
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        println!("{} folders: {}", format.name(), dirs.join(", "));
    }
    println!();
    if catalog.plugins.is_empty() {
        println!("no plugins found");
    }
    for d in catalog.sorted() {
        let vendor = if d.vendor.is_empty() {
            String::new()
        } else {
            format!(" ({})", d.vendor)
        };
        println!(
            "{}{vendor} · {} · {} · {}",
            d.name,
            d.format.name(),
            kind(d),
            ports(d)
        );
        println!("    {}  {}", d.id, d.path);
    }
    for f in &catalog.failed {
        println!("failed: {}: {}", f.path, f.reason);
    }
    Ok(ExitCode::SUCCESS)
}

/// What a hidden `audioc scan-bundle <path>` child does.
pub fn scan_bundle(path: &std::path::Path) -> ExitCode {
    match ac_host::child_main(path) {
        0 => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

/// A plugin host for a project with Plugin nodes, or `None` without any.
/// It starts from the cached catalog; when a plugin the project uses
/// isn't where the cache says, the folders are scanned again first.
pub fn host_for(project: &Project) -> Option<Arc<PluginHost>> {
    let wanted = project_plugins(project);
    if wanted.is_empty() {
        return None;
    }
    let catalog = cache_path().map(|p| Catalog::load(&p)).unwrap_or_default();
    let host = PluginHost::new(catalog);
    if wanted.iter().any(|d| host.locate(d).is_none()) {
        host.set_catalog(rescan(false, false, false));
    }
    Some(Arc::new(host))
}
