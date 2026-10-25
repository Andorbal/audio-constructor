//! The node trait, node metadata, and the contexts nodes run in.

use std::fmt;
use std::sync::Arc;

use crate::io::{AudioIn, Io};
use crate::plugin::{PluginProcessor, PluginRequest};
use crate::signal::Signature;
use crate::time::TransportView;
use crate::value::Config;
use crate::whole::{WholeCtx, WholeInputs, WholeOutputs};

/// Library category; maps to the header color in the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Io,
    Generators,
    Level,
    Filters,
    Time,
    Spectral,
    Math,
    Modulation,
    Triggers,
    Sequencing,
    Controls,
    Display,
    Utility,
}

impl Category {
    pub const ALL: [Category; 13] = [
        Category::Io,
        Category::Generators,
        Category::Level,
        Category::Filters,
        Category::Time,
        Category::Spectral,
        Category::Math,
        Category::Modulation,
        Category::Triggers,
        Category::Sequencing,
        Category::Controls,
        Category::Display,
        Category::Utility,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::Io => "Input / Output",
            Category::Generators => "Generators",
            Category::Level => "Level & Dynamics",
            Category::Filters => "Filters",
            Category::Time => "Time",
            Category::Spectral => "Spectral",
            Category::Math => "Math & Logic",
            Category::Modulation => "Modulation",
            Category::Triggers => "Triggers & Gates",
            Category::Sequencing => "Sequencing & Notes",
            Category::Controls => "Controls",
            Category::Display => "Display",
            Category::Utility => "Utility",
        }
    }

    /// Short identifier used in file names (`docs/reference/<slug>.md`).
    pub fn slug(self) -> &'static str {
        match self {
            Category::Io => "io",
            Category::Generators => "generators",
            Category::Level => "level",
            Category::Filters => "filters",
            Category::Time => "time",
            Category::Spectral => "spectral",
            Category::Math => "math",
            Category::Modulation => "modulation",
            Category::Triggers => "triggers",
            Category::Sequencing => "sequencing",
            Category::Controls => "controls",
            Category::Display => "display",
            Category::Utility => "utility",
        }
    }
}

/// Whether a node can run live or needs its whole input first (ARCHITECTURE §9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Causality {
    #[default]
    Streaming,
    WholeInput,
}

/// For source nodes: can the output be computed ahead of time?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Determinism {
    #[default]
    Deterministic,
    /// Fed by the outside world (devices); can't be baked.
    Live,
}

/// Static description of a node kind: identity, docs, and engine hints.
#[derive(Clone, Debug)]
pub struct NodeMeta {
    /// Registry key, stable forever (snake_case).
    pub kind: &'static str,
    /// Display name.
    pub name: &'static str,
    pub category: Category,
    /// Markdown, from the node's doc comment (via `node!`).
    pub doc: &'static str,
    /// Search synonyms ("volume" finds Gain, "lpf" finds Biquad).
    pub keywords: &'static [&'static str],
    /// A live source for causality analysis (AudioIn, MidiIn).
    pub live: bool,
    /// The `Feedback` node: the compiler splits it into a read and a write step.
    pub feedback: bool,
    /// Config keys that hold asset paths the host must load (e.g. `"file"`).
    pub asset_keys: &'static [&'static str],
    /// Config keys holding state the node restores when it's created (a
    /// hosted plugin's saved state). A running node already holds that
    /// state, so changing these keys doesn't rebuild it on recompile.
    pub state_keys: &'static [&'static str],
    /// Number of `f32` telemetry values the node publishes.
    pub telemetry: usize,
    /// Length of the node's scope ring (0 for none).
    pub scope: usize,
    /// Needs its whole input before producing output (ARCHITECTURE §9): the
    /// engine bakes it with [`Node::render_whole`] instead of calling
    /// `process`, and disables it when a live source feeds it.
    pub whole_input: bool,
    /// Writes to the device outputs. The multi-core scheduler runs these
    /// one at a time, in schedule order, so outputs sum the same way on any
    /// number of cores. A node must set this to write `io.host.output`;
    /// every other node sees an empty device output.
    pub host_output: bool,
    /// Wants to know the path latency at its output
    /// ([`Node::path_latency`]).
    pub latency_probe: bool,
}

