//! The null host: no devices, silent input, clocked by the caller. It drives
//! the engine exactly like a real-time device would (through the handle's
//! rings and the processor's callback path), so tests and headless CI can
//! exercise the live code path.

use ac_core::{ControlEvent, MidiEvent, MidiMsg};

use crate::engine::{Engine, EngineHandle, HostIo, Processor};

pub struct NullHost {
    handle: EngineHandle,
    processor: Processor,
    in_channels: usize,
    out_channels: usize,
    input: Vec<f32>,
    output: Vec<f32>,
    midi_out: Vec<MidiEvent>,
    /// Frames run so far, to stamp sent MIDI.
    frame: u64,
    sent: Vec<(u64, MidiMsg)>,
}

/// MIDI out messages a callback can carry.
const MIDI_OUT_CAPACITY: usize = 1024;

impl NullHost {
    pub fn new(engine: Engine, in_channels: usize, out_channels: usize) -> NullHost {
        let (handle, processor) = engine.split();
        NullHost {
            handle,
            processor,
            in_channels,
            out_channels: out_channels.max(1),
            input: Vec::new(),
            output: Vec::new(),
            midi_out: Vec::with_capacity(MIDI_OUT_CAPACITY),
            frame: 0,
            sent: Vec::new(),
        }
    }

    pub fn handle(&mut self) -> &mut EngineHandle {
        &mut self.handle
    }

    pub fn processor(&mut self) -> &mut Processor {
        &mut self.processor
    }

    /// Run one "device callback" of `frames` with silent input. Returns the
    /// channel-major output for this block.
    pub fn tick(&mut self, frames: usize) -> &[f32] {
        self.input.clear();
        self.input.resize(self.in_channels * frames, 0.0);
        let input = std::mem::take(&mut self.input);
        self.tick_with(frames, &input, &[]);
        self.input = input;
        &self.output
    }

    /// Run one callback with given channel-major input and MIDI.
    pub fn tick_with(&mut self, frames: usize, input: &[f32], midi: &[MidiEvent]) -> &[f32] {
        self.tick_events(frames, input, midi, &[])
    }

    /// Run one callback with given channel-major input, MIDI, and control
    /// events (keys, gamepads, OSC).
    pub fn tick_events(
        &mut self,
        frames: usize,
        input: &[f32],
        midi: &[MidiEvent],
        controls: &[ControlEvent],
    ) -> &[f32] {
        self.output.clear();
        self.output.resize(self.out_channels * frames, 0.0);
        let mut io = HostIo {
            frames,
            input,
            in_channels: if input.len() >= self.in_channels * frames {
                self.in_channels
            } else {
                0
            },
            output: &mut self.output,
            out_channels: self.out_channels,
            midi,
            midi_out: Some(&mut self.midi_out),
            controls,
            osc_out: None,
        };
        self.processor.process(&mut io);
        self.handle.collect_garbage();
        let frame = self.frame;
        self.sent.extend(
            self.midi_out
                .drain(..)
                .map(|e| (frame + u64::from(e.offset), e.msg)),
        );
        self.frame += frames as u64;
        &self.output
    }

    /// MIDI the patch sent to the devices so far, at absolute frames, and
    /// forget it.
    pub fn take_sent_midi(&mut self) -> Vec<(u64, MidiMsg)> {
        std::mem::take(&mut self.sent)
    }

    /// Run `total` frames in callbacks of `block` and collect each channel.
    pub fn run(&mut self, total: usize, block: usize) -> Vec<Vec<f32>> {
        let block = block.max(1);
        let mut chans = vec![Vec::with_capacity(total); self.out_channels];
        let mut done = 0;
        while done < total {
            let n = block.min(total - done);
            let out = self.tick(n).to_vec();
            for (c, ch) in chans.iter_mut().enumerate() {
                ch.extend_from_slice(&out[c * n..(c + 1) * n]);
            }
            done += n;
        }
        chans
    }
}
