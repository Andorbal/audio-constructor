//! The panels for screen readers and the keyboard (docs/UX.md
//! "Accessibility and scaling"). The grid hit-tests and paints its cards by
//! hand, so each frame this gives every card a widget of its own: it senses
//! nothing (the pointer still plays the grid), but Tab reaches it, it has a
//! name, a role, and a value for AccessKit, and the keys and a screen
//! reader's actions play it. The header buttons are named too, for screen
//! readers; from the keyboard, Delete, `+` and `-` do what they do.

use ac_core::{ConfigExt, NodeId, SignalType, SocketSpec, Value};
use egui::accesskit::{self, ActionData, Role, Toggled};
use egui::{Key, Sense, Stroke, StrokeKind, vec2};

use super::{PanelHit, PerformView, ValueEdit};
use crate::canvas::CanvasIo;
use crate::canvas::controls::{self, Click};
use crate::canvas::live::{Live, LiveState};
use crate::canvas::scene::{Body, NodeView, Scene};
use crate::canvas::widgets;
use crate::i18n::{tr, tr_args};
use crate::theme;

/// What a card is to a screen reader.
struct Spoken {
    role: Role,
    value: Option<String>,
    /// A number the keys and a screen reader can set, with its range.
    number: Option<(f64, Option<(f64, f64)>)>,
    toggled: Option<bool>,
    /// It does something when pressed (Click).
    presses: bool,
}

/// What the keys or a screen reader asked of a card this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ask {
    /// Step the value: direction and whether by a tenth of the range.
    Step(f64, bool),
    /// To one end of the range.
    End(bool),
    Set(f64),
    /// Enter: type a value, open a choice, flip, press, or choose a file.
    Activate,
    /// Space went down on a Button (it holds until Space comes up).
    Hold,
    Shrink,
    Grow,
    Unpin,
}

impl PerformView {
    /// The widget a card is drawn as, for egui and AccessKit.
    pub(super) fn cell_id(node: NodeId) -> egui::Id {
        egui::Id::unique("panel-a11y").with(node.0)
    }

    /// Give the cards the keyboard (F6): the first card takes it.
    pub fn take_keyboard(&self, ctx: &egui::Context) {
        if let Some(c) = self.cells.first() {
            ctx.memory_mut(|m| m.request_focus(Self::cell_id(c.node)));
        }
    }

    /// The grid as a screen reader says it: the panel and how many cards
    /// it has, or that there are no panels.
    pub(super) fn name_grid(&self, io: &CanvasIo, grid: &egui::Response) {
        let label = match io.doc.project.panels.get(self.current) {
            Some(p) => tr_args(
                "a11y-panel",
                &[("panel", (&p.name).into()), ("count", p.items.len().into())],
            ),
            None => tr("perform-no-panels"),
        };
        grid.widget_info(|| egui::WidgetInfo::labeled(Role::Group, true, &label));
    }