impl NodeMeta {
    pub fn new(kind: &'static str, name: &'static str, category: Category) -> Self {
        NodeMeta {
            kind,
            name,
            category,
            doc: "",
            keywords: &[],
            live: false,
            feedback: false,
            asset_keys: &[],
            state_keys: &[],
            telemetry: 0,
            scope: 0,
            whole_input: false,
            host_output: false,
            latency_probe: false,
        }
    }

    pub fn with_doc(mut self, doc: &'static str) -> Self {
        self.doc = doc;
        self
    }

    pub fn with_keywords(mut self, k: &'static [&'static str]) -> Self {
        self.keywords = k;
        self
    }

    pub fn live(mut self) -> Self {
        self.live = true;
        self
    }

    pub fn feedback(mut self) -> Self {
        self.feedback = true;
        self
    }

    pub fn assets(mut self, keys: &'static [&'static str]) -> Self {
        self.asset_keys = keys;
        self
    }

    pub fn state_keys(mut self, keys: &'static [&'static str]) -> Self {
        self.state_keys = keys;
        self
    }

    pub fn telemetry(mut self, values: usize) -> Self {
        self.telemetry = values;
        self
    }

    pub fn scope(mut self, len: usize) -> Self {
        self.scope = len;
        self
    }

    pub fn whole_input(mut self) -> Self {
        self.whole_input = true;
        self
    }

    pub fn host_output(mut self) -> Self {
        self.host_output = true;
        self
    }

    pub fn latency_probe(mut self) -> Self {
        self.latency_probe = true;
        self
    }

    /// The doc comment with the single leading space of each `///` line removed.
    pub fn doc_text(&self) -> String {
        let mut out = String::with_capacity(self.doc.len());
        for line in self.doc.lines() {
            out.push_str(line.strip_prefix(' ').unwrap_or(line));
            out.push('\n');
        }
        out.trim_end().to_string()
    }

    /// The "Try this" tip (NODES.md): the paragraph of the docs that starts
    /// with "Try this", without that label. The editor's help shows it
    /// highlighted.
    pub fn tip(&self) -> Option<String> {
        split_tip(&self.doc_text()).1
    }

    /// The docs without the tip paragraph.
    pub fn description(&self) -> String {
        split_tip(&self.doc_text()).0
    }

    /// The first paragraph of the docs: a one-line summary.
    pub fn summary(&self) -> String {
        self.doc_text()
            .split("\n\n")
            .next()
            .unwrap_or("")
            .replace('\n', " ")
            .trim()
            .to_string()
    }
}

