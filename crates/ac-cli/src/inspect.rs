//! `audioc validate`, `inspect`, and `fmt`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use ac_core::{Graph, GroupDef, Project};
use ac_engine::{CompileOptions, Severity, compile};
use anyhow::{Result, anyhow};

use crate::util::load;

pub fn validate(path: &Path, sample_rate: Option<u32>) -> Result<ExitCode> {
    let registry = Arc::new(ac_nodes::registry());
    let (project, resources) = load(path, &registry)?;
    let compiled = compile(
        &project,
        &registry,
        resources.as_ref(),
        &CompileOptions {
            sample_rate: f64::from(sample_rate.unwrap_or(project.settings.sample_rate)),
            max_block: project.settings.block_size as usize,
            socket_telemetry: false,
            generation: 0,
        },
        None,
    );
    let r = &compiled.report;
    for d in &r.diagnostics {
        println!("{d}");
    }
    let s = &r.stats;
    println!(
        "{}: {} nodes, {} steps, {} audio buffers, {} parameters",
        path.display(),
        s.nodes,
        s.steps,
        s.sample_buffers,
        s.params
    );
    if !s.live_sources.is_empty() {
        let list: Vec<String> = s.live_sources.iter().map(ToString::to_string).collect();
        println!("live sources: {}", list.join(", "));
    }
    println!("disabled nodes: none (no whole-input nodes in Phase 1)");
    println!("latency: {} samples", s.latency);
    let errors = r
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    if errors > 0 {
        println!("{errors} error(s)");
        Ok(ExitCode::FAILURE)
    } else {
        println!("ok");
        Ok(ExitCode::SUCCESS)
    }
}

pub fn inspect(path: &Path) -> Result<ExitCode> {
    let project = ac_io::load_project(path).map_err(|e| anyhow!(e))?;
    let registry = ac_nodes::registry();
    let s = &project.settings;
    println!(
        "{}",
        if project.name.is_empty() {
            "(unnamed project)"
        } else {
            &project.name
        }
    );
    println!(
        "  {} Hz, {} s, block {}, seed {}",
        s.sample_rate, s.length, s.block_size, s.seed
    );
    for t in &s.tempo {
        println!(
            "  tempo: {} BPM {}/{} from beat {}",
            t.bpm, t.time_sig[0], t.time_sig[1], t.beat
        );
    }
    if let Some([a, b]) = s.loop_region {
        println!("  loop: beats {a}..{b}");
    }
    print_graph(&project, &project.graph, &registry, "");
    for (id, def) in &project.group_defs {
        print_def(&project, id.0.as_str(), def, &registry);
    }
    Ok(ExitCode::SUCCESS)
}

fn print_def(project: &Project, id: &str, def: &GroupDef, registry: &ac_core::Registry) {
    println!("\ngroup `{id}`: {} (v{})", def.name, def.version);
    if !def.description.is_empty() {
        println!("  {}", def.description);
    }
    for i in &def.inputs {
        let targets: Vec<String> = i.targets.iter().map(ToString::to_string).collect();
        println!("  in  {}: {} → {}", i.name, i.ty, targets.join(", "));
    }
    for o in &def.outputs {
        println!("  out {}: {} ← {}", o.name, o.ty, o.source);
    }
    print_graph(project, &def.graph, registry, "  ");
}

fn print_graph(project: &Project, g: &Graph, registry: &ac_core::Registry, indent: &str) {
    println!("{indent}nodes ({}):", g.nodes.len());
    for n in &g.nodes {
        let name = registry.get(&n.kind).map_or_else(
            || format!("{} (unknown kind)", n.kind),
            |e| e.meta.name.to_string(),
        );
        let label =
            n.ui.label
                .as_deref()
                .map(|l| format!(" \"{l}\""))
                .unwrap_or_default();
        let muted = if n.muted { " [muted]" } else { "" };
        print!("{indent}  {:>6}  {name}{label}{muted}", n.id.to_string());
        if let Some(def) = n.group_def() {
            let def_name = project
                .group_defs
                .get(&def)
                .map_or("missing", |d| d.name.as_str());
            print!(" → group `{def}` ({def_name})");
        }
        if !super::inspect::config_is_empty(&n.config) {
            print!("  config {}", n.config);
        }
        if !n.params.is_empty() {
            let ps: Vec<String> = n.params.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
            print!("  {}", ps.join(" "));
        }
        println!();
    }
    println!("{indent}wires ({}):", g.wires.len());
    for w in &g.wires {
        println!("{indent}  {} → {}", w.from, w.to);
    }
}

fn config_is_empty(c: &ac_core::Config) -> bool {
    c.as_object().is_none_or(|m| m.is_empty())
}

pub fn fmt(files: &[PathBuf], check: bool) -> Result<ExitCode> {
    let mut bad = 0;
    for f in files {
        let text = std::fs::read_to_string(f).map_err(|e| anyhow!("{}: {e}", f.display()))?;
        let p = Project::from_json(&text).map_err(|e| anyhow!("{}: {e}", f.display()))?;
        let canonical = p.to_json();
        if canonical != text {
            if check {
                println!("{} is not canonical", f.display());
                bad += 1;
            } else {
                ac_io::save_project(f, &p).map_err(|e| anyhow!(e))?;
                println!("formatted {}", f.display());
            }
        }
    }
    Ok(if bad > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