    /// The grid's cards for AccessKit and the keyboard, after the
    /// cards are painted (the focus ring goes over them).
    pub(super) fn accessibility(
        &mut self,
        ui: &mut egui::Ui,
        io: &mut CanvasIo,
        scene: &Scene,
        live: &Live,
        now: f64,
    ) {
        let typing = self.edit.is_some() || self.rename.is_some() || self.choice.is_some();
        let mut asks = Vec::new();
        for (i, c) in self.cells.iter().enumerate() {
            let Some(n) = scene.node(c.node) else {
                continue;
            };
            let id = Self::cell_id(c.node);
            let r = ui.interact(c.rect, id, Sense::focusable_noninteractive());
            let spoken = speak(io, live, &self.live, n);
            let name = n.title.clone();
            ui.ctx().accesskit_node_builder(id, |b| {
                b.set_role(spoken.role);
                b.set_label(name);
                if let Some(v) = &spoken.value {
                    b.set_value(v.clone());
                }
                if let Some((v, range)) = spoken.number {
                    b.set_numeric_value(v);
                    if let Some((lo, hi)) = range {
                        b.set_min_numeric_value(lo);
                        b.set_max_numeric_value(hi);
                    }
                    b.add_action(accesskit::Action::Increment);
                    b.add_action(accesskit::Action::Decrement);
                    b.add_action(accesskit::Action::SetValue);
                }
                if let Some(on) = spoken.toggled {
                    b.set_toggled(if on { Toggled::True } else { Toggled::False });
                }
                if spoken.presses {
                    b.add_action(accesskit::Action::Click);
                }
            });
            let settable = spoken.number.is_some();
            ui.input(|inp| {
                if inp.has_accesskit_action_request(id, accesskit::Action::Click) {
                    asks.push((i, Ask::Activate));
                }
                if inp.has_accesskit_action_request(id, accesskit::Action::Increment) {
                    asks.push((i, Ask::Step(1.0, false)));
                }
                if inp.has_accesskit_action_request(id, accesskit::Action::Decrement) {
                    asks.push((i, Ask::Step(-1.0, false)));
                }
                for req in inp.accesskit_action_requests(id, accesskit::Action::SetValue) {
                    if let Some(ActionData::NumericValue(v)) = req.data {
                        asks.push((i, Ask::Set(v)));
                    }
                }
            });
            if r.has_focus() && !typing {
                // The arrows are the card's while it has the keyboard, not
                // egui's to move focus with.
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        id,
                        egui::EventFilter {
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            ..Default::default()
                        },
                    );
                });
                asks.extend(keys(ui, settable, c.buttons.is_some()).map(|a| (i, a)));
                let ring = c.rect.expand(3.0);
                ui.painter().rect_stroke(
                    ring,
                    theme::radius(theme::NODE_RADIUS + 3.0),
                    Stroke::new(2.0 * theme::outline_width(), theme::focus_ring()),
                    StrokeKind::Outside,
                );
            }
            if let Some(buttons) = c.buttons {
                let names = [
                    tr("a11y-panel-shrink"),
                    tr("a11y-panel-grow"),
                    tr("a11y-panel-unpin"),
                ];
                let what = [Ask::Shrink, Ask::Grow, Ask::Unpin];
                for (k, rect) in buttons.into_iter().enumerate() {
                    let bid = id.with(k);
                    let _ = ui.interact(rect, bid, Sense::hover());
                    let name = tr_args(
                        "a11y-panel-button",
                        &[("button", (&names[k]).into()), ("card", (&n.title).into())],
                    );
                    ui.ctx().accesskit_node_builder(bid, |b| {
                        b.set_role(Role::Button);
                        b.set_label(name);
                        b.add_action(accesskit::Action::Click);
                    });
                    if ui.input(|inp| {
                        inp.has_accesskit_action_request(bid, accesskit::Action::Click)
                    }) {
                        asks.push((i, what[k]));
                    }
                }
            }
        }
        // A Button held with Space lets go when Space comes up, or when the
        // keyboard goes elsewhere.
        if let Some(node) = self.key_hold {
            let up = ui.input(|i| i.key_released(Key::Space));
            let gone = !ui.memory(|m| m.has_focus(Self::cell_id(node)));
            if up || gone {
                self.controls.release(io, now);
                self.key_hold = None;
            }
        }
        for (i, ask) in asks {
            self.answer(io, scene, i, ask, now);
        }
    }

    /// Do what was asked of the card at `i`.
    fn answer(&mut self, io: &mut CanvasIo, scene: &Scene, i: usize, ask: Ask, now: f64) {
        let Some(c) = self.cells.get(i).cloned() else {
            return;
        };
        let Some(n) = scene.node(c.node) else {
            return;
        };
        match ask {
            Ask::Shrink => {
                return self.click(io, scene, PanelHit::Shrink(i), Default::default(), now);
            }
            Ask::Grow => {
                return self.click(io, scene, PanelHit::Grow(i), Default::default(), now);
            }
            Ask::Unpin => {
                return self.click(io, scene, PanelHit::Unpin(i), Default::default(), now);
            }
            _ => {}
        }
        let Some((body, rect)) = c.body else {
            return;
        };
        match ask {
            Ask::Step(..) | Ask::End(_) | Ask::Set(_) => {
                if body == Body::Dropdown {
                    if let Ask::Step(d, _) = ask {
                        step_choice(io, n, d, now);
                    }
                    return;
                }
                let Some((spec, v)) = free_number(io, n) else {
                    return;
                };
                let next = match ask {
                    Ask::Step(d, big) => stepped(v, &spec, d, big),
                    Ask::End(top) => match spec.range {
                        Some((lo, hi)) => {
                            if top {
                                hi
                            } else {
                                lo
                            }
                        }
                        None => v,
                    },
                    Ask::Set(x) => x,
                    _ => v,
                };
                let value = if spec.ty == SignalType::Int {
                    Value::Int(next.round() as i64)
                } else {
                    Value::Float(next)
                };
                crate::params::set(io.doc, io.engine, n.id, "value", value, now);
            }
            Ask::Hold => {
                if matches!(body, Body::Button | Body::Pad) && self.key_hold.is_none() {
                    self.controls.press(io, n, rect, rect.center(), now);
                    self.key_hold = Some(n.id);
                } else if !matches!(body, Body::Button | Body::Pad) {
                    self.answer(io, scene, i, Ask::Activate, now);
                }
            }
            Ask::Activate => match body {
                // A press from the keyboard or a screen reader is a tap.
                Body::Button | Body::Pad => {
                    self.controls.press(io, n, rect, rect.center(), now);
                    self.controls.release(io, now);
                }
                Body::Knob | Body::Slider | Body::Number | Body::Fader => {
                    if let Some((spec, v)) = free_number(io, n) {
                        self.edit = Some(ValueEdit {
                            node: n.id,
                            socket: "value".into(),
                            text: controls::plain_number(v, &spec),
                            spec,
                            rect: rect.shrink2(vec2(6.0, 6.0)),
                            started: false,
                        });
                    }
                }
                _ => match controls::click(io, n) {
                    Click::Choose => {
                        self.choice = Some((n.id, "value".into(), rect.left_bottom()));
                    }
                    Click::Type { spec, text } => {
                        self.edit = Some(ValueEdit {
                            node: n.id,
                            socket: "value".into(),
                            spec,
                            text,
                            rect: rect.shrink2(vec2(6.0, 6.0)),
                            started: false,
                        });
                    }
                    Click::Done | Click::Select => {}
                },
            },
            Ask::Shrink | Ask::Grow | Ask::Unpin => {}
        }
    }
}

