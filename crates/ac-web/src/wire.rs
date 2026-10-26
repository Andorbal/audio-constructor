//! What crosses between the page and the audio worklet (DECISIONS D150,
//! D151), and how it's written down.
//!
//! Small, frequent messages are ring records ([`crate::ring`]):
//! [`ToAudio`] from the page (relayed transport commands and parameter
//! changes, MIDI and control events, barriers) and [`FromAudio`] back
//! (MIDI for the page's outputs). They're fixed-size, encode into a stack
//! buffer, and decode without allocating, so the worklet handles them
//! inside `process()`.
//!
//! Big, rare ones are [`Posted`] messages, sent with `postMessage` and
//! handled in the worklet's message handler, between render quanta: a
//! project to compile, the decoded audio files it plays. A posted project
//! has a sequence number, and the page puts a [`ToAudio::Barrier`] with
//! that number in the ring when it posts it: the worklet takes nothing past
//! the barrier until it has compiled that project, so a parameter change
//! the page relays after an edit always reaches the graph it was meant for.
//!
//! Everything is little-endian. A message that doesn't decode is dropped,
//! never trusted.

use ac_core::{
    AudioData, Control, GamepadAxis, GamepadButton, Key, MidiMsg, WholeOutputs, WholeValue,
};
use ac_engine::Relayed;

/// A ring record from the page to the worklet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToAudio {
    /// A transport command or parameter change the mirror engine relayed.
    Relayed(Relayed),
    /// MIDI from an input port (or the page's own keyboard).
    Midi(MidiMsg),
    /// A computer key, gamepad control, or other control event.
    Control(Control, f32),
    /// Take nothing past this until project `seq` is compiled.
    Barrier(u32),
}

/// A ring record from the worklet to the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FromAudio {
    /// MIDI Out nodes sent this.
    MidiOut(MidiMsg),
}

/// The longest encoding of a [`ToAudio`] or [`FromAudio`].
pub const RECORD_MAX: usize = 32;

struct Writer<'a> {
    out: &'a mut [u8],
    at: usize,
}

impl Writer<'_> {
    fn u8(&mut self, v: u8) {
        if let Some(b) = self.out.get_mut(self.at) {
            *b = v;
        }
        self.at += 1;
    }
    fn bytes(&mut self, v: &[u8]) {
        for b in v {
            self.u8(*b);
        }
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
}

/// Reads fields off the front of a message; any read past the end makes
/// the whole decode fail.
struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.data.len() < n {
            return None;
        }
        let (head, rest) = self.data.split_at(n);
        self.data = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Option<u64> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Some(u64::from_le_bytes(a))
    }
    fn f32(&mut self) -> Option<f32> {
        self.u32().map(f32::from_bits)
    }
    fn len(&mut self) -> Option<usize> {
        self.u32().map(|n| n as usize)
    }
    fn done(&self) -> bool {
        self.data.is_empty()
    }
}

fn midi(w: &mut Writer, m: &MidiMsg) {
    let (b, n) = m.to_bytes();
    w.u8(n as u8);
    w.bytes(&b);
}

fn read_midi(r: &mut Reader) -> Option<MidiMsg> {
    let n = usize::from(r.u8()?);
    let b = r.take(3)?;
    MidiMsg::from_bytes(b.get(..n)?)
}

fn button_index(b: GamepadButton) -> u8 {
    GamepadButton::ALL.iter().position(|x| *x == b).unwrap_or(0) as u8
}

fn axis_index(a: GamepadAxis) -> u8 {
    GamepadAxis::ALL.iter().position(|x| *x == a).unwrap_or(0) as u8
}

fn control(w: &mut Writer, c: &Control) {
    match *c {
        Control::Key(k) => {
            w.u8(0);
            w.u8(Key::all().position(|x| x == k).unwrap_or(0) as u8);
        }
        Control::GamepadButton { pad, button } => {
            w.u8(1);
            w.u8(pad);
            w.u8(button_index(button));
        }
        Control::GamepadAxis { pad, axis } => {
            w.u8(2);
            w.u8(pad);
            w.u8(axis_index(axis));
        }
        Control::Osc { address, arg } => {
            w.u8(3);
            w.u64(address);
            w.u8(arg);
        }
        Control::Macro { index } => {
            w.u8(4);
            w.u8(index);
        }
    }
}

