//! The Library tab: every node by category, the blocks in the library
//! (shipped, the person's own, and imported), the shared libraries added
//! by link, and the groups in this project, searchable with the palette's
//! engine. Click to add at the center of the view, or drag onto the
//! canvas; the play button on a block plays its preview over the patch.

use ac_core::{Category, GROUP_KIND, Origin, PLUGIN_KIND};
use ac_io::index::IndexBlock;
use ac_io::{Library, LibraryBlock};
use egui::{Align2, Color32, Rect, RichText, Sense, vec2};

use super::{PanelIo, PanelRequest, markdown, section};
use crate::dialogs::BlockToExport;
use crate::i18n::{self, tr, tr_args};
use crate::icons;
use crate::online::{LibraryState, Playing, PreviewKey, SharedLibrary, can_preview};
use crate::palette::{self, Candidate};
use crate::theme;

/// The drag-and-drop payload of a library row.
pub type LibraryItem = Candidate;

/// Which blocks the Library tab lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockFilter {
    /// Only blocks from here; `None` for all of them.
    pub origin: Option<Origin>,
    /// Only blocks with this tag (lowercase).
    pub tag: Option<String>,
}

/// The library's blocks matching `query` (name, tags, description) and
/// `filter`, by origin and then name.
pub fn filter_blocks<'a>(
    library: &'a Library,
    query: &str,
    filter: &BlockFilter,
) -> Vec<&'a LibraryBlock> {
    library
        .search(query)
        .into_iter()
        .filter(|b| filter.origin.is_none_or(|o| b.source.origin == o))
        .filter(|b| {
            filter
                .tag
                .as_ref()
                .is_none_or(|t| b.block.def.tags.iter().any(|x| x.eq_ignore_ascii_case(t)))
        })
        .collect()
}

/// The first sentence of `text` (up to a full stop, question or
/// exclamation mark followed by a space, or the end of the first line).
pub fn first_sentence(text: &str) -> &str {
    let line = text.trim().lines().next().unwrap_or("");
    for (i, c) in line.char_indices() {
        if matches!(c, '.' | '!' | '?') {
            let end = i + c.len_utf8();
            if line[end..].chars().next().is_none_or(char::is_whitespace) {
                return &line[..end];
            }
        }
    }
    line
}

/// How the tab names each origin.
fn origin_label(o: Origin) -> String {
    match o {
        Origin::Shipped => tr("library-shipped"),
        Origin::User => tr("library-mine"),
        Origin::Imported => tr("library-imported"),
    }
}

fn origin_color(o: Origin) -> Color32 {
    match o {
        Origin::Shipped => theme::accent(),
        Origin::User => theme::ok(),
        Origin::Imported => theme::warning(),
    }
}

pub(super) fn show(
    ui: &mut egui::Ui,
    io: &mut PanelIo,
    query: &mut String,
    filter: &mut BlockFilter,
) {
    ui.add(
        egui::TextEdit::singleline(query)
            .hint_text(format!(
                "{}  {}",
                icons::MAGNIFYING_GLASS,
                tr("library-search")
            ))
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 5)),
    )
    .accessible_name(tr("library-search"));
    ui.add_space(4.0);
    // Blocks have their own area below, with its filters.
    let plugins = io.plugins.map_or(&[][..], |p| p.list());
    let all: Vec<Candidate> =
        palette::candidates(io.registry, io.library, io.doc, query, None, plugins)
            .into_iter()
            .filter(|c| c.block.is_none())
            .collect();
    if !query.trim().is_empty() {
        if all.is_empty() {
            ui.label(RichText::new(tr("library-no-nodes-match")).color(theme::text_muted()));
        }
        for c in &all {
            item(ui, io, c);
        }
        blocks(ui, io, query, filter);
        return;
    }
    for cat in Category::ALL {
        let items: Vec<&Candidate> = all
            .iter()
            .filter(|c| c.category == Some(cat) && c.kind != PLUGIN_KIND)
            .collect();
        if items.is_empty() {
            continue;
        }
        let header = RichText::new(format!("{}  ({})", i18n::category(cat), items.len()))
            .color(theme::text());
        egui::CollapsingHeader::new(header)
            .id_salt(("library", cat.slug()))
            .default_open(false)
            .show(ui, |ui| {
                for c in items {
                    item(ui, io, c);
                }
            });
    }
    let found: Vec<&Candidate> = all.iter().filter(|c| c.kind == PLUGIN_KIND).collect();
    if io.plugins.is_some() {
        let header = RichText::new(format!("Plugins  ({})", found.len())).color(theme::text());
        egui::CollapsingHeader::new(header)
            .id_salt(("library", "plugins"))
            .default_open(false)
            .show(ui, |ui| {
                if found.is_empty() {
                    ui.label(
                        RichText::new(
                            "No plugins found yet. Install CLAP or VST3 plugins, then rescan \
                             in the Devices tab.",
                        )
                        .small()
                        .color(theme::text_muted()),
                    );
                }
                for c in found {
                    item(ui, io, c);
                }
            });
    }
    blocks(ui, io, query, filter);
    let groups: Vec<&Candidate> = all.iter().filter(|c| c.kind == GROUP_KIND).collect();
    if !groups.is_empty() {
        section(ui, &tr("section-project-groups"));
        for c in groups {
            item(ui, io, c);
        }
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new(tr("library-hint"))
            .small()
            .color(theme::text_faint()),
    );
}

