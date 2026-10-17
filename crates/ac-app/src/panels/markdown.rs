//! A small Markdown renderer for node docs: headings, paragraphs, bold,
//! italics, inline code, lists, code blocks, and tables. Node doc comments
//! use only these.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, RichText};

use crate::theme;

#[derive(Debug, PartialEq)]
enum Block {
    Heading(usize, String),
    Paragraph(String),
    Item(String),
    Code(String),
    Table(Vec<Vec<String>>),
}

fn parse(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para = String::new();
    let mut lines = md.lines().peekable();
    let flush = |para: &mut String, out: &mut Vec<Block>| {
        if !para.trim().is_empty() {
            out.push(Block::Paragraph(para.trim().to_string()));
        }
        para.clear();
    };
    while let Some(line) = lines.next() {
        let t = line.trim();
        if t.starts_with("```") {
            flush(&mut para, &mut out);
            let mut code = String::new();
            for l in lines.by_ref() {
                if l.trim().starts_with("```") {
                    break;
                }
                code.push_str(l);
                code.push('\n');
            }
            out.push(Block::Code(code.trim_end().to_string()));
        } else if t.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(h) = t.strip_prefix('#') {
            flush(&mut para, &mut out);
            let level = 1 + h.chars().take_while(|c| *c == '#').count();
            out.push(Block::Heading(
                level,
                h.trim_start_matches('#').trim().to_string(),
            ));
        } else if let Some(item) = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .or_else(|| numbered(t))
        {
            flush(&mut para, &mut out);
            let mut text = item.to_string();
            // Continuation lines of the item, indented.
            while let Some(next) = lines.peek() {
                if next.starts_with("  ")
                    && !next.trim().is_empty()
                    && numbered(next.trim()).is_none()
                    && !next.trim().starts_with("- ")
                {
                    text.push(' ');
                    text.push_str(next.trim());
                    lines.next();
                } else {
                    break;
                }
            }
            out.push(Block::Item(text));
        } else if t.starts_with('|') {
            flush(&mut para, &mut out);
            let mut rows = vec![cells(t)];
            while let Some(next) = lines.peek() {
                let n = next.trim();
                if !n.starts_with('|') {
                    break;
                }
                if !n.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
                    rows.push(cells(n));
                }
                lines.next();
            }
            out.push(Block::Table(rows));
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(t);
        }
    }
    flush(&mut para, &mut out);
    out
}

fn numbered(t: &str) -> Option<&str> {
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    t[digits..].strip_prefix(". ")
}