fn read_control(r: &mut Reader) -> Option<Control> {
    Some(match r.u8()? {
        0 => Control::Key(Key::all().nth(usize::from(r.u8()?))?),
        1 => Control::GamepadButton {
            pad: r.u8()?,
            button: *GamepadButton::ALL.get(usize::from(r.u8()?))?,
        },
        2 => Control::GamepadAxis {
            pad: r.u8()?,
            axis: *GamepadAxis::ALL.get(usize::from(r.u8()?))?,
        },
        3 => Control::Osc {
            address: r.u64()?,
            arg: r.u8()?,
        },
        4 => Control::Macro { index: r.u8()? },
        _ => return None,
    })
}

fn relayed(w: &mut Writer, m: &Relayed) {
    match *m {
        Relayed::Play => w.u8(0),
        Relayed::Stop => w.u8(1),
        Relayed::Seek(p) => {
            w.u8(2);
            w.u64(p);
        }
        Relayed::SetRecording(on) => {
            w.u8(3);
            w.u8(u8::from(on));
        }
        Relayed::SetLoop(None) => w.u8(4),
        Relayed::SetLoop(Some((a, b))) => {
            w.u8(5);
            w.u64(a);
            w.u64(b);
        }
        Relayed::Reset => w.u8(6),
        Relayed::Param {
            generation,
            slot,
            value,
        } => {
            w.u8(7);
            w.u64(generation);
            w.u32(slot);
            w.f32(value);
        }
        Relayed::Fire { generation, slot } => {
            w.u8(8);
            w.u64(generation);
            w.u32(slot);
        }
        Relayed::Panic => w.u8(9),
    }
}

fn read_relayed(r: &mut Reader) -> Option<Relayed> {
    Some(match r.u8()? {
        0 => Relayed::Play,
        1 => Relayed::Stop,
        2 => Relayed::Seek(r.u64()?),
        3 => Relayed::SetRecording(r.u8()? != 0),
        4 => Relayed::SetLoop(None),
        5 => Relayed::SetLoop(Some((r.u64()?, r.u64()?))),
        6 => Relayed::Reset,
        7 => Relayed::Param {
            generation: r.u64()?,
            slot: r.u32()?,
            value: r.f32()?,
        },
        8 => Relayed::Fire {
            generation: r.u64()?,
            slot: r.u32()?,
        },
        9 => Relayed::Panic,
        _ => return None,
    })
}

impl ToAudio {
    /// Write the record into `out` and return its length.
    pub fn encode(&self, out: &mut [u8; RECORD_MAX]) -> usize {
        let mut w = Writer { out, at: 0 };
        match self {
            ToAudio::Relayed(m) => {
                w.u8(0);
                relayed(&mut w, m);
            }
            ToAudio::Midi(m) => {
                w.u8(1);
                midi(&mut w, m);
            }
            ToAudio::Control(c, v) => {
                w.u8(2);
                control(&mut w, c);
                w.f32(*v);
            }
            ToAudio::Barrier(seq) => {
                w.u8(3);
                w.u32(*seq);
            }
        }
        w.at
    }

    pub fn decode(data: &[u8]) -> Option<ToAudio> {
        let mut r = Reader { data };
        let m = match r.u8()? {
            0 => ToAudio::Relayed(read_relayed(&mut r)?),
            1 => ToAudio::Midi(read_midi(&mut r)?),
            2 => ToAudio::Control(read_control(&mut r)?, r.f32()?),
            3 => ToAudio::Barrier(r.u32()?),
            _ => return None,
        };
        r.done().then_some(m)
    }
}

impl FromAudio {
    /// Write the record into `out` and return its length.
    pub fn encode(&self, out: &mut [u8; RECORD_MAX]) -> usize {
        let mut w = Writer { out, at: 0 };
        match self {
            FromAudio::MidiOut(m) => {
                w.u8(0);
                midi(&mut w, m);
            }
        }
        w.at
    }

    pub fn decode(data: &[u8]) -> Option<FromAudio> {
        let mut r = Reader { data };
        let m = match r.u8()? {
            0 => FromAudio::MidiOut(read_midi(&mut r)?),
            _ => return None,
        };
        r.done().then_some(m)
    }
}

