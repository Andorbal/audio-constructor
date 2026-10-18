use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use ac_core::control::{OSC_ARGS_MAX, valid_osc_address};
use ac_core::{
    Config, ConfigExt, FloatIn, Io, Node, NodeMeta, OscMessage, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, Unit, node,
};

use super::osc_in::{ADDRESS_PROBLEM, value_name};

node! {
    kind: "osc_out",
    name: "OSC Out",
    category: Io,
    keywords: ["open sound control", "network", "send", "phone", "tablet", "remote", "control surface", "wireless", "output"],
    /// Sends numbers from the patch as Open Sound Control messages, to move
    /// the faders of a phone or tablet control surface or to drive
    /// anything else on the network that speaks OSC.
    ///
    /// `host` and `port` (config) say where to send: an IP address such as
    /// `192.168.1.20`, or `localhost` (default `127.0.0.1`, port 9000).
    /// Names other than `localhost` aren't looked up. `address` is the OSC
    /// address (default `/fader/1`), and `args` (1–4, default 1) how many
    /// numbers each message carries, from `value` and then `value_2` to
    /// `value_4`.
    ///
    /// A message goes out on every `send` trigger, and, while `on_change`
    /// is on (the default), whenever the values change, at most once every
    /// `interval` seconds (default 0.02, so a moving signal sends 50
    /// messages a second; never more often than every millisecond). The
    /// latest values always go out once the interval has passed, so the far
    /// end ends up in sync. With `on_change` on, the current values are
    /// also sent when the patch starts and when the transport restarts from
    /// zero. Values that aren't numbers are sent as 0.
    ///
    /// A config it can't send to (an invalid address, host, or port) sends
    /// nothing. The host passes the messages to the network.
    ///
    /// **Try this:** wire an `LFO` into `value` and watch a fader on your
    /// phone follow it, or wire a `Clock` into `send` and a `Transport`'s
    /// `bar` into `value` to show the bar number on a remote display.
    pub struct OscOut {
        /// Where messages go, or `None` when the config can't be sent to.
        target: Option<SocketAddr>,
        address: String,
        args: usize,
        /// The values last sent.
        last: [f32; OSC_ARGS_MAX],
        /// Whether anything has been sent since the start or the last reset.
        sent: bool,
        /// Samples until a change may be sent again.
        cooldown: u64,
    }
}

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 9000;
const DEFAULT_ADDRESS: &str = "/fader/1";
const DEFAULT_INTERVAL: f64 = 0.02;
const MIN_INTERVAL: f64 = 0.001;
const MAX_INTERVAL: f64 = 60.0;

/// The `host` config as an IP address: a literal, or `localhost`.
fn host_of(config: &Config) -> Option<IpAddr> {
    let host = match config.get("host") {
        None => DEFAULT_HOST,
        Some(v) => v.as_str()?,
    }
    .trim();
    if host.eq_ignore_ascii_case("localhost") {
        return Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse().ok()
}

/// The `port` config: a number (or numeric text) from 1 to 65535.
fn port_of(config: &Config) -> Option<u16> {
    let Some(v) = config.get("port") else {
        return Some(DEFAULT_PORT);
    };
    let n = v
        .as_u64()
        .or_else(|| {
            v.as_f64()
                .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                .map(|f| f as u64)
        })
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))?;
    u16::try_from(n).ok().filter(|&p| p != 0)
}

/// The address from config, trimmed, if it's valid.
fn address_of(config: &Config) -> Option<&str> {
    let address = config.get_str_or("address", DEFAULT_ADDRESS).trim();
    valid_osc_address(address).then_some(address)
}

fn args_of(config: &Config) -> usize {
    config.get_u64_or("args", 1).clamp(1, OSC_ARGS_MAX as u64) as usize
}

/// What keeps this config from sending, for the editor to show; `None`
/// when it's fine.
pub fn problem(config: &Config) -> Option<&'static str> {
    if address_of(config).is_none() {
        Some(ADDRESS_PROBLEM)
    } else if host_of(config).is_none() {
        Some("The host must be an IP address, such as 192.168.1.20, or localhost.")
    } else if port_of(config).is_none() {
        Some("The port must be a number from 1 to 65535.")
    } else {
        None
    }
}

