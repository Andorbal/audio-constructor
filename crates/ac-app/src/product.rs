//! The desktop editor's two links to the world beyond this computer: the
//! update check and crash reports (D173, D174). Both are the person's to
//! start; nothing is sent by itself.
//!
//! - **Updates.** Until the person decides, the editor asks once whether to
//!   look for new versions, and checks nothing. Checking daily, it asks the
//!   release feed (`ac_io::update`) at most once a day, at start, on its
//!   own thread, and says something only when a newer release is out:
//!   **Download** opens the release's page, **Skip This Version** keeps
//!   quiet about that one. Help ▸ Check for Updates… asks now and always
//!   answers.
//! - **Crash reports.** `ac_io::crash`'s panic hook saves a report when
//!   the editor or `audioc` stops because of a bug. At the next start the
//!   editor says so and offers to show the report or open the project's
//!   issue form with it filled in, in the browser, where the person reads
//!   it and decides whether to send it.
//!
//! A plugin's host and the browser have their own ways for both, so only
//! the desktop editor turns this on ([`crate::EditorApp::enable_product`]).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use ac_io::crash::{self, Saved};
use ac_io::update::{self, Check, Release, Version};
use egui::RichText;

use crate::icons;
use crate::settings::{AppSettings, UpdateChecks};
use crate::theme;

/// A day, between automatic checks.
const DAY: u64 = 86_400;

/// Where the update check and crash reports look.
#[derive(Clone, Debug)]
pub struct ProductOptions {
    /// The release feed ([`update::feed_url`]).
    pub feed: String,
    /// Where crash reports are saved (`None`: none are offered).
    pub crash_dir: Option<PathBuf>,
    /// This build's version (a newer build in tests).
    pub current: Version,
}

impl ProductOptions {
    /// The real feed and the platform's crash folder.
    pub fn desktop() -> ProductOptions {
        ProductOptions {
            feed: update::feed_url(),
            crash_dir: crash::default_dir(),
            current: Version::current(),
        }
    }
}

/// What the update corner shows.
#[derive(Clone, Debug, PartialEq)]
enum Notice {
    /// Should the editor check by itself?
    Ask,
    Newer(Release),
    /// A check the person asked for found nothing newer.
    UpToDate(Version),
    /// A check the person asked for failed.
    Failed(String),
}

