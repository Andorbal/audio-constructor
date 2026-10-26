//! The bank tabs above the Pattern grid and the piano roll (DECISIONS
//! D212): one tab per bank, numbered from 0 like the node's `bank` input,
//! then `+`. Click a tab to show its bank, `+` to add a copy of the bank
//! shown, and right-click a tab to remove its bank.

use egui::{Align2, Painter, Pos2, Rect, Response, Stroke, StrokeKind, pos2, vec2};

use crate::theme;

/// Height of the tabs, and the gap below them.
pub const HEIGHT: f32 = 18.0;
pub const GAP: f32 = 4.0;
const WIDTH: f32 = 22.0;

/// What the person did with the tabs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Show(usize),
    Add,
    Remove(usize),
}

/// A row of tabs for `count` banks, at most `max`.
pub struct Tabs {
    origin: Pos2,
    w: f32,
    count: usize,
    max: usize,
}

impl Tabs {
    /// Tabs along the top of `strip`.
    pub fn new(strip: Rect, count: usize, max: usize) -> Self {
        Tabs {
            origin: strip.min,
            w: WIDTH.min(strip.width() / (count + 1) as f32 - 2.0).max(8.0),
            count,
            max,
        }
    }

    pub fn rect(&self, k: usize) -> Rect {
        Rect::from_min_size(
            pos2(self.origin.x + k as f32 * (self.w + 2.0), self.origin.y),
            vec2(self.w, HEIGHT),
        )
    }

    /// The tab under `p`: a bank, or `count` for `+` (when there's room).
    pub fn at(&self, p: Pos2) -> Option<usize> {
        let last = if self.count < self.max {
            self.count
        } else {
            self.count - 1
        };
        (0..=last).find(|&k| self.rect(k).contains(p))
    }

    /// What a click or right-click on the editor's area did to the tabs.
    pub fn action(&self, resp: &Response) -> Option<Action> {
        let p = resp.interact_pointer_pos()?;
        let k = self.at(p)?;
        if resp.secondary_clicked() {
            (k < self.count && self.count > 1).then_some(Action::Remove(k))
        } else if resp.clicked() {
            Some(if k < self.count {
                Action::Show(k)
            } else {
                Action::Add
            })
        } else {
            None
        }
    }

    /// Paint the tabs: `shown` highlighted, a dot on the one `playing`.
    pub fn paint(
        &self,
        painter: &Painter,
        shown: usize,
        playing: Option<usize>,
        hover: Option<Pos2>,
    ) {
        let color = theme::category_color(ac_core::Category::Sequencing);
        let small = egui::FontId::proportional(theme::FONT_SMALL);
        let last = if self.count < self.max {
            self.count
        } else {
            self.count - 1
        };
        for k in 0..=last {
            let r = self.rect(k);
            let on = k == shown;
            let fill = if on {
                theme::fade(color, 0.45)
            } else if hover.is_some_and(|p| r.contains(p)) {
                theme::widget_bg_hover()
            } else {
                theme::widget_bg()
            };
            painter.rect_filled(r, theme::radius(3.0), fill);
            if on {
                painter.rect_stroke(
                    r,
                    theme::radius(3.0),
                    Stroke::new(1.0, color),
                    StrokeKind::Inside,
                );
            }
            if playing == Some(k) && self.count > 1 {
                painter.circle_filled(r.right_top() + vec2(-4.0, 4.0), 2.5, theme::play());
            }
            let label = if k < self.count {
                k.to_string()
            } else {
                "+".to_string()
            };
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                label,
                small.clone(),
                if on {
                    theme::text_strong()
                } else {
                    theme::text_muted()
                },
            );
        }
    }
}

/// Apply `action` to `banks` (adding copies the bank `shown`, up to `max`);
/// returns the bank to show and whether the banks changed.
pub fn apply<T: Clone>(
    banks: &mut Vec<T>,
    shown: usize,
    action: Action,
    max: usize,
) -> (usize, bool) {
    match action {
        Action::Show(k) => (k.min(banks.len().saturating_sub(1)), false),
        Action::Add if banks.len() < max => {
            let Some(copy) = banks.get(shown).or(banks.last()).cloned() else {
                return (shown, false);
            };
            let at = (shown + 1).min(banks.len());
            banks.insert(at, copy);
            (at, true)
        }
        Action::Remove(k) if banks.len() > 1 && k < banks.len() => {
            banks.remove(k);
            (shown.min(banks.len() - 1), true)
        }
        _ => (shown, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_copies_and_removing_keeps_one() {
        let mut b = vec!['a'];
        assert_eq!(apply(&mut b, 0, Action::Add, 3), (1, true));
        assert_eq!(b, ['a', 'a']);
        b[1] = 'b';
        assert_eq!(apply(&mut b, 0, Action::Add, 3), (1, true));
        assert_eq!(b, ['a', 'a', 'b'], "inserted after the bank shown");
        assert_eq!(apply(&mut b, 2, Action::Add, 3), (2, false), "full");
        assert_eq!(apply(&mut b, 2, Action::Remove(0), 3), (1, true));
        assert_eq!(apply(&mut b, 1, Action::Remove(0), 3), (0, true));
        assert_eq!(apply(&mut b, 0, Action::Remove(0), 3), (0, false));
        assert_eq!(b, ['b']);
        assert_eq!(apply(&mut b, 0, Action::Show(5), 3), (0, false));
    }

    #[test]
    fn tabs_are_found_where_they_are_drawn() {
        let strip = Rect::from_min_size(Pos2::ZERO, vec2(300.0, HEIGHT));
        let t = Tabs::new(strip, 2, 16);
        assert_eq!(t.at(pos2(11.0, 9.0)), Some(0));
        assert_eq!(t.at(pos2(35.0, 9.0)), Some(1));
        assert_eq!(t.at(pos2(59.0, 9.0)), Some(2), "the + tab");
        assert_eq!(t.at(pos2(83.0, 9.0)), None);
        let full = Tabs::new(strip, 2, 2);
        assert_eq!(full.at(pos2(59.0, 9.0)), None, "no + when full");
    }
}
