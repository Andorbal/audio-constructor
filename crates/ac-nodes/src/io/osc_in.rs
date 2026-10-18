use ac_core::control::{OSC_ARGS_MAX, valid_osc_address};
use ac_core::{
    Config, ConfigExt, Control, ControlEvent, Determinism, Io, Node, NodeMeta, PrepareCtx,
    ProcessCtx, Signature, SocketSpec, Unit, node, osc_address_hash,
};

use super::{fill_range, stamped};

node! {
    kind: "osc_in",
    name: "OSC In",
    category: Io,
    keywords: ["open sound control", "network", "phone", "tablet", "touch", "remote", "control surface", "wireless", "input"],
    /// Receives Open Sound Control messages, such as the faders and buttons
    /// of a phone or tablet control surface, as numbers and a trigger.
    ///
    /// The `address` config is the address to listen to, exactly as the
    /// sender writes it (default `/fader/1`; patterns with wildcards aren't
    /// matched). `args` (1–4, default 1) is how many of each message's
    /// arguments to bring out: the first comes out of `value`, the others
    /// out of `value_2` to `value_4`. Each holds the last number received
    /// (true and false arrive as 1 and 0, and anything that isn't a finite
    /// number is skipped), starting at 0. `received` fires on the sample
    /// each message arrives on. A message without arguments sets `value`
    /// to 1, so a plain button works as a trigger; a button that sends 1
    /// and 0 works as a gate on `value`.
    ///
    /// An address that isn't valid (it must start with `/`, contain no
    /// spaces or any of `* ? [ ] { } # ,`, and be at most 64 characters)
    /// receives nothing, and the node stays at 0. The host decides which
    /// network port to listen on. Restarting the transport keeps the last
    /// values, the way the control surface still shows them.
    ///
    /// OSC In is a live source: nodes that need their whole input before
    /// they can play are disabled downstream of it.
    ///
    /// **Try this:** set `address` to a fader on your phone and wire
    /// `value` through a `Map` into a filter's `cutoff`, or wire a button's
    /// `received` into a `Clip`'s `start` to fire a sample from across the
    /// room.
    pub struct OscIn {
        /// The address's hash, or `None` for an invalid address.
        address: Option<u64>,
        args: usize,
        /// Each argument's last value.
        values: [f32; OSC_ARGS_MAX],
    }
}

const DEFAULT_ADDRESS: &str = "/fader/1";

/// The address (trimmed, valid) and argument count from config, repaired.
fn setup(config: &Config) -> (Option<&str>, usize) {
    let address = config.get_str_or("address", DEFAULT_ADDRESS).trim();
    let args = config.get_u64_or("args", 1).clamp(1, OSC_ARGS_MAX as u64) as usize;
    (valid_osc_address(address).then_some(address), args)
}

/// The socket name of argument `k` (0-based): `value`, `value_2`, ...
pub(crate) fn value_name(k: usize) -> String {
    if k == 0 {
        "value".to_string()
    } else {
        format!("value_{}", k + 1)
    }
}

/// What keeps this config from receiving, for the editor to show; `None`
/// when it's fine.
pub fn problem(config: &Config) -> Option<&'static str> {
    if setup(config).0.is_none() {
        return Some(ADDRESS_PROBLEM);
    }
    None
}

/// Why an OSC address was refused.
pub(crate) const ADDRESS_PROBLEM: &str = "The OSC address must start with /, have no spaces or * ? [ ] { } # , and be at most 64 characters.";

impl Node for OscIn {
    fn meta() -> NodeMeta {
        Self::base_meta().live()
    }