/// The keys pressed this frame on a focused card.
fn keys(ui: &egui::Ui, settable: bool, editing: bool) -> impl Iterator<Item = Ask> {
    let mut out = Vec::new();
    ui.input(|i| {
        let big = i.modifiers.shift;
        let plain = !i.modifiers.any();
        let pressed = |k| i.key_pressed(k);
        for (k, d) in [
            (Key::ArrowUp, 1.0),
            (Key::ArrowRight, 1.0),
            (Key::ArrowDown, -1.0),
            (Key::ArrowLeft, -1.0),
        ] {
            if pressed(k) {
                out.push(Ask::Step(d, big));
            }
        }
        if pressed(Key::PageUp) {
            out.push(Ask::Step(1.0, true));
        }
        if pressed(Key::PageDown) {
            out.push(Ask::Step(-1.0, true));
        }
        if settable && pressed(Key::Home) {
            out.push(Ask::End(false));
        }
        if settable && pressed(Key::End) {
            out.push(Ask::End(true));
        }
        if plain && (pressed(Key::Enter) || pressed(Key::F2)) {
            out.push(Ask::Activate);
        }
        let space = i.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: Key::Space,
                    pressed: true,
                    repeat: false,
                    ..
                }
            )
        });
        if plain && space {
            out.push(Ask::Hold);
        }
        if editing && plain && (pressed(Key::Delete) || pressed(Key::Backspace)) {
            out.push(Ask::Unpin);
        }
        if editing && (pressed(Key::Plus) || pressed(Key::Equals)) && !i.modifiers.command {
            out.push(Ask::Grow);
        }
        if editing && pressed(Key::Minus) && !i.modifiers.command {
            out.push(Ask::Shrink);
        }
    });
    out.into_iter()
}

