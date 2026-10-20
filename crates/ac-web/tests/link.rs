//! The whole web link, natively: the editor with its mirror engine on one
//! side, the worklet's engine on the other, joined by the same rings,
//! messages, and snapshots the browser uses (in heap memory here). The
//! worklet's engine must play exactly what a local engine plays, and the
//! editor must see the worklet's telemetry.

use std::sync::Arc;

use ac_app::audio::HostAudio;
use ac_app::{AppOptions, EditorApp};
use ac_core::{AudioData, MidiEvent, MidiMsg, NodePath, Project, Value};
use ac_engine::{Engine, EngineOptions, GraphEdit, HostIo};
use ac_web::audio::AudioSide;
use ac_web::link::PageLink;
use ac_web::ring::{Consumer, HeapShared, Producer, init_ring, ring_bytes};
use ac_web::snapshot::SnapshotWriter;

const RATE: u32 = 48_000;
const QUANTUM: usize = 128;

fn example(name: &str) -> Project {
    let path = format!(
        "{}/../../examples/{name}.acproj",
        env!("CARGO_MANIFEST_DIR")
    );
    ac_io::load_project(std::path::Path::new(&path)).expect("the example loads")
}

fn audio() -> HostAudio {
    HostAudio {
        host: "Web Audio".into(),
        sample_rate: RATE,
        max_block: QUANTUM,
        in_channels: 2,
        out_channels: 2,
    }
}

/// Shared buffers, as the page sets them up.
struct Buffers {
    to_audio: HeapShared,
    from_audio: HeapShared,
    snapshot: HeapShared,
}

impl Buffers {
    fn new() -> Buffers {
        let b = Buffers {
            to_audio: HeapShared::new(ring_bytes(1 << 16)),
            from_audio: HeapShared::new(ring_bytes(1 << 14)),
            snapshot: HeapShared::new(1 << 20),
        };
        init_ring(&b.to_audio).unwrap();
        init_ring(&b.from_audio).unwrap();
        b
    }
}

/// The page and the worklet, with a local engine to compare against.
struct Web<'a> {
    editor: EditorApp,
    mirror: ac_engine::Processor,
    link: PageLink<&'a HeapShared>,
    side: AudioSide,
    inbox: Consumer<&'a HeapShared>,
    outbox: Producer<&'a HeapShared>,
    snapshot: SnapshotWriter<&'a HeapShared>,
}

