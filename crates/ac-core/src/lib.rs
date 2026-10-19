//! Audio Constructor core: the graph model, signal types, the node trait and
//! registry, musical time, and the project file format.
//!
//! This crate has no I/O and builds for `wasm32-unknown-unknown`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod binding;
pub mod block;
pub mod clipboard;
pub mod control;
pub mod events;
pub mod graph;
pub mod group;
pub mod id;
pub mod io;
pub mod node;
pub mod panel;
pub mod param;
pub mod project;
pub mod registry;
pub mod signal;
pub mod spectrum;
pub mod testing;
pub mod text;
pub mod time;
pub mod util;
pub mod value;
pub mod whole;

pub use binding::{
    Binding, Effect, GateMode, InputEvent, Learner, Mapping, Reading, Source, SourceKey, Target,
    TransportAction,
};
pub use block::{BLOCK_EXTENSION, Block, BlockSource, Origin, UpdateReport, slug, update_def};
pub use clipboard::Subgraph;
pub use control::{
    CONTROL_CAPACITY, Control, ControlEvent, GamepadAxis, GamepadButton, Key, MACRO_COUNT,
    OSC_OUT_CAPACITY, OscMessage, osc_address_hash,
};
pub use events::{MIDI_CAPACITY, MidiEvent, MidiMsg, note_to_hz};
pub use graph::{
    Endpoint, Frame, GROUP_KIND, Graph, NodeInstance, NodeUi, REROUTE_KIND, VOICE_KIND,
    VOICES_KIND, Wire,
};
pub use group::{GroupDef, GroupInput, GroupOutput};
pub use id::{GroupId, IdGen, NodeId, NodePath};
pub use io::{
    AudioIn, AudioOut, AutoFlags, BoolIn, BufferRef, FloatIn, FloatOut, FloatSrc, HostAccess,
    InputSlot, Inputs, IntSrc, Io, LOCAL, Local, MidiOut, OutputSlot, Outputs, ParamSlot,
    TelemetryAccess, TextSrc, TriggerOut, TriggerSrc,
};
pub use node::{
    AudioData, Category, Causality, Determinism, EngineInfo, FileRequest, NoResources, Node,
    NodeMeta, Phase, PrepareCtx, ProcessCtx, RecordRequest, RecordSink, Resources,
};
pub use panel::{Panel, PanelItem};
pub use param::{param_text, param_value};
pub use project::{
    LoadError, Project, SCHEMA_VERSION, Settings, TempoSegment, View, referenced_assets,
};
pub use registry::{
    NodeEntry, Registry, RenderWholeFn, fuzzy_score, group_signature, voices_signature,
};
pub use signal::{
    AutoTrigger, Channels, Conversion, OutRate, Rate, Scale, SignalType, Signature, SocketSpec,
    SpectrumShape, Unit, conversion,
};
pub use spectrum::SpectrumBuf;
pub use text::{TEXT_MAX, TextBuf};
pub use time::{TempoMap, TransportView};
pub use value::{Config, ConfigExt, Value, empty_config};
pub use whole::{WholeCtx, WholeInputs, WholeOutputs, WholeValue};