/// Split Markdown docs into everything but the "Try this" paragraph, and
/// that paragraph's text after its label (`**Try this:**`, bold or not).
/// Paragraphs are separated by blank lines; the first tip paragraph wins.
pub fn split_tip(doc: &str) -> (String, Option<String>) {
    const LABEL: &str = "try this";
    let mut rest = Vec::new();
    let mut tip = None;
    for para in doc.split("\n\n") {
        let t = para.trim();
        let body = t.strip_prefix("**").unwrap_or(t);
        let is_tip = tip.is_none()
            && body
                .get(..LABEL.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(LABEL))
            && !body[LABEL.len()..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
        if is_tip {
            let text = body[LABEL.len()..].trim_start_matches([':', '*', ' ', '\n']);
            tip = Some(capitalize(text.trim()));
        } else if !t.is_empty() {
            rest.push(para.trim_matches('\n'));
        }
    }
    (rest.join("\n\n"), tip.filter(|t| !t.is_empty()))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Decoded audio, as loaded by the host (any sample rate).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AudioData {
    pub sample_rate: u32,
    /// Non-interleaved channels, all the same length.
    pub channels: Vec<Vec<f32>>,
}

impl AudioData {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

/// What a `Record` node asks the host for.
#[derive(Clone, Debug)]
pub struct RecordRequest<'a> {
    /// Flattened node path, unique within the project.
    pub node_key: &'a str,
    /// Name to use for takes (the node's label or kind).
    pub name: &'a str,
    pub channels: usize,
    pub sample_rate: u32,
}

/// What a `FileOut` node asks the host for: a file its input renders to.
#[derive(Clone, Copy, Debug)]
pub struct FileRequest<'a> {
    /// Flattened node path, unique within the project.
    pub node_key: &'a str,
    /// The node's label or kind name.
    pub name: &'a str,
    /// The path from the node's config, as written.
    pub path: &'a str,
    pub channels: usize,
    pub sample_rate: u32,
}

/// A real-time-safe sink for recorded audio. The host's disk thread owns the
/// file; every method here must be wait-free and allocation-free.
pub trait RecordSink: Send {
    /// Begin a new take.
    fn start(&mut self);
    /// Append one block.
    fn write(&mut self, audio: &AudioIn<'_>);
    /// Finish the take.
    fn stop(&mut self);
    /// Samples dropped because the disk thread fell behind.
    fn dropped(&self) -> u64 {
        0
    }

    /// A finished take's audio, once, for sinks that keep takes in memory
    /// (`Record` outputs it as a `Buffer`). The sink must keep a handle of
    /// its own to every buffer it returns, off the audio thread, until no one
    /// else holds it, so the audio thread never drops the last one
    /// (DECISIONS D53).
    fn take_buffer(&mut self) -> Option<Arc<AudioData>> {
        None
    }
}

/// Host-provided services available while preparing nodes. The engine never
/// touches files or devices; hosts implement this.
pub trait Resources: Send + Sync {
    /// A decoded audio file referenced by a node's config (see `NodeMeta::asset_keys`).
    fn audio_asset(&self, _path: &str) -> Option<Arc<AudioData>> {
        None
    }

    /// A sink for a `Record` node, or `None` if recording isn't available.
    fn recorder(&self, _req: &RecordRequest<'_>) -> Option<Box<dyn RecordSink>> {
        None
    }

    /// A sink for a `FileOut` node: everything written between `start` and
    /// `stop` becomes the file. Only offline renders provide one.
    fn file_out(&self, _req: &FileRequest<'_>) -> Option<Box<dyn RecordSink>> {
        None
    }

    /// A running instance of the plugin a `Plugin` node describes, or why
    /// there isn't one. Called on the thread that compiles the graph,
    /// which hosts keep on their main thread (plugins need it there).
    fn plugin(&self, _req: &PluginRequest<'_>) -> Result<Box<dyn PluginProcessor>, String> {
        Err(crate::NO_PLUGIN_HOST.into())
    }
}

/// Resources for hosts with no assets or recording (tests, quick renders).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoResources;

impl Resources for NoResources {}

/// Everything a node may use while building its runtime state.
pub struct PrepareCtx<'a> {
    /// The engine's running rate (the device rate live, the render rate offline).
    pub sample_rate: f64,
    /// The largest block the node will be asked to process.
    pub max_block: usize,
    /// PRNG seed derived from the project seed and this node's path.
    pub seed: u64,
    /// Flattened node path, unique within the project.
    pub node_key: &'a str,
    /// Display name (label or kind name).
    pub name: &'a str,
    pub resources: &'a dyn Resources,
}

impl fmt::Debug for PrepareCtx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrepareCtx")
            .field("sample_rate", &self.sample_rate)
            .field("max_block", &self.max_block)
            .field("seed", &self.seed)
            .field("node_key", &self.node_key)
            .finish()
    }
}

/// Which part of a split node is running (only `Feedback` sees anything but `Main`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Main,
    /// Write last block's stored input to the outputs (inputs unavailable).
    FeedbackRead,
    /// Store this block's input (outputs unavailable).
    FeedbackWrite,
}

/// Engine figures a node can read while processing (the `Info` node).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EngineInfo {
    /// Last block's processing time as a fraction of its real-time budget
    /// (0 offline, where nothing is timed).
    pub load: f32,
    /// Blocks that overran their budget, plus device under/overruns.
    pub xruns: u64,
    /// Total output latency of the running graph, in samples.
    pub latency: u32,
}

/// Per-block context.
#[derive(Clone, Copy, Debug)]
pub struct ProcessCtx<'a> {
    /// Frames in this block (at most `PrepareCtx::max_block`, at least 1).
    pub frames: usize,
    pub sample_rate: f64,
    pub transport: TransportView<'a>,
    pub phase: Phase,
    pub engine: EngineInfo,
}

/// A node: one primitive processing step. Every built-in node implements
/// this in its own file (DECISIONS D10).
///
/// `process`, `reset`, and `take_over` run on the audio thread: no
/// allocation, locks, I/O, syscalls, or panics (ARCHITECTURE §8).
pub trait Node: std::any::Any + Send {
    /// Static metadata: name, category, docs, keywords for search.
    fn meta() -> NodeMeta
    where
        Self: Sized;