/// The Blocks area: origin filter, tag chips, the matching blocks, and
/// importing a block file.
fn blocks(ui: &mut egui::Ui, io: &mut PanelIo, query: &str, filter: &mut BlockFilter) {
    section(ui, &tr("section-blocks"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let choices = [
            None,
            Some(Origin::Shipped),
            Some(Origin::User),
            Some(Origin::Imported),
        ];
        for origin in choices {
            let label = origin.map_or_else(|| tr("library-all"), origin_label);
            let count = io
                .library
                .blocks()
                .iter()
                .filter(|b| origin.is_none_or(|o| b.source.origin == o))
                .count();
            let on = filter.origin == origin;
            let text = RichText::new(format!("{label} {count}"))
                .size(theme::FONT_SMALL)
                .color(if on {
                    theme::text_strong()
                } else {
                    theme::text_muted()
                });
            let b = egui::Button::selectable(on, text).min_size(vec2(0.0, 22.0));
            if ui.add(b).clicked() {
                filter.origin = origin;
            }
        }
    });
    let tags = io.library.tags();
    if !tags.is_empty() {
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for t in tags {
                let on = filter.tag.as_deref() == Some(t.as_str());
                let text = RichText::new(format!("#{t}"))
                    .size(theme::FONT_SMALL - 0.5)
                    .color(if on {
                        theme::text_strong()
                    } else {
                        theme::text_muted()
                    });
                let chip = egui::Button::new(text)
                    .selected(on)
                    .fill(if on {
                        theme::accent_dim()
                    } else {
                        theme::border()
                    })
                    .corner_radius(theme::radius(9.0));
                let hint = if on {
                    tr("library-tag-on-tip")
                } else {
                    tr("library-tag-tip")
                };
                if ui.add(chip).on_hover_text(hint).clicked() {
                    filter.tag = if on { None } else { Some(t.clone()) };
                }
            }
        });
    }
    ui.add_space(4.0);
    let list = filter_blocks(io.library, query, filter);
    if list.is_empty() {
        let hint = match filter.origin {
            Some(Origin::User) if filter.tag.is_none() && query.trim().is_empty() => {
                tr("library-no-user-blocks")
            }
            Some(Origin::Imported) if filter.tag.is_none() && query.trim().is_empty() => {
                tr("library-no-imported-blocks")
            }
            _ => tr("library-no-blocks-match"),
        };
        ui.label(RichText::new(hint).small().color(theme::text_muted()));
    }
    for b in list {
        block_row(ui, io, b);
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if icons::spoken(ui.button(format!(
            "{}  {}",
            icons::DOWNLOAD_SIMPLE,
            tr("library-import")
        )))
        .on_hover_text(tr("library-import-tip"))
        .clicked()
        {
            io.requests.push(PanelRequest::ImportBlockFile);
        }
        if icons::spoken(ui.button(format!(
            "{}  {}",
            icons::LINK,
            crate::actions::Action::ImportFromLink.label()
        )))
        .on_hover_text(tr("library-import-link-tip"))
        .clicked()
        {
            io.requests
                .push(PanelRequest::Run(crate::actions::Action::ImportFromLink));
        }
    });
    shared_libraries(ui, io, query);
}

