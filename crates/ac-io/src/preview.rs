//! Block previews (DECISIONS D190): a few seconds of a block playing,
//! rendered offline through the engine so the same block always gives the
//! same file, to hear before adding it.
//!
//! The preview is a small project: the block, something to play it, and an
//! Audio Out on the block's first audio output. What plays it depends on
//! what the block takes:
//!
//! - **audio** (an effect): the preview phrase, two bars of drums, a chord
//!   pad, and an arpeggio built from ordinary nodes (`preview/phrase.acblock`,
//!   its noise seeded by the project). With two or more audio inputs the
//!   first gets the chords and arpeggio and the rest the drums, so a
//!   sidechain ducks to the beat and a vocoder speaks drums;
//! - **notes**: a block with a Voice inside plays the phrase's chords as a
//!   Voices node; a MIDI input on any other block gets its arpeggio;
//! - **triggers**: the phrase's drum hits: an input named for one (`kick`,
//!   `snare` or `clap`, `hat`) gets that one, and the others, in order, the
//!   snare (a backbeat, heard before and after it lands), the kick, and
//!   the hat;
//! - a `gate` (Bool) and a `freq` (Float, Hz) on a block without audio
//!   inputs follow the arpeggio, as a monophonic voice expects;
//! - everything else keeps the block's defaults, so a block that plays by
//!   itself (a drum machine on the transport) simply plays.
//!
//! The render is 5 s at 48 kHz in blocks of 256, 120 BPM. It's kept as one
//! channel when both come out the same, brought down to −1 dBFS if it
//! would clip (never raised), and written as 16-bit PCM.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ac_core::{
    Block, BlockPreview, Endpoint, GROUP_KIND, GroupDef, GroupId, NoResources, NodeId,
    NodeInstance, Project, Registry, SignalType, Unit, VOICES_KIND, preview_file_name, slug,
};
use ac_engine::{RenderOptions, render};

use crate::hash::sha256_hex;
use crate::wav::{BitDepth, wav_bytes};

/// The preview's sample rate.
pub const PREVIEW_RATE: u32 = 48_000;
/// How long a preview is: the phrase's two bars at 120 BPM and a second
/// for tails.
pub const PREVIEW_SECONDS: f64 = 5.0;
/// The render's block size (fixed, so the samples are too).
pub const PREVIEW_BLOCK: u32 = 256;
/// The seed the phrase's noise (and the block's random nodes) start from.
pub const PREVIEW_SEED: u64 = 0x5EED_B10C;
/// The loudest a preview may peak, −1 dBFS: louder ones are turned down.
const CEILING: f32 = 0.891_250_9;
/// Below this peak (−100 dBFS) a preview is silence.
const SILENT: f32 = 1e-5;

/// The phrase that plays effects and instruments.
const PHRASE: &str = include_str!("preview/phrase.acblock");

fn phrase() -> Option<&'static Block> {
    static PHRASE_BLOCK: OnceLock<Option<Block>> = OnceLock::new();
    PHRASE_BLOCK
        .get_or_init(|| Block::from_json(PHRASE).ok())
        .as_ref()
}

/// A rendered preview.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    /// One channel, or two when they differ.
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
    /// What fed each of the block's inputs, for people:
    /// `"in ← the preview phrase (drums, chords, arpeggio)"`.
    pub feeds: Vec<String>,
    /// Warnings from compiling the preview.
    pub warnings: Vec<String>,
    /// How far it was turned down to stay under −1 dBFS, in dB (0 if not).
    pub reduced_db: f32,
}

impl Preview {
    /// The preview as a 16-bit WAV file.
    pub fn wav(&self) -> Vec<u8> {
        // Writing to memory can't fail short of running out of it.
        wav_bytes(&self.channels, self.sample_rate, BitDepth::Pcm16).unwrap_or_default()
    }

    pub fn seconds(&self) -> f64 {
        self.channels.first().map_or(0, Vec::len) as f64 / f64::from(self.sample_rate.max(1))
    }
}