/// A card's role, value, and state, as its widget shows them.
fn speak(io: &CanvasIo, live: &Live, meters: &LiveState, n: &NodeView) -> Spoken {
    let mut s = Spoken {
        role: Role::Group,
        value: None,
        number: None,
        toggled: None,
        presses: false,
    };
    let Some((body, _)) = n.body else {
        return s;
    };
    let path = live.path(n.id);
    match body {
        Body::Knob | Body::Slider | Body::Number | Body::Fader => {
            s.role = if body == Body::Number {
                Role::SpinButton
            } else {
                Role::Slider
            };
            let input = n.socket("value", true);
            if let (Some(spec), Some(inst)) =
                (input.and_then(|x| x.spec.as_ref()), io.shown_node(n.id))
            {
                let wired = input.is_some_and(|x| x.wired);
                let v = if wired {
                    live.tele.socket_value(&path, "out").map_or(0.0, f64::from)
                } else {
                    widgets::number(&inst, spec)
                };
                s.value = Some(widgets::format(v, spec));
                if !wired {
                    s.number = Some((v, spec.range));
                    s.presses = true;
                }
            }
        }
        Body::Toggle => {
            s.role = Role::Switch;
            let on = live
                .tele
                .socket_value(&path, "out")
                .map(|v| v > 0.5)
                .or_else(|| {
                    io.shown_node(n.id)
                        .and_then(|x| x.params.get("value").and_then(Value::as_bool))
                })
                .unwrap_or(false);
            s.toggled = Some(on);
            s.presses = true;
        }
        Body::Button | Body::Pad => {
            s.role = Role::Button;
            s.presses = true;
            if live.node_value(n.id, 0).is_some_and(|v| v > 0.5) {
                s.value = Some(tr("a11y-lit"));
            }
        }
        Body::Light => {
            s.role = Role::Status;
            s.value = Some(if live.node_value(n.id, 0).unwrap_or(0.0) > 0.5 {
                tr("a11y-light-on")
            } else {
                tr("a11y-light-off")
            });
        }
        Body::Dropdown => {
            s.role = Role::ComboBox;
            let (items, current) = controls::choice_items(io, n, "value");
            s.value = current.and_then(|k| items.get(k)).map(|(l, _)| l.clone());
            s.presses = !items.is_empty();
        }
        Body::Meter | Body::AudioOut => {
            s.role = Role::Meter;
            let peak = meters
                .meter_reading(&path)
                .map_or(0.0, |m| m[0].peak.max(m[1].peak));
            s.value = Some(if peak > 0.0 {
                tr_args(
                    "a11y-peak",
                    &[("db", format!("{:.1}", 20.0 * peak.log10()).into())],
                )
            } else {
                tr("a11y-silent")
            });
        }
        Body::Readout => {
            s.role = Role::Status;
            s.value = Some(controls::readout_text(live.node_value(n.id, 0)));
        }
        Body::Scope | Body::Analyzer | Body::Spectrogram => s.role = Role::Image,
        Body::Clip => {
            s.role = Role::Button;
            s.presses = true;
            s.value = io
                .doc
                .graph()
                .node(n.id)
                .map(|x| x.config.get_str_or("file", "").to_string())
                .filter(|f| !f.is_empty());
        }
        Body::Record => {
            let rec = live.node_value(n.id, 0).unwrap_or(0.0) > 0.5;
            s.value = Some(if rec {
                tr("a11y-recording")
            } else {
                tr("a11y-not-recording")
            });
        }
        // Played with the pointer (or Play keys for a Keyboard).
        Body::XyPad | Body::Keyboard | Body::Pads => {}
    }
    s
}