/// `widget` at `rect`, over what's drawn there, without moving the
/// layout's cursor (`Ui::put` would move it back up into the row). `salt`
/// tells the rows' buttons apart.
fn put_over(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    rect: Rect,
    widget: impl egui::Widget,
) -> egui::Response {
    ui.new_child(egui::UiBuilder::new().id_salt(salt).max_rect(rect))
        .put(rect, widget)
}

/// The play button on `name`'s row: plays `key`'s preview, or stops it.
/// It shows only an icon, so it says what it does to screen readers.
fn preview_button(
    ui: &mut egui::Ui,
    io: &mut PanelIo,
    rect: Rect,
    key: PreviewKey,
    name: &str,
    tip: String,
) {
    let args = [("name", name.into())];
    let (icon, tip, spoken) = match &io.online.playing {
        Playing::Playing(k, _) if *k == key => (
            icons::STOP,
            tr("library-preview-stop-tip"),
            tr_args("library-preview-stop", &args),
        ),
        Playing::Loading(k) if *k == key => (
            icons::DOTS_THREE,
            tr("library-preview-loading-tip"),
            tr_args("library-preview-cancel", &args),
        ),
        _ => (icons::PLAY, tip, tr_args("library-preview-play", &args)),
    };
    let button = egui::Button::new(
        RichText::new(icon)
            .size(theme::FONT_UI)
            .color(theme::text_strong()),
    )
    .corner_radius(theme::radius(5.0));
    let r = put_over(ui, ("preview", &key), rect, button)
        .on_hover_text(tip)
        .accessible_name(spoken);
    if r.clicked() {
        io.requests.push(PanelRequest::Preview(key));
    }
}

/// Whether `entry` matches `query` the way library blocks do (words, any
/// order, in the name, tags, or description).
fn entry_matches(entry: &IndexBlock, query: &str) -> bool {
    let hay = format!(
        "{} {} {}",
        entry.name,
        entry.tags.join(" "),
        entry.description
    )
    .to_lowercase();
    query
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

/// The shared libraries added by link: each one's blocks, to preview and
/// install.
fn shared_libraries(ui: &mut egui::Ui, io: &mut PanelIo, query: &str) {
    section(ui, &tr("section-shared-libraries"));
    if io.online.libraries.is_empty() {
        ui.label(
            RichText::new(tr("library-no-shared"))
                .small()
                .color(theme::text_muted()),
        );
        return;
    }
    for lib in &io.online.libraries {
        shared_library(ui, io, lib, query);
    }
}

fn shared_library(ui: &mut egui::Ui, io: &mut PanelIo, lib: &SharedLibrary, query: &str) {
    if lib.state == LibraryState::NotLoaded {
        io.requests
            .push(PanelRequest::RefreshLibrary(lib.link.clone()));
    }
    let count = lib.ready().map(|r| r.index.blocks.len());
    let title = match count {
        Some(n) => format!("{}  {}  ({n})", icons::BOOKS, lib.name()),
        None => format!("{}  {}", icons::BOOKS, lib.name()),
    };
    egui::CollapsingHeader::new(RichText::new(title).color(theme::text()))
        .id_salt(("shared-library", &lib.link))
        .default_open(true)
        .show(ui, |ui| {
            match &lib.state {
                LibraryState::NotLoaded | LibraryState::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new(tr("library-reading"))
                                .small()
                                .color(theme::text_muted()),
                        );
                    });
                }
                LibraryState::Failed(e) => {
                    ui.label(
                        RichText::new(format!("{}  {e}", icons::WARNING))
                            .small()
                            .color(theme::warning()),
                    );
                }
                LibraryState::Ready(r) => {
                    if !r.index.description.is_empty() {
                        ui.label(
                            RichText::new(&r.index.description)
                                .small()
                                .color(theme::text_muted()),
                        );
                    }
                    let shown: Vec<&IndexBlock> = r
                        .index
                        .blocks
                        .iter()
                        .filter(|e| entry_matches(e, query))
                        .collect();
                    if shown.is_empty() {
                        let text = if r.index.blocks.is_empty() {
                            tr("library-shared-empty")
                        } else {
                            tr("library-no-blocks-match")
                        };
                        ui.label(RichText::new(text).small().color(theme::text_muted()));
                    }
                    for e in shown {
                        remote_row(ui, io, lib, e);
                    }
                }
            }
            ui.horizontal(|ui| {
                let busy = lib.state == LibraryState::Loading;
                if icons::spoken(
                    ui.add_enabled(
                        !busy,
                        egui::Button::new(format!(
                            "{}  {}",
                            icons::ARROWS_CLOCKWISE,
                            tr("library-shared-refresh")
                        ))
                        .small(),
                    ),
                )
                .on_hover_text(tr_args(
                    "library-shared-refresh-tip",
                    &[("link", (&lib.link).into())],
                ))
                .clicked()
                {
                    io.requests
                        .push(PanelRequest::RefreshLibrary(lib.link.clone()));
                }
                if icons::spoken(
                    ui.add(
                        egui::Button::new(format!("{}  {}", icons::X, tr("library-shared-remove")))
                            .small(),
                    ),
                )
                .on_hover_text(tr("library-shared-remove-tip"))
                .clicked()
                {
                    io.requests
                        .push(PanelRequest::RemoveLibrary(lib.link.clone()));
                }
            });
        });
}

