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

use crate::i18n::{tr, tr_args};
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
    /// A window just appeared: its first button takes the keyboard, so it
    /// can be answered without the pointer (and a screen reader reads it).
    want_focus: bool,
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
            want_focus: false,
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
        p.want_focus = p.crash.is_some() || p.notice.is_some();
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
            Err(e) if asked => {
                self.notice = Some(Notice::Failed(e.to_string()));
                self.want_focus = true;
            }
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
        // The issue goes to the project, whose tracker is in English.
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
            Err(TryRecvError::Disconnected) => Err(tr("update-check-stopped")),
        };
        self.checking = None;
        let before = self.notice.clone();
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
        if self.notice.is_some() && self.notice != before {
            self.want_focus = true;
        }
    }

    /// Give `r` the keyboard if a window just appeared, unless the person
    /// is typing somewhere.
    fn take_focus(&mut self, r: &egui::Response) {
        if !self.want_focus {
            return;
        }
        self.want_focus = false;
        let typing = r
            .ctx
            .memory(|m| m.focused())
            .is_some_and(|id| egui::text_edit::TextEditState::load(&r.ctx, id).is_some());
        if !typing {
            r.request_focus();
        }
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
            Notice::Ask => tr("update-ask-title"),
            Notice::Newer(_) => tr("update-newer-title"),
            Notice::UpToDate(_) | Notice::Failed(_) => tr("update-title"),
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
                        ui.label(tr("update-ask-text"));
                        ui.horizontal_wrapped(|ui| {
                            let daily = ui.button(tr("update-check-daily"));
                            self.take_focus(&daily);
                            if daily.clicked() {
                                settings.set_update_checks(UpdateChecks::Daily);
                                settings.set_last_update_check(now());
                                self.check(ctx, false);
                                close = true;
                            }
                            if ui.button(tr("update-dont-check")).clicked() {
                                settings.set_update_checks(UpdateChecks::Off);
                                close = true;
                            }
                        });
                        ui.label(
                            RichText::new(tr("update-change-later"))
                                .small()
                                .color(theme::text_muted()),
                        );
                    }
                    Notice::Newer(r) => {
                        ui.label(tr_args(
                            "update-newer-text",
                            &[
                                ("version", r.version.to_string().into()),
                                ("current", self.opts.current.to_string().into()),
                            ],
                        ));
                        ui.horizontal_wrapped(|ui| {
                            let download = icons::spoken(ui.button(format!(
                                "{}  {}",
                                icons::DOWNLOAD_SIMPLE,
                                tr("update-download")
                            )));
                            self.take_focus(&download);
                            if download.clicked() {
                                self.open(ctx, r.page.clone());
                                close = true;
                            }
                            if !r.notes.trim().is_empty()
                                && ui.button(tr("update-whats-new")).clicked()
                            {
                                self.show_notes = !self.show_notes;
                            }
                            if ui.button(tr("update-skip")).clicked() {
                                settings.set_skip_version(&r.version.to_string());
                                close = true;
                            }
                            if ui.button(tr("update-later")).clicked() {
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
                        ui.label(tr_args(
                            "update-up-to-date",
                            &[("version", v.to_string().into())],
                        ));
                        let ok = ui.button(tr("dialog-ok"));
                        self.take_focus(&ok);
                        if ok.clicked() {
                            close = true;
                        }
                    }
                    Notice::Failed(why) => {
                        ui.label(tr_args("update-failed", &[("why", why.into())]));
                        let ok = ui.button(tr("dialog-ok"));
                        self.take_focus(&ok);
                        if ok.clicked() {
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
        egui::Window::new(tr("crash-title"))
            .id(egui::Id::unique("product-crash"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                let program = report
                    .text
                    .lines()
                    .find_map(|l| l.strip_prefix("program:"))
                    .map_or_else(|| tr("crash-the-editor"), |p| p.trim().to_string());
                ui.label(tr_args(
                    "crash-text",
                    &[("count", count.into()), ("program", program.into())],
                ));
                ui.label(tr("crash-report-text"));
                ui.horizontal_wrapped(|ui| {
                    let report_button = ui.button(tr("crash-report"));
                    self.take_focus(&report_button);
                    if report_button.clicked() {
                        self.open(ctx, crash::issue_url(&report.text));
                        done = true;
                    }
                    if ui
                        .button(if self.show_report {
                            tr("crash-hide-report")
                        } else {
                            tr("crash-show-report")
                        })
                        .clicked()
                    {
                        self.show_report = !self.show_report;
                    }
                    if ui.button(tr("crash-dismiss")).clicked() {
                        done = true;
                    }
                });
                let mut never = !settings.prefs.crash_prompt;
                if ui.checkbox(&mut never, tr("crash-never")).changed() {
                    settings.set_crash_prompt(!never);
                }
                if self.show_report {
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new(&report.text).monospace().small());
                        });
                    ui.label(
                        RichText::new(tr_args(
                            "crash-saved-as",
                            &[("path", report.path.display().to_string().into())],
                        ))
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
            // The update question, if there is one, is next.
            self.want_focus = self.notice.is_some();
        }
    }
}
