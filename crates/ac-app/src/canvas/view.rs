//! Pan and zoom: the mapping between canvas units and screen points.

use egui::{Pos2, Rect, Vec2, pos2};

use crate::theme;

/// The canvas point at the center of the view, and the zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub center: Pos2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        View {
            center: pos2(200.0, 60.0),
            zoom: 1.0,
        }
    }
}

/// A view applied to a screen rectangle.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub view: View,
    pub screen: Rect,
}

impl Transform {
    pub fn to_screen(&self, p: Pos2) -> Pos2 {
        self.screen.center() + (p - self.view.center) * self.view.zoom
    }

    pub fn to_canvas(&self, p: Pos2) -> Pos2 {
        self.view.center + (p - self.screen.center()) / self.view.zoom
    }

    pub fn rect_to_screen(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.to_screen(r.min), self.to_screen(r.max))
    }

    pub fn rect_to_canvas(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.to_canvas(r.min), self.to_canvas(r.max))
    }

    /// Canvas units → screen points (a length).
    pub fn s(&self, len: f32) -> f32 {
        len * self.view.zoom
    }
}

impl View {
    /// Zoom by `factor`, keeping the canvas point under `anchor` (screen) fixed.
    pub fn zoom_at(&mut self, screen: Rect, anchor: Pos2, factor: f32) {
        let t = Transform {
            view: *self,
            screen,
        };
        let fixed = t.to_canvas(anchor);
        self.zoom = (self.zoom * factor).clamp(theme::ZOOM_MIN, theme::ZOOM_MAX);
        self.center = fixed - (anchor - screen.center()) / self.zoom;
    }

    /// Move the content by a screen-space delta.
    pub fn pan(&mut self, delta: Vec2) {
        self.center -= delta / self.zoom;
    }

    /// A view showing `bounds` (canvas) inside `screen` with a margin, at
    /// no more than `max_zoom`.
    pub fn fit(bounds: Rect, screen: Rect, max_zoom: f32) -> View {
        let margin = 60.0;
        let w = (screen.width() - 2.0 * margin).max(50.0);
        let h = (screen.height() - 2.0 * margin).max(50.0);
        let zoom = (w / bounds.width().max(1.0))
            .min(h / bounds.height().max(1.0))
            .clamp(theme::ZOOM_MIN, max_zoom);
        View {
            center: bounds.center(),
            zoom,
        }
    }

    /// Move part of the way toward `target` (frame-rate independent), for
    /// smooth zoom steps and fits. Returns true while still moving.
    pub fn approach(&mut self, target: View, dt: f32) -> bool {
        let k = 1.0 - (-dt * 22.0).exp();
        let lz = self.zoom.ln() + (target.zoom.ln() - self.zoom.ln()) * k;
        self.zoom = lz.exp();
        self.center += (target.center - self.center) * k;
        let done = (self.zoom / target.zoom - 1.0).abs() < 1e-3
            && (self.center - target.center).length() * self.zoom < 0.3;
        if done {
            *self = target;
        }
        !done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_zoom_keeps_the_anchor() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let mut v = View::default();
        let t = Transform { view: v, screen };
        let p = pos2(123.0, -45.0);
        assert!((t.to_canvas(t.to_screen(p)) - p).length() < 1e-3);
        let anchor = pos2(100.0, 100.0);
        let before = t.to_canvas(anchor);
        v.zoom_at(screen, anchor, 2.0);
        let after = Transform { view: v, screen }.to_canvas(anchor);
        assert!((before - after).length() < 1e-3);
        v.zoom_at(screen, anchor, 1000.0);
        assert_eq!(v.zoom, theme::ZOOM_MAX);
    }

    #[test]
    fn fit_contains_the_bounds() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let b = Rect::from_min_max(pos2(-1000.0, -200.0), pos2(1000.0, 200.0));
        let v = View::fit(b, screen, 1.0);
        let t = Transform { view: v, screen };
        assert!(screen.contains(t.to_screen(b.min)) && screen.contains(t.to_screen(b.max)));
    }
}
