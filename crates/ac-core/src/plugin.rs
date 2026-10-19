//! Hosted plugins (ARCHITECTURE §13): what a `Plugin` node stores about the
//! plugin it plays, what it asks its host for, and the processor the host
//! hands back.
//!
//! The engine never loads code. A `Plugin` node's config holds a
//! [`PluginDesc`]: the plugin's identity and a description of its ports and
//! parameters, written when someone inserts it. The node's sockets come
//! from that description alone, so a project opens with its sockets and
//! wires even on a machine without the plugin. When the node is prepared it
//! asks [`crate::Resources::plugin`] for a [`PluginProcessor`]; a host that
//! can run plugins (the editor, `audioc`) loads it, and one that can't (the
//! web build, tests) says why, and the node plays silence.

use serde::{Deserialize, Serialize};

use crate::events::MidiEvent;
use crate::value::Config;

/// Most audio ports of each direction a plugin node exposes.
pub const MAX_PLUGIN_PORTS: usize = 8;
/// Most channels per audio port.
pub const MAX_PLUGIN_CHANNELS: u32 = 32;
/// Most parameters a plugin node turns into sockets (DECISIONS D131).
pub const MAX_PLUGIN_PARAMS: usize = 128;
/// Events one block can send a plugin; past it, the rest wait for the next
/// block (parameters) or are dropped (MIDI, beyond any real stream).
pub const PLUGIN_EVENT_CAPACITY: usize = 2048;
/// Samples between the parameter events a sample-rate input sends while it
/// moves, counted from the node's start so any block size gives the same
/// events.
pub const PLUGIN_PARAM_STRIDE: u64 = 32;

/// The plugin standard a plugin is built for.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PluginFormat {
    #[default]
    Clap,
    Vst3,
}

impl PluginFormat {
    pub fn name(self) -> &'static str {
        match self {
            PluginFormat::Clap => "CLAP",
            PluginFormat::Vst3 => "VST3",
        }
    }
}

/// One audio port of a plugin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginPort {
    #[serde(default)]
    pub name: String,
    pub channels: u32,
}

/// One automatable parameter of a plugin, as a node socket.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginParam {
    /// The plugin's own id for the parameter (stable across versions).
    pub id: u32,
    /// The name the plugin gives it.
    pub name: String,
    /// The socket it becomes: snake_case, unique on the node, chosen when
    /// the plugin is inserted and kept after, so wires survive a plugin
    /// that renames the parameter.
    pub socket: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Takes whole-number values (a switch, a choice).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stepped: bool,
}

/// What a `Plugin` node knows about its plugin without loading it: who it
/// is, where it was found, and its ports and parameters. Stored in the
/// node's config under `"plugin"`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginDesc {
    pub format: PluginFormat,
    /// The plugin's id: a CLAP id (`com.example.synth`) or a VST3 class id
    /// (32 hex digits).
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vendor: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// Where the plugin was found when it was described (a hint: hosts
    /// look the id up in their own scan first).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// Makes sound from notes (a synth) rather than processing audio.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub instrument: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_in: Vec<PluginPort>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio_out: Vec<PluginPort>,
    /// Takes notes and MIDI.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub midi_in: bool,
    /// Sends notes and MIDI.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub midi_out: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<PluginParam>,
}

/// Socket names a plugin's ports use, which parameters must not take.
const RESERVED: &[&str] = &["midi", "midi_out"];

impl PluginDesc {
    /// The description in a node's config (`config.plugin`), repaired
    /// ([`PluginDesc::repair`]); `None` when there is none or it can't be
    /// read.
    pub fn from_config(config: &Config) -> Option<PluginDesc> {
        let v = config.get("plugin")?;
        let mut d: PluginDesc = serde_json::from_value(v.clone()).ok()?;
        d.repair();
        Some(d)
    }

    /// Name of the socket for audio port `i` of a direction (`in`, `in_2`, …).
    pub fn port_socket(base: &str, i: usize) -> String {
        if i == 0 {
            base.to_string()
        } else {
            format!("{base}_{}", i + 1)
        }
    }

