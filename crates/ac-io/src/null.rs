//! Hosts without devices (ARCHITECTURE §11 "null host").
//!
//! - [`NullHost`] (from the engine) is clocked by the caller: tests tick it
//!   block by block and read the output.
//! - [`NullAudio`] is an [`AudioHost`] that runs the same device callback as
//!   the real backends on its own thread, paced to real time with
//!   `sleep`, with silent input. `audioc run --null` uses it for headless
//!   smoke tests; anything that drives an [`AudioStream`] can be tested
//!   against it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use ac_engine::NullHost;
use ac_engine::Processor;

use crate::latency::{Latency, LatencyError};
use crate::midi::MidiReceiver;
use crate::runtime::{
    AudioError, AudioHost, AudioStream, CallbackCore, CoreControl, CoreParts, Stopped,
    StreamCounters, StreamInfo, StreamStats,
};

/// When the pacing thread falls this many periods behind (a suspended
/// laptop, a debugger), it stops trying to catch up.
const MAX_LAG_PERIODS: u32 = 4;

/// A device-less audio host paced by a timer.
#[derive(Clone, Debug)]
pub struct NullAudio {
    info: StreamInfo,
}

impl NullAudio {
    pub fn new(
        sample_rate: u32,
        buffer_size: u32,
        in_channels: usize,
        out_channels: usize,
    ) -> NullAudio {
        NullAudio {
            info: StreamInfo {
                host: "null".into(),
                output_device: None,
                input_device: None,
                sample_rate: sample_rate.max(1),
                buffer_size: buffer_size.clamp(1, 1 << 16),
                in_channels,
                out_channels: out_channels.max(1),
                notes: Vec::new(),
            },
        }
    }
}