    /// Sockets, given this instance's config. May be dynamic (N inputs).
    /// Inputs and outputs are indexed at runtime by their position here.
    fn describe(config: &Config) -> Signature
    where
        Self: Sized;

    /// Build runtime state for a given sample rate and max block size.
    /// Allocate everything `process` will need here.
    fn new(config: &Config, ctx: &PrepareCtx) -> Self
    where
        Self: Sized;

    /// Process one block.
    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io);

    /// Samples of delay this node introduces (for latency compensation).
    fn latency(&self) -> u32 {
        0
    }

    /// A problem the node found while it was created (a plugin that isn't
    /// installed), shown as a compile warning on the node for as long as
    /// this instance runs. Called on the control thread after `new`.
    fn warning(&self) -> Option<String> {
        None
    }

    /// Streaming (can run live) or WholeInput (needs the entire input first).
    /// The compiler goes by [`NodeMeta::whole_input`]; this mirrors it.
    fn causality(&self) -> Causality {
        Causality::Streaming
    }

    /// For whole-input nodes ([`NodeMeta::whole_input`]): compute every
    /// output from every input at once. Runs off the audio thread, so it may
    /// allocate; it must still not panic. The engine streams the result.
    fn render_whole(_config: &Config, ctx: &WholeCtx, inputs: &WholeInputs) -> WholeOutputs
    where
        Self: Sized,
    {
        let _ = (ctx, inputs);
        WholeOutputs::default()
    }

    /// For latency probes ([`NodeMeta::latency_probe`]): how far behind the
    /// sources this node's output runs, in samples (the slowest path into
    /// it plus its own latency), which is what latency compensation lines
    /// other paths up to. Called on the control thread after `new`.
    fn path_latency(&mut self, samples: u32) {
        let _ = samples;
    }

    /// For source nodes: can the output be computed ahead of time?
    fn determinism(&self) -> Determinism {
        Determinism::Deterministic
    }

    /// Return to the initial state (transport restart from zero). No allocation.
    fn reset(&mut self);

    /// Continue where `previous` left off: it's the node this one replaces
    /// at the same place in the graph, of the same kind, rebuilt because
    /// its config changed (a cell switched on a step grid while playing).
    /// Keep what should outlive the edit, such as whether a sequencer plays
    /// and which step it's on, and return `true`; the new node then isn't
    /// treated as just created (no *On load* start). The default keeps
    /// nothing (DECISIONS D196). Downcast `previous` with
    /// `(previous as &dyn Any).downcast_ref::<Self>()`.
    fn take_over(&mut self, previous: &dyn Node) -> bool {
        let _ = previous;
        false
    }
}

/// Declares a node struct and captures its doc comment for the generated
/// reference and in-app help.
///
/// ```ignore
/// node! {
///     kind: "gain",
///     name: "Gain",
///     category: Level,
///     keywords: ["volume", "amp"],
///     /// Scales a signal by a gain in decibels.
///     pub struct Gain { smoothed: f32 }
/// }
/// // Then: impl Node for Gain { fn meta() -> NodeMeta { Self::base_meta() } ... }
/// ```
#[macro_export]
macro_rules! node {
    (
        kind: $kind:literal,
        name: $name:literal,
        category: $cat:ident,
        keywords: [$($kw:literal),* $(,)?],
        $(#[doc = $doc:literal])*
        $(#[derive($($derive:path),* $(,)?)])?
        $vis:vis struct $ty:ident { $($body:tt)* }
    ) => {
        $(#[doc = $doc])*
        $(#[derive($($derive),*)])?
        $vis struct $ty { $($body)* }

        impl $ty {
            /// Registry key.
            #[allow(dead_code)]
            pub const KIND: &'static str = $kind;
            /// The doc comment, verbatim.
            pub const DOC: &'static str = concat!($($doc, "\n"),*);

            /// Metadata from the macro header; extend it in `Node::meta`.
            pub fn base_meta() -> $crate::NodeMeta {
                $crate::NodeMeta::new($kind, $name, $crate::Category::$cat)
                    .with_doc(Self::DOC)
                    .with_keywords(&[$($kw),*])
            }
        }
    };
}
