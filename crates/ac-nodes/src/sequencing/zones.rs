use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};

node! {
    kind: "zones",
    name: "Zones",
    category: Sequencing,
    keywords: [
        "keymap", "key zone", "velocity layer", "split", "layer", "multisample", "map",
        "range", "sampler"
    ],
    /// Finds which zone of a key and velocity map a note falls in: the
    /// heart of a keymapped or velocity-layered instrument.
    ///
    /// The `zones` config lists the zones, each `[low, high, soft, hard]`:
    /// the notes from `low` to `high` (MIDI numbers, both included) played
    /// at a velocity from `soft` to `hard` (0 to 1). Draw them in the Node
    /// tab's map: drag a zone to move it, its edges to resize it. The
    /// default is one zone over every note and velocity. Up to 32.
    ///
    /// On each `trigger`, the `note` and `velocity` at that sample pick
    /// the zones they fall in: every one fires its `trigger_k`, and `zone`
    /// becomes the first of them (−1 when none). With `trigger` unwired,
    /// `zone` follows `note` and `velocity` all the time.
    ///
    /// **Try this:** in a `Voices` body, wire the Voice node's `note`,
    /// `velocity`, and `trigger` into Zones with two zones, `[0, 59, 0, 1]`
    /// and `[60, 127, 0, 1]`, and `trigger_0` and `trigger_1` into the
    /// `start` of two Samplers: a low sound and a high one. Split a zone by
    /// velocity instead for a soft and a hard layer.
    pub struct Zones {
        zones: Vec<Zone>,
        zone: i32,
    }
}

const NOTE: usize = 0;
const VELOCITY: usize = 1;
const TRIGGER: usize = 2;

const ZONE: usize = 0;

/// Most zones one map holds.
pub const MAX_ZONES: usize = 32;

/// One zone of the map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    pub low: f32,
    pub high: f32,
    pub soft: f32,
    pub hard: f32,
}

impl Zone {
    fn holds(&self, note: f32, velocity: f32) -> bool {
        note >= self.low - 0.5
            && note <= self.high + 0.5
            && velocity >= self.soft
            && velocity <= self.hard
    }
}

/// The `zones` config, repaired: ranges in order and in bounds; entries that
/// aren't four numbers are skipped.
pub fn zone_list(config: &Config) -> Vec<Zone> {
    let Some(list) = config.get("zones").and_then(|v| v.as_array()) else {
        return vec![Zone {
            low: 0.0,
            high: 127.0,
            soft: 0.0,
            hard: 1.0,
        }];
    };
    list.iter()
        .filter_map(|z| {
            let a = z.as_array()?;
            let n = |i: usize| {
                a.get(i)
                    .and_then(serde_json::Value::as_f64)
                    .filter(|x| x.is_finite())
                    .map(|x| x as f32)
            };
            let (l, h) = (n(0)?.clamp(0.0, 127.0), n(1)?.clamp(0.0, 127.0));
            let (s, d) = (n(2)?.clamp(0.0, 1.0), n(3)?.clamp(0.0, 1.0));
            Some(Zone {
                low: l.min(h).round(),
                high: l.max(h).round(),
                soft: s.min(d),
                hard: s.max(d),
            })
        })
        .take(MAX_ZONES)
        .collect()
}

impl Node for Zones {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new()
            .input(
                SocketSpec::float("note", Unit::Generic, 60.0)
                    .range(0.0, 127.0)
                    .doc("The note number."),
            )
            .input(
                SocketSpec::float("velocity", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .doc("How hard it was played, 0 to 1."),
            )
            .input(SocketSpec::trigger("trigger").doc("When to look the note up."))
            .output(
                SocketSpec::int("zone", -1)
                    .rate(Rate::Block)
                    .doc("The first zone the note falls in, from 0; −1 for none."),
            );
        for k in 0..zone_list(config).len() {
            sig = sig.output(
                SocketSpec::trigger(format!("trigger_{k}"))
                    .doc("Fires when a triggered note falls in this zone."),
            );
        }
        sig
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Zones {
            zones: zone_list(config),
            zone: -1,
        }
    }

    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let note = io.inputs.float(NOTE);
        let velocity = io.inputs.float(VELOCITY);
        let first = |i: usize, zones: &[Zone]| {
            let (n, v) = (note.at(i), velocity.at(i));
            zones
                .iter()
                .position(|z| z.holds(n, v))
                .map_or(-1, |k| k as i32)
        };
        if io.inputs.is_wired(TRIGGER) {
            for &at in io.inputs.triggers(TRIGGER) {
                let i = at as usize;
                let (n, v) = (note.at(i), velocity.at(i));
                for (k, z) in self.zones.iter().enumerate() {
                    if z.holds(n, v) {
                        io.outputs.trigger(1 + k).push(at);
                    }
                }
                self.zone = first(i, &self.zones);
            }
        } else {
            self.zone = first(0, &self.zones);
        }
        io.outputs.set_int(ZONE, self.zone);
    }

    fn reset(&mut self) {
        self.zone = -1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn zones_are_repaired_and_the_default_covers_everything() {
        assert_eq!(zone_list(&json!({})).len(), 1);
        let z = zone_list(&json!({ "zones": [[72, 60, 1.5, 0.2], "bad", [1, 2]] }));
        assert_eq!(
            z,
            vec![Zone {
                low: 60.0,
                high: 72.0,
                soft: 0.2,
                hard: 1.0
            }]
        );
        let sig = Zones::describe(&json!({ "zones": [[0, 59, 0, 1], [60, 127, 0, 1]] }));
        let outs: Vec<_> = sig.outputs.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(outs, ["zone", "trigger_0", "trigger_1"]);
    }

    #[test]
    fn a_trigger_fires_every_zone_the_note_falls_in() {
        // A key split, with a soft and a hard layer on the top half.
        let config = json!({ "zones": [[0, 59, 0, 1], [60, 127, 0, 0.6], [60, 127, 0.6, 1]] });
        let mut h = Harness::new::<Zones>(config);
        let note: Vec<f32> = (0..64).map(|i| if i < 20 { 48.0 } else { 64.0 }).collect();
        let vel: Vec<f32> = (0..64).map(|i| if i < 40 { 0.3 } else { 0.9 }).collect();
        h.set_samples("note", note)
            .set_samples("velocity", vel)
            .triggers("trigger", &[5, 30, 50]);
        let out = h.run(64);
        assert_eq!(out.triggers("trigger_0"), &[5]);
        assert_eq!(out.triggers("trigger_1"), &[30]);
        assert_eq!(out.triggers("trigger_2"), &[50]);
        assert_eq!(out.int("zone")[0], 2);
        // Untriggered, nothing fires and the zone stays.
        h.triggers("trigger", &[]);
        let out = h.run(64);
        assert!(out.triggers("trigger_2").is_empty());
        assert_eq!(out.int("zone")[0], 2);
    }

    #[test]
    fn unwired_trigger_follows_the_note_and_a_gap_is_minus_one() {
        let mut h = Harness::new::<Zones>(json!({ "zones": [[60, 72, 0, 1]] }));
        h.set("note", 65.0);
        assert_eq!(h.run(16).int("zone")[0], 0);
        h.set("note", 80.0);
        assert_eq!(h.run(16).int("zone")[0], -1);
    }
}