    /// Bring a hand-edited or outdated description within limits instead
    /// of rejecting it (DECISIONS D29): ports and parameters past the caps
    /// are dropped, channel counts clamp to 1..=32, reversed ranges swap,
    /// non-finite numbers become 0..1, and socket names that are missing,
    /// not snake_case, clash with a port, or repeat get fresh ones.
    pub fn repair(&mut self) {
        self.audio_in.truncate(MAX_PLUGIN_PORTS);
        self.audio_out.truncate(MAX_PLUGIN_PORTS);
        for p in self.audio_in.iter_mut().chain(self.audio_out.iter_mut()) {
            p.channels = p.channels.clamp(1, MAX_PLUGIN_CHANNELS);
        }
        self.params.truncate(MAX_PLUGIN_PARAMS);
        let mut taken: Vec<String> = RESERVED.iter().map(|s| s.to_string()).collect();
        for i in 0..MAX_PLUGIN_PORTS {
            taken.push(Self::port_socket("in", i));
            taken.push(Self::port_socket("out", i));
        }
        for p in &mut self.params {
            if !p.min.is_finite() || !p.max.is_finite() {
                p.min = 0.0;
                p.max = 1.0;
            }
            if p.min > p.max {
                std::mem::swap(&mut p.min, &mut p.max);
            }
            if !p.default.is_finite() {
                p.default = p.min;
            }
            p.default = p.default.clamp(p.min, p.max);
            let ok = crate::signal::is_snake_case(&p.socket) && !taken.contains(&p.socket);
            if !ok {
                let base = if crate::signal::is_snake_case(&p.socket) {
                    p.socket.clone()
                } else {
                    param_socket_base(&p.name, p.id)
                };
                p.socket = unique(&base, &taken);
            }
            taken.push(p.socket.clone());
        }
    }

    /// Socket names for a list of parameter names, as a host describing a
    /// plugin should give them: snake_case from the name (`Filter Cutoff`
    /// → `filter_cutoff`), `param_<id>` when the name has no letters or
    /// digits, and `_2`, `_3`, … when one repeats or clashes with a port.
    pub fn assign_sockets(params: &mut [PluginParam]) {
        for p in params.iter_mut() {
            p.socket.clear();
        }
        let mut d = PluginDesc {
            params: params.to_vec(),
            ..PluginDesc::default()
        };
        d.repair();
        for (p, fixed) in params.iter_mut().zip(d.params) {
            p.socket = fixed.socket;
        }
    }

    /// Short "Name (Vendor, FORMAT)" for messages.
    pub fn label(&self) -> String {
        if self.vendor.is_empty() {
            format!("{} ({})", self.name, self.format.name())
        } else {
            format!("{} ({}, {})", self.name, self.vendor, self.format.name())
        }
    }
}

/// snake_case of a parameter name, or `param_<id>`.
fn param_socket_base(name: &str, id: u32) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else {
            gap = true;
        }
        if out.len() >= 40 {
            break;
        }
    }
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        format!("param_{id}")
    } else {
        out
    }
}

fn unique(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|c| !taken.iter().any(|t| t == c))
        .unwrap_or_else(|| base.to_string())
}

/// What a `Plugin` node asks its host for while it's prepared.
#[derive(Clone, Copy, Debug)]
pub struct PluginRequest<'a> {
    /// Flattened node path, unique within the project.
    pub node_key: &'a str,
    /// The node's label or kind name.
    pub name: &'a str,
    pub desc: &'a PluginDesc,
    /// The plugin's saved state as the project stores it (base64), or
    /// empty for a fresh instance.
    pub state: &'a str,
    pub sample_rate: f64,
    /// The largest block the processor will be asked for.
    pub max_block: usize,
}

/// One event for a plugin, stamped with its offset in the block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PluginEvent {
    Midi(MidiEvent),
    /// Set parameter `index` (into [`PluginDesc::params`]) to `value`.
    Param {
        offset: u32,
        index: u32,
        value: f64,
    },
}

impl PluginEvent {
    pub fn offset(&self) -> u32 {
        match self {
            PluginEvent::Midi(e) => e.offset,
            PluginEvent::Param { offset, .. } => *offset,
        }
    }
}

/// The song position a plugin sees, at the first frame of the block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PluginTransport {
    pub playing: bool,
    pub recording: bool,
    /// Samples since transport zero.
    pub sample: u64,
    pub seconds: f64,
    /// Quarter notes since transport zero.
    pub beats: f64,
    pub bpm: f64,
    /// Beat position where the current bar started.
    pub bar_start: f64,
    /// Bar number, from 0.
    pub bar: i64,
    pub time_sig: [u8; 2],
}

/// MIDI a plugin sends during a block, into room reserved beforehand: a
/// full sink drops the rest rather than allocating.
#[derive(Debug)]
pub struct MidiSink<'a> {
    buf: &'a mut Vec<MidiEvent>,
}

impl<'a> MidiSink<'a> {
    /// A sink over `buf`, emptied; it never grows past `buf`'s capacity.
    pub fn new(buf: &'a mut Vec<MidiEvent>) -> Self {
        buf.clear();
        MidiSink { buf }
    }

