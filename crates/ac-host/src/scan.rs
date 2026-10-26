//! Finding plugins: the standard folders per format and OS, the bundles in
//! them, and a catalog describing every plugin found (DECISIONS D135).
//!
//! Describing a plugin means loading its library and creating it once,
//! which runs its code; a broken plugin can crash while that happens. So a
//! scan can describe each bundle in a child process ([`Scanner`]): a
//! bundle that crashes its child is listed as failed and the scan goes on.
//! Results are cached by each bundle's size and modification time, so a
//! rescan only describes what changed.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

use ac_core::{PluginDesc, PluginFormat};
use serde::{Deserialize, Serialize};

/// How long one bundle may take to describe in a child process.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(30);

/// Folders a format's plugins are installed in on this OS: the
/// format's own variable first (`CLAP_PATH`, or `VST3_PATH`, each a list
/// separated like `PATH`), then the standard locations, user before system.
pub fn search_paths(format: PluginFormat) -> Vec<PathBuf> {
    let var = match format {
        PluginFormat::Clap => "CLAP_PATH",
        PluginFormat::Vst3 => "VST3_PATH",
    };
    let mut out: Vec<PathBuf> = std::env::var_os(var)
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let name = match format {
        PluginFormat::Clap => "CLAP",
        PluginFormat::Vst3 => "VST3",
    };
    if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            out.push(h.join("Library/Audio/Plug-Ins").join(name));
        }
        out.push(Path::new("/Library/Audio/Plug-Ins").join(name));
    } else if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(local).join("Programs\\Common").join(name));
        }
        if let Some(common) = std::env::var_os("COMMONPROGRAMFILES") {
            out.push(PathBuf::from(common).join(name));
        }
    } else {
        let dot = match format {
            PluginFormat::Clap => ".clap",
            PluginFormat::Vst3 => ".vst3",
        };
        let lib = match format {
            PluginFormat::Clap => "clap",
            PluginFormat::Vst3 => "vst3",
        };
        if let Some(h) = &home {
            out.push(h.join(dot));
        }
        out.push(Path::new("/usr/lib").join(lib));
        out.push(Path::new("/usr/local/lib").join(lib));
    }
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// `path` as a bundle of `format` that sits in one of the plugin folders
/// ([`search_paths`]), with links resolved; `None` if it's anywhere else.
/// A project records where each of its plugins was found, and this is the
/// only way that record can name a library to load (DECISIONS D197): a
/// project from someone else can't make the host load a file of its
/// choosing.
pub fn in_plugin_folders(path: &Path, format: PluginFormat) -> Option<PathBuf> {
    within(path, format, &search_paths(format))
}

/// [`in_plugin_folders`] with the folders given: the path must be
/// absolute, not on a network share, a bundle of `format` by its
/// extension, and, once both are resolved, inside one of `folders`.
fn within(path: &Path, format: PluginFormat, folders: &[PathBuf]) -> Option<PathBuf> {
    if !path.is_absolute() || on_network(path) || format_of(path) != Some(format) {
        return None;
    }
    let real = std::fs::canonicalize(path).ok()?;
    if on_network(&real) {
        return None;
    }
    folders
        .iter()
        .filter_map(|f| std::fs::canonicalize(f).ok())
        .any(|f| real.starts_with(f))
        .then_some(real)
}

/// A path on another computer: a Windows UNC path (`\\server\share`).
fn on_network(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(
        path.components().next(),
        Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
    )
}

fn extension(format: PluginFormat) -> &'static str {
    match format {
        PluginFormat::Clap => "clap",
        PluginFormat::Vst3 => "vst3",
    }
}

/// The format a bundle path is for, by its extension.
pub fn format_of(path: &Path) -> Option<PluginFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "clap" => Some(PluginFormat::Clap),
        "vst3" => Some(PluginFormat::Vst3),
        _ => None,
    }
}

/// Every bundle of a format under its search paths (a `.clap` or `.vst3`
/// file or bundle folder), each once, in a stable order.
pub fn find_bundles(format: PluginFormat) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for dir in search_paths(format) {
        walk(&dir, extension(format), 0, &mut out, &mut seen);
    }
    out
}

fn walk(dir: &Path, ext: &str, depth: usize, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    if depth > 8 {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let is_bundle = p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(ext));
        if is_bundle {
            let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
            if seen.insert(key) {
                out.push(p);
            }
        } else if p.is_dir() {
            walk(&p, ext, depth + 1, out, seen);
        }
    }
}