/// A block in a shared library: what the index says of it, its preview,
/// and Install.
fn remote_row(ui: &mut egui::Ui, io: &mut PanelIo, lib: &SharedLibrary, e: &IndexBlock) {
    let width = ui.available_width();
    let height = if e.tags.is_empty() { 38.0 } else { 54.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, theme::radius(5.0), theme::row_hover());
    }
    p.rect_filled(
        Rect::from_min_size(rect.min + vec2(4.0, 7.0), vec2(3.0, rect.height() - 14.0)),
        theme::radius(1.5),
        theme::fade(theme::group_header(), 0.6),
    );
    let installing = lib.installing.contains(&e.id);
    let installed = io.library.blocks().iter().any(|b| {
        b.source.origin != Origin::Shipped
            && b.block.def.name == e.name
            && b.block.def.version == e.version
    });
    // Buttons on the right: Install (or what it's doing), then the preview.
    let install_w = 64.0;
    let mut right = rect.right() - 6.0;
    let install_rect = Rect::from_min_size(
        egui::pos2(right - install_w, rect.top() + 4.0),
        vec2(install_w, 22.0),
    );
    right -= install_w + 4.0;
    let play_rect =
        Rect::from_min_size(egui::pos2(right - 26.0, rect.top() + 4.0), vec2(26.0, 22.0));
    if e.preview.is_some() {
        right -= 30.0;
    }
    let text_w = (right - rect.left() - 18.0).max(40.0);
    let mut name = egui::text::LayoutJob::single_section(
        format!("{}  {}", icons::STACK, e.name),
        egui::TextFormat::simple(theme::ui_font(theme::FONT_UI), theme::text_strong()),
    );
    name.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
    let g = p.layout_job(name);
    p.galley(rect.min + vec2(14.0, 5.0), g, theme::text_strong());
    let by = if e.author.is_empty() {
        String::new()
    } else {
        format!(" · {}", e.author)
    };
    let mut job = egui::text::LayoutJob::single_section(
        format!("v{}{by} · {}", e.version, first_sentence(&e.description)),
        egui::TextFormat::simple(theme::ui_font(theme::FONT_SMALL), theme::text_muted()),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
    let g = p.layout_job(job);
    p.galley(rect.min + vec2(14.0, 21.0), g, theme::text_muted());
    if !e.tags.is_empty() {
        let tags: Vec<String> = e
            .tags
            .iter()
            .map(|t| format!("#{}", t.to_lowercase()))
            .collect();
        let mut job = egui::text::LayoutJob::single_section(
            tags.join("  "),
            egui::TextFormat::simple(theme::ui_font(theme::FONT_SMALL - 0.5), theme::text_faint()),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
        let g = p.layout_job(job);
        p.galley(rect.min + vec2(14.0, 37.0), g, theme::text_faint());
    }
    if e.preview.is_some() {
        let key = PreviewKey::Remote {
            link: lib.link.clone(),
            id: e.id.clone(),
        };
        preview_button(
            ui,
            io,
            play_rect,
            key,
            &e.name,
            tr("library-preview-remote-tip"),
        );
    }
    if installing {
        put_over(
            ui,
            ("installing", &e.id),
            install_rect,
            egui::Spinner::new(),
        );
    } else if installed {
        put_over(
            ui,
            ("installed", &e.id),
            install_rect,
            egui::Label::new(
                RichText::new(format!("{} {}", icons::CHECK, tr("library-installed")))
                    .small()
                    .color(theme::ok()),
            ),
        )
        .on_hover_text(tr("library-installed-tip"));
    } else {
        let r = put_over(
            ui,
            ("install", &lib.link, &e.id),
            install_rect,
            egui::Button::new(tr("library-install")).small(),
        )
        .on_hover_text(tr("library-install-tip"))
        .accessible_name(tr_args(
            "library-install-named",
            &[("name", (&e.name).into())],
        ));
        if r.clicked() {
            io.requests.push(PanelRequest::Install {
                link: lib.link.clone(),
                id: e.id.clone(),
            });
        }
    }
    // The row is painted: it tells screen readers what the block is.
    let spoken = tr_args(
        "library-remote-row",
        &[
            ("name", (&e.name).into()),
            ("version", e.version.to_string().into()),
            ("about", first_sentence(&e.description).into()),
        ],
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, &spoken));
    resp.on_hover_ui(|ui| {
        ui.set_max_width(340.0);
        if e.description.is_empty() {
            ui.label(RichText::new(tr("library-no-description")).color(theme::text_muted()));
        } else {
            markdown::show(ui, &e.description);
        }
        ui.label(
            RichText::new(tr_args(
                "library-shared-from",
                &[("library", lib.name().into()), ("file", (&e.file).into())],
            ))
            .small()
            .color(theme::text_faint()),
        );
    });
}

