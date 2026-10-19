//! Audio Constructor engine: compiler, scheduler, transport, real-time
//! processor, offline renderer, and null host.
//!
//! Builds for `wasm32-unknown-unknown`; never touches devices or files.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod bake;
mod bindings;
mod chains;
mod compile;
mod engine;
mod flatten;
mod graph;
mod internal;
mod loops;
mod null_host;
mod pool;
mod render;
mod report;
mod telemetry;

pub use bake::{BakeInput, BakeJob, BakeResult, BakeSpec, BakeStore, WholeState};
pub use bindings::BindingValues;
pub use compile::{CarryLayout, CompileOptions, Compiled, ParamMap, ParamTarget, compile};
pub use engine::{ClockFn, Engine, EngineHandle, EngineOptions, GraphEdit, HostIo, Processor};
pub use graph::CompiledGraph;
pub use null_host::NullHost;
pub use pool::{Job, MAX_WORKERS, RtPool};
pub use render::{
    RenderOptions, Rendered, RenderedFile, apply_overrides, render, required_output_channels,
};
pub use report::{CompileReport, CompileStats, Diagnostic, Severity};
pub use telemetry::{
    EngineStatus, NodeTelemetry, TelemetryLayout, TelemetryReader, TelemetryStore,
};