/// What the scan knew about a bundle, to tell whether it changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub path: String,
    pub len: u64,
    pub modified: u64,
}

impl Stamp {
    pub fn of(path: &Path) -> Stamp {
        // A bundle folder's binary changes inside it; the folder's own
        // time changes when its contents are replaced, which installers do.
        let meta = std::fs::metadata(path).ok();
        Stamp {
            path: path.to_string_lossy().into_owned(),
            len: meta
                .as_ref()
                .map_or(0, |m| if m.is_file() { m.len() } else { 0 }),
            modified: meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs()),
        }
    }
}

/// A bundle that couldn't be described, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    pub path: String,
    pub reason: String,
}

/// Every plugin a scan found, and the bundles that failed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub plugins: Vec<PluginDesc>,
    #[serde(default)]
    pub failed: Vec<Failure>,
    #[serde(default)]
    pub bundles: Vec<Stamp>,
}

impl Catalog {
    /// The plugin with this format and id.
    pub fn find(&self, format: PluginFormat, id: &str) -> Option<&PluginDesc> {
        self.plugins
            .iter()
            .find(|p| p.format == format && p.id == id)
    }

    /// A cached catalog, or an empty one when there's none or it can't
    /// be read.
    pub fn load(path: &Path) -> Catalog {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Plugins sorted for a menu: by name, then vendor, then format.
    pub fn sorted(&self) -> Vec<&PluginDesc> {
        let mut v: Vec<&PluginDesc> = self.plugins.iter().collect();
        v.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.vendor.cmp(&b.vendor))
                .then_with(|| a.format.cmp(&b.format))
        });
        v
    }
}

/// Where the editor and `audioc` keep the scan cache: the file
/// `AC_PLUGIN_CACHE` names, else `plugins.json` in the platform cache
/// folder.
pub fn default_cache_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AC_PLUGIN_CACHE") {
        return Some(PathBuf::from(p));
    }
    directories::ProjectDirs::from("", "", "Audio Constructor")
        .map(|d| d.cache_dir().join("plugins.json"))
}

/// Describe every plugin in one bundle, in this process.
pub fn describe_bundle(path: &Path) -> Result<Vec<PluginDesc>, String> {
    match format_of(path) {
        Some(PluginFormat::Clap) => crate::clap::describe_bundle(path),
        Some(PluginFormat::Vst3) => crate::vst3::describe_bundle(path),
        None => Err("not a CLAP or VST3 bundle".into()),
    }
}

/// How to describe a bundle in a child process: run `program` with
/// `args` and the bundle's path, and read a JSON list of descriptions
/// from its standard output (what [`child_main`] prints).
#[derive(Clone, Debug)]
pub struct Scanner {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Scanner {
    /// Describe `bundle` in a child process.
    pub fn describe(&self, bundle: &Path) -> Result<Vec<PluginDesc>, String> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .arg(bundle)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("couldn't start the scanner: {e}"))?;
        let mut stdout = child.stdout.take().ok_or("no scanner output")?;
        let mut stderr = child.stderr.take().ok_or("no scanner output")?;
        // Read while waiting, so a full pipe can't stall the child.
        let out = std::thread::spawn(move || {
            let mut s = Vec::new();
            let _ = stdout.read_to_end(&mut s);
            s
        });
        let err = std::thread::spawn(move || {
            let mut s = Vec::new();
            let _ = stderr.read_to_end(&mut s);
            s
        });
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break Some(s),
                Ok(None) if start.elapsed() > SCAN_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => return Err(format!("the scanner failed: {e}")),
            }
        };
        let out = out.join().unwrap_or_default();
        let err = err.join().unwrap_or_default();
        let Some(status) = status else {
            return Err(format!(
                "it took longer than {} s to describe",
                SCAN_TIMEOUT.as_secs()
            ));
        };
        if !status.success() {
            let msg = String::from_utf8_lossy(&err);
            let last = msg
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            return Err(match status.code() {
                Some(_) if !last.is_empty() => last.trim().to_string(),
                Some(code) => format!("the scanner stopped with code {code}"),
                None => "it crashed while being described".into(),
            });
        }
        // Plugins print to standard output too; the answer is its own line.
        let text = String::from_utf8_lossy(&out);
        let line = text
            .lines()
            .rev()
            .find_map(|l| l.strip_prefix(RESULT_PREFIX))
            .ok_or("the scanner gave no answer")?;
        serde_json::from_str(line).map_err(|e| format!("the scanner's answer was unreadable: {e}"))
    }
}