pub struct Product {
    opts: ProductOptions,
    /// A check in flight, and whether the person asked for it.
    checking: Option<(Receiver<Result<Check, String>>, bool)>,
    notice: Option<Notice>,
    show_notes: bool,
    /// The newest unseen crash report, and how many there are.
    crash: Option<(Saved, usize)>,
    show_report: bool,
    /// The last link opened (tests read it; opening goes through egui).
    opened: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Product {
    /// Starts up: finds unseen crash reports and, if it's time, checks for
    /// a newer release in the background.
    pub fn start(opts: ProductOptions, settings: &mut AppSettings, ctx: &egui::Context) -> Product {
        let mut p = Product {
            opts,
            checking: None,
            notice: None,
            show_notes: false,
            crash: None,
            show_report: false,
            opened: None,
        };
        if settings.prefs.crash_prompt
            && let Some(dir) = &p.opts.crash_dir
        {
            let pending = crash::pending(dir);
            let count = pending.len();
            p.crash = pending.into_iter().next().map(|newest| (newest, count));
        }
        match settings.prefs.update_checks {
            UpdateChecks::Ask => p.notice = Some(Notice::Ask),
            UpdateChecks::Daily => {
                let t = now();
                if t.saturating_sub(settings.prefs.last_update_check) >= DAY {
                    settings.set_last_update_check(t);
                    p.check(ctx, false);
                }
            }
            UpdateChecks::Off => {}
        }
        p
    }

    /// Asks the feed now; `asked` means the person asked, so the answer is
    /// shown whatever it is.
    pub fn check(&mut self, ctx: &egui::Context, asked: bool) {
        if self.checking.is_some() {
            if asked && let Some((_, a)) = &mut self.checking {
                *a = true;
            }
            return;
        }
        let (tx, rx) = channel();
        let feed = self.opts.feed.clone();
        let current = self.opts.current;
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("update check".into())
            .spawn(move || {
                let _ = tx.send(update::check(&feed, current));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.checking = Some((rx, asked)),
            Err(e) if asked => self.notice = Some(Notice::Failed(e.to_string())),
            Err(_) => {}
        }
    }

    pub fn is_checking(&self) -> bool {
        self.checking.is_some()
    }

    /// The link the person last chose to open.
    pub fn opened(&self) -> Option<&str> {
        self.opened.as_deref()
    }

    fn open(&mut self, ctx: &egui::Context, url: String) {
        ctx.open_url(egui::OpenUrl::new_tab(&url));
        self.opened = Some(url);
    }

    /// Help ▸ Report a Problem…: the issue form, with the version and
    /// system filled in.
    pub fn report_problem(&mut self, ctx: &egui::Context) {
        let body = format!(
            "What happened, and what did you expect?\n\n\n---\nAudio Constructor {} on {} {}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        let url = format!(
            "{}/issues/new?body={}",
            env!("CARGO_PKG_REPOSITORY").trim_end_matches('/'),
            ac_io::net::encode_component(&body)
        );
        self.open(ctx, url);
    }

    fn poll(&mut self, settings: &AppSettings) {
        let Some((rx, asked)) = &self.checking else {
            return;
        };
        let asked = *asked;
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the check stopped unexpectedly".into()),
        };
        self.checking = None;
        self.notice = match result {
            Ok(Check::Newer(release))
                if asked || release.version.to_string() != settings.prefs.skip_version =>
            {
                Some(Notice::Newer(release))
            }
            Ok(Check::Newer(_)) => None,
            Ok(Check::UpToDate(_)) if asked => Some(Notice::UpToDate(self.opts.current)),
            Err(e) if asked => Some(Notice::Failed(e)),
            // Nobody asked: say nothing about a quiet day or a failed check.
            _ => self.notice.take().filter(|n| *n == Notice::Ask),
        };
    }

    /// Draws whatever there is to say, each frame.
    pub fn ui(&mut self, ctx: &egui::Context, settings: &mut AppSettings) {
        self.poll(settings);
        self.crash_window(ctx, settings);
        self.update_window(ctx, settings);
    }

    fn update_window(&mut self, ctx: &egui::Context, settings: &mut AppSettings) {
        let Some(notice) = self.notice.clone() else {
            return;
        };
        let mut close = false;
        let title = match &notice {
            Notice::Ask => "Check for updates?",
            Notice::Newer(_) => "A new version is out",
            Notice::UpToDate(_) | Notice::Failed(_) => "Updates",
        };
        egui::Window::new(title)
            .id(egui::Id::unique("product-update"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -36.0))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_max_width(340.0);
                match &notice {
                    Notice::Ask => {
                        ui.label(
                            "Audio Constructor can look once a day for a newer version \
                             and tell you when one is out. It only asks the project's \
                             release page; nothing about you or your work is sent.",
                        );
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Check Daily").clicked() {
                                settings.set_update_checks(UpdateChecks::Daily);
                                settings.set_last_update_check(now());
                                self.check(ctx, false);
                                close = true;
                            }
                            if ui.button("Don't Check").clicked() {
                                settings.set_update_checks(UpdateChecks::Off);
                                close = true;
                            }
                        });
                        ui.label(
                            RichText::new("Change it any time under Help.")
                                .small()
                                .color(theme::text_muted()),
                        );
                    }
                    Notice::Newer(r) => {
                        ui.label(format!(
                            "Audio Constructor {} is out. You have {}.",
                            r.version, self.opts.current
                        ));
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .button(format!("{}  Download", icons::DOWNLOAD_SIMPLE))
                                .clicked()
                            {
                                self.open(ctx, r.page.clone());
                                close = true;
                            }
                            if !r.notes.trim().is_empty() && ui.button("What's New").clicked() {
                                self.show_notes = !self.show_notes;
                            }
                            if ui.button("Skip This Version").clicked() {
                                settings.set_skip_version(&r.version.to_string());
                                close = true;
                            }
                            if ui.button("Later").clicked() {
                                close = true;
                            }
                        });
                        if self.show_notes {
                            egui::ScrollArea::vertical()
                                .max_height(240.0)
                                .show(ui, |ui| {
                                    ui.label(RichText::new(r.notes.trim()).small());
                                });
                        }
                    }
                    Notice::UpToDate(v) => {
                        ui.label(format!("You have the newest version, {v}."));
                        if ui.button("OK").clicked() {
                            close = true;
                        }
                    }
                    Notice::Failed(why) => {
                        ui.label(format!("Couldn't check for updates: {why}."));
                        if ui.button("OK").clicked() {
                            close = true;
                        }
                    }
                }
            });
        if close {
            self.notice = None;
            self.show_notes = false;
        }
    }

    fn crash_window(&mut self, ctx: &egui::Context, settings: &mut AppSettings) {
        let Some((report, count)) = self.crash.clone() else {
            return;
        };
        let mut done = false;
        egui::Window::new("Audio Constructor stopped unexpectedly")
            .id(egui::Id::unique("product-crash"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                let which = if count > 1 {
                    format!("{count} reports were saved, the newest from")
                } else {
                    "A report was saved from".to_string()
                };
                let program = report
                    .text
                    .lines()
                    .find_map(|l| l.strip_prefix("program:"))
                    .map_or("the editor", str::trim)
                    .to_string();
                ui.label(format!(
                    "Last time, a bug stopped it. {which} {program}: the version, your \
                     system, and where it stopped. It's on this computer only; \
                     nothing has been sent."
                ));
                ui.label(
                    "Reporting it helps get the bug fixed: the project's issue form \
                     opens in your browser with the report filled in, for you to read, \
                     add what you were doing, and send, or not.",
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Report…").clicked() {
                        self.open(ctx, crash::issue_url(&report.text));
                        done = true;
                    }
                    if ui
                        .button(if self.show_report {
                            "Hide Report"
                        } else {
                            "Show Report"
                        })
                        .clicked()
                    {
                        self.show_report = !self.show_report;
                    }
                    if ui.button("Dismiss").clicked() {
                        done = true;
                    }
                });
                let mut never = !settings.prefs.crash_prompt;
                if ui
                    .checkbox(&mut never, "Don't offer crash reports again")
                    .changed()
                {
                    settings.set_crash_prompt(!never);
                }
                if self.show_report {
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new(&report.text).monospace().small());
                        });
                    ui.label(
                        RichText::new(format!("Saved as {}", report.path.display()))
                            .small()
                            .color(theme::text_muted()),
                    );
                }
            });
        if done {
            // Every report so far has now been offered.
            if let Some(dir) = &self.opts.crash_dir {
                for r in crash::pending(dir) {
                    let _ = crash::mark_seen(&r.path);
                }
            }
            self.crash = None;
            self.show_report = false;
        }
    }
}
