//! Align and distribute (`Shift+Alt+A`, Edit ▸ Align / Distribute): line
//! cards up by an edge or their centers, or space them with equal gaps.
//! Only cards move; frames stay where they are (D101).

use crate::i18n::tr;
use ac_core::NodeId;
use egui::{Rect, Vec2, vec2};

/// How to line cards up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
    /// Equal horizontal gaps; the leftmost and rightmost cards stay.
    Horizontally,
    /// Equal vertical gaps; the top and bottom cards stay.
    Vertically,
}

impl Align {
    pub const ALL: [Align; 8] = [
        Align::Left,
        Align::Center,
        Align::Right,
        Align::Top,
        Align::Middle,
        Align::Bottom,
        Align::Horizontally,
        Align::Vertically,
    ];

    /// Whether this spreads cards out rather than lining them up.
    pub fn distributes(self) -> bool {
        matches!(self, Align::Horizontally | Align::Vertically)
    }

    /// The fewest cards it does anything with.
    pub fn needs(self) -> usize {
        if self.distributes() { 3 } else { 2 }
    }

    /// The undo step's name.
    pub fn undo_label(self) -> String {
        match self {
            Align::Left => tr("undo-align-left"),
            Align::Center => tr("undo-align-centers"),
            Align::Right => tr("undo-align-right"),
            Align::Top => tr("undo-align-top"),
            Align::Middle => tr("undo-align-middles"),
            Align::Bottom => tr("undo-align-bottom"),
            Align::Horizontally => tr("undo-distribute-horizontally"),
            Align::Vertically => tr("undo-distribute-vertically"),
        }
    }
}

/// How far each card moves. Cards whose rectangle isn't finite (a damaged
/// file) don't take part.
pub fn moves(cards: &[(NodeId, Rect)], how: Align) -> Vec<(NodeId, Vec2)> {
    let cards: Vec<(NodeId, Rect)> = cards
        .iter()
        .copied()
        .filter(|(_, r)| r.is_finite())
        .collect();
    if cards.len() < how.needs() {
        return Vec::new();
    }
    let Some(bounds) = cards.iter().map(|(_, r)| *r).reduce(|a, b| a.union(b)) else {
        return Vec::new();
    };
    match how {
        Align::Left => each(&cards, |r| vec2(bounds.min.x - r.min.x, 0.0)),
        Align::Center => each(&cards, |r| vec2(bounds.center().x - r.center().x, 0.0)),
        Align::Right => each(&cards, |r| vec2(bounds.max.x - r.max.x, 0.0)),
        Align::Top => each(&cards, |r| vec2(0.0, bounds.min.y - r.min.y)),
        Align::Middle => each(&cards, |r| vec2(0.0, bounds.center().y - r.center().y)),
        Align::Bottom => each(&cards, |r| vec2(0.0, bounds.max.y - r.max.y)),
        Align::Horizontally => spread(&cards, bounds, true),
        Align::Vertically => spread(&cards, bounds, false),
    }
}

fn each(cards: &[(NodeId, Rect)], f: impl Fn(Rect) -> Vec2) -> Vec<(NodeId, Vec2)> {
    cards.iter().map(|(id, r)| (*id, f(*r))).collect()
}

