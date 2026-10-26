//! The zone map for `Zones`: notes across, velocity up, each zone a box.
//! Drag a box to move it, an edge to resize it; double-click an empty
//! place to add a zone there; right-click a zone to remove it.

use ac_core::Config;
use ac_nodes::sequencing::zones::{MAX_ZONES, Zone, zone_list};
use egui::{Align2, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use super::{Edit, area, grid, load, store};
use crate::theme;

/// Which part of a zone the pointer holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Body,
    Low,
    High,
    Soft,
    Hard,
}

/// How near an edge counts as on it, in points.
const EDGE: f32 = 5.0;

/// A zone's box in `rect`.
pub fn zone_rect(rect: Rect, z: &Zone) -> Rect {
    let x = |n: f32| rect.min.x + n / 128.0 * rect.width();
    let y = |v: f32| rect.max.y - v * rect.height();
    Rect::from_min_max(pos2(x(z.low), y(z.hard)), pos2(x(z.high + 1.0), y(z.soft)))
}

/// The zone and part under `p`: edges first, the topmost (last) zone wins.
pub fn hit(rect: Rect, zones: &[Zone], p: Pos2) -> Option<(usize, Part)> {
    for (k, z) in zones.iter().enumerate().rev() {
        let r = zone_rect(rect, z);
        if !r.expand(EDGE).contains(p) {
            continue;
        }
        let part = if (p.x - r.min.x).abs() <= EDGE {
            Part::Low
        } else if (p.x - r.max.x).abs() <= EDGE {
            Part::High
        } else if (p.y - r.min.y).abs() <= EDGE {
            Part::Hard
        } else if (p.y - r.max.y).abs() <= EDGE {
            Part::Soft
        } else if r.contains(p) {
            Part::Body
        } else {
            continue;
        };
        return Some((k, part));
    }
    None
}

/// `z` with `part` dragged by `notes` and `velocity`, kept in range and at
/// least one note and a hair of velocity wide.
pub fn dragged(z: Zone, part: Part, notes: f32, velocity: f32) -> Zone {
    let mut z = z;
    match part {
        Part::Body => {
            let w = z.high - z.low;
            z.low = (z.low + notes).round().clamp(0.0, 127.0 - w);
            z.high = z.low + w;
            let h = z.hard - z.soft;
            z.soft = (z.soft + velocity).clamp(0.0, 1.0 - h);
            z.hard = z.soft + h;
        }
        Part::Low => z.low = (z.low + notes).round().clamp(0.0, z.high),
        Part::High => z.high = (z.high + notes).round().clamp(z.low, 127.0),
        Part::Soft => z.soft = (z.soft + velocity).clamp(0.0, z.hard - 0.01),
        Part::Hard => z.hard = (z.hard + velocity).clamp(z.soft + 0.01, 1.0),
    }
    z
}

/// The `zones` config for `zones`.
pub fn to_config(zones: &[Zone]) -> serde_json::Value {
    let round = |v: f32| (f64::from(v) * 100.0).round() / 100.0;
    serde_json::Value::Array(
        zones
            .iter()
            .map(|z| json!([z.low, z.high, round(z.soft), round(z.hard)]))
            .collect(),
    )
}

/// A drag in progress: the zone, the part, where it started, and the zone
/// as it was then.
#[derive(Clone, Copy)]
struct Drag {
    index: usize,
    part: Part,
    origin: Pos2,
    start: Zone,
}