/// The project that plays `block` for its preview, with a line per fed
/// input. Fails when the block has no audio output, so nothing to hear.
pub fn preview_project(block: &Block) -> Result<(Project, Vec<String>), String> {
    let def = &block.def;
    let name = if def.name.trim().is_empty() {
        "This block"
    } else {
        def.name.as_str()
    };
    let output = def
        .outputs
        .iter()
        .find(|o| o.ty == SignalType::Audio)
        .ok_or_else(|| format!("{name} has no audio output, so a preview would be silent"))?;
    let phrase = phrase().ok_or("the preview phrase didn't load (a bug: please report it)")?;

    let mut project = Project {
        name: format!("Preview of {name}"),
        ..Project::default()
    };
    project.settings.sample_rate = PREVIEW_RATE;
    project.settings.block_size = PREVIEW_BLOCK;
    project.settings.length = PREVIEW_SECONDS;
    project.settings.seed = PREVIEW_SEED;
    project.settings.tempo = vec![ac_core::TempoSegment::default()];

    // The block's definitions first, as it has them; the phrase's under ids
    // nothing of the block's uses.
    let mut defs: BTreeMap<GroupId, GroupDef> = block.uses.clone();
    let block_id = free_id(&slug(&def.name), &defs);
    let mut d = def.clone();
    d.source = None;
    defs.insert(block_id.clone(), d);
    let mut renamed = BTreeMap::new();
    for id in phrase.uses.keys() {
        let new = free_id(&id.0, &defs);
        defs.insert(new.clone(), GroupDef::new(""));
        renamed.insert(id.clone(), new);
    }
    for (id, inner) in &phrase.uses {
        let mut inner = inner.clone();
        repoint(&mut inner, &renamed);
        defs.insert(renamed[id].clone(), inner);
    }
    let phrase_id = free_id("preview-phrase", &defs);
    let mut p = phrase.def.clone();
    repoint(&mut p, &renamed);
    defs.insert(phrase_id.clone(), p);
    project.group_defs = defs;

    let (phrase_node, block_node, out_node) = (NodeId(1), NodeId(2), NodeId(3));
    let poly = block.is_polyphonic();
    let g = &mut project.graph;
    g.nodes.push(
        NodeInstance::new(phrase_node, GROUP_KIND)
            .with_config(serde_json::json!({ "def": phrase_id.0 }))
            .with_label("Preview phrase")
            .at(0.0, 0.0),
    );
    g.nodes.push(
        NodeInstance::new(block_node, if poly { VOICES_KIND } else { GROUP_KIND })
            .with_config(serde_json::json!({ "def": block_id.0 }))
            .with_label(name)
            .at(320.0, 0.0),
    );
    g.nodes
        .push(NodeInstance::new(out_node, "audio_out").at(640.0, 0.0));
    g.connect(
        Endpoint::new(block_node, &output.name),
        Endpoint::new(out_node, "in"),
    );

    let mut feeds = Vec::new();
    let mut feed = |g: &mut ac_core::Graph, from: &'static str, to: &str, what: &str| {
        g.connect(
            Endpoint::new(phrase_node, from),
            Endpoint::new(block_node, to),
        );
        feeds.push(format!("{to} ← {what}"));
    };
    if poly {
        feed(g, "notes", "midi", "the preview phrase's chords");
    }
    let audio: Vec<&str> = def
        .inputs
        .iter()
        .filter(|i| i.ty == SignalType::Audio)
        .map(|i| i.name.as_str())
        .collect();
    match audio.as_slice() {
        [] => {}
        [only] => feed(
            g,
            "mix",
            only,
            "the preview phrase (drums, chords, arpeggio)",
        ),
        [first, rest @ ..] => {
            feed(
                g,
                "music",
                first,
                "the preview phrase's chords and arpeggio",
            );
            for r in rest {
                feed(g, "drums", r, "the preview phrase's drums");
            }
        }
    }
    let effect = !audio.is_empty();
    const KICK: (&str, &str) = ("kick", "the phrase's kick drum");
    const SNARE: (&str, &str) = ("snare", "the phrase's snare");
    const HAT: (&str, &str) = ("hat", "the phrase's hi-hat");
    let named = |n: &str| {
        let n = n.to_ascii_lowercase();
        if n.contains("kick") {
            Some(KICK)
        } else if n.contains("snare") || n.contains("clap") {
            Some(SNARE)
        } else if n.contains("hat") || n.contains("cymbal") {
            Some(HAT)
        } else {
            None
        }
    };
    let hits = [SNARE, KICK, HAT];
    let mut triggers = 0;
    for input in &def.inputs {
        let n = input.name.as_str();
        match input.ty {
            // A Voices node takes the notes itself.
            SignalType::Midi if !poly => feed(g, "melody", n, "the preview phrase's arpeggio"),
            SignalType::Trigger => {
                let (from, what) = named(n).unwrap_or_else(|| {
                    triggers += 1;
                    hits[(triggers - 1) % hits.len()]
                });
                feed(g, from, n, what);
            }
            SignalType::Bool if !effect && n.eq_ignore_ascii_case("gate") => {
                feed(g, "gate", n, "the arpeggio's notes (held while each plays)")
            }
            SignalType::Float
                if !effect && n.eq_ignore_ascii_case("freq") && input.unit == Unit::Hz =>
            {
                feed(g, "freq", n, "the arpeggio's pitch")
            }
            _ => {}
        }
    }
    Ok((project, feeds))
}

