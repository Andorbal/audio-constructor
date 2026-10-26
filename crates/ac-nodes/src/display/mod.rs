//! Display nodes: they publish telemetry for the editor to draw and, where
//! they have an output, pass their input through unchanged.

use ac_core::{AudioIn, AudioOut, Registry};

pub mod analyzer;
pub mod light;
pub mod meter;
pub mod midi_monitor;
pub mod readout;
pub mod scope;
pub mod spectrogram;
pub mod tuner;
pub mod waveform;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<meter::Meter>();
    r.register::<scope::Scope>();
    r.register::<readout::Readout>();
    r.register::<analyzer::Analyzer>();
    r.register::<spectrogram::Spectrogram>();
    r.register::<light::Light>();
    r.register::<waveform::Waveform>();
    r.register::<midi_monitor::MidiMonitor>();
    r.register::<tuner::Tuner>();
}

/// Copy every channel of `input` to `out` bit for bit, so a display node can
/// sit inline without changing the signal.
pub(crate) fn pass_through(input: &AudioIn, out: &mut AudioOut) {
    for (c, dst) in out.iter_mut().enumerate() {
        let src = input.channel(c);
        let (head, tail) = dst.split_at_mut(src.len().min(dst.len()));
        for (d, s) in head.iter_mut().zip(src) {
            *d = *s;
        }
        tail.fill(0.0);
    }
}
