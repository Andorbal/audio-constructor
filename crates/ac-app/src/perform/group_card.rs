//! A group or block instance pinned to a panel (DECISIONS D203): one card
//! with a widget for each of its inputs, chosen by type and unit, and a
//! meter, light, or readout for each output, so the Track block pins as a
//! channel strip. The widgets set the instance's own inputs (its `params`,
//! as the Node tab does); a wired input shows the value arriving on its
//! wire instead.

use ac_core::{NodeId, NodePath, SignalType, SocketSpec, Unit, Value};
use egui::{Align2, Painter, Pos2, Rect, pos2, vec2};

use crate::canvas::live::{Live, LiveState, source_path};
use crate::canvas::scene::NodeView;
use crate::canvas::{CanvasIo, bodies, widgets};
use crate::theme;

/// What a socket of a pinned group is drawn as.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A level in dB, with the meter of `meter` (an audio output) beside it.
    Fader {
        meter: Option<String>,
    },
    Knob,
    Toggle,
    /// Fires the trigger input.
    Fire,
    Choice,
    Number,
    Meter,
    Light,
    Readout,
}

impl Kind {
    /// Lines (half cells) the widget takes on the card.
    pub fn lines(&self) -> u32 {
        match self {
            Kind::Fader { .. } => 5,
            Kind::Knob => 2,
            _ => 1,
        }
    }

    /// Played with the pointer (inputs), or only shown (outputs).
    pub fn is_input(&self) -> bool {
        !matches!(self, Kind::Meter | Kind::Light | Kind::Readout)
    }
}

/// One widget of a pinned group's card.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub socket: String,
    pub kind: Kind,
    /// An input with a wire into it: shows the wire's value, can't be set.
    pub wired: bool,
    pub spec: Option<SocketSpec>,
}

/// Whether a node is a group (or Voices) instance, which pins as a card of
/// its sockets.
pub fn is_group(n: &ac_core::NodeInstance) -> bool {
    n.kind == ac_core::GROUP_KIND || n.kind == ac_core::VOICES_KIND
}

/// The widgets of a group instance's card, inputs then outputs.
pub fn widgets(n: &NodeView) -> Vec<Widget> {
    let inputs: Vec<(&str, SignalType, Option<&SocketSpec>, bool)> = n
        .inputs
        .iter()
        .map(|s| (s.name.as_str(), s.ty, s.spec.as_ref(), s.wired))
        .collect();
    let outputs: Vec<(&str, SignalType, Option<&SocketSpec>)> = n
        .outputs
        .iter()
        .map(|s| (s.name.as_str(), s.ty, s.spec.as_ref()))
        .collect();
    widgets_from(&inputs, &outputs)
}

/// The widgets of a group with this signature, where `wired` says which
/// inputs have a wire into them.
pub fn widgets_of(sig: &ac_core::Signature, wired: impl Fn(&str) -> bool) -> Vec<Widget> {
    let inputs: Vec<(&str, SignalType, Option<&SocketSpec>, bool)> = sig
        .inputs
        .iter()
        .map(|s| (s.name.as_ref(), s.ty, Some(s), wired(&s.name)))
        .collect();
    let outputs: Vec<(&str, SignalType, Option<&SocketSpec>)> = sig
        .outputs
        .iter()
        .map(|s| (s.name.as_ref(), s.ty, Some(s)))
        .collect();
    widgets_from(&inputs, &outputs)
}

type In<'a> = (&'a str, SignalType, Option<&'a SocketSpec>, bool);
type Out<'a> = (&'a str, SignalType, Option<&'a SocketSpec>);