/// Equal gaps along one axis, in the order the cards' leading edges come,
/// filling the span from the first leading edge to the last trailing one.
fn spread(cards: &[(NodeId, Rect)], bounds: Rect, x_axis: bool) -> Vec<(NodeId, Vec2)> {
    let lead = |r: &Rect| if x_axis { r.min.x } else { r.min.y };
    let other = |r: &Rect| if x_axis { r.min.y } else { r.min.x };
    let len = |r: &Rect| if x_axis { r.width() } else { r.height() };
    let mut sorted = cards.to_vec();
    sorted.sort_by(|(a, ra), (b, rb)| {
        lead(ra)
            .total_cmp(&lead(rb))
            .then(other(ra).total_cmp(&other(rb)))
            .then(a.cmp(b))
    });
    let span = if x_axis {
        bounds.width()
    } else {
        bounds.height()
    };
    let total: f32 = sorted.iter().map(|(_, r)| len(r)).sum();
    let gap = (span - total) / (sorted.len() - 1) as f32;
    let mut at = if x_axis { bounds.min.x } else { bounds.min.y };
    sorted
        .iter()
        .map(|(id, r)| {
            let d = at - lead(r);
            at += len(r) + gap;
            (*id, if x_axis { vec2(d, 0.0) } else { vec2(0.0, d) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    fn after(cards: &[(NodeId, Rect)], how: Align) -> Vec<Rect> {
        let m = moves(cards, how);
        cards
            .iter()
            .map(|(id, rect)| {
                let d = m
                    .iter()
                    .find(|(i, _)| i == id)
                    .map_or(Vec2::ZERO, |(_, d)| *d);
                rect.translate(d)
            })
            .collect()
    }

    fn three() -> Vec<(NodeId, Rect)> {
        vec![
            (NodeId(1), r(10.0, 0.0, 100.0, 40.0)),
            (NodeId(2), r(300.0, 100.0, 200.0, 80.0)),
            (NodeId(3), r(50.0, 400.0, 50.0, 20.0)),
        ]
    }

    #[test]
    fn edges_and_centers_line_up() {
        let c = three();
        let lefts: Vec<f32> = after(&c, Align::Left).iter().map(|r| r.min.x).collect();
        assert_eq!(lefts, vec![10.0; 3]);
        let rights: Vec<f32> = after(&c, Align::Right).iter().map(|r| r.max.x).collect();
        assert_eq!(rights, vec![500.0; 3]);
        let centers: Vec<f32> = after(&c, Align::Center)
            .iter()
            .map(|r| r.center().x)
            .collect();
        assert_eq!(centers, vec![255.0; 3]);
        let tops: Vec<f32> = after(&c, Align::Top).iter().map(|r| r.min.y).collect();
        assert_eq!(tops, vec![0.0; 3]);
        let bottoms: Vec<f32> = after(&c, Align::Bottom).iter().map(|r| r.max.y).collect();
        assert_eq!(bottoms, vec![420.0; 3]);
        let middles: Vec<f32> = after(&c, Align::Middle)
            .iter()
            .map(|r| r.center().y)
            .collect();
        assert_eq!(middles, vec![210.0; 3]);
        // Aligning moves along one axis only.
        for (a, b) in after(&c, Align::Left).iter().zip(&c) {
            assert_eq!(a.min.y, b.1.min.y);
        }
    }

    #[test]
    fn distributing_leaves_equal_gaps_between_the_outer_cards() {
        let c = vec![
            (NodeId(1), r(0.0, 0.0, 100.0, 40.0)),
            (NodeId(2), r(120.0, 50.0, 50.0, 40.0)),
            (NodeId(3), r(600.0, 0.0, 100.0, 40.0)),
            (NodeId(4), r(200.0, 0.0, 150.0, 40.0)),
        ];
        let out = after(&c, Align::Horizontally);
        let mut spans: Vec<(f32, f32)> = out.iter().map(|r| (r.min.x, r.max.x)).collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let gaps: Vec<f32> = spans.windows(2).map(|w| w[1].0 - w[0].1).collect();
        // 700 wide, 400 of cards: three gaps of 100.
        assert_eq!(gaps, vec![100.0; 3]);
        assert_eq!(out[0].min.x, 0.0, "the first stays");
        assert_eq!(out[2].max.x, 700.0, "the last stays");
        // Only along the axis.
        assert_eq!(out[1].min.y, 50.0);
        let v = after(&three(), Align::Vertically);
        assert_eq!(v[0].min.y, 0.0);
        assert_eq!(v[2].max.y, 420.0);
        // 420 tall, 140 of cards: two gaps of 140.
        assert_eq!(v[1].min.y, 180.0);
    }

    #[test]
    fn too_few_cards_do_nothing() {
        let one = vec![(NodeId(1), r(0.0, 0.0, 10.0, 10.0))];
        assert!(moves(&one, Align::Left).is_empty());
        let two = vec![
            (NodeId(1), r(0.0, 0.0, 10.0, 10.0)),
            (NodeId(2), r(50.0, 9.0, 10.0, 10.0)),
        ];
        assert!(moves(&two, Align::Horizontally).is_empty());
        assert_eq!(moves(&two, Align::Top).len(), 2);
        // A card with no usable rectangle is left out.
        let bad = vec![
            (NodeId(1), r(f32::NAN, 0.0, 10.0, 10.0)),
            (NodeId(2), r(50.0, 9.0, 10.0, 10.0)),
        ];
        assert!(moves(&bad, Align::Top).is_empty());
    }
}