impl AudioHost for NullAudio {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn start(
        self: Box<Self>,
        processor: Processor,
        midi: Option<MidiReceiver>,
    ) -> Result<Box<dyn AudioStream>, AudioError> {
        let info = self.info;
        let counters = Arc::new(StreamCounters::default());
        let status = processor.status().clone();
        let (mut core, ctl) = CallbackCore::new(CoreParts {
            processor,
            counters: counters.clone(),
            sample_rate: info.sample_rate,
            in_channels: info.in_channels,
            out_channels: info.out_channels,
            input_ring: None,
            midi,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let frames = info.buffer_size as usize;
        let period =
            Duration::from_secs_f64(f64::from(info.buffer_size) / f64::from(info.sample_rate));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("ac-null-audio".into())
            .spawn(move || {
                let mut next = Instant::now();
                while !flag.load(Ordering::Acquire) {
                    core.run(frames, |_, _, _| {});
                    // Absolute deadlines so sleep overshoot doesn't add up.
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else if now - next > period * MAX_LAG_PERIODS {
                        status.device_xruns.fetch_add(1, Ordering::Relaxed);
                        next = now;
                    }
                }
                core
            })
            .map_err(|e| {
                AudioError::Backend(format!("couldn't start the null audio thread: {e}"))
            })?;
        Ok(Box::new(NullStream {
            info,
            counters,
            stop,
            thread: Some(thread),
            ctl,
        }))
    }
}

/// A running [`NullAudio`].
pub struct NullStream {
    info: StreamInfo,
    counters: Arc<StreamCounters>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<CallbackCore>>,
    ctl: CoreControl,
}

impl NullStream {
    fn join(&mut self) -> Option<CallbackCore> {
        self.stop.store(true, Ordering::Release);
        self.thread.take().and_then(|t| t.join().ok())
    }
}

impl AudioStream for NullStream {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn stats(&self) -> StreamStats {
        self.counters.snapshot()
    }

    fn start_latency(&mut self) -> Result<(), LatencyError> {
        Err(LatencyError::Unsupported)
    }

    fn poll_latency(&mut self) -> Option<Result<Latency, LatencyError>> {
        None
    }

    fn stop(mut self: Box<Self>) -> Option<Stopped> {
        let core = self.join()?;
        drop(core);
        self.ctl.returned.try_recv().ok()
    }
}

impl Drop for NullStream {
    fn drop(&mut self) {
        self.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{NoResources, Project, Registry};
    use ac_engine::{Engine, EngineOptions};

    fn engine(rate: u32) -> Engine {
        let (engine, _) = Engine::new(
            Project::default(),
            Arc::new(Registry::new()),
            Arc::new(NoResources),
            EngineOptions {
                sample_rate: f64::from(rate),
                max_block: 256,
                socket_telemetry: false,
            },
        );
        engine
    }

    #[test]
    fn runs_in_real_time_and_returns_the_processor() {
        let (mut handle, processor) = engine(48_000).split();
        let host = Box::new(NullAudio::new(48_000, 480, 2, 2));
        assert_eq!(host.info().max_block(), 480);
        let stream = host.start(processor, None).unwrap();
        handle.play();
        let t0 = Instant::now();
        std::thread::sleep(Duration::from_millis(300));
        handle.collect_garbage();
        let pos = handle.position();
        let elapsed = t0.elapsed().as_secs_f64();
        // Paced to the clock: roughly elapsed × rate, give or take a few
        // buffers (more slack below, for a busy test machine).
        let expect = elapsed * 48_000.0;
        assert!(
            (pos as f64) > expect - 9_600.0 && (pos as f64) < expect + 2_400.0,
            "{pos} frames after {elapsed:.3} s"
        );
        let stats = stream.stats();
        assert!(stats.callbacks >= 20, "{stats:?}");
        assert_eq!(stats.max_callback_frames, 480);
        assert!(handle.status().load_fraction() >= 0.0);
        let mut back = stream.stop().expect("processor returned").processor;
        // The returned processor still belongs to this engine.
        let mut out = vec![0.0; 2 * 64];
        let mut io = ac_engine::HostIo {
            frames: 64,
            input: &[],
            in_channels: 0,
            output: &mut out,
            out_channels: 2,
            midi: &[],
        };
        back.process(&mut io);
        assert!(handle.position() >= pos);
    }

    #[test]
    fn graphs_hot_swap_and_params_apply_while_running() {
        use ac_core::{Endpoint, NodeId, NodeInstance, NodePath, Value, Wire};
        use ac_engine::GraphEdit;

        let mut project = Project::default();
        project
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "audio_in"));
        project
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(2), "audio_out"));
        project.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(2), "in"),
        ));
        let (engine, report) = Engine::new(
            project.clone(),
            Arc::new(ac_nodes::registry()),
            Arc::new(NoResources),
            EngineOptions {
                sample_rate: 48_000.0,
                max_block: 128,
                socket_telemetry: true,
            },
        );
        assert!(!report.has_errors(), "{report}");
        let (mut handle, processor) = engine.split();
        let stream = Box::new(NullAudio::new(48_000, 128, 2, 2))
            .start(processor, None)
            .unwrap();
        handle.play();

        // Insert a Gain between in and out while the stream runs.
        project.graph.wires.clear();
        project
            .graph
            .nodes
            .push(NodeInstance::new(NodeId(3), "gain"));
        project.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(3), "in"),
        ));
        project.graph.wires.push(Wire::new(
            Endpoint::new(NodeId(3), "out"),
            Endpoint::new(NodeId(2), "in"),
        ));
        let report = handle.edit(GraphEdit::Replace(Box::new(project)));
        assert!(!report.has_errors(), "{report}");
        let t0 = Instant::now();
        while handle.status().generation.load(Ordering::Relaxed) != 1 {
            assert!(
                t0.elapsed() < Duration::from_secs(2),
                "graph never swapped in"
            );
            handle.collect_garbage();
            std::thread::sleep(Duration::from_millis(5));
        }
        let gain = NodePath::root(NodeId(3));
        assert!(handle.set_param(&gain, "gain", Value::Float(-6.0)));
        std::thread::sleep(Duration::from_millis(50));
        handle.collect_garbage();
        assert!(handle.position() > 0);
        assert!(stream.stop().is_some());
    }

    #[test]
    fn latency_is_unsupported() {
        let (_handle, processor) = engine(8_000).split();
        let mut stream = Box::new(NullAudio::new(8_000, 64, 1, 1))
            .start(processor, None)
            .unwrap();
        assert_eq!(stream.start_latency(), Err(LatencyError::Unsupported));
        assert_eq!(
            stream.measure_latency(Duration::from_millis(10)),
            Err(LatencyError::Unsupported)
        );
    }
}