fn widgets_from(inputs: &[In<'_>], outputs: &[Out<'_>]) -> Vec<Widget> {
    let mut out = Vec::new();
    let mut claimed: Option<String> = None;
    let first_audio = outputs
        .iter()
        .find(|s| s.1 == SignalType::Audio)
        .map(|s| s.0.to_string());
    for &(name, ty, spec, wired) in inputs {
        let Some(spec) = spec.cloned() else { continue };
        // A trigger a wire fires isn't a button to press (D222).
        if spec.hidden || (wired && ty == SignalType::Trigger) {
            continue;
        }
        let kind = match ty {
            SignalType::Float if spec.unit == Unit::Db => {
                let meter = if claimed.is_none() {
                    claimed = first_audio.clone();
                    claimed.clone()
                } else {
                    None
                };
                Kind::Fader { meter }
            }
            SignalType::Float => Kind::Knob,
            SignalType::Int if !spec.choices.is_empty() => Kind::Choice,
            SignalType::Int => Kind::Number,
            SignalType::Bool => Kind::Toggle,
            SignalType::Trigger => Kind::Fire,
            _ => continue,
        };
        out.push(Widget {
            socket: name.to_string(),
            kind,
            wired,
            spec: Some(spec),
        });
    }
    for &(name, ty, spec) in outputs {
        let kind = match ty {
            SignalType::Audio if claimed.as_deref() == Some(name) => continue,
            SignalType::Audio => Kind::Meter,
            SignalType::Bool => Kind::Light,
            SignalType::Float | SignalType::Int => Kind::Readout,
            _ => continue,
        };
        out.push(Widget {
            socket: name.to_string(),
            kind,
            wired: false,
            spec: spec.cloned(),
        });
    }
    out
}

/// Lines a card of these widgets takes, with its header.
pub fn card_lines(widgets: &[Widget]) -> u32 {
    1 + widgets.iter().map(|w| w.kind.lines()).sum::<u32>()
}

/// Each widget's rectangle in a card's content area, top to bottom, each
/// taking its lines of `line` points.
pub fn place(widgets: &[Widget], content: Rect, line: f32) -> Vec<Rect> {
    let mut y = content.min.y;
    widgets
        .iter()
        .map(|w| {
            let h = w.kind.lines() as f32 * line;
            let r = Rect::from_min_size(pos2(content.min.x, y), vec2(content.width(), h));
            y += h;
            r.shrink2(vec2(0.0, 2.0))
        })
        .collect()
}

/// A widget's value as the card shows it: an unwired input's own (or a
/// binding's), a wired input's arriving value, an output's live one.
pub fn value(io: &CanvasIo, live: &Live, n: &NodeView, w: &Widget) -> Option<f64> {
    let spec = w.spec.as_ref()?;
    if !w.kind.is_input() {
        return live
            .tele
            .socket_value(&live.path(n.id), &w.socket)
            .map(f64::from);
    }
    if w.wired {
        let wire = io
            .doc
            .graph()
            .wire_into(&ac_core::Endpoint::new(n.id, w.socket.clone()))?;
        let (path, socket) = source_path(io.doc, &wire.from)?;
        return live.tele.socket_value(&path, &socket).map(f64::from);
    }
    Some(widgets::number(&io.shown_node(n.id)?, spec))
}

/// A key for a widget's meter ballistics, apart from the node's own.
fn meter_key(path: &NodePath, socket: &str) -> NodePath {
    let mut h = ac_core::util::Fnv64::new();
    h.write(socket.as_bytes());
    path.child(NodeId(h.finish()))
}

/// What the view shows a widget as this frame.
pub struct Look {
    pub zoom: f32,
    pub alpha: f32,
    pub hovered: Option<usize>,
    pub dragging: Option<String>,
}