fn assets(editor: &EditorApp) -> Vec<(String, Arc<AudioData>)> {
    let store = editor.assets();
    let mut v: Vec<_> = store
        .paths()
        .filter_map(|p| store.get(p).map(|a| (p.clone(), a)))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

impl<'a> Web<'a> {
    fn new(b: &'a Buffers, project: Project) -> Web<'a> {
        // As the page makes it: the page runs the audio and keeps the files.
        let mut editor = EditorApp::embedded(AppOptions::web(
            audio(),
            1,
            ac_app::settings::AppSettings::in_memory(),
        ));
        editor.load_project(project, None);
        let mirror = editor
            .take_processor()
            .expect("the host runs the processor");
        Web {
            editor,
            mirror,
            link: PageLink::new(&b.to_audio, &b.from_audio, &b.snapshot).unwrap(),
            side: AudioSide::new(
                Arc::new(ac_nodes::registry()),
                f64::from(RATE),
                QUANTUM,
                2,
                2,
            ),
            inbox: Consumer::new(&b.to_audio).unwrap(),
            outbox: Producer::new(&b.from_audio).unwrap(),
            snapshot: SnapshotWriter::new(&b.snapshot),
        }
    }

    /// One page frame: the editor's upkeep, then the link's.
    fn frame(&mut self) {
        self.editor.upkeep();
        let assets = assets(&self.editor);
        for post in self
            .link
            .sync(&self.editor.engine.handle, &mut self.mirror, &assets)
        {
            self.side.message(&post).expect("the worklet reads it");
        }
    }

    /// One render quantum in the worklet, with `input` on both channels.
    fn quantum(&mut self, input: &[f32]) -> Vec<f32> {
        let buf = self.side.input_mut();
        buf[..QUANTUM].copy_from_slice(input);
        buf[QUANTUM..2 * QUANTUM].copy_from_slice(input);
        self.side.process(
            QUANTUM,
            &mut self.inbox,
            &mut self.outbox,
            &mut self.snapshot,
        );
        self.link.import(&self.editor.engine.handle);
        self.side.output()[..2 * QUANTUM].to_vec()
    }
}

fn local(project: &Project) -> Engine {
    Engine::new(
        project.clone(),
        Arc::new(ac_nodes::registry()),
        Arc::new(ac_core::NoResources),
        EngineOptions {
            sample_rate: f64::from(RATE),
            max_block: QUANTUM,
            socket_telemetry: true,
            offline: false,
        },
    )
    .0
}

fn run_local(engine: &mut Engine, input: &[f32], midi: &[MidiEvent]) -> Vec<f32> {
    let mut stereo = input.to_vec();
    stereo.extend_from_slice(input);
    let mut out = vec![0.0f32; 2 * QUANTUM];
    engine.process(&mut HostIo {
        frames: QUANTUM,
        input: &stereo,
        in_channels: 2,
        output: &mut out,
        out_channels: 2,
        midi,
        midi_out: None,
        controls: &[],
        osc_out: None,
    });
    out
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[test]
fn the_start_example_plays_in_the_worklet_as_it_does_locally() {
    let b = Buffers::new();
    let project = example("start");
    let mut web = Web::new(&b, project.clone());
    let mut reference = local(&project);
    let silence = [0.0f32; QUANTUM];
    let osc: NodePath = "2".parse().unwrap();
    for k in 0..300 {
        match k {
            0 => {
                web.editor.engine.handle.play();
                reference.handle.play();
            }
            100 => {
                assert!(
                    web.editor
                        .engine
                        .handle
                        .set_param(&osc, "freq", Value::Float(440.0))
                );
                assert!(
                    reference
                        .handle
                        .set_param(&osc, "freq", Value::Float(440.0))
                );
            }
            200 => {
                // An edit: a new project compiles on both sides.
                let mut p = web.editor.project();
                p.graph
                    .node_mut(ac_core::NodeId(3))
                    .unwrap()
                    .params
                    .insert("gain".into(), Value::Float(-6.0));
                web.editor.load_project(p.clone(), None);
                web.editor.engine.handle.play();
                reference.handle.edit(GraphEdit::Replace(Box::new(p)));
                reference.handle.stop();
                reference.handle.seek(0);
                reference.handle.play();
            }
            _ => {}
        }
        web.frame();
        let got = web.quantum(&silence);
        let want = run_local(&mut reference, &silence, &[]);
        assert_eq!(got, want, "quantum {k}");
    }
    assert!(peak(&web.side.output()[..QUANTUM]) > 0.4, "-6 dB of a saw");
    // The editor sees the worklet's transport and meters.
    let handle = &web.editor.engine.handle;
    assert!(handle.is_playing());
    assert!(handle.position() > 0);
    assert_eq!(handle.position(), reference.handle.position());
    assert!(web.link.in_step, "the snapshot is for the project shown");
    let meter: NodePath = "4".parse().unwrap();
    let level = handle.telemetry().node_value(&meter, 0).unwrap();
    assert!(level > 0.3, "the editor's meter moves: {level}");
}

#[test]
fn the_pedalboard_processes_the_input() {
    let b = Buffers::new();
    let project = example("guitar-pedalboard");
    let mut web = Web::new(&b, project.clone());
    let mut reference = local(&project);
    // A 1 kHz tone into the input; the pedalboard needs no play.
    let mut n = 0usize;
    let mut loudest = 0.0f32;
    for k in 0..400 {
        let input: Vec<f32> = (0..QUANTUM)
            .map(|i| {
                let t = (n + i) as f32 / RATE as f32;
                0.5 * (std::f32::consts::TAU * 1000.0 * t).sin()
            })
            .collect();
        n += QUANTUM;
        web.frame();
        let got = web.quantum(&input);
        let want = run_local(&mut reference, &input, &[]);
        assert_eq!(got, want, "quantum {k}");
        loudest = loudest.max(peak(&got));
    }
    assert!(loudest > 0.2, "the tone comes through: {loudest}");
}

#[test]
fn the_synth_sounds_when_a_note_comes_in() {
    let b = Buffers::new();
    let mut project = example("subtractive-synth");
    let mut web = Web::new(&b, project.clone());
    // Demo off, through the editor, as a person would click it.
    let demo: NodePath = "d".parse().unwrap();
    assert!(
        web.editor
            .engine
            .handle
            .set_param(&demo, "value", Value::Bool(false))
    );
    project
        .graph
        .node_mut(ac_core::NodeId(0xd))
        .unwrap()
        .params
        .insert("value".into(), Value::Bool(false));
    let mut reference = local(&project);
    let silence = [0.0f32; QUANTUM];
    web.editor.engine.handle.play();
    reference.handle.play();
    let mut quiet = 0.0f32;
    let mut loud = 0.0f32;
    for k in 0..600 {
        let midi: Vec<MidiEvent> = match k {
            100 => {
                let on = MidiMsg::NoteOn {
                    channel: 0,
                    note: 69,
                    velocity: 110,
                };
                web.link.send_midi(on);
                vec![MidiEvent::new(0, on)]
            }
            400 => {
                let off = MidiMsg::NoteOff {
                    channel: 0,
                    note: 69,
                    velocity: 0,
                };
                web.link.send_midi(off);
                vec![MidiEvent::new(0, off)]
            }
            _ => vec![],
        };
        web.frame();
        let got = web.quantum(&silence);
        let want = run_local(&mut reference, &silence, &midi);
        assert_eq!(got, want, "quantum {k}");
        if (20..100).contains(&k) {
            quiet = quiet.max(peak(&got));
        }
        if (100..400).contains(&k) {
            loud = loud.max(peak(&got));
        }
    }
    // Before the note only the closed amplifier's floor (−60 dB) leaks.
    assert!(quiet < 0.002, "silent before the note: {quiet}");
    assert!(
        loud > 0.05 && loud > 50.0 * quiet,
        "the note sounds: {loud}"
    );
}

#[test]
fn a_broken_project_message_does_not_stall_the_worklet() {
    let mut side = AudioSide::new(Arc::new(ac_nodes::registry()), 48_000.0, 128, 0, 2);
    assert!(side.message(b"garbage").is_err());
    let post = ac_web::wire::Posted::Project {
        seq: 1,
        generation: 1,
        assets: 0,
        json: "{ not json".into(),
    };
    assert!(side.message(&post.encode()).is_err());
    // The barrier for that project passes: it counts as handled.
    let b = Buffers::new();
    let mut tx = Producer::new(&b.to_audio).unwrap();
    let mut inbox = Consumer::new(&b.to_audio).unwrap();
    let mut outbox = Producer::new(&b.from_audio).unwrap();
    let mut snap = SnapshotWriter::new(&b.snapshot);
    let mut rec = [0u8; ac_web::wire::RECORD_MAX];
    for m in [
        ac_web::wire::ToAudio::Barrier(1),
        ac_web::wire::ToAudio::Relayed(ac_engine::Relayed::Play),
    ] {
        let n = m.encode(&mut rec);
        assert!(tx.push(&rec[..n]));
    }
    side.process(128, &mut inbox, &mut outbox, &mut snap);
    side.process(128, &mut inbox, &mut outbox, &mut snap);
    assert!(
        side.handle().is_playing(),
        "the play behind the barrier ran"
    );
}

#[test]
fn the_page_bakes_and_the_worklet_plays_the_bake() {
    let b = Buffers::new();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/reverse-delay.acproj");
    let text = std::fs::read_to_string(&path).unwrap();
    let project = Project::from_json(&text).unwrap();
    let registry = ac_nodes::registry();
    let assets: Vec<(String, Vec<u8>)> = ac_core::referenced_assets(&project, &registry)
        .into_iter()
        .map(|(p, _)| {
            let bytes = std::fs::read(path.parent().unwrap().join(&p)).unwrap();
            (p, bytes)
        })
        .collect();
    assert!(!assets.is_empty(), "the example plays a clip");
    let mut web = Web::new(&b, Project::default());
    web.editor.open_example_files(
        std::path::Path::new("examples/reverse-delay.acproj"),
        &text,
        assets,
    );
    // The mirror bakes (on the page's thread in a browser, on a thread of
    // its own here), then sends the result along.
    let reverse: NodePath = "3".parse().unwrap();
    let state = |h: &ac_engine::EngineHandle| {
        h.bakes()
            .iter()
            .find(|s| s.path == reverse)
            .map(|s| s.state.clone())
    };
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_secs() < 20 {
        web.frame();
        web.quantum(&[0.0; QUANTUM]);
        if state(web.side.handle()) == Some(ac_engine::WholeState::Baked) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        state(&web.editor.engine.handle),
        Some(ac_engine::WholeState::Baked),
        "the page baked"
    );
    assert_eq!(
        state(web.side.handle()),
        Some(ac_engine::WholeState::Baked),
        "the worklet took the page's bake"
    );
    // And it plays what a local engine that baked plays.
    let store = web.editor.assets().clone();
    let resources: Arc<dyn ac_core::Resources> = Arc::new(ac_io::FileResources {
        assets: store,
        recorder: None,
        plugins: None,
    });
    let (mut reference, _) = Engine::new(
        web.editor.engine.handle.project().clone(),
        Arc::new(ac_nodes::registry()),
        resources,
        EngineOptions {
            sample_rate: f64::from(RATE),
            max_block: QUANTUM,
            socket_telemetry: true,
            offline: false,
        },
    );
    reference.handle.bake_all();
    reference.handle.collect_garbage();
    web.editor.engine.handle.play();
    reference.handle.play();
    let mut loudest = 0.0f32;
    for k in 0..400 {
        web.frame();
        let got = web.quantum(&[0.0; QUANTUM]);
        let want = run_local(&mut reference, &[0.0; QUANTUM], &[]);
        assert_eq!(got, want, "quantum {k}");
        loudest = loudest.max(peak(&got));
    }
    assert!(loudest > 0.05, "the reversed clip plays: {loudest}");
}

/// Text signals cross like any other: a Constant's text read as a number
/// sets an oscillator's pitch, in the worklet as locally.
#[test]
fn text_signals_work_in_the_worklet() {
    let project = Project::from_json(TEXT_PITCH).expect("the text patch reads");
    let b = Buffers::new();
    let mut web = Web::new(&b, project.clone());
    let mut reference = local(&project);
    let silence = [0.0f32; QUANTUM];
    let mut crossings = 0;
    let mut last = 0.0f32;
    for k in 0..400 {
        web.frame();
        let got = web.quantum(&silence);
        let want = run_local(&mut reference, &silence, &[]);
        assert_eq!(got, want, "quantum {k}");
        if k >= 25 {
            for v in &got[..QUANTUM] {
                if last <= 0.0 && *v > 0.0 {
                    crossings += 1;
                }
                last = *v;
            }
        }
    }
    assert!(reference.handle.report().errors().count() == 0);
    // 375 quanta of 128 frames at 48 kHz is 1 s: 330 cycles.
    assert!(
        (crossings - 330i32).abs() <= 2,
        "330 Hz from the text: {crossings}"
    );
}

const TEXT_PITCH: &str = r#"{
    "schema_version": 2,
    "name": "Text Pitch",
    "graph": {
      "nodes": [
        { "id": "1", "kind": "constant", "config": { "type": "text" }, "params": { "value": "/pitch/330" } },
        { "id": "2", "kind": "text_to_number", "params": { "pattern": "/pitch/{}" } },
        { "id": "3", "kind": "oscillator", "params": { "shape": "sine", "start": "load" } },
        { "id": "4", "kind": "gain", "params": { "gain": -12.0 } },
        { "id": "5", "kind": "audio_out" }
      ],
      "wires": [
        { "from": "1:out", "to": "2:text" },
        { "from": "2:value", "to": "3:freq" },
        { "from": "3:out", "to": "4:in" },
        { "from": "4:out", "to": "5:in" }
      ]
    }
  }"#;
