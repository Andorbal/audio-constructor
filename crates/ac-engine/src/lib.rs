//! Audio Constructor engine: compiler, scheduler, transport, real-time
//! processor, offline renderer, and null host.
//!
//! Builds for `wasm32-unknown-unknown`; never touches devices or files.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod compile;
mod engine;
mod flatten;
mod graph;
mod null_host;
mod render;
mod report;
mod telemetry;

pub use compile::{CarryLayout, CompileOptions, Compiled, ParamMap, ParamTarget, compile};
pub use engine::{ClockFn, Engine, EngineHandle, EngineOptions, GraphEdit, HostIo, Processor};
pub use graph::CompiledGraph;
pub use null_host::NullHost;
pub use render::{RenderOptions, Rendered, apply_overrides, render, required_output_channels};
pub use report::{CompileReport, CompileStats, Diagnostic, Severity};
pub use telemetry::{
    EngineStatus, NodeTelemetry, TelemetryLayout, TelemetryReader, TelemetryStore,
};
