//! Relaying a mirror engine's messages to an engine in another address
//! space (the web build's audio worklet, DECISIONS D150): the remote engine
//! plays exactly what a local one would, and the mirror shows the remote
//! engine's telemetry.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use ac_core::*;
use ac_engine::*;
use common::{P, path};

const BLOCK: usize = 128;

fn options() -> EngineOptions {
    EngineOptions {
        sample_rate: 48_000.0,
        max_block: BLOCK,
        socket_telemetry: true,
        offline: false,
    }
}

fn new_engine(project: &Project) -> Engine {
    Engine::new(
        project.clone(),
        Arc::new(ac_nodes::registry()),
        Arc::new(NoResources),
        options(),
    )
    .0
}

/// Oscillator → Gain → Meter → Audio Out.
fn patch(freq: f64) -> Project {
    P::new()
        .node(1, "oscillator")
        .param(1, "freq", freq)
        .node(2, "gain")
        .param(2, "gain", -6.0)
        .node(3, "meter")
        .node(4, "audio_out")
        .wire(1, "out", 2, "in")
        .wire(2, "out", 3, "in")
        .wire(3, "out", 4, "in")
        .build()
}

fn block(processor: &mut Processor) -> Vec<f32> {
    let mut out = vec![0.0f32; 2 * BLOCK];
    let mut io = HostIo {
        frames: BLOCK,
        input: &[],
        in_channels: 0,
        output: &mut out,
        out_channels: 2,
        midi: &[],
        midi_out: None,
        controls: &[],
        osc_out: None,
    };
    processor.process(&mut io);
    out
}

/// What the page does each frame: relay everything the mirror's handle
/// sent, and send the project along whenever the mirror compiled a new one.
struct Link {
    mirror: EngineHandle,
    mirror_processor: Processor,
    remote: EngineHandle,
    /// The mirror generation of the project the remote engine compiled
    /// last.
    remote_project: u64,
}

impl Link {
    /// The link and the remote engine's processor.
    fn new(project: &Project) -> (Link, Processor) {
        let (mirror, mirror_processor) = new_engine(project).split();
        let (remote, remote_processor) = new_engine(project).split();
        let link = Link {
            remote_project: mirror.generation(),
            mirror,
            mirror_processor,
            remote,
        };
        (link, remote_processor)
    }

    fn sync(&mut self) {
        if self.mirror.generation() != self.remote_project {
            self.remote
                .edit(GraphEdit::Replace(Box::new(self.mirror.project().clone())));
            self.remote_project = self.mirror.generation();
        }
        let (remote, project) = (&mut self.remote, self.remote_project);
        self.mirror_processor.relay(&[], &[], &mut |m| {
            remote.apply_relayed(m, project);
        });
        self.mirror.collect_garbage();
        self.remote.collect_garbage();
    }
}

#[test]
fn a_relayed_engine_plays_exactly_like_a_local_one() {
    let mut local = new_engine(&patch(220.0));
    let (mut link, mut remote) = Link::new(&patch(220.0));
    let osc = path(1);
    for k in 0..60 {
        match k {
            0 => {
                local.handle.play();
                link.mirror.play();
            }
            10 => {
                assert!(local.handle.set_param(&osc, "freq", Value::Float(330.0)));
                assert!(link.mirror.set_param(&osc, "freq", Value::Float(330.0)));
            }
            20 => {
                // An edit recompiles both sides; state carries over.
                let p = patch(440.0);
                local.handle.edit(GraphEdit::Replace(Box::new(p.clone())));
                link.mirror.edit(GraphEdit::Replace(Box::new(p)));
            }
            30 => {
                local.handle.seek(4_800);
                link.mirror.seek(4_800);
            }
            40 => {
                local.handle.stop();
                link.mirror.stop();
            }
            45 => {
                local.handle.play_from_start();
                link.mirror.play_from_start();
            }
            _ => {}
        }
        link.sync();
        let want = block(&mut local.processor);
        local.handle.collect_garbage();
        let got = block(&mut remote);
        assert_eq!(got, want, "block {k}");
        assert_eq!(link.remote.position(), local.handle.position());
    }
    assert!(
        local.handle.position() > 0,
        "the transport ran: {}",
        local.handle.position()
    );
}

