//! Relaying the control half's messages to a processor in another address
//! space (DECISIONS D150).
//!
//! The web build runs the processor inside an audio worklet, a separate
//! wasm instance that shares no memory with the page. The page keeps a
//! *mirror* engine compiled from the same project: the editor talks to the
//! mirror's [`EngineHandle`] exactly as it talks to a local one, the
//! mirror's [`Processor`] hands everything it was sent to
//! [`Processor::relay`] instead of running it, and the worklet's own
//! engine, compiled from the same project the same way, applies those
//! messages with [`EngineHandle::apply_relayed`]. Graphs never cross: the
//! project does, and each side compiles it. Parameter slots are a function
//! of the project, so a slot the mirror resolved names the same input on
//! the other side.

use super::*;

/// A message from an engine's control half to its processor, in a form that
/// crosses to another address space. Fixed-size and `Copy`, so relaying
/// never allocates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Relayed {
    Play,
    Stop,
    Seek(u64),
    Locate(u64),
    SetRecording(bool),
    SetLoop(Option<(u64, u64)>),
    SetPunch(Option<(u64, u64)>),
    SetMetronome(bool),
    SetCountIn(u32),
    Reset,
    Panic,
    /// A parameter change for the graph of `generation`, already converted
    /// to the slot's value.
    Param {
        generation: u64,
        slot: u32,
        value: f32,
    },
    /// An inline trigger button pressed for the graph of `generation`.
    Fire {
        generation: u64,
        slot: u32,
    },
}

impl From<Command> for Relayed {
    fn from(c: Command) -> Relayed {
        match c {
            Command::Play => Relayed::Play,
            Command::Stop => Relayed::Stop,
            Command::Seek(p) => Relayed::Seek(p),
            Command::Locate(p) => Relayed::Locate(p),
            Command::SetRecording(on) => Relayed::SetRecording(on),
            Command::SetLoop(r) => Relayed::SetLoop(r),
            Command::SetPunch(r) => Relayed::SetPunch(r),
            Command::SetMetronome(on) => Relayed::SetMetronome(on),
            Command::SetCountIn(bars) => Relayed::SetCountIn(bars),
            Command::Reset => Relayed::Reset,
            Command::Panic => Relayed::Panic,
        }
    }
}

impl From<ParamMsg> for Relayed {
    fn from(m: ParamMsg) -> Relayed {
        if m.fire {
            Relayed::Fire {
                generation: m.generation,
                slot: m.slot,
            }
        } else {
            Relayed::Param {
                generation: m.generation,
                slot: m.slot,
                value: m.value,
            }
        }
    }
}

impl Processor {
    /// Act as a mirror: take everything the handle sent since the last call
    /// and hand it to `out`, without processing audio. Transport commands
    /// come first, then new graphs are installed (as a block's start would
    /// install them), then parameter changes: the order a block applies
    /// them in. While the editor learns a binding (the input monitor), the
    /// `midi` and `controls` the real processor is given are copied for
    /// [`EngineHandle::take_input_events`], so learning works on this side.
    /// Returns the generation of the graph now installed. Real-time safe.
    pub fn relay(
        &mut self,
        midi: &[MidiEvent],
        controls: &[ControlEvent],
        out: &mut impl FnMut(Relayed),
    ) -> u64 {
        while let Ok(c) = self.cmd_rx.pop() {
            command(&mut self.transport, c);
            out(c.into());
        }
        self.install_graphs();
        while let Ok(m) = self.param_rx.pop() {
            out(m.into());
        }
        if self.status.input_monitor.load(Ordering::Relaxed) {
            for e in midi {
                let _ = self.learn_tx.push(InputEvent::Midi(e.msg));
            }
            for e in controls {
                let _ = self.learn_tx.push(InputEvent::Control(e.control, e.value));
            }
        }
        self.graph.generation
    }
}

impl EngineHandle {
    /// The generation of the graph this handle compiled last.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The bake this handle holds for the whole-input node at `path`, if
    /// any (current or stale): what a mirror sends along so the other
    /// engine needn't bake it again ([`EngineHandle::adopt_bake`] there).
    pub fn bake_result(&self, path: &NodePath) -> Option<&BakeResult> {
        self.bakes.get(path)
    }

    /// Use a bake a mirror engine made for the whole-input node at `path`
    /// of the project this handle compiled last, as if this handle had
    /// baked it ([`EngineHandle::finish_bake`]). A bake's key hashes this
    /// handle's own history (how often its resources were replaced), so
    /// the result takes the key of the node's current spec here; it's only
    /// meaningful when the mirror compiled the same project. Returns
    /// whether a node at `path` was waiting for it (and so recompiled).
    pub fn adopt_bake(&mut self, path: &NodePath, outputs: Arc<ac_core::WholeOutputs>) -> bool {
        let Some(spec) = self.bake_specs.iter().find(|s| &s.path == path) else {
            return false;
        };
        if spec.state == WholeState::Baked || matches!(spec.state, WholeState::Disabled { .. }) {
            return false;
        }
        let key = spec.key;
        self.finish_bake(BakeResult {
            path: path.clone(),
            key,
            outputs,
        })
    }

    /// Apply a message a mirror engine relayed ([`Processor::relay`]).
    /// Transport commands act as if they were called here. Parameter
    /// changes go to the slot in the graph this handle compiled last, so
    /// they only mean something when the mirror compiled the same project:
    /// `mirror_generation` is the mirror's generation of the project this
    /// handle compiled last, and changes for any other generation are
    /// dropped, as a processor drops changes meant for another graph.
    /// Returns whether the message was queued. Real-time safe: nothing
    /// allocates.
    pub fn apply_relayed(&mut self, m: Relayed, mirror_generation: u64) -> bool {
        let (generation, slot, value, fire) = match m {
            Relayed::Play => return self.queue(Command::Play),
            Relayed::Stop => return self.queue(Command::Stop),
            Relayed::Seek(p) => return self.queue(Command::Seek(p)),
            Relayed::Locate(p) => return self.queue(Command::Locate(p)),
            Relayed::SetRecording(on) => return self.queue(Command::SetRecording(on)),
            Relayed::SetLoop(r) => return self.queue(Command::SetLoop(r)),
            Relayed::SetPunch(r) => return self.queue(Command::SetPunch(r)),
            Relayed::SetMetronome(on) => return self.queue(Command::SetMetronome(on)),
            Relayed::SetCountIn(bars) => return self.queue(Command::SetCountIn(bars)),
            Relayed::Reset => return self.queue(Command::Reset),
            Relayed::Panic => return self.queue(Command::Panic),
            Relayed::Param {
                generation,
                slot,
                value,
            } => (generation, slot, value, false),
            Relayed::Fire { generation, slot } => (generation, slot, 0.0, true),
        };
        // A compiled graph still waiting for room in the ring has slots the
        // processor isn't running yet.
        if generation != mirror_generation || self.pending.is_some() {
            return false;
        }
        let msg = ParamMsg {
            generation: self.generation,
            slot,
            value,
            fire,
        };
        if self.param_tx.push(msg).is_err() {
            self.status.dropped_messages.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn queue(&mut self, c: Command) -> bool {
        if self.cmd_tx.push(c).is_err() {
            self.status.dropped_messages.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        true
    }
}