/// `base`, or `base-2`, `base-3`… if a definition already has that id.
fn free_id(base: &str, defs: &BTreeMap<GroupId, GroupDef>) -> GroupId {
    let mut id = GroupId(base.to_string());
    let mut n = 2;
    while defs.contains_key(&id) {
        id = GroupId(format!("{base}-{n}"));
        n += 1;
    }
    id
}

/// Point group and voices nodes in `def` at renamed definitions.
fn repoint(def: &mut GroupDef, renamed: &BTreeMap<GroupId, GroupId>) {
    for n in &mut def.graph.nodes {
        let Some(old) = n.group_def() else { continue };
        if let (Some(new), Some(obj)) = (renamed.get(&old), n.config.as_object_mut()) {
            obj.insert("def".into(), new.0.clone().into());
        }
    }
}

/// Render `block`'s preview. Fails, saying why, when there's nothing to
/// hear: no audio output, errors compiling it, or silence.
pub fn render_preview(block: &Block, registry: Arc<Registry>) -> Result<Preview, String> {
    let (project, feeds) = preview_project(block)?;
    let name = project
        .name
        .strip_prefix("Preview of ")
        .unwrap_or("The block")
        .to_string();
    let out = render(
        &project,
        registry,
        Arc::new(NoResources),
        &RenderOptions::default(),
    );
    if out.report.has_errors() {
        let errors: Vec<String> = out.report.errors().map(ToString::to_string).collect();
        return Err(format!(
            "{name} doesn't compile for a preview: {}",
            errors.join("; ")
        ));
    }
    let warnings: Vec<String> = out
        .report
        .diagnostics
        .iter()
        .filter(|d| d.severity != ac_engine::Severity::Error)
        .map(ToString::to_string)
        .collect();
    let mut channels = out.channels;
    channels.truncate(2);
    for ch in &mut channels {
        for s in ch.iter_mut() {
            if !s.is_finite() {
                *s = 0.0;
            }
        }
    }
    if channels.len() == 2 && channels[0] == channels[1] {
        channels.truncate(1);
    }
    let peak = channels
        .iter()
        .flatten()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    if peak < SILENT {
        return Err(format!(
            "{name} is silent when the preview plays it ({})",
            if feeds.is_empty() {
                "nothing feeds it".to_string()
            } else {
                feeds.join(", ")
            }
        ));
    }
    let mut reduced_db = 0.0;
    if peak > CEILING {
        let scale = CEILING / peak;
        for s in channels.iter_mut().flatten() {
            *s *= scale;
        }
        reduced_db = -20.0 * scale.log10();
    }
    Ok(Preview {
        channels,
        sample_rate: out.sample_rate,
        feeds,
        warnings,
        reduced_db,
    })
}

/// What [`export_block`] wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct Exported {
    pub block: PathBuf,
    /// The preview file, if one was written.
    pub preview: Option<PathBuf>,
    /// The rendered preview (its feeds and warnings), if any.
    pub rendered: Option<Preview>,
    /// Why there's no preview, when one was asked for.
    pub no_preview: Option<String>,
}