#[test]
fn the_mirror_shows_the_remote_engines_telemetry() {
    let (mut link, mut remote) = Link::new(&patch(220.0));
    link.mirror.play();
    for _ in 0..40 {
        link.sync();
        block(&mut remote);
    }
    let theirs = link.remote.telemetry();
    let ours = link.mirror.telemetry();
    let meter = path(3);
    let level = theirs.node_value(&meter, 0).expect("the meter publishes");
    assert!(level > 0.1, "the oscillator plays: {level}");
    assert_eq!(ours.node_value(&meter, 0), Some(0.0), "nothing ran here");
    assert!(!link.mirror.is_playing() || link.mirror.position() == 0);

    let mut words = vec![0u32; theirs.export_len()];
    assert_eq!(theirs.export(&mut words), words.len());
    assert!(ours.import(&words), "same project, same shape");
    assert_eq!(ours.node_value(&meter, 0), Some(level));
    assert_eq!(
        ours.socket_level(&path(2), "out"),
        theirs.socket_level(&path(2), "out")
    );
    assert!(link.mirror.is_playing());
    assert_eq!(link.mirror.position(), link.remote.position());
    assert_eq!(link.mirror.position(), 40 * BLOCK as u64);
    assert_eq!(ours.status().master_peak(), theirs.status().master_peak());

    // A short buffer holds part of an export, which is refused.
    let mut short = vec![0u32; words.len() - 1];
    assert_eq!(theirs.export(&mut short), words.len());
    assert!(!ours.import(&short));

    // A graph of another shape refuses the store but takes the status.
    let other = P::new()
        .node(1, "oscillator")
        .node(2, "audio_out")
        .wire(1, "out", 2, "in")
        .build();
    let (elsewhere, _) = new_engine(&other).split();
    let reader = elsewhere.telemetry();
    assert!(!reader.import(&words));
    assert_eq!(
        reader.status().position.load(Ordering::Relaxed),
        link.remote.position()
    );
}

#[test]
fn parameter_changes_for_another_project_are_dropped() {
    let (mut remote, mut processor) = new_engine(&patch(220.0)).split();
    remote.play();
    let mirror_generation = 7;
    // A slot resolved against a project the remote engine didn't compile
    // (or hasn't yet) would name some other input: it's dropped.
    let stale = Relayed::Param {
        generation: 6,
        slot: 0,
        value: 1.0,
    };
    assert!(!remote.apply_relayed(stale, mirror_generation));
    let fire = Relayed::Fire {
        generation: 8,
        slot: 0,
    };
    assert!(!remote.apply_relayed(fire, mirror_generation));
    // Transport commands always go through.
    assert!(remote.apply_relayed(Relayed::Seek(960), mirror_generation));
    assert!(remote.apply_relayed(Relayed::Stop, mirror_generation));
    block(&mut processor);
    assert_eq!(remote.position(), 960);
    assert!(!remote.is_playing());
}

#[test]
fn binding_values_cross_as_words() {
    let mut project = patch(220.0);
    project.bindings.push(Binding::input(
        Source::MidiCc {
            channel: None,
            cc: 74,
        },
        path(1),
        "freq",
    ));
    let (remote, mut processor) = new_engine(&project).split();
    let mirror = new_engine(&project).handle;
    let (theirs, ours) = (remote.binding_values(), mirror.binding_values());
    assert_eq!(theirs.len(), 1);
    // A controller moves on the remote side only.
    let cc = [MidiEvent::new(
        0,
        MidiMsg::ControlChange {
            channel: 0,
            controller: 74,
            value: 127,
        },
    )];
    let mut out = vec![0.0f32; 2 * BLOCK];
    processor.process(&mut HostIo {
        frames: BLOCK,
        input: &[],
        in_channels: 0,
        output: &mut out,
        out_channels: 2,
        midi: &cc,
        midi_out: None,
        controls: &[],
        osc_out: None,
    });
    assert!(theirs.get(0).is_some(), "the binding set the input");
    assert_eq!(ours.get(0), None);
    let mut words = vec![0u32; theirs.len()];
    assert_eq!(theirs.export(&mut words), 1);
    assert!(ours.import(&words));
    assert_eq!(ours.get(0), theirs.get(0));
    assert!(!ours.import(&[0, 0]), "one word per binding");
}
