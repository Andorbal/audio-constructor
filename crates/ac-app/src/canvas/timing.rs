//! The per-node timing chip in a card's header (`Ctrl+T`), heat-colored by
//! the node's share of the block's budget (see `crate::timing`).

use egui::{Painter, Rect, pos2, vec2};

use super::Canvas;
use super::scene::NodeView;
use crate::theme;
use crate::timing::{self, Reading, Timings};

impl Canvas {
    /// Paint the chip right-aligned at `right` (screen x) in `header`, and
    /// return the x left of it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_timing(
        &mut self,
        painter: &Painter,
        header: Rect,
        right: f32,
        r: &Reading,
        timings: &Timings,
        n: &NodeView,
        alpha: f32,
    ) -> f32 {
        let z = self.view.zoom;
        let color = theme::heat(r.heat());
        let text = timing::format_ns(r.ns);
        self.header_timing.push((n.id, text.clone()));
        let galley = painter.layout_no_wrap(
            text,
            theme::canvas_font(theme::FONT_SMALL, z, true),
            theme::fade(color, alpha),
        );
        let pad = vec2(5.0 * z, 1.5 * z);
        let size = galley.size() + 2.0 * pad;
        let chip =
            Rect::from_min_size(pos2(right - size.x, header.center().y - size.y / 2.0), size);
        painter.rect_filled(
            chip,
            theme::radius(size.y / 2.0),
            theme::fade(color, 0.16 * alpha),
        );
        painter.galley(chip.min + pad, galley, color);
        let voices = n.kind == ac_core::VOICES_KIND;
        self.tips.push((
            chip.expand(2.0),
            timing::describe(r, timings.budget_ns(), voices),
        ));
        chip.min.x - 6.0 * z
    }
}