/// A message the page posts to the worklet, handled outside `process()`.
#[derive(Clone, Debug, PartialEq)]
pub enum Posted {
    /// Compile this project. `generation` is the mirror's generation of it
    /// (the tag on parameter changes for it), `seq` the number its barrier
    /// carries, and `assets` the revision of the audio files it plays.
    Project {
        seq: u32,
        generation: u64,
        assets: u32,
        json: String,
    },
    /// Every audio file the project plays, decoded, as revision
    /// `revision`: `(path as node configs write it, audio)`.
    Assets {
        revision: u32,
        files: Vec<(String, AudioData)>,
    },
    /// Drop graphs the processor has finished with (sent now and then, so
    /// freeing never happens in `process()`).
    Collect,
    /// The page's mirror baked the whole-input node at `path` (an engine
    /// path; `key` is the mirror's): the worklet uses the result instead
    /// of baking on the audio thread.
    Bake {
        path: String,
        key: u64,
        outputs: WholeOutputs,
    },
}

const POSTED_MAGIC: u32 = 0x4143_5731; // "ACW1"

impl Posted {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&POSTED_MAGIC.to_le_bytes());
        let put = |out: &mut Vec<u8>, b: &[u8]| out.extend_from_slice(b);
        match self {
            Posted::Project {
                seq,
                generation,
                assets,
                json,
            } => {
                put(&mut out, &[0]);
                put(&mut out, &seq.to_le_bytes());
                put(&mut out, &generation.to_le_bytes());
                put(&mut out, &assets.to_le_bytes());
                put(&mut out, &(json.len() as u32).to_le_bytes());
                put(&mut out, json.as_bytes());
            }
            Posted::Assets { revision, files } => {
                put(&mut out, &[1]);
                put(&mut out, &revision.to_le_bytes());
                put(&mut out, &(files.len() as u32).to_le_bytes());
                for (path, audio) in files {
                    put(&mut out, &(path.len() as u32).to_le_bytes());
                    put(&mut out, path.as_bytes());
                    put(&mut out, &audio.sample_rate.to_le_bytes());
                    put(&mut out, &(audio.channels.len() as u32).to_le_bytes());
                    put(&mut out, &(audio.frames() as u32).to_le_bytes());
                    for ch in &audio.channels {
                        for i in 0..audio.frames() {
                            let v = ch.get(i).copied().unwrap_or(0.0);
                            put(&mut out, &v.to_le_bytes());
                        }
                    }
                }
            }
            Posted::Collect => put(&mut out, &[2]),
            Posted::Bake { path, key, outputs } => {
                put(&mut out, &[3]);
                put(&mut out, &(path.len() as u32).to_le_bytes());
                put(&mut out, path.as_bytes());
                put(&mut out, &key.to_le_bytes());
                put(&mut out, &(outputs.values.len() as u32).to_le_bytes());
                for v in &outputs.values {
                    whole_value(&mut out, v);
                }
            }
        }
        out
    }

    pub fn decode(data: &[u8]) -> Result<Posted, String> {
        let bad = || "a message the audio side can't read".to_string();
        let mut r = Reader { data };
        if r.u32() != Some(POSTED_MAGIC) {
            return Err(bad());
        }
        let m = match r.u8().ok_or_else(bad)? {
            0 => {
                let seq = r.u32().ok_or_else(bad)?;
                let generation = r.u64().ok_or_else(bad)?;
                let assets = r.u32().ok_or_else(bad)?;
                let n = r.len().ok_or_else(bad)?;
                let text = r.take(n).ok_or_else(bad)?;
                let json = String::from_utf8(text.to_vec()).map_err(|_| bad())?;
                Posted::Project {
                    seq,
                    generation,
                    assets,
                    json,
                }
            }
            1 => {
                let revision = r.u32().ok_or_else(bad)?;
                let count = r.len().ok_or_else(bad)?;
                let mut files = Vec::new();
                for _ in 0..count {
                    let n = r.len().ok_or_else(bad)?;
                    let path = String::from_utf8(r.take(n).ok_or_else(bad)?.to_vec())
                        .map_err(|_| bad())?;
                    let sample_rate = r.u32().ok_or_else(bad)?;
                    let channels = r.len().ok_or_else(bad)?;
                    let frames = r.len().ok_or_else(bad)?;
                    // Check the size before allocating for it.
                    let bytes = channels
                        .checked_mul(frames)
                        .and_then(|n| n.checked_mul(4))
                        .ok_or_else(bad)?;
                    let raw = r.take(bytes).ok_or_else(bad)?;
                    let mut chans = Vec::with_capacity(channels);
                    for c in 0..channels {
                        let part = &raw[c * frames * 4..(c + 1) * frames * 4];
                        chans.push(
                            part.chunks_exact(4)
                                .map(|q| f32::from_le_bytes([q[0], q[1], q[2], q[3]]))
                                .collect(),
                        );
                    }
                    files.push((
                        path,
                        AudioData {
                            sample_rate,
                            channels: chans,
                        },
                    ));
                }
                Posted::Assets { revision, files }
            }
            2 => Posted::Collect,
            3 => {
                let n = r.len().ok_or_else(bad)?;
                let path =
                    String::from_utf8(r.take(n).ok_or_else(bad)?.to_vec()).map_err(|_| bad())?;
                let key = r.u64().ok_or_else(bad)?;
                let count = r.len().ok_or_else(bad)?;
                let mut values = Vec::new();
                for _ in 0..count {
                    values.push(read_whole_value(&mut r).ok_or_else(bad)?);
                }
                Posted::Bake {
                    path,
                    key,
                    outputs: WholeOutputs { values },
                }
            }
            _ => return Err(bad()),
        };
        if r.done() { Ok(m) } else { Err(bad()) }
    }
}

