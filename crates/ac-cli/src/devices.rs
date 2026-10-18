//! `audioc devices`: audio hosts and devices, MIDI ports, gamepads, and
//! how OSC works.

use std::fmt::Write as _;
use std::net::IpAddr;
use std::process::ExitCode;

use ac_io::{DEFAULT_OSC_PORT, DeviceInfo, GamepadError, GamepadInfo, HostInfo};
use anyhow::Result;

pub fn devices() -> Result<ExitCode> {
    let hosts = ac_io::enumerate();
    let midi = ac_io::midi::list_inputs();
    let midi_out = ac_io::list_outputs();
    let pads = ac_io::list_gamepads();
    let ip = ac_io::osc::local_ip();
    print!("{}", report(&hosts, &midi, &midi_out, &pads, ip));
    Ok(ExitCode::SUCCESS)
}

type Ports = Result<Vec<ac_io::MidiPortInfo>, ac_io::midi::MidiError>;
type Pads = Result<Vec<GamepadInfo>, GamepadError>;

/// The whole listing as text (separate from printing so it can be tested).
pub fn report(
    hosts: &[HostInfo],
    midi: &Ports,
    midi_out: &Ports,
    pads: &Pads,
    ip: Option<IpAddr>,
) -> String {
    let mut s = String::new();
    if hosts.is_empty() {
        s.push_str("audio: no hosts compiled in\n");
    }
    for h in hosts {
        let _ = writeln!(
            s,
            "{}{}",
            h.name,
            if h.default { " (default host)" } else { "" }
        );
        if let Some(e) = &h.error {
            let _ = writeln!(s, "  error: {e}");
        }
        section(&mut s, "outputs", &h.outputs);
        section(&mut s, "inputs", &h.inputs);
    }
    ports(&mut s, "MIDI inputs", midi);
    ports(&mut s, "MIDI outputs", midi_out);
    s.push_str("Gamepads\n");
    match pads {
        Ok(p) if p.is_empty() => s.push_str("  (none connected)\n"),
        Ok(p) => {
            for g in p {
                let _ = writeln!(s, "  {}  {}", g.index, g.name);
            }
        }
        Err(e) => {
            let _ = writeln!(s, "  unavailable: {e}");
        }
    }
    s.push_str("OSC\n");
    let port = DEFAULT_OSC_PORT;
    let _ = writeln!(
        s,
        "  in: off until asked for; `audioc run --osc-in {port}` listens on UDP port {port}"
    );
    if let Some(ip) = ip {
        let _ = writeln!(s, "      (send from a phone or tablet to {ip}:{port})");
    }
    s.push_str("  out: OSC Out nodes send to the address and port in their settings\n");
    s
}

fn ports(s: &mut String, title: &str, ports: &Ports) {
    let _ = writeln!(s, "{title}");
    match ports {
        Ok(ports) if ports.is_empty() => s.push_str("  (none)\n"),
        Ok(ports) => {
            for p in ports {
                let _ = writeln!(s, "  {}", p.name);
            }
        }
        Err(e) => {
            let _ = writeln!(s, "  unavailable: {e}");
        }
    }
}

fn section(s: &mut String, title: &str, devices: &[DeviceInfo]) {
    let _ = writeln!(s, "  {title}:");
    if devices.is_empty() {
        s.push_str("    (none)\n");
    }
    for d in devices {
        let _ = writeln!(s, "  {} {}", if d.default { "*" } else { " " }, d.name);
        let _ = writeln!(s, "      {}", describe(d));
    }
}

/// `2 ch · 48000 Hz (8000–192000) · buffer 32–8192 frames · f32`
pub fn describe(d: &DeviceInfo) -> String {
    let mut parts = vec![format!("{} ch", d.channels)];
    match (d.default_sample_rate, d.sample_rates) {
        (Some(r), Some((lo, hi))) if lo != hi => parts.push(format!("{r} Hz ({lo}–{hi})")),
        (Some(r), _) => parts.push(format!("{r} Hz")),
        (None, Some((lo, hi))) => parts.push(format!("{lo}–{hi} Hz")),
        (None, None) => parts.push("rate unknown".into()),
    }
    parts.push(match d.buffer_sizes {
        Some((lo, hi)) => format!("buffer {lo}–{hi} frames"),
        None => "buffer size set by the driver".into(),
    });
    if let Some(f) = &d.sample_format {
        parts.push(f.clone());
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(name: &str, default: bool) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            default,
            channels: 2,
            default_sample_rate: Some(48_000),
            sample_rates: Some((8_000, 192_000)),
            buffer_sizes: Some((32, 8_192)),
            sample_format: Some("f32".into()),
        }
    }

    #[test]
    fn lists_hosts_devices_and_midi() {
        let hosts = vec![HostInfo {
            name: "ALSA".into(),
            default: true,
            outputs: vec![dev("Speakers", true), dev("HDMI", false)],
            inputs: vec![],
            error: None,
        }];
        let midi = Ok(vec![ac_io::MidiPortInfo {
            name: "Keys 1".into(),
            id: "1".into(),
        }]);
        let out = Ok(vec![ac_io::MidiPortInfo {
            name: "Synth".into(),
            id: "2".into(),
        }]);
        let pads = Ok(vec![GamepadInfo {
            index: 0,
            name: "Wireless Controller".into(),
        }]);
        let text = report(&hosts, &midi, &out, &pads, Some([10, 0, 0, 5].into()));
        assert!(text.contains("ALSA (default host)"));
        assert!(text.contains("  * Speakers"));
        assert!(text.contains("    HDMI"));
        assert!(text.contains("2 ch · 48000 Hz (8000–192000) · buffer 32–8192 frames · f32"));
        assert!(text.contains("inputs:\n    (none)"));
        assert!(text.contains("MIDI inputs\n  Keys 1"));
        assert!(text.contains("MIDI outputs\n  Synth"));
        assert!(text.contains("Gamepads\n  0  Wireless Controller"));
        assert!(text.contains("--osc-in 9000"));
        assert!(text.contains("10.0.0.5:9000"));
    }

    #[test]
    fn nothing_present_still_reads_well() {
        let midi = Err(ac_io::midi::MidiError("no sequencer".into()));
        let pads = Err(GamepadError(
            "gamepads aren't supported on this system".into(),
        ));
        let text = report(&[], &midi, &Ok(Vec::new()), &pads, None);
        assert!(text.contains("no hosts"));
        assert!(text.contains("unavailable: no sequencer"));
        assert!(text.contains("Gamepads\n  unavailable: gamepads aren't supported"));
        assert!(!text.contains("send from a phone"));
        let none = report(&[], &midi, &Ok(Vec::new()), &Ok(Vec::new()), None);
        assert!(none.contains("Gamepads\n  (none connected)"));
        let mut d = dev("x", false);
        d.default_sample_rate = None;
        d.sample_rates = None;
        d.buffer_sizes = None;
        d.sample_format = None;
        assert_eq!(
            describe(&d),
            "2 ch · rate unknown · buffer size set by the driver"
        );
    }
}