fn block_row(ui: &mut egui::Ui, io: &mut PanelIo, b: &LibraryBlock) {
    let c = palette::block_candidate(b, None);
    let d = &b.block.def;
    let width = ui.available_width();
    let height = if d.tags.is_empty() { 38.0 } else { 54.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, theme::radius(5.0), theme::row_hover());
    }
    p.rect_filled(
        egui::Rect::from_min_size(rect.min + vec2(4.0, 7.0), vec2(3.0, rect.height() - 14.0)),
        theme::radius(1.5),
        theme::group_header(),
    );
    let name = p.text(
        rect.min + vec2(14.0, 5.0),
        Align2::LEFT_TOP,
        format!("{}  {}", icons::STACK, d.name),
        theme::ui_font(theme::FONT_UI),
        theme::text_strong(),
    );
    // Where it came from, as a small badge after the name.
    let origin = b.source.origin;
    let badge = p.layout_no_wrap(
        origin_label(origin).to_lowercase(),
        theme::ui_font(theme::FONT_SMALL - 1.0),
        origin_color(origin),
    );
    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(
            name.right() + 8.0,
            name.center().y - badge.size().y / 2.0 - 1.0,
        ),
        badge.size() + vec2(10.0, 2.0),
    );
    p.rect_filled(
        badge_rect,
        theme::radius(4.0),
        theme::fade(origin_color(origin), 0.16),
    );
    p.galley(badge_rect.min + vec2(5.0, 1.0), badge, origin_color(origin));
    let previewable = can_preview(&b.block);
    p.text(
        rect.right_top() + vec2(if previewable { -38.0 } else { -8.0 }, 7.0),
        Align2::RIGHT_TOP,
        format!("v{}", d.version),
        theme::mono_font(theme::FONT_SMALL),
        theme::text_faint(),
    );
    let mut job = egui::text::LayoutJob::single_section(
        first_sentence(&d.description).to_string(),
        egui::TextFormat::simple(theme::ui_font(theme::FONT_SMALL), theme::text_muted()),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width - 20.0);
    let g = p.layout_job(job);
    p.galley(rect.min + vec2(14.0, 21.0), g, theme::text_muted());
    if !d.tags.is_empty() {
        let tags: Vec<String> = d
            .tags
            .iter()
            .map(|t| format!("#{}", t.to_lowercase()))
            .collect();
        let mut job = egui::text::LayoutJob::single_section(
            tags.join("  "),
            egui::TextFormat::simple(theme::ui_font(theme::FONT_SMALL - 0.5), theme::text_faint()),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(width - 20.0);
        let g = p.layout_job(job);
        p.galley(rect.min + vec2(14.0, 37.0), g, theme::text_faint());
    }
    // One button to screen readers: the block, where it's from, and what
    // it does.
    let spoken = tr_args(
        "library-block-row",
        &[
            ("name", (&d.name).into()),
            ("origin", origin_label(origin).into()),
            ("version", d.version.to_string().into()),
            ("about", first_sentence(&d.description).into()),
        ],
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &spoken));
    resp.dnd_set_drag_payload(c.clone());
    if resp.clicked() {
        io.requests.push(PanelRequest::Add(c.clone()));
    }
    // Over the row, so a click plays instead of adding.
    if previewable {
        let play = Rect::from_min_size(
            egui::pos2(rect.right() - 32.0, rect.top() + 5.0),
            vec2(26.0, 22.0),
        );
        let tip = if b.preview_path().is_some() {
            tr("library-preview-tip")
        } else {
            tr("library-preview-render-tip")
        };
        preview_button(
            ui,
            io,
            play,
            PreviewKey::Library(b.source.clone()),
            &d.name,
            tip,
        );
    }
    resp.context_menu(|ui| {
        if icons::spoken(ui.button(format!("{}  {}", icons::PLUS, tr("library-add-to-canvas"))))
            .clicked()
        {
            io.requests.push(PanelRequest::Add(c.clone()));
            ui.close();
        }
        if icons::spoken(ui.button(format!("{}  {}", icons::EXPORT, tr("library-export"))))
            .on_hover_text(tr("library-export-tip"))
            .clicked()
        {
            io.requests
                .push(PanelRequest::ExportBlock(BlockToExport::Library(
                    b.source.clone(),
                )));
            ui.close();
        }
        if origin == Origin::Shipped {
            ui.label(
                RichText::new(tr("library-shipped-stay"))
                    .small()
                    .color(theme::text_muted()),
            );
        } else if icons::spoken(ui.button(format!("{}  {}", icons::TRASH, tr("library-remove"))))
            .clicked()
        {
            io.requests
                .push(PanelRequest::RemoveBlock(b.source.clone()));
            ui.close();
        }
    });
    resp.on_hover_ui(|ui| {
        ui.set_max_width(340.0);
        if d.description.is_empty() {
            ui.label(RichText::new(tr("library-no-description")).color(theme::text_muted()));
        } else {
            markdown::show(ui, &d.description);
        }
        let args = [
            ("origin", origin_label(origin).into()),
            ("version", d.version.to_string().into()),
            ("author", (&d.author).into()),
        ];
        let text = if d.author.is_empty() {
            tr_args("library-block-tip", &args)
        } else {
            tr_args("library-block-tip-by", &args)
        };
        ui.label(RichText::new(text).small().color(theme::text_faint()));
    });
}