fn whole_value(out: &mut Vec<u8>, v: &WholeValue) {
    let n32 = |out: &mut Vec<u8>, n: usize| out.extend_from_slice(&(n as u32).to_le_bytes());
    match v {
        WholeValue::None => out.push(0),
        WholeValue::Audio(chans) => {
            out.push(1);
            n32(out, chans.len());
            for ch in chans {
                n32(out, ch.len());
                for x in ch {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
        WholeValue::Float(xs) => {
            out.push(2);
            n32(out, xs.len());
            for x in xs {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        WholeValue::Int(xs) => {
            out.push(3);
            n32(out, xs.len());
            for x in xs {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        WholeValue::Triggers(xs) => {
            out.push(4);
            n32(out, xs.len());
            for x in xs {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        WholeValue::Midi(ms) => {
            out.push(5);
            n32(out, ms.len());
            for (at, m) in ms {
                out.extend_from_slice(&at.to_le_bytes());
                let (b, n) = m.to_bytes();
                out.push(n as u8);
                out.extend_from_slice(&b);
            }
        }
    }
}

/// A list of `count` fixed-size items, checked against what's left before
/// anything is allocated for it.
fn read_list<T>(
    r: &mut Reader,
    size: usize,
    mut item: impl FnMut(&mut Reader) -> Option<T>,
) -> Option<Vec<T>> {
    let n = r.len()?;
    if n.checked_mul(size)? > r.data.len() {
        return None;
    }
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(item(r)?);
    }
    Some(v)
}

fn read_whole_value(r: &mut Reader) -> Option<WholeValue> {
    Some(match r.u8()? {
        0 => WholeValue::None,
        1 => WholeValue::Audio(read_list(r, 4, |r| read_list(r, 4, |r| r.f32()))?),
        2 => WholeValue::Float(read_list(r, 4, |r| r.f32())?),
        3 => WholeValue::Int(read_list(r, 4, |r| r.u32().map(|v| v as i32))?),
        4 => WholeValue::Triggers(read_list(r, 8, |r| r.u64())?),
        5 => WholeValue::Midi(read_list(r, 12, |r| Some((r.u64()?, read_midi(r)?)))?),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records() -> Vec<ToAudio> {
        let mut v = vec![
            ToAudio::Relayed(Relayed::Play),
            ToAudio::Relayed(Relayed::Stop),
            ToAudio::Relayed(Relayed::Seek(u64::MAX - 3)),
            ToAudio::Relayed(Relayed::SetRecording(true)),
            ToAudio::Relayed(Relayed::SetLoop(None)),
            ToAudio::Relayed(Relayed::SetLoop(Some((480, 96_000)))),
            ToAudio::Relayed(Relayed::Reset),
            ToAudio::Relayed(Relayed::Panic),
            ToAudio::Relayed(Relayed::Param {
                generation: 41,
                slot: 9,
                value: -0.25,
            }),
            ToAudio::Relayed(Relayed::Fire {
                generation: 3,
                slot: 70_000,
            }),
            ToAudio::Midi(MidiMsg::NoteOn {
                channel: 15,
                note: 60,
                velocity: 100,
            }),
            ToAudio::Midi(MidiMsg::ProgramChange {
                channel: 2,
                program: 7,
            }),
            ToAudio::Midi(MidiMsg::PitchBend {
                channel: 0,
                value: -8192,
            }),
            ToAudio::Control(
                Control::Osc {
                    address: ac_core::osc_address_hash("/fader/1"),
                    arg: 2,
                },
                0.5,
            ),
            ToAudio::Control(Control::Macro { index: 15 }, 1.0),
            ToAudio::Barrier(12),
        ];
        for k in Key::all() {
            v.push(ToAudio::Control(Control::Key(k), 1.0));
        }
        for button in GamepadButton::ALL {
            v.push(ToAudio::Control(
                Control::GamepadButton { pad: 3, button },
                0.75,
            ));
        }
        for axis in GamepadAxis::ALL {
            v.push(ToAudio::Control(
                Control::GamepadAxis { pad: 1, axis },
                -1.0,
            ));
        }
        v
    }

    #[test]
    fn ring_records_round_trip() {
        let mut buf = [0u8; RECORD_MAX];
        for m in records() {
            let n = m.encode(&mut buf);
            assert!(n <= RECORD_MAX);
            assert_eq!(ToAudio::decode(&buf[..n]), Some(m), "{m:?}");
            // Cut short, or with a byte too many, it's refused.
            assert_eq!(ToAudio::decode(&buf[..n - 1]), None, "{m:?}");
            let mut long = buf[..n].to_vec();
            long.push(0);
            assert_eq!(ToAudio::decode(&long), None, "{m:?}");
        }
        let out = FromAudio::MidiOut(MidiMsg::ControlChange {
            channel: 1,
            controller: 74,
            value: 127,
        });
        let n = out.encode(&mut buf);
        assert_eq!(FromAudio::decode(&buf[..n]), Some(out));
        assert_eq!(FromAudio::decode(&[9]), None);
        assert_eq!(ToAudio::decode(&[]), None);
        assert_eq!(ToAudio::decode(&[200, 1, 2]), None);
    }

    #[test]
    fn posted_messages_round_trip() {
        let project = Posted::Project {
            seq: 4,
            generation: 1 << 40,
            assets: 2,
            json: "{\"name\": \"Ünïcode\"}".into(),
        };
        assert_eq!(Posted::decode(&project.encode()), Ok(project));
        let assets = Posted::Assets {
            revision: 7,
            files: vec![
                (
                    "assets/drums.wav".into(),
                    AudioData {
                        sample_rate: 44_100,
                        channels: vec![vec![0.5, -0.5, 0.25], vec![1.0, 0.0, -1.0]],
                    },
                ),
                (
                    "empty.wav".into(),
                    AudioData {
                        sample_rate: 48_000,
                        channels: vec![],
                    },
                ),
            ],
        };
        assert_eq!(Posted::decode(&assets.encode()), Ok(assets));
        assert_eq!(
            Posted::decode(&Posted::Collect.encode()),
            Ok(Posted::Collect)
        );
        let bake = Posted::Bake {
            path: "5/3".into(),
            key: 0xfeed_beef_0000_0001,
            outputs: WholeOutputs {
                values: vec![
                    WholeValue::None,
                    WholeValue::Audio(vec![vec![0.5, -0.25], vec![1.0, 0.0]]),
                    WholeValue::Float(vec![0.1, 0.2, 0.3]),
                    WholeValue::Int(vec![-3, 7]),
                    WholeValue::Triggers(vec![0, 480, 96_000]),
                    WholeValue::Midi(vec![(
                        12,
                        MidiMsg::NoteOn {
                            channel: 1,
                            note: 64,
                            velocity: 99,
                        },
                    )]),
                ],
            },
        };
        assert_eq!(Posted::decode(&bake.encode()), Ok(bake.clone()));
        let bytes = bake.encode();
        for n in 0..bytes.len() {
            assert!(Posted::decode(&bytes[..n]).is_err(), "cut at {n}");
        }
    }

    #[test]
    fn broken_posted_messages_are_refused_without_panicking() {
        let good = Posted::Assets {
            revision: 1,
            files: vec![(
                "a.wav".into(),
                AudioData {
                    sample_rate: 8_000,
                    channels: vec![vec![0.1; 16]],
                },
            )],
        }
        .encode();
        for n in 0..good.len() {
            assert!(Posted::decode(&good[..n]).is_err(), "cut at {n}");
        }
        assert!(Posted::decode(b"nonsense").is_err());
        // A size that would overflow, or ask for far more than was sent.
        let mut huge = good.clone();
        let frames_at = 4 + 1 + 4 + 4 + 4 + 5 + 4 + 4;
        huge[frames_at..frames_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Posted::decode(&huge).is_err());
    }
}
