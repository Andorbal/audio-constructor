//! The minimap (UX.md "Canvas"): a small overview of the graph in the
//! canvas's bottom-right corner with the visible part outlined. It shows
//! while the graph doesn't fit the view (`Shift+M` or View ▸ Minimap turns
//! it off and on); pressing or dragging in it moves the view there.

use egui::{Painter, Pos2, Rect, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::scene::{Class, Scene};
use super::{Canvas, frame_rect};
use crate::document::Document;
use crate::theme;

/// Largest size of the minimap (screen points), and its distance from the
/// canvas's right and bottom edges (above the zoom readout).
pub const MAX_SIZE: Vec2 = vec2(220.0, 150.0);
pub const MARGIN_RIGHT: f32 = 12.0;
pub const MARGIN_BOTTOM: f32 = 30.0;
/// Inside padding (screen points) and padding around the graph (canvas
/// units).
const PAD: f32 = 6.0;
const WORLD_PAD: f32 = 40.0;
/// A canvas smaller than this has no room for a minimap.
const MIN_SCREEN: Vec2 = vec2(420.0, 300.0);

/// The mapping between the graph (canvas units) and the minimap (screen).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiniMap {
    /// What the minimap shows (canvas units).
    pub world: Rect,
    /// Where it is on screen.
    pub rect: Rect,
    scale: f32,
    origin: Pos2,
}

impl MiniMap {
    /// Fit `world` into the corner of `screen`, keeping its proportions.
    /// `None` when the canvas is too small or there's nothing to show.
    pub fn new(world: Rect, screen: Rect) -> Option<MiniMap> {
        let finite = world.min.x.is_finite()
            && world.min.y.is_finite()
            && world.max.x.is_finite()
            && world.max.y.is_finite();
        if !finite || world.width() <= 0.0 || world.height() <= 0.0 {
            return None;
        }
        if screen.width() < MIN_SCREEN.x || screen.height() < MIN_SCREEN.y {
            return None;
        }
        let avail = MAX_SIZE - vec2(2.0 * PAD, 2.0 * PAD);
        let scale = (avail.x / world.width()).min(avail.y / world.height());
        let inner = world.size() * scale;
        // Keep it wide and tall enough to hit even for a thin graph.
        let size = vec2(inner.x.max(60.0), inner.y.max(40.0)) + vec2(2.0 * PAD, 2.0 * PAD);
        let rect = Rect::from_min_size(
            pos2(
                screen.max.x - MARGIN_RIGHT - size.x,
                screen.max.y - MARGIN_BOTTOM - size.y,
            ),
            size,
        );
        let origin = rect.center() - inner / 2.0;
        Some(MiniMap {
            world,
            rect,
            scale,
            origin,
        })
    }

    /// A canvas point on the minimap.
    pub fn to_map(&self, p: Pos2) -> Pos2 {
        self.origin + (p - self.world.min) * self.scale
    }

    /// The canvas point under a minimap point.
    pub fn to_canvas(&self, p: Pos2) -> Pos2 {
        self.world.min + (p - self.origin) / self.scale
    }

    pub fn rect_to_map(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.to_map(r.min), self.to_map(r.max))
    }
}

/// The graph's bounds (cards and frames), if it has any.
pub fn content(scene: &Scene, doc: &Document) -> Option<Rect> {
    scene
        .bounds(None)
        .into_iter()
        .chain(doc.graph().frames.iter().map(frame_rect))
        .reduce(|a, b| a.union(b))
}

/// What the minimap shows: the graph, padded, and the visible part of the
/// canvas, so the outline of the view is always on it.
pub fn world(content: Rect, visible: Rect) -> Rect {
    content.expand(WORLD_PAD).union(visible)
}

/// Does the graph need a minimap: is any of it out of view?
pub fn needed(content: Rect, visible: Rect) -> bool {
    !visible.contains_rect(content)
}

impl Canvas {
    /// The minimap for this frame, if it shows. While it's being dragged it
    /// keeps the mapping it started with, so the view follows the pointer
    /// steadily.
    pub(super) fn minimap_layout(&self, scene: &Scene, doc: &Document) -> Option<MiniMap> {
        if let super::Gesture::Minimap { map } = &self.gesture {
            return MiniMap::new(map.world, self.screen);
        }
        if !self.minimap {
            return None;
        }
        let content = content(scene, doc)?;
        let visible = self.transform().rect_to_canvas(self.screen);
        if !needed(content, visible) {
            return None;
        }
        MiniMap::new(world(content, visible), self.screen)
    }

    /// Center the view on the canvas point under `p` on the minimap.
    pub(super) fn minimap_pan(&mut self, map: &MiniMap, p: Pos2) {
        let p = pos2(
            p.x.clamp(map.rect.min.x, map.rect.max.x),
            p.y.clamp(map.rect.min.y, map.rect.max.y),
        );
        self.target = None;
        self.view.center = map.to_canvas(p);
    }

    /// The minimap's rectangle in the last drawn frame (tests).
    pub fn minimap_rect(&self) -> Option<Rect> {
        self.minimap_now.map(|m| m.rect)
    }

    /// The minimap of the last drawn frame (tests map points with it).
    pub fn minimap(&self) -> Option<MiniMap> {
        self.minimap_now
    }