/// Write `block` to `out` (an `.acblock`) and, with `preview`, its preview
/// next to it as `<name>.preview.wav`, recorded in the block's file. A
/// block that can't be previewed is still written, without one, and
/// [`Exported::no_preview`] says why.
pub fn export_block(
    mut block: Block,
    out: &Path,
    preview: Option<Arc<Registry>>,
) -> Result<Exported, String> {
    block.def.source = None;
    block.preview = None;
    let dir = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stem = out
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("{} isn't a file name", out.display()))?;
    let mut exported = Exported {
        block: out.to_path_buf(),
        preview: None,
        rendered: None,
        no_preview: None,
    };
    if let Some(registry) = preview {
        match render_preview(&block, registry) {
            Ok(p) => {
                let file = preview_file_name(stem);
                let path = dir.join(&file);
                let bytes = p.wav();
                std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
                block.preview = Some(BlockPreview {
                    file,
                    sha256: sha256_hex(&bytes),
                });
                exported.preview = Some(path);
                exported.rendered = Some(p);
            }
            Err(e) => exported.no_preview = Some(e),
        }
    }
    let tmp = out.with_extension("acblock.tmp");
    std::fs::write(&tmp, block.to_json()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(exported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::shipped_blocks;
    use ac_core::{GroupInput, GroupOutput};

    fn registry() -> Arc<Registry> {
        Arc::new(ac_nodes::registry())
    }

    fn shipped(id: &str) -> Block {
        shipped_blocks()
            .into_iter()
            .find(|b| b.source.id == id)
            .unwrap_or_else(|| panic!("no shipped block {id}"))
            .block
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn the_phrase_loads_and_plays_by_itself() {
        let p = phrase().expect("phrase.acblock parses");
        assert!(p.uses.len() == 1, "its keys voice");
        // As a block of its own it's a sound maker: no inputs to feed.
        let preview = render_preview(p, registry()).unwrap();
        assert!(preview.feeds.is_empty());
        assert!(preview.warnings.is_empty(), "{:?}", preview.warnings);
        assert_eq!(preview.sample_rate, PREVIEW_RATE);
        assert_eq!(
            preview.channels[0].len(),
            (PREVIEW_SECONDS * f64::from(PREVIEW_RATE)) as usize
        );
        let ch = &preview.channels[0];
        let peak = ch.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.2 && peak <= CEILING + 1e-6, "peak {peak}");
        // Two bars play, then the tail dies away.
        let bar = |b: usize| &ch[b * 96_000..(b + 1) * 96_000];
        assert!(rms(bar(0)) > 0.03 && rms(bar(1)) > 0.03);
        assert!(rms(&ch[4 * 48_000 + 24_000..]) < rms(bar(0)) / 20.0);
    }

    #[test]
    fn effects_get_the_phrase_and_sidechains_the_drums() {
        let (p, feeds) = preview_project(&shipped("tremolo")).unwrap();
        assert_eq!(feeds, ["in ← the preview phrase (drums, chords, arpeggio)"]);
        assert_eq!(p.settings.sample_rate, 48_000);
        let (_, feeds) = preview_project(&shipped("sidechain-ducker")).unwrap();
        assert_eq!(
            feeds,
            [
                "in ← the preview phrase's chords and arpeggio",
                "sidechain ← the preview phrase's drums"
            ]
        );
        let (_, feeds) = preview_project(&shipped("tape-stop")).unwrap();
        assert!(feeds.contains(&"stop ← the phrase's snare".to_string()));
        assert!(feeds.contains(&"start ← the phrase's kick drum".to_string()));
    }

    #[test]
    fn instruments_get_notes_and_drum_machines_play_alone() {
        let synth = shipped("subtractive-synth-voice");
        let (p, feeds) = preview_project(&synth).unwrap();
        assert_eq!(feeds, ["midi ← the preview phrase's chords"]);
        assert_eq!(p.graph.node(NodeId(2)).unwrap().kind, VOICES_KIND);
        let (_, feeds) = preview_project(&shipped("drum-machine")).unwrap();
        assert!(feeds.is_empty());
        let preview = render_preview(&synth, registry()).unwrap();
        assert!(rms(&preview.channels[0][..96_000]) > 0.01);
    }

    /// A monophonic voice: `gate` and `freq` in, a sine out at that pitch.
    fn mono_voice() -> Block {
        let mut def = GroupDef::new("Mono Beep");
        def.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "oscillator"));
        def.graph.nodes.push(NodeInstance::new(NodeId(2), "adsr"));
        def.graph
            .nodes
            .push(NodeInstance::new(NodeId(3), "math").with_param("op", "multiply"));
        def.graph.connect(
            Endpoint::new(NodeId(1), "out"),
            Endpoint::new(NodeId(3), "a"),
        );
        def.graph.connect(
            Endpoint::new(NodeId(2), "out"),
            Endpoint::new(NodeId(3), "b"),
        );
        def.inputs.push(GroupInput {
            name: "gate".into(),
            ty: SignalType::Bool,
            unit: Unit::Generic,
            default: None,
            range: None,
            targets: vec![Endpoint::new(NodeId(2), "gate")],
        });
        def.inputs.push(GroupInput {
            name: "freq".into(),
            ty: SignalType::Float,
            unit: Unit::Hz,
            default: None,
            range: None,
            targets: vec![Endpoint::new(NodeId(1), "freq")],
        });
        def.outputs.push(GroupOutput {
            name: "out".into(),
            ty: SignalType::Audio,
            source: Endpoint::new(NodeId(3), "out"),
        });
        Block {
            def,
            uses: BTreeMap::new(),
            preview: None,
        }
    }

    #[test]
    fn a_gate_and_freq_voice_follows_the_arpeggio() {
        let (_, feeds) = preview_project(&mono_voice()).unwrap();
        assert_eq!(
            feeds,
            [
                "gate ← the arpeggio's notes (held while each plays)",
                "freq ← the arpeggio's pitch"
            ]
        );
        let p = render_preview(&mono_voice(), registry()).unwrap();
        assert_eq!(p.channels.len(), 1, "a mono block makes a mono preview");
        // Notes come and go: loud in a note, quiet between the bars' end and the tail.
        assert!(rms(&p.channels[0][..24_000]) > 0.05);
    }

    #[test]
    fn trigger_inputs_get_the_hit_they_are_named_for() {
        let mut b = mono_voice();
        b.def.inputs.clear();
        for n in ["Kick In", "hit", "open_hat", "other"] {
            b.def.inputs.push(GroupInput {
                name: n.into(),
                ty: SignalType::Trigger,
                unit: Unit::Generic,
                default: None,
                range: None,
                targets: vec![Endpoint::new(NodeId(2), "on")],
            });
        }
        let (_, feeds) = preview_project(&b).unwrap();
        assert_eq!(
            feeds,
            [
                "Kick In ← the phrase's kick drum",
                "hit ← the phrase's snare",
                "open_hat ← the phrase's hi-hat",
                "other ← the phrase's kick drum",
            ]
        );
    }

    #[test]
    fn previews_are_the_same_every_time() {
        let a = render_preview(&shipped("chorus"), registry()).unwrap();
        let b = render_preview(&shipped("chorus"), registry()).unwrap();
        assert_eq!(a.wav(), b.wav());
        assert_eq!(a.channels.len(), 2, "the chorus widens it to stereo");
    }

    #[test]
    fn nothing_to_hear_is_a_reason_not_a_panic() {
        let mut b = mono_voice();
        b.def.outputs.clear();
        let e = render_preview(&b, registry()).unwrap_err();
        assert!(e.contains("no audio output"), "{e}");
        // Only an output and nothing feeding it: silence.
        let mut b = mono_voice();
        b.def.inputs.clear();
        let e = render_preview(&b, registry()).unwrap_err();
        assert!(e.contains("silent"), "{e}");
        // Ids the block already uses don't clash with the phrase's.
        let mut b = shipped("tremolo");
        b.uses
            .insert(GroupId("preview-keys".into()), GroupDef::new("Mine"));
        let (p, _) = preview_project(&b).unwrap();
        assert!(p.group_defs.contains_key(&GroupId("preview-keys-2".into())));
        assert!(render_preview(&b, registry()).is_ok());
    }

    #[test]
    fn export_writes_the_block_and_its_preview_side_by_side() {
        let dir = std::env::temp_dir().join(format!(
            "ac-preview-export-{}-{}",
            std::process::id(),
            crate::clock::now_ns()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("my trem.acblock");
        let e = export_block(shipped("tremolo"), &out, Some(registry())).unwrap();
        let wav = dir.join("my trem.preview.wav");
        assert_eq!(e.preview.as_deref(), Some(wav.as_path()));
        let back = Block::from_json(&std::fs::read_to_string(&out).unwrap()).unwrap();
        let rec = back.preview.clone().unwrap();
        assert_eq!(rec.file, "my trem.preview.wav");
        assert_eq!(rec.sha256, sha256_hex(&std::fs::read(&wav).unwrap()));
        assert!(back.same_definition(&shipped("tremolo")));
        // Without a preview, or without anything to hear, only the block.
        let out2 = dir.join("quiet.acblock");
        let mut silent = mono_voice();
        silent.def.outputs.clear();
        let e = export_block(silent, &out2, Some(registry())).unwrap();
        assert!(e.preview.is_none() && e.no_preview.is_some());
        let back = Block::from_json(&std::fs::read_to_string(&out2).unwrap()).unwrap();
        assert_eq!(back.preview, None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