    /// Add an event; returns false (and drops it) when full.
    pub fn push(&mut self, ev: MidiEvent) -> bool {
        if self.buf.len() < self.buf.capacity() {
            self.buf.push(ev);
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// One block of work for a plugin. Every channel buffer is at least
/// `frames` long; ports and channels are as the node's description has
/// them (a host maps them onto the plugin's actual ports).
#[derive(Debug)]
pub struct PluginBlock<'a> {
    pub frames: usize,
    /// Audio in, per port, per channel.
    pub audio_in: &'a mut [Vec<Vec<f32>>],
    /// Audio out, per port, per channel; the host fills them.
    pub audio_out: &'a mut [Vec<Vec<f32>>],
    /// Notes, MIDI, and parameter changes, in time order.
    pub events: &'a [PluginEvent],
    /// Notes and MIDI the plugin sends.
    pub midi_out: MidiSink<'a>,
    pub transport: PluginTransport,
}

/// A running plugin instance, as the engine sees it. Hosts implement this
/// (`ac-host`); the processor lives inside a `Plugin` node, moves to the
/// audio thread with the graph, and is dropped off the audio thread when
/// the graph retires.
///
/// `process` and `reset` run on the audio thread: the host's own code
/// around the plugin must not allocate, lock, or panic (ARCHITECTURE §8).
pub trait PluginProcessor: Send {
    /// Process one block. Returns false when the plugin failed it (the
    /// node outputs silence for the block).
    fn process(&mut self, block: &mut PluginBlock<'_>) -> bool;

    /// Samples of delay the plugin reports, for latency compensation.
    fn latency(&self) -> u32 {
        0
    }

    /// Clear voices, tails, and buffers (transport restart from zero).
    fn reset(&mut self);
}

/// The plugin-hosting part of a host's resources (`ac-host` implements
/// it; [`crate::Resources::plugin`] delegates to it in hosts that run
/// plugins).
pub trait PluginLoader: Send + Sync {
    /// Load, instantiate, and activate the plugin `req` describes, or say
    /// why it can't run (not installed, failed to start).
    fn load(&self, req: &PluginRequest<'_>) -> Result<Box<dyn PluginProcessor>, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn param(id: u32, name: &str, socket: &str) -> PluginParam {
        PluginParam {
            id,
            name: name.into(),
            socket: socket.into(),
            min: 0.0,
            max: 1.0,
            default: 0.5,
            stepped: false,
        }
    }

    #[test]
    fn socket_names_come_from_parameter_names() {
        let mut ps = vec![
            param(1, "Filter Cutoff", ""),
            param(2, "filter cutoff", ""),
            param(3, "  ", ""),
            param(4, "1st Osc", ""),
            param(5, "MIDI", ""),
            param(6, "Out", ""),
        ];
        PluginDesc::assign_sockets(&mut ps);
        let names: Vec<_> = ps.iter().map(|p| p.socket.as_str()).collect();
        assert_eq!(
            names,
            [
                "filter_cutoff",
                "filter_cutoff_2",
                "param_3",
                "param_4",
                "midi_2",
                "out_9"
            ]
        );
    }

    #[test]
    fn bad_descriptions_are_repaired() {
        let config = json!({ "plugin": {
            "format": "clap", "id": "x", "name": "X",
            "audio_in": [{ "channels": 0 }],
            "audio_out": [{ "name": "main", "channels": 99 }],
            "params": [
                { "id": 1, "name": "A", "socket": "Bad Name", "min": 1.0, "max": 0.0, "default": 5.0 },
                { "id": 2, "name": "B", "socket": "a", "min": 0.0, "max": 1.0, "default": 0.0 },
                { "id": 3, "name": "C", "socket": "a", "min": 0.0, "max": 1.0, "default": 0.0 },
            ],
        }});
        let d = PluginDesc::from_config(&config).unwrap();
        assert_eq!(d.audio_in[0].channels, 1);
        assert_eq!(d.audio_out[0].channels, MAX_PLUGIN_CHANNELS);
        assert_eq!((d.params[0].min, d.params[0].max), (0.0, 1.0));
        assert_eq!(d.params[0].default, 1.0);
        assert_eq!(d.params[0].socket, "a");
        assert_eq!(d.params[1].socket, "a_2");
        assert_eq!(d.params[2].socket, "a_3");
        assert!(PluginDesc::from_config(&json!({})).is_none());
        assert!(PluginDesc::from_config(&json!({ "plugin": 3 })).is_none());
    }

    #[test]
    fn descriptions_round_trip_and_stay_small() {
        let d = PluginDesc {
            format: PluginFormat::Vst3,
            id: "ABCD".into(),
            name: "Synth".into(),
            audio_out: vec![PluginPort {
                name: "Out".into(),
                channels: 2,
            }],
            midi_in: true,
            params: vec![param(7, "Level", "level")],
            ..PluginDesc::default()
        };
        let v = serde_json::to_value(&d).unwrap();
        assert!(v.get("vendor").is_none());
        assert!(v.get("audio_in").is_none());
        assert_eq!(v["format"], "vst3");
        let back: PluginDesc = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.label(), "Synth (VST3)");
    }

    #[test]
    fn a_full_sink_drops_instead_of_growing() {
        let mut buf = Vec::with_capacity(2);
        let mut sink = MidiSink::new(&mut buf);
        let ev = MidiEvent::new(
            0,
            crate::MidiMsg::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
        );
        assert!(sink.push(ev));
        assert!(sink.push(ev));
        assert!(!sink.push(ev));
        assert_eq!(sink.len(), 2);
        assert_eq!(buf.capacity(), 2);
    }
}
