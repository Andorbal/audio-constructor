//! The Library tab: every node by category, the blocks in the library
//! (shipped, the person's own, and imported), and the groups in this
//! project, searchable with the palette's engine. Click to add at the
//! center of the view, or drag onto the canvas.

use ac_core::{Category, GROUP_KIND, Origin, PLUGIN_KIND};
use ac_io::{Library, LibraryBlock};
use egui::{Align2, Color32, RichText, Sense, vec2};

use super::{PanelIo, PanelRequest, markdown, section};
use crate::icons;
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
fn origin_label(o: Origin) -> &'static str {
    match o {
        Origin::Shipped => "Shipped",
        Origin::User => "Mine",
        Origin::Imported => "Imported",
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
                "{}  Search nodes and blocks",
                icons::MAGNIFYING_GLASS
            ))
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 5)),
    );
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
            ui.label(RichText::new("No nodes match.").color(theme::text_muted()));
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
        let header =
            RichText::new(format!("{}  ({})", cat.name(), items.len())).color(theme::text());
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
        section(ui, "Groups in this project");
        for c in groups {
            item(ui, io, c);
        }
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new("Drag a node or block onto the canvas, or click to add it in the middle.")
            .small()
            .color(theme::text_faint()),
    );
}

/// The Blocks area: origin filter, tag chips, the matching blocks, and
/// importing a block file.
fn blocks(ui: &mut egui::Ui, io: &mut PanelIo, query: &str, filter: &mut BlockFilter) {
    section(ui, "Blocks");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let choices = [
            None,
            Some(Origin::Shipped),
            Some(Origin::User),
            Some(Origin::Imported),
        ];
        for origin in choices {
            let label = origin.map_or("All", origin_label);
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
                    "Click again to show every tag"
                } else {
                    "Only blocks with this tag"
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
                "None yet. Select a group and press Save to library in the Node tab."
            }
            Some(Origin::Imported) if filter.tag.is_none() && query.trim().is_empty() => {
                "None yet. Blocks arrive with packs and block files you open."
            }
            _ => "No blocks match.",
        };
        ui.label(RichText::new(hint).small().color(theme::text_muted()));
    }
    for b in list {
        block_row(ui, io, b);
    }
    ui.add_space(4.0);
    if ui
        .button(format!("{}  Import block file…", icons::DOWNLOAD_SIMPLE))
        .on_hover_text("Add a block file someone shared to your library, under Imported.")
        .clicked()
    {
        io.requests.push(PanelRequest::ImportBlockFile);
    }
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
    p.text(
        rect.right_top() + vec2(-8.0, 7.0),
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
    resp.dnd_set_drag_payload(c.clone());
    if resp.clicked() {
        io.requests.push(PanelRequest::Add(c.clone()));
    }
    resp.context_menu(|ui| {
        if ui
            .button(format!("{}  Add to canvas", icons::PLUS))
            .clicked()
        {
            io.requests.push(PanelRequest::Add(c.clone()));
            ui.close();
        }
        if origin == Origin::Shipped {
            ui.label(
                RichText::new("Shipped blocks stay in the library.")
                    .small()
                    .color(theme::text_muted()),
            );
        } else if ui
            .button(format!("{}  Remove from library", icons::TRASH))
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
            ui.label(RichText::new("No description.").color(theme::text_muted()));
        } else {
            markdown::show(ui, &d.description);
        }
        let by = if d.author.is_empty() {
            String::new()
        } else {
            format!(" · by {}", d.author)
        };
        ui.label(
            RichText::new(format!(
                "{} block · version {}{by} · right-click for more",
                origin_label(origin),
                d.version
            ))
            .small()
            .color(theme::text_faint()),
        );
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