/// Paint a group's card widgets, each in its rectangle.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    painter: &Painter,
    io: &CanvasIo,
    live: &Live,
    state: &mut LiveState,
    n: &NodeView,
    widgets: &[Widget],
    rects: &[Rect],
    look: &Look,
    now: f64,
    dt: f32,
) {
    let z = look.zoom;
    let path = live.path(n.id);
    for (k, (w, r)) in widgets.iter().zip(rects).enumerate() {
        let alpha = if w.wired {
            0.6 * look.alpha
        } else {
            look.alpha
        };
        let hovered = look.hovered == Some(k) && !w.wired;
        let active = look.dragging.as_deref() == Some(w.socket.as_str());
        let name = crate::canvas::paint_pretty(&w.socket);
        let v = value(io, live, n, w);
        let label_font = theme::canvas_font(theme::FONT_SMALL, z, false);
        let label = |painter: &Painter, at: Pos2, align: Align2| {
            painter.text(
                at,
                align,
                &name,
                label_font.clone(),
                theme::fade(theme::text_muted(), alpha),
            );
        };
        match &w.kind {
            Kind::Fader { meter } => {
                label(
                    painter,
                    r.left_top() + vec2(0.0, 6.0 * z),
                    Align2::LEFT_CENTER,
                );
                let spec = w.spec.as_ref();
                let t = spec
                    .zip(v)
                    .and_then(|(s, v)| widgets::normalized(v, s))
                    .unwrap_or(0.0);
                let text = spec
                    .zip(v)
                    .map_or("–".into(), |(s, v)| widgets::format(v, s));
                let level = meter
                    .as_deref()
                    .and_then(|m| live.tele.socket_level(&path, m))
                    .unwrap_or(0.0);
                let key = meter_key(&path, &w.socket);
                let ch = state.meter(&key, [(level, level), (level, level)], now, dt);
                let body = Rect::from_min_max(pos2(r.min.x, r.min.y + 12.0 * z), r.max);
                bodies::fader(painter, body, z, t, &text, ch, hovered, active, alpha);
            }
            Kind::Knob => {
                let spec = w.spec.as_ref();
                let t = spec
                    .zip(v)
                    .and_then(|(s, v)| widgets::normalized(v, s))
                    .unwrap_or(0.0);
                let zero = spec.map_or(0.0, widgets::zero_point);
                let text = spec
                    .zip(v)
                    .map_or("–".into(), |(s, v)| widgets::format(v, s));
                let knob = Rect::from_min_max(r.min, pos2(r.min.x + r.height() * 1.1, r.max.y));
                bodies::knob(painter, knob, z, t, zero, &text, hovered, active, alpha);
                label(
                    painter,
                    pos2(knob.max.x + 6.0 * z, r.center().y),
                    Align2::LEFT_CENTER,
                );
            }
            Kind::Toggle => {
                let on = v.is_some_and(|v| v > 0.5);
                let sw = Rect::from_min_max(r.min, pos2(r.min.x + r.height() * 2.6, r.max.y));
                bodies::toggle(painter, sw, z, on, false, hovered, alpha);
                label(painter, pos2(r.max.x, r.center().y), Align2::RIGHT_CENTER);
            }
            Kind::Fire => {
                bodies::button(painter, *r, z, active, false, hovered, alpha);
                // The button's own text says "press"; the name goes over it.
                painter.rect_filled(
                    r.shrink(2.0 * z),
                    theme::radius(5.0 * z),
                    theme::fade(
                        if active {
                            theme::accent()
                        } else {
                            theme::raised()
                        },
                        alpha,
                    ),
                );
                label(painter, r.center(), Align2::CENTER_CENTER);
            }
            Kind::Choice => {
                let text = w
                    .spec
                    .as_ref()
                    .zip(io.shown_node(n.id))
                    .map(|(s, inst)| widgets::choice_label(s, &widgets::choice(&inst, s)))
                    .unwrap_or_default();
                bodies::dropdown(painter, *r, z, &text, hovered, false, alpha);
            }
            Kind::Number => {
                let text = w
                    .spec
                    .as_ref()
                    .zip(v)
                    .map_or("–".into(), |(s, v)| widgets::format(v, s));
                bodies::number(
                    painter,
                    *r,
                    z,
                    &format!("{name}  {text}"),
                    hovered,
                    active,
                    alpha,
                );
            }
            Kind::Meter => {
                let level = live.tele.socket_level(&path, &w.socket).unwrap_or(0.0);
                let key = meter_key(&path, &w.socket);
                let ch = state.meter(&key, [(level, level), (level, level)], now, dt);
                let bar = Rect::from_min_max(
                    pos2(r.min.x, r.center().y - 4.0 * z),
                    pos2(r.max.x, r.center().y + 4.0 * z),
                );
                bodies::meter_bar(painter, bar, z, ch[0], alpha);
            }
            Kind::Light => {
                let on = v.unwrap_or(0.0) as f32;
                let lamp = Rect::from_min_max(r.min, pos2(r.min.x + r.height(), r.max.y));
                bodies::light(painter, lamp, z, on, bodies::lit_color(), alpha);
                label(
                    painter,
                    pos2(lamp.max.x + 4.0 * z, r.center().y),
                    Align2::LEFT_CENTER,
                );
            }
            Kind::Readout => {
                let text = crate::canvas::controls::readout_text(v.map(|v| v as f32));
                bodies::readout(painter, *r, z, &format!("{name}  {text}"), alpha);
            }
        }
    }
}