/// The card's own unwired `value` input and what it's set to.
fn free_number(io: &CanvasIo, n: &NodeView) -> Option<(SocketSpec, f64)> {
    let input = n.socket("value", true).filter(|s| !s.wired)?;
    let spec = input.spec.clone()?;
    let inst = io.shown_node(n.id)?;
    let v = widgets::number(&inst, &spec);
    Some((spec, v))
}

/// One step of an arrow key: a hundredth of the range (a tenth with
/// Shift), as the widget is drawn (log for Hz), or a little for a number
/// without a range; whole numbers always move by at least one.
fn stepped(v: f64, spec: &SocketSpec, dir: f64, big: bool) -> f64 {
    let frac = if big { 0.1 } else { 0.01 };
    let next = match (widgets::normalized(v, spec), spec.range) {
        (Some(t), Some((lo, hi))) => widgets::at_position(f64::from(t) + dir * frac, spec)
            .unwrap_or(v)
            .clamp(lo, hi),
        _ if spec.ty == SignalType::Int => v + dir * if big { 10.0 } else { 1.0 },
        _ => v + dir * frac * (v.abs().max(1.0)),
    };
    if spec.ty != SignalType::Int {
        return next;
    }
    let (r, cur) = (next.round(), v.round());
    if r == cur {
        let moved = cur + dir;
        match spec.range {
            Some((lo, hi)) => moved.clamp(lo, hi),
            None => moved,
        }
    } else {
        r
    }
}

/// The arrows on a Dropdown pick the choice before or after.
fn step_choice(io: &mut CanvasIo, n: &NodeView, dir: f64, now: f64) {
    let (items, current) = controls::choice_items(io, n, "value");
    if items.is_empty() {
        return;
    }
    let last = items.len() - 1;
    let at = current.unwrap_or(0);
    let next = if dir > 0.0 {
        (at + 1).min(last)
    } else {
        at.saturating_sub(1)
    };
    if Some(next) != current
        && let Some((_, value)) = items.into_iter().nth(next)
    {
        crate::params::set(io.doc, io.engine, n.id, "value", value, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::Unit;

    fn spec(ty: SignalType, range: Option<(f64, f64)>) -> SocketSpec {
        let mut s = if ty == SignalType::Int {
            SocketSpec::int("value", 0)
        } else {
            SocketSpec::float("value", Unit::Generic, 0.0)
        };
        s.range = range;
        s
    }

    #[test]
    fn arrows_step_a_hundredth_of_the_range_and_shift_a_tenth() {
        let s = spec(SignalType::Float, Some((0.0, 1.0)));
        assert!((stepped(0.5, &s, 1.0, false) - 0.51).abs() < 1e-9);
        assert!((stepped(0.5, &s, -1.0, true) - 0.4).abs() < 1e-9);
        assert_eq!(stepped(1.0, &s, 1.0, true), 1.0, "stops at the end");
    }

    #[test]
    fn whole_numbers_always_move_by_one_at_least() {
        let s = spec(SignalType::Int, Some((0.0, 10.0)));
        assert_eq!(stepped(3.0, &s, 1.0, false), 4.0);
        assert_eq!(stepped(0.0, &s, -1.0, false), 0.0);
        let free = spec(SignalType::Int, None);
        assert_eq!(stepped(3.0, &free, -1.0, true), -7.0);
    }

    #[test]
    fn a_number_without_a_range_moves_in_proportion() {
        let s = spec(SignalType::Float, None);
        assert!((stepped(200.0, &s, 1.0, false) - 202.0).abs() < 1e-9);
        assert!((stepped(0.0, &s, 1.0, false) - 0.01).abs() < 1e-9);
    }
}