    fn describe(config: &Config) -> Signature {
        let (_, args) = setup(config);
        let mut sig = Signature::new();
        for k in 0..args {
            let doc = if k == 0 {
                "The message's first argument, held until the next."
            } else {
                "A later argument, held until the next message."
            };
            sig = sig.output(SocketSpec::float(value_name(k), Unit::Generic, 0.0).doc(doc));
        }
        sig.output(SocketSpec::trigger("received").doc("Fires on each message."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let (address, args) = setup(config);
        OscIn {
            address: address.map(osc_address_hash),
            args,
            values: [0.0; OSC_ARGS_MAX],
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let events = io.host.controls();
        let address = self.address;
        // The argument index of an event for this node's address.
        let arg_of = |ev: &ControlEvent| match ev.control {
            Control::Osc { address: a, arg } if Some(a) == address => Some(usize::from(arg)),
            _ => None,
        };

        for k in 0..self.args {
            let Some(value) = self.values.get_mut(k) else {
                break;
            };
            let mut out = io.outputs.float(k);
            let mut from = 0;
            for (at, ev) in stamped(events, frames) {
                if arg_of(ev) != Some(k) || !ev.value.is_finite() {
                    continue;
                }
                fill_range(&mut out, from, at, false, |_| *value);
                *value = ev.value;
                from = at;
            }
            fill_range(&mut out, from, frames, false, |_| *value);
        }

        // A message's arguments arrive as separate events on one sample,
        // in order; an argument that doesn't come after the one before it
        // starts the next message.
        let mut received = io.outputs.trigger(self.args);
        let mut last: Option<(usize, usize)> = None;
        for (at, ev) in stamped(events, frames) {
            let Some(arg) = arg_of(ev) else {
                continue;
            };
            if last.is_none_or(|(when, prev)| at != when || arg <= prev) {
                received.push(at as u32);
            }
            last = Some((at, arg));
        }
    }

    fn determinism(&self) -> Determinism {
        Determinism::Live
    }

    fn reset(&mut self) {
        // The values mirror the sender, which a transport restart doesn't
        // move (DECISIONS D90).
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn osc(address: &str, arg: u8) -> Control {
        Control::Osc {
            address: osc_address_hash(address),
            arg,
        }
    }

    #[test]
    fn values_change_on_the_exact_sample_and_hold() {
        let mut h = Harness::new::<OscIn>(json!({"address": "/fader/1"}));
        h.host_controls(&[
            (10, osc("/fader/1", 0), 0.25),
            (40, osc("/fader/2", 0), 0.9),
            (100, osc("/fader/1", 0), 0.75),
        ]);
        let out = h.run(192);
        let v = out.float("value");
        assert!(v[..10].iter().all(|&x| x == 0.0));
        assert!(v[10..100].iter().all(|&x| x == 0.25));
        assert!(v[100..].iter().all(|&x| x == 0.75));
        assert_eq!(out.triggers("received"), &[10, 100]);
        // The next block holds the last value.
        assert!(h.run(64).float("value").iter().all(|&x| x == 0.75));
    }

    #[test]
    fn several_arguments_fire_one_trigger_per_message() {
        let mut h = Harness::new::<OscIn>(json!({"address": "/xy", "args": 2}));
        h.host_controls(&[
            (5, osc("/xy", 0), 0.1),
            (5, osc("/xy", 1), 0.2),
            (5, osc("/xy", 2), 0.3),
            // A second message on the same sample.
            (5, osc("/xy", 0), 0.4),
            (5, osc("/xy", 1), 0.5),
            (9, osc("/xy", 1), 0.6),
        ]);
        let out = h.run(16);
        assert_eq!(out.triggers("received"), &[5, 5, 9]);
        assert_eq!(out.float("value")[5], 0.4);
        assert_eq!(out.float("value_2")[5], 0.5);
        assert_eq!(out.float("value_2")[9], 0.6);
        assert_eq!(out.float("value")[15], 0.4);
        let names: Vec<_> = h
            .signature()
            .outputs
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(names, ["value", "value_2", "received"]);
    }

    #[test]
    fn bare_messages_work_as_triggers() {
        let mut h = Harness::new::<OscIn>(json!({"address": "/go"}));
        h.host_controls(&[(3, osc("/go", 0), 1.0), (30, osc("/go", 0), 1.0)]);
        let out = h.run(64);
        assert_eq!(out.triggers("received"), &[3, 30]);
        assert_eq!(out.float("value")[3], 1.0);
    }

    #[test]
    fn non_finite_values_are_skipped_but_still_received() {
        let mut h = Harness::new::<OscIn>(json!({}));
        h.host_controls(&[
            (1, osc("/fader/1", 0), 0.5),
            (2, osc("/fader/1", 0), f32::NAN),
            (3, osc("/fader/1", 0), f32::INFINITY),
            (4, osc("/fader/1", 0), -1e30),
        ]);
        let out = h.run(8);
        assert_eq!(&out.float("value")[..5], &[0.0, 0.5, 0.5, 0.5, -1e30]);
        assert_eq!(out.triggers("received"), &[1, 2, 3, 4]);
    }

    #[test]
    fn invalid_addresses_receive_nothing() {
        for address in ["", "fader", "/with space", "/wild/*", "/a,b"] {
            let config = json!({ "address": address });
            assert_eq!(problem(&config), Some(ADDRESS_PROBLEM));
            let mut h = Harness::new::<OscIn>(config);
            h.host_controls(&[(0, osc(address, 0), 1.0)]);
            let out = h.run(64);
            assert!(out.float("value").iter().all(|&x| x == 0.0));
            assert!(out.triggers("received").is_empty());
        }
        let long = format!("/{}", "a".repeat(64));
        assert!(problem(&json!({ "address": long })).is_some());
        assert!(
            problem(&json!({ "address": 7 })).is_none(),
            "default address"
        );
        assert!(problem(&json!({})).is_none());
        assert!(problem(&json!({ "address": " /trimmed " })).is_none());
    }

    #[test]
    fn config_is_repaired() {
        assert_eq!(setup(&json!({})), (Some("/fader/1"), 1));
        assert_eq!(setup(&json!({"args": 0})).1, 1);
        assert_eq!(setup(&json!({"args": 99})).1, 4);
        assert_eq!(setup(&json!({"args": "two"})).1, 1);
        let sig = OscIn::describe(&json!({"args": 4}));
        let names: Vec<_> = sig.outputs.iter().map(|s| s.name.clone()).collect();
        assert_eq!(
            names,
            ["value", "value_2", "value_3", "value_4", "received"]
        );
    }

    #[test]
    fn arguments_past_args_only_trigger() {
        let mut h = Harness::new::<OscIn>(json!({"address": "/m"}));
        h.host_controls(&[(2, osc("/m", 3), 0.7)]);
        let out = h.run(8);
        assert!(out.float("value").iter().all(|&x| x == 0.0));
        assert_eq!(out.triggers("received"), &[2]);
    }

    #[test]
    fn block_size_does_not_matter() {
        let mut evs = Vec::new();
        let mut s = 3u32;
        for i in 0..500u64 {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let at = i * 8 + u64::from(s % 3);
            let address = if s.is_multiple_of(5) { "/other" } else { "/v" };
            for arg in 0..(s >> 4) % 4 {
                evs.push((at, osc(address, arg as u8), (s >> 8) as f32 / 1e6));
            }
        }
        let run = |block| {
            let mut h = Harness::with_resources::<OscIn>(
                json!({"address": "/v", "args": 3}),
                48_000.0,
                block,
                &NoResources,
            );
            h.host_controls(&evs);
            let out = h.run(4_000);
            (out.floats, out.triggers)
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(a.1["received"].len() > 100);
    }

    #[test]
    fn reset_keeps_the_last_values() {
        let mut h = Harness::new::<OscIn>(json!({}));
        h.host_controls(&[(0, osc("/fader/1", 0), 0.3)]);
        h.run(64);
        h.reset();
        let out = h.run(64);
        assert!(out.float("value").iter().all(|&x| x == 0.3));
        assert!(out.triggers("received").is_empty());
    }

    #[test]
    fn is_a_live_source() {
        assert!(OscIn::meta().live);
        let ctx = PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "OSC In",
            resources: &NoResources,
        };
        assert_eq!(
            OscIn::new(&json!({}), &ctx).determinism(),
            Determinism::Live
        );
    }
}