/// The widget under screen point `p`, if any.
pub fn hit(rects: &[Rect], p: Pos2) -> Option<usize> {
    rects.iter().position(|r| r.contains(p))
}

/// Toggle a Bool input from its card: one undo step.
pub fn flip(io: &mut CanvasIo, n: &NodeView, w: &Widget, now: f64) {
    if w.wired {
        return;
    }
    let on = w
        .spec
        .as_ref()
        .zip(io.shown_node(n.id))
        .is_some_and(|(s, inst)| widgets::number(&inst, s) > 0.5);
    crate::params::set(io.doc, io.engine, n.id, &w.socket, Value::Bool(!on), now);
    io.doc.end_coalescing();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::scene::{Class, SocketView};

    fn socket(name: &str, ty: SignalType, spec: Option<SocketSpec>, input: bool) -> SocketView {
        SocketView {
            name: name.into(),
            ty,
            spec,
            pos: Pos2::ZERO,
            row: Rect::NOTHING,
            wired: false,
            input,
            reserve: 0.0,
        }
    }

    #[test]
    fn a_track_block_pins_as_a_channel_strip() {
        let n = NodeView {
            id: NodeId(1),
            kind: ac_core::GROUP_KIND.into(),
            class: Class::Normal,
            title: "Drums".into(),
            category: None,
            accent: egui::Color32::GRAY,
            rect: Rect::NOTHING,
            header: Rect::NOTHING,
            collapsed: false,
            muted: false,
            selected: false,
            inputs: vec![
                socket("in", SignalType::Audio, Some(SocketSpec::audio("in")), true),
                socket(
                    "volume",
                    SignalType::Float,
                    Some(SocketSpec::float("volume", Unit::Db, 0.0)),
                    true,
                ),
                socket(
                    "pan",
                    SignalType::Float,
                    Some(SocketSpec::float("pan", Unit::Generic, 0.0)),
                    true,
                ),
                socket(
                    "mute",
                    SignalType::Bool,
                    Some(SocketSpec::boolean("mute", false)),
                    true,
                ),
                socket(
                    "go",
                    SignalType::Trigger,
                    Some(SocketSpec::trigger("go")),
                    true,
                ),
            ],
            outputs: vec![
                socket(
                    "out",
                    SignalType::Audio,
                    Some(SocketSpec::audio("out")),
                    false,
                ),
                socket(
                    "soloed",
                    SignalType::Bool,
                    Some(SocketSpec::boolean("soloed", false)),
                    false,
                ),
                socket("midi", SignalType::Midi, None, false),
            ],
            body: None,
            more: None,
        };
        let w = widgets(&n);
        let kinds: Vec<(&str, &Kind)> = w.iter().map(|w| (w.socket.as_str(), &w.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                (
                    "volume",
                    &Kind::Fader {
                        meter: Some("out".into())
                    }
                ),
                ("pan", &Kind::Knob),
                ("mute", &Kind::Toggle),
                ("go", &Kind::Fire),
                ("soloed", &Kind::Light),
            ],
            "the audio output is the fader's meter"
        );
        assert_eq!(card_lines(&w), 1 + 5 + 2 + 1 + 1 + 1);
        let rects = place(
            &w,
            Rect::from_min_size(Pos2::ZERO, vec2(100.0, 500.0)),
            20.0,
        );
        assert_eq!(rects.len(), 5);
        assert!(rects[1].min.y >= 100.0, "the fader takes five lines");
        assert_eq!(hit(&rects, pos2(10.0, 125.0)), Some(1));
    }
}
