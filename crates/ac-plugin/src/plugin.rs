//! The two plugins and their glue to the plugin framework.

use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::sync::Arc;

use nice_plug::prelude::*;

use crate::bridge::{Bridge, HostTransport, midi_to_note, note_to_midi};
use crate::control_thread::ControlThread;
use crate::editor::{NoEditor, WindowKind, WithEditor};
use crate::params::PluginParams;
use crate::session::{Kind, Shared};

const VENDOR: &str = "Audio Constructor";
const URL: &str = "https://github.com/Andorbal/audio-constructor";

/// What differs between the effect and the instrument.
pub trait Variant: Send + 'static {
    const KIND: Kind;
    const NAME: &'static str;
    /// Stable forever: hosts store it with every session.
    const CLAP_ID: &'static str;
    const DESCRIPTION: &'static str;
    const CLAP_FEATURES: &'static [ClapFeature];
    /// Stable forever, like the CLAP id.
    const VST3_CLASS_ID: [u8; 16];
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory];
    const LAYOUTS: &'static [AudioIOLayout];
    /// Whether MIDI Out nodes send to the host.
    const MIDI_OUT: bool;
}

/// Audio in, audio out, notes in.
#[derive(Debug)]
pub struct EffectVariant;

/// Notes in, audio out, notes out.
#[derive(Debug)]
pub struct InstrumentVariant;

const STEREO: Option<NonZeroU32> = NonZeroU32::new(2);
const MONO: Option<NonZeroU32> = NonZeroU32::new(1);

impl Variant for EffectVariant {
    const KIND: Kind = Kind::Effect;
    const NAME: &'static str = "Audio Constructor Effect";
    const CLAP_ID: &'static str = "io.github.andorbal.audio-constructor.effect";
    const DESCRIPTION: &'static str = "A node-based patch as an audio effect";
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::MultiEffects,
        ClapFeature::Stereo,
        ClapFeature::Mono,
    ];
    const VST3_CLASS_ID: [u8; 16] = *b"AudioConstrEffct";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Tools];
    const LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: STEREO,
            main_output_channels: STEREO,
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: MONO,
            main_output_channels: MONO,
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: MONO,
            main_output_channels: STEREO,
            ..AudioIOLayout::const_default()
        },
    ];
    const MIDI_OUT: bool = false;
}

impl Variant for InstrumentVariant {
    const KIND: Kind = Kind::Instrument;
    const NAME: &'static str = "Audio Constructor Instrument";
    const CLAP_ID: &'static str = "io.github.andorbal.audio-constructor.instrument";
    const DESCRIPTION: &'static str = "A node-based patch as an instrument";
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
    const VST3_CLASS_ID: [u8; 16] = *b"AudioConstrInstr";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
    const LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: STEREO,
        ..AudioIOLayout::const_default()
    }];
    const MIDI_OUT: bool = true;
}

/// Audio Constructor Effect, with the editor in its window (CLAP).
pub type Effect = AcPlugin<EffectVariant>;
/// Audio Constructor Instrument, with the editor in its window (CLAP).
pub type Instrument = AcPlugin<InstrumentVariant>;
/// The effect as VST3: no window yet, so hosts show their generic controls
/// (see `editor.rs`).
pub type Vst3Effect = AcPlugin<EffectVariant, NoEditor>;
/// The instrument as VST3, without a window like [`Vst3Effect`].
pub type Vst3Instrument = AcPlugin<InstrumentVariant, NoEditor>;

/// One plugin instance. `W` says whether it has a window.
pub struct AcPlugin<V: Variant, W: WindowKind = WithEditor> {
    shared: Arc<Shared>,
    params: Arc<PluginParams>,
    audio: Option<AudioSide>,
    control: Option<ControlThread>,
    _variant: PhantomData<(V, W)>,
}

struct AudioSide {
    processor: ac_engine::Processor,
    bridge: Bridge,
    latency: u32,
}

impl<V: Variant, W: WindowKind> AcPlugin<V, W> {
    /// What the instance shares with its editor and the host's threads.
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.control.take() {
            c.stop();
        }
        // Dropped here, on the host's main thread.
        self.audio = None;
        self.shared.lock().stop();
    }
}

impl<V: Variant, W: WindowKind> Default for AcPlugin<V, W> {
    fn default() -> Self {
        let shared = Shared::new(V::KIND);
        AcPlugin {
            params: Arc::new(PluginParams::new(shared.clone())),
            shared,
            audio: None,
            control: None,
            _variant: PhantomData,
        }
    }
}