    pub(super) fn paint_minimap(&self, painter: &Painter, scene: &Scene, doc: &Document) {
        let Some(m) = self.minimap_now else { return };
        let radius = theme::radius(6.0);
        painter.add(theme::popup_shadow().as_shape(m.rect, radius));
        painter.rect_filled(m.rect, radius, theme::fade(theme::panel_bg(), 0.94));
        let p = painter.with_clip_rect(m.rect.shrink(1.0));
        for f in &doc.graph().frames {
            let r = m.rect_to_map(frame_rect(f));
            p.rect_filled(r, theme::radius(2.0), theme::frame_fill());
            p.rect_stroke(
                r,
                theme::radius(2.0),
                Stroke::new(1.0, theme::frame_border()),
                StrokeKind::Inside,
            );
        }
        for n in &scene.nodes {
            let r = m.rect_to_map(n.rect);
            let r = Rect::from_center_size(r.center(), r.size().max(vec2(2.0, 2.0)));
            let color = match &n.class {
                Class::Reroute | Class::Portal(_) => n
                    .outputs
                    .first()
                    .map_or(theme::text_muted(), |s| theme::type_color(s.ty)),
                Class::Comment => theme::note_bg(),
                _ => n.accent,
            };
            p.rect_filled(r, theme::radius(1.0), color);
            if n.selected {
                p.rect_stroke(
                    r.expand(1.0),
                    theme::radius(1.0),
                    Stroke::new(1.0, theme::node_selected()),
                    StrokeKind::Outside,
                );
            }
        }
        let view = m.rect_to_map(self.transform().rect_to_canvas(self.screen));
        p.rect_filled(view, theme::radius(2.0), theme::fade(theme::accent(), 0.12));
        p.rect_stroke(
            view,
            theme::radius(2.0),
            Stroke::new(1.2, theme::accent()),
            StrokeKind::Inside,
        );
        painter.rect_stroke(
            m.rect,
            radius,
            Stroke::new(1.0, theme::border()),
            StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0))
    }

    #[test]
    fn graph_to_minimap_to_canvas_round_trips() {
        let world = Rect::from_min_max(pos2(-500.0, -200.0), pos2(4500.0, 1800.0));
        let m = MiniMap::new(world, screen()).unwrap();
        // In the bottom-right corner, no bigger than allowed.
        assert!(m.rect.width() <= MAX_SIZE.x + 1e-3 && m.rect.height() <= MAX_SIZE.y + 1e-3);
        assert!((m.rect.max.x - (1200.0 - MARGIN_RIGHT)).abs() < 1e-3);
        assert!((m.rect.max.y - (800.0 - MARGIN_BOTTOM)).abs() < 1e-3);
        // The whole world is on it, proportions kept.
        let w = m.rect_to_map(world);
        assert!(m.rect.expand(1e-3).contains_rect(w));
        assert!((w.width() / w.height() - world.width() / world.height()).abs() < 1e-3);
        for p in [world.min, world.max, world.center(), pos2(1234.0, 56.0)] {
            let back = m.to_canvas(m.to_map(p));
            assert!((back - p).length() < 1e-2, "{p:?} → {back:?}");
        }
    }

    #[test]
    fn thin_graphs_and_small_canvases() {
        // A single row of nodes still gets a minimap tall enough to hit.
        let row = Rect::from_min_max(pos2(0.0, 0.0), pos2(8000.0, 100.0));
        let m = MiniMap::new(row, screen()).unwrap();
        assert!(m.rect.height() >= 40.0);
        let p = m.to_canvas(m.rect.center());
        assert!((p - row.center()).length() < 1.0);
        // No room, or nothing to show: no minimap.
        let tiny = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 200.0));
        assert!(MiniMap::new(row, tiny).is_none());
        assert!(MiniMap::new(Rect::NOTHING, screen()).is_none());
        let flat = Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 0.0));
        assert!(MiniMap::new(flat, screen()).is_none());
    }

    #[test]
    fn shows_only_when_the_graph_is_out_of_view() {
        let content = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 600.0));
        let all = Rect::from_min_max(pos2(-100.0, -100.0), pos2(1200.0, 800.0));
        assert!(!needed(content, all));
        let part = Rect::from_min_max(pos2(0.0, 0.0), pos2(500.0, 400.0));
        assert!(needed(content, part));
        // The view's outline is always on the map, even panned away.
        let away = Rect::from_min_max(pos2(5000.0, 5000.0), pos2(5400.0, 5300.0));
        let w = world(content, away);
        assert!(w.contains_rect(away) && w.contains_rect(content));
    }

    #[test]
    fn a_minimap_point_becomes_the_view_center() {
        let mut c = Canvas {
            screen: screen(),
            ..Canvas::default()
        };
        let world = Rect::from_min_max(pos2(0.0, 0.0), pos2(4000.0, 2000.0));
        let m = MiniMap::new(world, c.screen).unwrap();
        let target = pos2(3000.0, 500.0);
        c.minimap_pan(&m, m.to_map(target));
        assert!((c.view.center - target).length() < 0.5);
        // The view is now centered there on screen.
        let t = c.transform();
        assert!((t.to_screen(target) - c.screen.center()).length() < 0.5);
        // Points past the edge pan to the edge.
        c.minimap_pan(&m, m.rect.max + vec2(50.0, 50.0));
        let edge = m.to_canvas(m.rect.max);
        assert!((c.view.center - edge).length() < 0.5);
    }
}