pub fn ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    config: &Config,
    width: f32,
    height: f32,
) -> Option<Edit> {
    let (resp, rect) = area(ui, width, height);
    let mut zones = zone_list(config);
    if config.get("zones").is_none() {
        // The default map is one zone; edits start from it.
        zones.truncate(1);
    }
    let p = ui.painter_at(rect);
    // Octave lines at every C, labelled.
    for c in (0..128).step_by(12) {
        let x = rect.min.x + c as f32 / 128.0 * rect.width();
        p.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0, grid(true)),
        );
        p.text(
            pos2(x + 2.0, rect.bottom() - 2.0),
            Align2::LEFT_BOTTOM,
            format!("C{}", c / 12 - 1),
            theme::mono_font(theme::FONT_SMALL - 1.0),
            theme::text_faint(),
        );
    }
    p.line_segment(
        [
            pos2(rect.left(), rect.center().y),
            pos2(rect.right(), rect.center().y),
        ],
        Stroke::new(1.0, grid(false)),
    );
    for (k, z) in zones.iter().enumerate() {
        let r = zone_rect(rect, z);
        let names = crate::perform::CARD_COLORS;
        let color =
            crate::perform::card_color(names[k % (names.len() - 1)]).unwrap_or_else(theme::accent);
        p.rect_filled(r, theme::radius(2.0), theme::fade(color, 0.35));
        p.rect_stroke(
            r,
            theme::radius(2.0),
            Stroke::new(1.5, color),
            StrokeKind::Inside,
        );
        p.text(
            r.left_top() + vec2(4.0, 2.0),
            Align2::LEFT_TOP,
            k.to_string(),
            theme::mono_font(theme::FONT_SMALL),
            theme::text_strong(),
        );
    }
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let to_units = |d: egui::Vec2| (d.x / rect.width() * 128.0, -d.y / rect.height());
    let mut out = None;
    if resp.drag_started()
        && let Some(o) = ui.input(|i| i.pointer.press_origin())
        && let Some((index, part)) = hit(rect, &zones, o)
    {
        store(
            ui,
            id,
            Some(Drag {
                index,
                part,
                origin: o,
                start: zones[index],
            }),
        );
    }
    if let (Some(d), Some(now)) = (load::<Drag>(ui, id), pointer)
        && resp.dragged()
    {
        let (notes, velocity) = to_units(now - d.origin);
        if let Some(z) = zones.get_mut(d.index) {
            *z = dragged(d.start, d.part, notes, velocity);
            out = Some(Edit {
                value: to_config(&zones),
                done: false,
            });
        }
    }
    if resp.drag_stopped() && load::<Drag>(ui, id).is_some() {
        store::<Drag>(ui, id, None);
        out = Some(Edit {
            value: to_config(&zones),
            done: true,
        });
    }
    if resp.double_clicked()
        && let Some(at) = pointer
        && hit(rect, &zones, at).is_none()
        && zones.len() < MAX_ZONES
    {
        let note = ((at.x - rect.min.x) / rect.width() * 128.0).round();
        zones.push(Zone {
            low: (note - 6.0).clamp(0.0, 127.0),
            high: (note + 5.0).clamp(0.0, 127.0),
            soft: 0.0,
            hard: 1.0,
        });
        out = Some(Edit {
            value: to_config(&zones),
            done: true,
        });
    }
    if resp.secondary_clicked()
        && let Some(at) = pointer
        && let Some((k, _)) = hit(rect, &zones, at)
    {
        zones.remove(k);
        out = Some(Edit {
            value: to_config(&zones),
            done: true,
        });
    }
    if let Some(at) = pointer
        && resp.hovered()
    {
        let icon = match hit(rect, &zones, at).map(|h| h.1) {
            Some(Part::Low | Part::High) => egui::CursorIcon::ResizeHorizontal,
            Some(Part::Soft | Part::Hard) => egui::CursorIcon::ResizeVertical,
            Some(Part::Body) => egui::CursorIcon::Grab,
            None => egui::CursorIcon::Default,
        };
        ui.ctx().set_cursor_icon(icon);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 100.0))
    }

    fn zone(low: f32, high: f32) -> Zone {
        Zone {
            low,
            high,
            soft: 0.0,
            hard: 1.0,
        }
    }

    #[test]
    fn edges_and_bodies_are_hit_where_they_are() {
        // 10 points a note: the zone 60..=71 spans 600..720.
        let zones = [zone(60.0, 71.0)];
        assert_eq!(hit(rect(), &zones, pos2(601.0, 50.0)), Some((0, Part::Low)));
        assert_eq!(
            hit(rect(), &zones, pos2(719.0, 50.0)),
            Some((0, Part::High))
        );
        assert_eq!(
            hit(rect(), &zones, pos2(650.0, 50.0)),
            Some((0, Part::Body))
        );
        assert_eq!(hit(rect(), &zones, pos2(650.0, 1.0)), Some((0, Part::Hard)));
        assert_eq!(hit(rect(), &zones, pos2(100.0, 50.0)), None);
    }

    #[test]
    fn dragging_moves_and_resizes_within_range() {
        let z = zone(60.0, 71.0);
        let moved = dragged(z, Part::Body, 12.3, 0.0);
        assert_eq!((moved.low, moved.high), (72.0, 83.0));
        let far = dragged(z, Part::Body, 200.0, 0.0);
        assert_eq!((far.low, far.high), (116.0, 127.0));
        let wider = dragged(z, Part::Low, -5.0, 0.0);
        assert_eq!(wider.low, 55.0);
        let soft = dragged(z, Part::Soft, 0.0, 0.5);
        assert_eq!(soft.soft, 0.5);
        let crossed = dragged(z, Part::High, -40.0, 0.0);
        assert_eq!(crossed.high, 60.0, "an edge stops at the other");
    }

    #[test]
    fn the_config_round_trips() {
        let zones = vec![
            zone(0.0, 59.0),
            dragged(zone(60.0, 127.0), Part::Soft, 0.0, 0.5),
        ];
        let c = json!({ "zones": to_config(&zones) });
        assert_eq!(zone_list(&c), zones);
    }
}