/// A value as sent: NaN as 0, infinities as the largest finite numbers.
fn sendable(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-f32::MAX, f32::MAX)
    }
}

/// The `interval` input in samples, at least one.
fn interval_samples(seconds: f32, sample_rate: f64) -> u64 {
    let s = if seconds.is_nan() {
        DEFAULT_INTERVAL
    } else {
        f64::from(seconds).clamp(MIN_INTERVAL, MAX_INTERVAL)
    };
    (s * sample_rate).round().max(1.0) as u64
}

impl Node for OscOut {
    fn meta() -> NodeMeta {
        // Writes to the host's OSC output, so it runs on the device-output
        // path, one at a time.
        Self::base_meta().host_output()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new();
        for k in 0..args_of(config) {
            let doc = if k == 0 {
                "The message's first number."
            } else {
                "A later number in the message."
            };
            sig = sig.input(
                SocketSpec::float(value_name(k), Unit::Generic, 0.0)
                    .no_smooth()
                    .doc(doc),
            );
        }
        sig.input(SocketSpec::trigger("send").doc("Sends the values now."))
            .input(
                SocketSpec::boolean("on_change", true)
                    .doc("Also send whenever the values change, at most once per interval."),
            )
            .input(
                SocketSpec::float("interval", Unit::Seconds, DEFAULT_INTERVAL)
                    .range(MIN_INTERVAL, 1.0)
                    .log()
                    .doc("Shortest time between messages sent for changes."),
            )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let address = address_of(config);
        let target = match (address, host_of(config), port_of(config)) {
            (Some(_), Some(ip), Some(port)) => Some(SocketAddr::new(ip, port)),
            _ => None,
        };
        OscOut {
            target,
            address: address.unwrap_or_default().to_string(),
            args: args_of(config),
            last: [0.0; OSC_ARGS_MAX],
            sent: false,
            cooldown: 0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let Some(target) = self.target else {
            return;
        };
        let frames = ctx.frames;
        let args = self.args.min(OSC_ARGS_MAX);
        let send_at = io.inputs.triggers(args);
        let on_change = io.inputs.boolean(args + 1);
        let interval = io.inputs.float(args + 2);
        let mut values = [FloatIn::Const(0.0); OSC_ARGS_MAX];
        for (k, v) in values.iter_mut().enumerate().take(args) {
            *v = io.inputs.float(k);
        }
        let last_frame = frames.saturating_sub(1);
        let mut next = 0;
        for i in 0..frames {
            // Several triggers on one sample send once.
            let mut fired = false;
            while let Some(&t) = send_at.get(next) {
                if (t as usize).min(last_frame) > i {
                    break;
                }
                fired = true;
                next += 1;
            }
            let mut now = [0.0f32; OSC_ARGS_MAX];
            for (n, v) in now.iter_mut().zip(&values).take(args) {
                *n = sendable(v.at(i));
            }
            let changed = !self.sent || now != self.last;
            if fired || (changed && self.cooldown == 0 && on_change.at(i)) {
                let Some(msg) = now
                    .get(..args)
                    .and_then(|a| OscMessage::new(i as u32, target, &self.address, a))
                else {
                    return;
                };
                io.host.send_osc(msg);
                self.last = now;
                self.sent = true;
                self.cooldown = interval_samples(interval.at(i), ctx.sample_rate);
            }
            self.cooldown = self.cooldown.saturating_sub(1);
        }
    }

    fn reset(&mut self) {
        // Send the current values again from the restart.
        self.sent = false;
        self.cooldown = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, Output};
    use serde_json::json;

    /// `(frame, address, args)` of every message sent, checking the target.
    fn sent(out: &Output, to: &str) -> Vec<(u64, String, Vec<f32>)> {
        out.host_osc_out
            .iter()
            .map(|(at, m)| {
                assert_eq!(m.target.to_string(), to);
                (*at, m.address().to_string(), m.args().to_vec())
            })
            .collect()
    }

    fn msg(at: u64, address: &str, args: &[f32]) -> (u64, String, Vec<f32>) {
        (at, address.to_string(), args.to_vec())
    }

    #[test]
    fn triggers_send_the_values_at_their_offsets() {
        let mut h = Harness::new::<OscOut>(json!({
            "host": "192.168.1.20", "port": 8000, "address": "/synth/cutoff", "args": 2
        }));
        h.set("on_change", false);
        let ramp: Vec<f32> = (0..256).map(|i| i as f32).collect();
        h.set_samples("value", ramp).set("value_2", 0.5);
        h.triggers("send", &[3, 70, 70, 200]);
        let out = h.run(256);
        assert_eq!(
            sent(&out, "192.168.1.20:8000"),
            [
                msg(3, "/synth/cutoff", &[3.0, 0.5]),
                msg(70, "/synth/cutoff", &[70.0, 0.5]),
                msg(200, "/synth/cutoff", &[200.0, 0.5]),
            ]
        );
    }

    #[test]
    fn changes_send_at_most_once_per_interval_and_end_in_sync() {
        let mut h = Harness::new::<OscOut>(json!({}));
        // 10 ms at 48 kHz: 480 samples.
        h.set("interval", 0.01);
        // Moves every sample for 1000 samples, then holds.
        let v: Vec<f32> = (0..4_000).map(|i| (i.min(1_000)) as f32).collect();
        h.set_samples("value", v);
        let out = h.run(2_000);
        assert_eq!(
            sent(&out, "127.0.0.1:9000"),
            [
                msg(0, "/fader/1", &[0.0]),
                msg(480, "/fader/1", &[480.0]),
                msg(960, "/fader/1", &[960.0]),
                // The value settled at 1000 after the last send: it goes
                // out as soon as the interval allows.
                msg(1_440, "/fader/1", &[1_000.0]),
            ]
        );
        // Nothing changes afterwards: nothing more is sent.
        assert!(h.run(2_000).host_osc_out.is_empty());
    }

    #[test]
    fn a_still_value_is_sent_once_at_the_start_and_after_a_reset() {
        let mut h = Harness::new::<OscOut>(json!({"host": "localhost"}));
        h.set("value", 0.25);
        let out = h.run(256);
        assert_eq!(sent(&out, "127.0.0.1:9000"), [msg(0, "/fader/1", &[0.25])]);
        h.reset();
        let out = h.run(128);
        assert_eq!(
            sent(&out, "127.0.0.1:9000"),
            [msg(256, "/fader/1", &[0.25])]
        );
        // With on_change off, only triggers send.
        let mut h = Harness::new::<OscOut>(json!({}));
        h.set("on_change", false).set("value", 0.25);
        assert!(h.run(256).host_osc_out.is_empty());
    }

    #[test]
    fn a_trigger_sends_even_during_the_interval() {
        let mut h = Harness::new::<OscOut>(json!({}));
        h.set("interval", 1.0).set("value", 2.0);
        h.triggers("send", &[10]);
        let out = h.run(64);
        assert_eq!(
            sent(&out, "127.0.0.1:9000"),
            [msg(0, "/fader/1", &[2.0]), msg(10, "/fader/1", &[2.0])]
        );
    }

    #[test]
    fn the_interval_is_in_seconds_at_any_rate() {
        let count = |rate: f64| {
            let mut h = Harness::with_resources::<OscOut>(json!({}), rate, 64, &NoResources);
            let n = rate as usize;
            h.set("interval", 0.1)
                .set_samples("value", (0..n).map(|i| i as f32).collect());
            h.run(n).host_osc_out.len()
        };
        assert_eq!(count(48_000.0), 10);
        assert_eq!(count(44_100.0), 10);
        assert_eq!(count(96_000.0), 10);
        // A tiny interval is floored at a millisecond.
        let mut h = Harness::new::<OscOut>(json!({}));
        h.set("interval", 0.0)
            .set_samples("value", (0..4_800).map(|i| i as f32).collect());
        assert_eq!(h.run(4_800).host_osc_out.len(), 100);
        assert_eq!(interval_samples(f32::NAN, 1_000.0), 20);
        assert_eq!(interval_samples(-5.0, 1.0), 1);
        assert_eq!(interval_samples(1e9, 10.0), 600);
    }

    #[test]
    fn odd_values_are_sent_as_numbers() {
        let mut h = Harness::new::<OscOut>(json!({"args": 3}));
        h.set_samples("value", vec![f32::NAN; 5_000])
            .set_samples("value_2", vec![f32::INFINITY; 5_000])
            .set_samples("value_3", vec![f32::NEG_INFINITY; 5_000]);
        let out = h.run(8);
        assert_eq!(
            sent(&out, "127.0.0.1:9000"),
            [msg(0, "/fader/1", &[0.0, f32::MAX, -f32::MAX])]
        );
        // NaN doesn't count as a change every sample.
        assert!(h.run(4_800).host_osc_out.is_empty());
    }

    #[test]
    fn invalid_configs_send_nothing() {
        let bad = [
            json!({"address": "no-slash"}),
            json!({"address": "/a b"}),
            json!({"host": "example.org"}),
            json!({"host": ""}),
            json!({"host": 5}),
            json!({"port": 0}),
            json!({"port": 70_000}),
            json!({"port": "http"}),
            json!({"port": -1}),
        ];
        for config in bad {
            assert!(problem(&config).is_some(), "{config}");
            let mut h = Harness::new::<OscOut>(config);
            h.set("value", 1.0).triggers("send", &[0, 5]);
            assert!(h.run(64).host_osc_out.is_empty());
        }
    }

    #[test]
    fn config_is_read_and_repaired() {
        assert!(problem(&json!({})).is_none());
        let target = |c| OscOut::new(&c, &ctx()).target.map(|t| t.to_string());
        assert_eq!(target(json!({})), Some("127.0.0.1:9000".into()));
        assert_eq!(
            target(json!({"host": " LOCALHOST ", "port": "7000"})),
            Some("127.0.0.1:7000".into())
        );
        assert_eq!(
            target(json!({"host": "::1", "port": 9001.0})),
            Some("[::1]:9001".into())
        );
        assert_eq!(
            target(json!({"host": "[fe80::1]", "port": 1})),
            Some("[fe80::1]:1".into())
        );
        assert_eq!(target(json!({"port": 9000.5})), None);
        let sig = OscOut::describe(&json!({"args": 9}));
        let names: Vec<_> = sig.inputs.iter().map(|s| s.name.clone()).collect();
        assert_eq!(
            names,
            [
                "value",
                "value_2",
                "value_3",
                "value_4",
                "send",
                "on_change",
                "interval"
            ]
        );
        assert_eq!(OscOut::describe(&json!({"args": 0})).inputs.len(), 4);
    }

    #[test]
    fn block_size_does_not_matter() {
        let run = |block| {
            let mut h = Harness::with_resources::<OscOut>(
                json!({"args": 2}),
                48_000.0,
                block,
                &NoResources,
            );
            let v: Vec<f32> = (0..6_000).map(|i| ((i / 700) % 3) as f32).collect();
            let gate: Vec<f32> = (0..6_000).map(|i| ((i / 1_300) % 2) as f32).collect();
            let interval: Vec<f32> = (0..6_000)
                .map(|i| 0.001 + (i % 900) as f32 * 1e-5)
                .collect();
            h.set_samples("value", v)
                .set_samples("value_2", (0..6_000).map(|i| i as f32).collect())
                .set_samples("on_change", gate)
                .set_samples("interval", interval)
                .triggers("send", &[5, 777, 778, 4_000]);
            let out = h.run(6_000);
            // A message's `offset` is within its block; compare the rest.
            sent(&out, "127.0.0.1:9000")
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(a.len() > 10);
    }

    #[test]
    fn runs_on_the_device_output_path() {
        assert!(OscOut::meta().host_output);
        assert!(!OscOut::meta().live);
    }

    fn ctx() -> PrepareCtx<'static> {
        PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "OSC Out",
            resources: &NoResources,
        }
    }
}