/// The start of the line a scanner child answers on.
const RESULT_PREFIX: &str = "ac-scan-result: ";

/// What a scanner child does: describe `bundle` and print the JSON list
/// on standard output, on a line of its own after `ac-scan-result: `
/// (whatever the plugin prints itself goes around it). Returns the exit
/// code.
pub fn child_main(bundle: &Path) -> i32 {
    match describe_bundle(bundle) {
        Ok(list) => match serde_json::to_string(&list) {
            Ok(json) => {
                println!("\n{RESULT_PREFIX}{json}");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Scan every standard folder, reusing `previous` for bundles that didn't
/// change, describing the rest with `scanner` (in a child process) or in
/// this process when it's `None`. `progress` hears each bundle described.
pub fn scan(
    scanner: Option<&Scanner>,
    previous: &Catalog,
    mut progress: impl FnMut(&Path),
) -> Catalog {
    let mut out = Catalog::default();
    for format in [PluginFormat::Clap, PluginFormat::Vst3] {
        for bundle in find_bundles(format) {
            let stamp = Stamp::of(&bundle);
            let key = stamp.path.clone();
            out.bundles.push(stamp.clone());
            if previous.bundles.contains(&stamp) {
                out.plugins
                    .extend(previous.plugins.iter().filter(|p| p.path == key).cloned());
                out.failed
                    .extend(previous.failed.iter().filter(|f| f.path == key).cloned());
                continue;
            }
            progress(&bundle);
            let described = match scanner {
                Some(s) => s.describe(&bundle),
                None => describe_bundle(&bundle),
            };
            match described {
                Ok(list) => out.plugins.extend(list),
                Err(reason) => out.failed.push(Failure { path: key, reason }),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder with `plugins/` (a plugin folder) and `elsewhere/`.
    fn folders(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ac-scan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plugins = dir.join("plugins");
        std::fs::create_dir_all(plugins.join("Vendor")).unwrap();
        std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
        (dir, plugins)
    }

    #[test]
    fn only_bundles_inside_the_plugin_folders_count() {
        let (dir, plugins) = folders("within");
        let roots = [plugins.clone()];
        let inside = plugins.join("Vendor").join("Synth.clap");
        let outside = dir.join("elsewhere").join("Synth.clap");
        let wrong_kind = plugins.join("Synth.so");
        for f in [&inside, &outside, &wrong_kind] {
            std::fs::write(f, b"library").unwrap();
        }
        let clap = PluginFormat::Clap;
        assert_eq!(
            within(&inside, clap, &roots),
            Some(std::fs::canonicalize(&inside).unwrap())
        );
        assert_eq!(within(&inside, PluginFormat::Vst3, &roots), None);
        assert_eq!(within(&outside, clap, &roots), None);
        assert_eq!(within(&wrong_kind, clap, &roots), None);
        // `..` can't climb out of a plugin folder.
        let climbing = plugins.join("..").join("elsewhere").join("Synth.clap");
        assert_eq!(within(&climbing, clap, &roots), None);
        // Relative paths and files that aren't there never count.
        assert_eq!(within(Path::new("Vendor/Synth.clap"), clap, &roots), None);
        assert_eq!(within(&plugins.join("Gone.clap"), clap, &roots), None);
        assert_eq!(within(&inside, clap, &[]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_a_plugin_folder_counts_where_it_leads() {
        let (dir, plugins) = folders("links");
        let roots = [plugins.clone()];
        let outside = dir.join("elsewhere").join("Synth.clap");
        std::fs::write(&outside, b"library").unwrap();
        let link = plugins.join("Linked.clap");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert_eq!(within(&link, PluginFormat::Clap, &roots), None);
        // A plugin folder that is itself a link is followed.
        let linked_folder = dir.join("linked-plugins");
        std::os::unix::fs::symlink(&plugins, &linked_folder).unwrap();
        let real = plugins.join("Real.clap");
        std::fs::write(&real, b"library").unwrap();
        assert!(within(&real, PluginFormat::Clap, &[linked_folder]).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn network_paths_never_count() {
        assert!(on_network(Path::new(r"\\server\share\Synth.clap")));
        assert!(on_network(Path::new(r"\\?\UNC\server\share\Synth.clap")));
        assert!(!on_network(Path::new(
            r"C:\Program Files\Common Files\CLAP"
        )));
    }
}
