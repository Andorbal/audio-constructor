//! Argument parsing helpers.

use std::path::Path;
use std::sync::Arc;

use ac_core::{NodePath, Project, Registry, Value};
use ac_io::{AssetStore, FileResources};
use anyhow::{Context, Result, anyhow};

/// Parse a time: seconds (`12.5`), `m:ss(.frac)`, or `h:mm:ss`.
pub fn parse_time(s: &str) -> Result<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    let mut total = 0.0;
    for p in &parts {
        let v: f64 = p
            .trim()
            .parse()
            .with_context(|| format!("invalid time `{s}`"))?;
        total = total * 60.0 + v;
    }
    if !total.is_finite() || total < 0.0 {
        return Err(anyhow!("invalid time `{s}`"));
    }
    Ok(total)
}

/// Parse `node.socket=value`, where node is a hex id or a path like `2/a`.
pub fn parse_set(s: &str) -> Result<(NodePath, String, Value)> {
    let (lhs, rhs) = s
        .split_once('=')
        .ok_or_else(|| anyhow!("expected node.socket=value, got `{s}`"))?;
    let (node, socket) = lhs
        .rsplit_once('.')
        .ok_or_else(|| anyhow!("expected node.socket=value, got `{s}`"))?;
    let path: NodePath = node.parse().map_err(|e: String| anyhow!(e))?;
    Ok((path, socket.to_string(), parse_value(rhs)))
}

pub fn parse_value(s: &str) -> Value {
    let t = s.trim();
    match t {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => {
            if let Ok(i) = t.parse::<i64>() {
                Value::Int(i)
            } else if let Ok(f) = t.parse::<f64>() {
                Value::Float(f)
            } else {
                Value::Text(t.to_string())
            }
        }
    }
}

/// Load a project and its assets. Asset problems are printed, not fatal.
pub fn load(path: &Path, registry: &Registry) -> Result<(Project, Arc<FileResources>)> {
    let (project, assets) = load_with_assets(path, registry)?;
    Ok((
        project,
        Arc::new(FileResources {
            assets,
            recorder: None,
            plugins: None,
        }),
    ))
}

/// The directory a project's relative asset paths start from.
pub fn project_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// Load a project and the asset store for it, printing asset problems.
pub fn load_with_assets(path: &Path, registry: &Registry) -> Result<(Project, AssetStore)> {
    let project = ac_io::load_project(path).map_err(|e| anyhow!(e))?;
    let assets = AssetStore::load_for(&project, project_dir(path), registry);
    for issue in &assets.issues {
        match issue {
            ac_io::assets::AssetIssue::Missing { path, reason } => {
                eprintln!("warning: missing asset {path}: {reason}")
            }
            ac_io::assets::AssetIssue::Changed { path } => {
                eprintln!("warning: asset {path} changed since it was added (hash differs)")
            }
        }
    }
    Ok((project, assets))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("12.5").unwrap(), 12.5);
        assert_eq!(parse_time("1:30").unwrap(), 90.0);
        assert_eq!(parse_time("1:00:01").unwrap(), 3601.0);
        assert!(parse_time("x").is_err());
        assert!(parse_time("-3").is_err());
    }

    #[test]
    fn sets() {
        let (p, s, v) = parse_set("a.freq=220").unwrap();
        assert_eq!(p.to_string(), "a");
        assert_eq!(s, "freq");
        assert_eq!(v, Value::Int(220));
        let (p, _, v) = parse_set("2/a.shape=saw").unwrap();
        assert_eq!(p.to_string(), "2/a");
        assert_eq!(v, Value::Text("saw".into()));
        assert!(parse_set("nope").is_err());
    }
}
