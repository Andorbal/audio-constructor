//! Crash reports: what a panic leaves behind, for the person to read and,
//! if they choose, send.
//!
//! [`install`] adds a panic hook that writes a plain-text report into a
//! folder (the editor and `audioc` use `crashes/` in the platform config
//! folder) before the usual message is printed. Nothing is ever sent: the
//! editor offers saved reports on its next start, and [`issue_url`] makes a
//! link to the project's new-issue form with the report filled in, which
//! the person reads, edits, and submits themselves (or not). The person's
//! home folder is written as `~` in every report.
//!
//! The engine doesn't panic (CLAUDE.md); a report means a bug somewhere
//! else, or one that slipped through.

use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reports kept, newest first; older ones are deleted when a new one is
/// written.
pub const KEEP: usize = 20;

/// The longest link [`issue_url`] makes; browsers and servers refuse much
/// longer ones.
pub const MAX_URL: usize = 7_500;

const PREFIX: &str = "crash-";
/// A report the person has already been offered.
const SEEN: &str = ".seen";

/// Where reports go: `AC_CRASH_DIR`, or `crashes/` in the platform config
/// folder (beside the editor's settings).
pub fn default_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("AC_CRASH_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("", "", "Audio Constructor")
        .map(|d| d.config_dir().join("crashes"))
}

/// Writes a report into `dir` for every panic, then runs the previous hook
/// (which prints the panic as usual). `program` names who crashed in the
/// report (`audio-constructor`, `audioc`).
pub fn install(dir: PathBuf, program: &'static str) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = report(program, info, &Backtrace::force_capture());
        let saved = save(&dir, &text);
        previous(info);
        if let Ok(path) = saved {
            eprintln!(
                "{program} stopped because of a bug; a report is in {}",
                path.display()
            );
        }
    }));
}

/// The report's text.
pub fn report(program: &str, info: &PanicHookInfo<'_>, backtrace: &Backtrace) -> String {
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".into());
    let location = info.location().map_or_else(
        || "unknown".into(),
        |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
    );
    let thread = std::thread::current();
    let text = format_report(
        program,
        &now_utc(),
        thread.name().unwrap_or("unnamed"),
        &message,
        &location,
        &backtrace.to_string(),
    );
    scrub_home(&text, home().as_deref())
}

/// A report from its parts (split out so tests can check the layout).
pub fn format_report(
    program: &str,
    time: &str,
    thread: &str,
    message: &str,
    location: &str,
    backtrace: &str,
) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "Audio Constructor crash report");
    let _ = writeln!(s);
    let _ = writeln!(s, "program:  {program} {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        s,
        "system:   {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(s, "time:     {time}");
    let _ = writeln!(s, "thread:   {thread}");
    let _ = writeln!(s, "where:    {location}");
    let _ = writeln!(s, "message:  {}", message.trim());
    let _ = writeln!(s);
    let _ = writeln!(s, "backtrace:");
    s.push_str(backtrace.trim_end());
    s.push('\n');
    s
}

fn home() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .filter(|h| h.len() > 1)
}

/// `text` with the home folder written as `~`, so a report doesn't carry
/// the person's user name in its paths.
pub fn scrub_home(text: &str, home: Option<&str>) -> String {
    match home {
        Some(h) if !h.is_empty() => text.replace(h.trim_end_matches(['/', '\\']), "~"),
        _ => text.to_string(),
    }
}

/// Saves `text` as a new report in `dir`, keeping the newest [`KEEP`].
pub fn save(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = dir.join(format!("{PREFIX}{stamp}-{}.txt", std::process::id()));
    std::fs::write(&path, text)?;
    prune(dir);
    Ok(path)
}

/// A saved report.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub path: PathBuf,
    pub text: String,
}

/// Every report in `dir`, newest first, with whether it's been offered.
fn reports(dir: &Path) -> Vec<(PathBuf, bool)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, bool)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?;
            if !name.starts_with(PREFIX) {
                return None;
            }
            if name.ends_with(".txt") {
                Some((p.clone(), false))
            } else if name.ends_with(&format!(".txt{SEEN}")) {
                Some((p.clone(), true))
            } else {
                None
            }
        })
        .collect();
    // Names start with the time, so the newest sorts last.
    found.sort_by(|a, b| b.0.file_name().cmp(&a.0.file_name()));
    found
}

fn prune(dir: &Path) {
    for (old, _) in reports(dir).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old);
    }
}

/// Reports not offered yet, newest first.
pub fn pending(dir: &Path) -> Vec<Saved> {
    reports(dir)
        .into_iter()
        .filter(|(_, seen)| !seen)
        .filter_map(|(path, _)| {
            let text = std::fs::read_to_string(&path).ok()?;
            Some(Saved { path, text })
        })
        .collect()
}

/// Marks a report as offered, so it isn't offered again (it stays on disk,
/// among the [`KEEP`] newest).
pub fn mark_seen(report: &Path) -> std::io::Result<()> {
    let mut seen = report.as_os_str().to_owned();
    seen.push(SEEN);
    std::fs::rename(report, seen)
}