fn cells(row: &str) -> Vec<String> {
    row.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

/// Inline formatting: `**bold**`, `*italic*`, `` `code` ``.
fn inline(text: &str, size: f32, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let normal = TextFormat::simple(theme::ui_font(size), color);
    let mut bold = normal.clone();
    bold.color = theme::TEXT_STRONG;
    let mut italic = normal.clone();
    italic.italics = true;
    let mut code = TextFormat::simple(
        theme::mono_font(size - 1.0),
        Color32::from_rgb(0xE6, 0xC0, 0x8A),
    );
    code.background = Color32::from_rgb(0x2C, 0x30, 0x38);
    let mut rest = text;
    let mut buf = String::new();
    let push = |job: &mut LayoutJob, s: &mut String, f: &TextFormat| {
        if !s.is_empty() {
            job.append(s, 0.0, f.clone());
            s.clear();
        }
    };
    while let Some(c) = rest.chars().next() {
        if let Some(r) = rest.strip_prefix("**")
            && let Some(end) = r.find("**")
        {
            push(&mut job, &mut buf, &normal);
            job.append(&r[..end], 0.0, bold.clone());
            rest = &r[end + 2..];
            continue;
        }
        if c == '`'
            && let Some(end) = rest[1..].find('`')
        {
            push(&mut job, &mut buf, &normal);
            job.append(&rest[1..1 + end], 0.0, code.clone());
            rest = &rest[end + 2..];
            continue;
        }
        if c == '*'
            && let Some(end) = rest[1..].find('*')
            && end > 0
        {
            push(&mut job, &mut buf, &normal);
            job.append(&rest[1..1 + end], 0.0, italic.clone());
            rest = &rest[end + 2..];
            continue;
        }
        buf.push(c);
        rest = &rest[c.len_utf8()..];
    }
    push(&mut job, &mut buf, &normal);
    job
}

/// Render Markdown into the panel.
pub fn show(ui: &mut egui::Ui, md: &str) {
    let width = ui.available_width();
    for (i, block) in parse(md).into_iter().enumerate() {
        match block {
            Block::Heading(level, text) => {
                if i > 0 {
                    ui.add_space(6.0);
                }
                let size = match level {
                    1 => theme::FONT_TITLE,
                    2 => theme::FONT_HEADING,
                    _ => theme::FONT_UI + 1.0,
                };
                ui.label(RichText::new(text).size(size).color(theme::TEXT_STRONG));
            }
            Block::Paragraph(text) => {
                let mut job = inline(&text, theme::FONT_UI, theme::TEXT);
                job.wrap.max_width = width;
                ui.add(egui::Label::new(job).wrap());
                ui.add_space(4.0);
            }
            Block::Item(text) => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new("•").color(theme::TEXT_MUTED));
                    let mut job = inline(&text, theme::FONT_UI, theme::TEXT);
                    job.wrap.max_width = ui.available_width();
                    ui.add(egui::Label::new(job).wrap());
                });
            }
            Block::Code(code) => {
                egui::Frame::new()
                    .fill(theme::WELL_BG)
                    .corner_radius(theme::radius(5.0))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_width(width - 16.0);
                        ui.label(
                            RichText::new(code)
                                .font(theme::mono_font(theme::FONT_VALUE))
                                .color(theme::TEXT),
                        );
                    });
            }
            Block::Table(rows) => {
                egui::Grid::new(ui.next_auto_id())
                    .striped(true)
                    .spacing(egui::vec2(12.0, 4.0))
                    .show(ui, |ui| {
                        for (r, row) in rows.iter().enumerate() {
                            for c in row {
                                let color = if r == 0 {
                                    theme::TEXT_STRONG
                                } else {
                                    theme::TEXT
                                };
                                ui.add(
                                    egui::Label::new(inline(c, theme::FONT_SMALL + 0.5, color))
                                        .wrap(),
                                );
                            }
                            ui.end_row();
                        }
                    });
                ui.add_space(4.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_shapes_node_docs_use() {
        let md = "A **bold** start\nthat wraps.\n\n# Title\n- one\n  more\n- two\n1. first\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```\ncode\n```";
        let b = parse(md);
        assert_eq!(
            b[0],
            Block::Paragraph("A **bold** start that wraps.".into())
        );
        assert_eq!(b[1], Block::Heading(1, "Title".into()));
        assert_eq!(b[2], Block::Item("one more".into()));
        assert_eq!(b[3], Block::Item("two".into()));
        assert_eq!(b[4], Block::Item("first".into()));
        assert_eq!(
            b[5],
            Block::Table(vec![
                vec!["a".into(), "b".into()],
                vec!["1".into(), "2".into()]
            ])
        );
        assert_eq!(b[6], Block::Code("code".into()));
    }

    #[test]
    fn inline_formatting_splits_sections() {
        let job = inline("Try **this**: wire `out` into *it*", 13.0, theme::TEXT);
        let texts: Vec<&str> = job
            .sections
            .iter()
            .map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0])
            .collect();
        assert_eq!(texts, ["Try ", "this", ": wire ", "out", " into ", "it"]);
        // Unbalanced markers are plain text.
        let job = inline("2 * 3 and `x", 13.0, theme::TEXT);
        assert_eq!(job.text, "2 * 3 and `x");
    }
}