fn item(ui: &mut egui::Ui, io: &mut PanelIo, c: &Candidate) {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click_and_drag());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, theme::radius(5.0), theme::row_hover());
    }
    let color = c
        .category
        .map_or(theme::group_header(), theme::category_color);
    p.rect_filled(
        egui::Rect::from_min_size(rect.min + vec2(4.0, 7.0), vec2(3.0, rect.height() - 14.0)),
        theme::radius(1.5),
        color,
    );
    let icon = if c.kind == GROUP_KIND {
        format!("{}  ", icons::STACK)
    } else if c.kind == PLUGIN_KIND {
        format!("{}  ", icons::PUZZLE_PIECE)
    } else {
        String::new()
    };
    p.text(
        rect.min + vec2(14.0, 5.0),
        Align2::LEFT_TOP,
        format!("{icon}{}", c.name),
        theme::ui_font(theme::FONT_UI),
        theme::text_strong(),
    );
    let mut job = egui::text::LayoutJob::single_section(
        c.summary.clone(),
        egui::TextFormat::simple(theme::ui_font(theme::FONT_SMALL), theme::text_muted()),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width - 20.0);
    let g = p.layout_job(job);
    p.galley(rect.min + vec2(14.0, 21.0), g, theme::text_muted());
    let spoken = if c.summary.is_empty() {
        c.name.clone()
    } else {
        format!("{}: {}", c.name, c.summary)
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &spoken));
    resp.dnd_set_drag_payload(c.clone());
    if resp.clicked() {
        io.requests.push(PanelRequest::Add(c.clone()));
    }
    resp.on_hover_text(&c.summary);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{Block, GroupDef, NodeId, NodeInstance};

    fn block(name: &str, tags: &[&str]) -> Block {
        let mut def = GroupDef::new(name);
        def.description = format!("{name} does a thing. More about it.");
        def.tags = tags.iter().map(|t| t.to_string()).collect();
        def.graph.nodes.push(NodeInstance::new(NodeId(1), "gain"));
        Block {
            def,
            uses: Default::default(),
            preview: None,
        }
    }

    /// The shipped blocks, one of the person's, and one imported.
    fn library() -> Library {
        let mut lib = Library::shipped_only();
        // Tags no shipped block uses, so the shipped set can grow freely.
        lib.save_user(block("My Bass", &["t-synth", "T-Bass"]))
            .unwrap();
        lib.import(block("Their Echo", &["t-space", "t-delay"]), "echo")
            .unwrap();
        lib
    }

    fn names(list: &[&LibraryBlock]) -> Vec<String> {
        list.iter().map(|b| b.name().to_string()).collect()
    }

    #[test]
    fn blocks_filter_by_origin() {
        let lib = library();
        let all = filter_blocks(&lib, "", &BlockFilter::default());
        assert_eq!(all.len(), lib.blocks().len());
        assert_eq!(all[0].source.origin, Origin::Shipped, "shipped first");
        let only = |origin| {
            let f = BlockFilter {
                origin: Some(origin),
                tag: None,
            };
            names(&filter_blocks(&lib, "", &f))
        };
        assert!(only(Origin::Shipped).contains(&"Tremolo".to_string()));
        assert_eq!(only(Origin::User), ["My Bass"]);
        assert_eq!(only(Origin::Imported), ["Their Echo"]);
    }

    #[test]
    fn blocks_filter_by_tag_and_query() {
        let lib = library();
        let tagged = |tag: &str| BlockFilter {
            origin: None,
            tag: Some(tag.to_string()),
        };
        assert_eq!(
            names(&filter_blocks(&lib, "", &tagged("t-space"))),
            ["Their Echo"]
        );
        // Tags match whatever their case in the file.
        assert_eq!(
            names(&filter_blocks(&lib, "", &tagged("t-bass"))),
            ["My Bass"]
        );
        assert!(filter_blocks(&lib, "", &tagged("t-nothing")).is_empty());
        // The tag and the search both apply.
        assert!(filter_blocks(&lib, "bass", &tagged("t-space")).is_empty());
        let mine_synth = BlockFilter {
            origin: Some(Origin::User),
            tag: Some("t-synth".into()),
        };
        assert_eq!(names(&filter_blocks(&lib, "", &mine_synth)), ["My Bass"]);
        let imported_synth = BlockFilter {
            origin: Some(Origin::Imported),
            tag: Some("t-synth".into()),
        };
        assert!(filter_blocks(&lib, "", &imported_synth).is_empty());
        assert_eq!(
            names(&filter_blocks(&lib, "tremolo", &BlockFilter::default())),
            ["Tremolo"]
        );
    }

    #[test]
    fn first_sentences() {
        assert_eq!(
            first_sentence("Pulses the volume. `rate` sets how fast."),
            "Pulses the volume."
        );
        assert_eq!(first_sentence("No full stop\n\nSecond"), "No full stop");
        assert_eq!(first_sentence("Is it 2.5 dB? Yes."), "Is it 2.5 dB?");
        assert_eq!(first_sentence("Ends here."), "Ends here.");
        assert_eq!(first_sentence(""), "");
    }
}