/// A link to the project's new-issue form with `report` filled in, cut to
/// fit [`MAX_URL`] (the end of a long backtrace goes first).
pub fn issue_url(report: &str) -> String {
    let base = format!(
        "{}/issues/new?title=",
        env!("CARGO_PKG_REPOSITORY").trim_end_matches('/')
    );
    let title = report
        .lines()
        .find_map(|l| l.strip_prefix("message:"))
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map_or_else(
            || "Crash report".to_string(),
            |m| {
                let short: String = m.chars().take(80).collect();
                format!("Crash: {short}")
            },
        );
    let head = format!("{base}{}&body=", crate::net::encode_component(&title));
    let intro = "What were you doing when it stopped?\n\n\n";
    let note = "\n… (cut short; the whole report is in the crashes folder)\n";
    let wrap = |body: &str| format!("{intro}```\n{body}```\n");
    let full = crate::net::encode_component(&wrap(report));
    if head.len() + full.len() <= MAX_URL {
        return head + &full;
    }
    // Drop lines from the end until it fits.
    let mut lines: Vec<&str> = report.lines().collect();
    while !lines.is_empty() {
        lines.pop();
        let body = format!("{}{note}", lines.join("\n"));
        let encoded = crate::net::encode_component(&wrap(&body));
        if head.len() + encoded.len() <= MAX_URL {
            return head + &encoded;
        }
    }
    head
}

/// The time now in UTC, `2026-10-19 12:34:56 UTC`.
fn now_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    utc(secs)
}

/// `secs` since 1970 as a UTC date and time.
pub fn utc(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a civil date, from Howard Hinnant's
/// "chrono-Compatible Low-Level Date Algorithms".
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ac-crash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn dates_are_utc_calendar_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(utc(1_792_413_296), "2026-10-19 12:34:56 UTC");
    }

    #[test]
    fn a_report_names_the_build_the_system_and_where_it_stopped() {
        let r = format_report(
            "audioc",
            "2026-10-19 12:34:56 UTC",
            "main",
            "index out of bounds\n",
            "crates/x.rs:1:2",
            "   0: frame\n",
        );
        assert!(r.starts_with("Audio Constructor crash report\n"));
        assert!(r.contains(&format!("program:  audioc {}\n", env!("CARGO_PKG_VERSION"))));
        assert!(r.contains(&format!(
            "system:   {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )));
        assert!(r.contains("where:    crates/x.rs:1:2\n"));
        assert!(r.contains("message:  index out of bounds\n"));
        assert!(r.ends_with("backtrace:\n   0: frame\n"));
    }

    #[test]
    fn the_home_folder_becomes_a_tilde() {
        let text = "opened /home/sam/songs/a.acproj and /home/sam/x";
        assert_eq!(
            scrub_home(text, Some("/home/sam/")),
            "opened ~/songs/a.acproj and ~/x"
        );
        assert_eq!(
            scrub_home(r"C:\Users\Sam\a.acproj", Some(r"C:\Users\Sam")),
            r"~\a.acproj"
        );
        assert_eq!(scrub_home(text, None), text);
    }

    #[test]
    fn reports_are_saved_offered_once_and_pruned() {
        let dir = temp("saved");
        assert!(pending(&dir).is_empty(), "no folder yet");
        let first = save(&dir, "one").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let second = save(&dir, "two").unwrap();
        let p = pending(&dir);
        assert_eq!(
            p.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            ["two", "one"]
        );
        mark_seen(&second).unwrap();
        assert_eq!(
            pending(&dir),
            vec![Saved {
                path: first.clone(),
                text: "one".into()
            }]
        );
        // Junk in the folder is left alone and ignored.
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        for i in 0..KEEP + 5 {
            std::fs::write(
                dir.join(format!(
                    "{PREFIX}{:013}-1.txt",
                    2_000_000_000_000u64 + i as u64
                )),
                "n",
            )
            .unwrap();
        }
        prune(&dir);
        assert_eq!(reports(&dir).len(), KEEP);
        assert!(!first.exists(), "the oldest went");
        assert!(dir.join("notes.txt").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_issue_link_carries_the_report_and_fits() {
        let report = format_report(
            "audio-constructor",
            "t",
            "main",
            "boom & bust",
            "a.rs:1:1",
            "   0: f\n",
        );
        let url = issue_url(&report);
        assert!(url.starts_with("https://github.com/Andorbal/audio-constructor/issues/new?title=Crash%3A%20boom%20%26%20bust&body="), "{url}");
        assert!(url.contains("boom%20%26%20bust"));
        assert!(!url.contains(' ') && !url.contains('\n'));
        // A huge backtrace is cut from the end.
        let long = format_report(
            "audioc",
            "t",
            "main",
            "deep",
            "a.rs:1:1",
            &"   0: some::very::long::frame\n".repeat(2000),
        );
        let url = issue_url(&long);
        assert!(url.len() <= MAX_URL, "{}", url.len());
        assert!(url.contains("cut%20short"));
        assert!(url.contains("deep"), "the top of the report stays");
    }

    #[test]
    fn the_hook_writes_a_report_for_a_panic() {
        let dir = temp("hook");
        // Build the report the hook would, from a real panic's info.
        let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let c = captured.clone();
        let previous = std::panic::take_hook();
        // Other tests run alongside; only this test's thread is captured.
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().name() == Some("crash-test-worker") {
                *c.lock().unwrap() = report("audioc", info, &Backtrace::disabled());
            }
        }));
        let _ = std::thread::Builder::new()
            .name("crash-test-worker".into())
            .spawn(|| panic!("it broke at {}", 42))
            .unwrap()
            .join();
        std::panic::set_hook(previous);
        let text = captured.lock().unwrap().clone();
        assert!(text.contains("message:  it broke at 42"), "{text}");
        assert!(text.contains("thread:   crash-test-worker"), "{text}");
        assert!(text.contains("crash.rs"), "{text}");
        let saved = save(&dir, &text).unwrap();
        assert_eq!(std::fs::read_to_string(saved).unwrap(), text);
        std::fs::remove_dir_all(dir).ok();
    }
}