impl<V: Variant, W: WindowKind> Drop for AcPlugin<V, W> {
    fn drop(&mut self) {
        // Join the control thread before the host can unload this code.
        if let Some(mut c) = self.control.take() {
            c.stop();
        }
    }
}

impl<V: Variant, W: WindowKind> Plugin for AcPlugin<V, W> {
    const NAME: &'static str = V::NAME;
    const VENDOR: &'static str = VENDOR;
    const URL: &'static str = URL;
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = V::LAYOUTS;
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = if V::MIDI_OUT {
        MidiConfig::MidiCCs
    } else {
        MidiConfig::None
    };
    // The framework splits blocks where host parameters change, so a macro
    // lands on its sample like any bound control.
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type Editor = W::Editor;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    #[cfg(feature = "editor")]
    fn editor(&mut self, _executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        W::create(crate::editor::EditorContext {
            shared: self.shared.clone(),
            params: self.params.clone(),
        })
    }

    fn activate(
        &mut self,
        layout: &AudioIOLayout,
        config: &BufferConfig,
        context: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.stop();
        let ins = layout.main_input_channels.map_or(0, |n| n.get() as usize);
        let outs = layout.main_output_channels.map_or(0, |n| n.get() as usize);
        let max = (config.max_buffer_size as usize).max(1);
        let (processor, report) = self.shared.lock().start(f64::from(config.sample_rate), max);
        let latency = report.stats.latency;
        context.set_latency_samples(latency);
        self.audio = Some(AudioSide {
            processor,
            bridge: Bridge::new(ins, outs, max, V::MIDI_OUT),
            latency,
        });
        self.control = Some(ControlThread::start(self.shared.clone()));
        true
    }

    fn reset(&mut self) {
        if let Some(a) = self.audio.as_mut() {
            a.processor.reset();
            a.bridge.resend_macros();
        }
    }

    fn deactivate(&mut self) {
        self.stop();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let Some(audio) = self.audio.as_mut() else {
            return ProcessStatus::Normal;
        };
        let params = &self.params;
        assert_no_alloc::assert_no_alloc(|| process_block(audio, params, buffer, context))
    }
}

/// One host block: real-time safe (asserted in debug builds).
fn process_block<P: Plugin<SysExMessage = ()>>(
    audio: &mut AudioSide,
    params: &PluginParams,
    buffer: &mut Buffer,
    context: &mut impl ProcessContext<P>,
) -> ProcessStatus {
    let transport = host_transport(context.transport());
    while let Some(event) = context.next_event() {
        if let Some((offset, msg)) = note_to_midi(&event) {
            audio.bridge.push_midi(offset, msg);
        }
    }
    let macros = params.macro_values();
    audio.bridge.process(
        &mut audio.processor,
        buffer.as_slice(),
        &macros,
        transport,
        |e| {
            if let Some(note) = midi_to_note(e) {
                let _ = context.try_send_event(note);
            }
        },
    );
    let latency = audio
        .processor
        .status()
        .latency
        .load(std::sync::atomic::Ordering::Relaxed);
    if latency != audio.latency {
        audio.latency = latency;
        context.set_latency_samples(latency);
    }
    // A patch can sound with no input (generators, tails): never let
    // the host put it to sleep.
    ProcessStatus::KeepAlive
}

/// The host's transport, if it gives one: hosts that send none (or only
/// a stopped flag with no timeline) leave the engine free-running.
fn host_transport(t: &Transport) -> Option<HostTransport> {
    let position = t.pos_samples().map(|p| u64::try_from(p).unwrap_or(0));
    (t.playing || t.tempo.is_some() || position.is_some()).then_some(HostTransport {
        playing: t.playing,
        position,
    })
}

impl<V: Variant, W: WindowKind> ClapPlugin for AcPlugin<V, W> {
    const CLAP_ID: &'static str = V::CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> = Some(V::DESCRIPTION);
    const CLAP_MANUAL_URL: Option<&'static str> = Some(URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = Some(URL);
    const CLAP_FEATURES: &'static [ClapFeature] = V::CLAP_FEATURES;

    fn remote_controls(&self, context: &mut impl RemoteControlsContext) {
        context.add_section("Macros", |section| {
            section.add_page("Macros", |page| {
                for m in &self.params.macros {
                    page.add_param(&m.value);
                }
            });
        });
    }
}

impl<V: Variant, W: WindowKind> Vst3Plugin for AcPlugin<V, W> {
    const VST3_CLASS_ID: [u8; 16] = V::VST3_CLASS_ID;
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = V::VST3_SUBCATEGORIES;
}
