//! VST3 plugins, through the MIT/Apache `vst3` bindings (DECISIONS D118,
//! D137): loading a bundle's module, describing its audio classes for the
//! scanner, and creating running instances for `Plugin` nodes, with the
//! same thread split as CLAP (D132): the component and controller live on
//! the creating thread in `crate::registry`; the audio processor goes with
//! the node.
//!
//! VST3 parameters are normalized (0–1), and so are a VST3 node's
//! parameter sockets. MIDI controllers, channel pressure, and pitch bend
//! reach a plugin as the parameters its `IMidiMapping` assigns them;
//! VST3 has no MIDI events of its own.

#![allow(unsafe_code)]

mod com;
mod module;
mod process;

use std::ffi::{CString, c_char, c_void};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use ac_core::{PluginDesc, PluginFormat, PluginParam, PluginPort, PluginProcessor, PluginRequest};
use vst3::Steinberg::Vst::{
    BusDirections_, BusInfo, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler,
    IComponentTrait, IConnectionPoint, IConnectionPointTrait, IEditController,
    IEditControllerTrait, IMidiMapping, IMidiMappingTrait, MediaTypes_, ParameterInfo,
    ParameterInfo_::ParameterFlags_, ProcessModes_, ProcessSetup, RestartFlags_,
    SymbolicSampleSizes_,
};
use vst3::Steinberg::{
    FUnknown, IBStream, IPlugFrame, IPlugView, IPlugViewTrait, IPluginBaseTrait, IPluginFactory,
    IPluginFactory2, IPluginFactory2Trait, IPluginFactoryTrait, PClassInfo, PClassInfo2,
    PFactoryInfo, TUID, ViewRect, kResultOk, kResultTrue,
};
use vst3::{ComPtr, ComWrapper, Interface};

use self::com::{ComponentHandler, HostApp, PlugFrame, Stream, read_string128};
pub(crate) use self::module::Module;
use self::process::{MidiMap, Setup, Vst3Processor};
use crate::clap::host::Requests;
use crate::registry::{self, Entry, MainHalf};

fn text(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

/// A class id as text: 32 hex digits in the canonical (non-COM) byte
/// order, so a project names the same plugin on every system.
pub(crate) fn tuid_to_string(t: &TUID) -> String {
    let b: Vec<u8> = t.iter().map(|c| *c as u8).collect();
    let order: [usize; 16] = if cfg!(windows) {
        [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15]
    } else {
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    };
    order.iter().map(|&i| format!("{:02X}", b[i])).collect()
}

/// The class id `tuid_to_string` wrote, in this system's byte order.
pub(crate) fn tuid_from_string(s: &str) -> Option<TUID> {
    if s.len() != 32 || !s.is_ascii() {
        return None;
    }
    let mut canon = [0u8; 16];
    for (i, b) in canon.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    let order: [usize; 16] = if cfg!(windows) {
        [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15]
    } else {
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    };
    let mut t: TUID = [0; 16];
    for (k, &i) in order.iter().enumerate() {
        t[i] = canon[k] as c_char;
    }
    Some(t)
}

fn iid_of<I: Interface>() -> TUID {
    I::IID.map(|b| b as c_char)
}

/// Create `cid` as interface `I` from the factory.
fn create<I: Interface>(factory: &ComPtr<IPluginFactory>, cid: &TUID) -> Option<ComPtr<I>> {
    let iid = iid_of::<I>();
    let mut obj: *mut c_void = std::ptr::null_mut();
    // SAFETY: valid class and interface ids and an out pointer the factory
    // fills with an owned reference.
    let r = unsafe { factory.createInstance(cid.as_ptr(), iid.as_ptr(), &mut obj) };
    if r != kResultOk || obj.is_null() {
        return None;
    }
    // SAFETY: createInstance returned an owned pointer to an `I`.
    unsafe { ComPtr::<I>::from_raw(obj.cast()) }
}

/// The audio classes a factory exports: (class id, name, vendor,
/// version, sub-categories).
fn audio_classes(factory: &ComPtr<IPluginFactory>) -> Vec<(TUID, String, String, String, String)> {
    // SAFETY: plain calls through a valid factory; the info structs are
    // plain data it fills.
    let mut finfo: PFactoryInfo = unsafe { std::mem::zeroed() };
    let factory_vendor = if unsafe { factory.getFactoryInfo(&mut finfo) } == kResultOk {
        text(&finfo.vendor)
    } else {
        String::new()
    };
    let f2 = factory.cast::<IPluginFactory2>();
    let count = unsafe { factory.countClasses() };
    let mut out = Vec::new();
    for i in 0..count {
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        if unsafe { factory.getClassInfo(i, &mut info) } != kResultOk {
            continue;
        }
        if text(&info.category) != "Audio Module Class" {
            continue;
        }
        let (mut vendor, mut version, mut subs) =
            (factory_vendor.clone(), String::new(), String::new());
        if let Some(f2) = &f2 {
            let mut i2: PClassInfo2 = unsafe { std::mem::zeroed() };
            if unsafe { f2.getClassInfo2(i, &mut i2) } == kResultOk {
                let v = text(&i2.vendor);
                if !v.is_empty() {
                    vendor = v;
                }
                version = text(&i2.version);
                subs = text(&i2.subCategories);
            }
        }
        out.push((info.cid, text(&info.name), vendor, version, subs));
    }
    out
}

/// A component and its controller, initialized (and connected when they
/// are separate objects).
struct Parts {
    component: ComPtr<IComponent>,
    controller: Option<ComPtr<IEditController>>,
    /// The controller is its own object, initialized and terminated
    /// separately.
    separate: bool,
    connection: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    /// Kept for as long as the plugin may call it.
    _host: ComWrapper<HostApp>,
}

fn host_unknown(host: &ComWrapper<HostApp>) -> *mut FUnknown {
    host.as_com_ref::<vst3::Steinberg::Vst::IHostApplication>()
        .map_or(std::ptr::null_mut(), |r| r.as_ptr().cast())
}

fn make_parts(factory: &ComPtr<IPluginFactory>, cid: &TUID) -> Result<Parts, String> {
    let component = create::<IComponent>(factory, cid).ok_or("couldn't create it")?;
    let host = ComWrapper::new(HostApp);
    // SAFETY: initializing a fresh component with our host object, on the
    // thread that will be its main thread.
    if unsafe { component.initialize(host_unknown(&host)) } != kResultOk {
        return Err("it failed to initialize".into());
    }
    let (controller, separate) = match component.cast::<IEditController>() {
        Some(c) => (Some(c), false),
        None => {
            let mut ccid: TUID = [0; 16];
            // SAFETY: an out TUID the component fills.
            let has = unsafe { component.getControllerClassId(&mut ccid) } == kResultOk;
            let c = has
                .then(|| create::<IEditController>(factory, &ccid))
                .flatten();
            if let Some(c) = &c {
                // SAFETY: as for the component.
                if unsafe { c.initialize(host_unknown(&host)) } != kResultOk {
                    return Err("its controller failed to initialize".into());
                }
            }
            (c, true)
        }
    };
    let connection = if separate {
        match (
            component.cast::<IConnectionPoint>(),
            controller
                .as_ref()
                .and_then(|c| c.cast::<IConnectionPoint>()),
        ) {
            (Some(a), Some(b)) => {
                // SAFETY: two valid connection points of one plugin.
                unsafe {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                }
                Some((a, b))
            }
            _ => None,
        }
    } else {
        None
    };
    let parts = Parts {
        component,
        controller,
        separate,
        connection,
        _host: host,
    };
    // The controller learns the component's state (its defaults now).
    if let Some(c) = &parts.controller {
        let s = ComWrapper::new(Stream::default());
        if let Some(sp) = s.as_com_ref::<IBStream>() {
            // SAFETY: a valid stream object for the duration of the calls.
            unsafe {
                if parts.component.getState(sp.as_ptr()) == kResultOk {
                    s.rewind();
                    c.setComponentState(sp.as_ptr());
                }
            }
        }
    }
    Ok(parts)
}

impl Parts {
    fn buses(&self, audio: bool, input: bool) -> Vec<BusInfo> {
        let media = if audio {
            MediaTypes_::kAudio
        } else {
            MediaTypes_::kEvent
        } as i32;
        let dir = if input {
            BusDirections_::kInput
        } else {
            BusDirections_::kOutput
        } as i32;
        // SAFETY: plain queries of an initialized component.
        let n = unsafe { self.component.getBusCount(media, dir) };
        (0..n)
            .filter_map(|i| {
                let mut info: BusInfo = unsafe { std::mem::zeroed() };
                (unsafe { self.component.getBusInfo(media, dir, i, &mut info) } == kResultOk)
                    .then_some(info)
            })
            .take(ac_core::plugin::MAX_PLUGIN_PORTS)
            .collect()
    }

    fn params(&self) -> Vec<(u32, String, f64, bool, i32)> {
        let Some(c) = &self.controller else {
            return Vec::new();
        };
        // SAFETY: plain queries of an initialized controller.
        let n = unsafe { c.getParameterCount() };
        let mut out = Vec::new();
        for i in 0..n {
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            if unsafe { c.getParameterInfo(i, &mut info) } != kResultOk {
                continue;
            }
            out.push((
                info.id,
                read_string128(&info.title),
                info.defaultNormalizedValue,
                info.stepCount > 0,
                info.flags,
            ));
        }
        out
    }

    fn terminate(self) {
        // SAFETY: tearing down in the reverse order of `make_parts`, on the
        // thread that made them.
        unsafe {
            if let Some((a, b)) = &self.connection {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(c) = &self.controller {
                c.setComponentHandler(std::ptr::null_mut());
                if self.separate {
                    c.terminate();
                }
            }
            self.component.terminate();
        }
    }
}

/// Every audio class in a VST3 bundle, described by creating each once.
pub(crate) fn describe_bundle(path: &Path) -> Result<Vec<PluginDesc>, String> {
    let module = Module::load(path)?;
    let mut out = Vec::new();
    for (cid, name, vendor, version, subs) in audio_classes(module.factory()) {
        let parts = make_parts(module.factory(), &cid)?;
        let ports = |audio: bool, input: bool| -> Vec<PluginPort> {
            parts
                .buses(audio, input)
                .iter()
                .map(|b| PluginPort {
                    name: read_string128(&b.name),
                    channels: b.channelCount.max(1) as u32,
                })
                .collect()
        };
        let mut params: Vec<PluginParam> = parts
            .params()
            .into_iter()
            .filter(|p| {
                p.4 & (ParameterFlags_::kIsReadOnly | ParameterFlags_::kIsHidden) as i32 == 0
            })
            .map(|(id, name, default, stepped, _)| PluginParam {
                id,
                name,
                socket: String::new(),
                min: 0.0,
                max: 1.0,
                default,
                stepped,
            })
            .take(ac_core::plugin::MAX_PLUGIN_PARAMS)
            .collect();
        PluginDesc::assign_sockets(&mut params);
        let mut desc = PluginDesc {
            format: PluginFormat::Vst3,
            id: tuid_to_string(&cid),
            name,
            vendor,
            version,
            path: path.to_string_lossy().into_owned(),
            instrument: subs
                .split('|')
                .any(|s| s.eq_ignore_ascii_case("Instrument")),
            audio_in: ports(true, true),
            audio_out: ports(true, false),
            midi_in: !parts.buses(false, true).is_empty(),
            midi_out: !parts.buses(false, false).is_empty(),
            params,
        };
        desc.repair();
        parts.terminate();
        out.push(desc);
    }
    Ok(out)
}

/// The main-thread half of a running VST3 instance.
pub(crate) struct Vst3Instance {
    parts: Option<Parts>,
    handler: ComWrapper<ComponentHandler>,
    pub param_ids: Vec<Option<u32>>,
    /// Output parameter changes the audio half saw (id, value).
    from_audio: rtrb::Consumer<(u32, f64)>,
    pub view: Option<View>,
    /// Kept loaded until the instance is gone.
    _module: Arc<Module>,
}

/// An open plugin window.
pub(crate) struct View {
    view: ComPtr<IPlugView>,
    frame: ComWrapper<PlugFrame>,
    window: crate::gui::HostWindow,
}

/// What a running instance needs from `Setup` and the request.
fn midi_map(controller: &ComPtr<IEditController>) -> Option<MidiMap> {
    let mm = controller.cast::<IMidiMapping>()?;
    let mut map: MidiMap = Box::new([[None; 130]; 16]);
    for (ch, row) in map.iter_mut().enumerate() {
        for (n, slot) in row.iter_mut().enumerate() {
            let mut id = 0u32;
            // SAFETY: a plain query with an out id.
            if unsafe { mm.getMidiControllerAssignment(0, ch as i16, n as i16, &mut id) }
                == kResultTrue
            {
                *slot = Some(id);
            }
        }
    }
    Some(map)
}

/// The state blob: the component's state and the controller's, each with
/// its length (little-endian u64) first, after a 4-byte tag.
const STATE_TAG: &[u8; 4] = b"AV3S";

fn split_state(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = bytes.strip_prefix(STATE_TAG)?;
    let take = |b: &[u8]| -> Option<(usize, usize)> {
        let n = u64::from_le_bytes(b.get(..8)?.try_into().ok()?) as usize;
        (b.len() >= 8 + n).then_some((8, 8 + n))
    };
    let (a0, a1) = take(rest)?;
    let comp = &rest[a0..a1];
    let rest = &rest[a1..];
    let (b0, b1) = take(rest)?;
    Some((comp, &rest[b0..b1]))
}

/// Create, restore, set up, and activate an instance for a node.
pub(crate) fn instantiate(
    module: &Arc<Module>,
    req: &PluginRequest<'_>,
    state: Option<&[u8]>,
) -> Result<Box<dyn PluginProcessor>, String> {
    let cid = tuid_from_string(&req.desc.id).ok_or("isn't installed on this computer")?;
    let has_it = audio_classes(module.factory()).iter().any(|c| c.0 == cid);
    if !has_it {
        return Err("isn't installed on this computer".into());
    }
    let parts = make_parts(module.factory(), &cid)?;
    let processor = parts
        .component
        .cast::<IAudioProcessor>()
        .ok_or("it has no audio processor")?;
    if let Some((comp, ctrl)) = state.and_then(split_state) {
        let restored = (|| {
            let s = ComWrapper::new(Stream::with(comp));
            let sp = s.as_com_ref::<IBStream>()?;
            // SAFETY: valid stream objects for the duration of the calls,
            // on the instance's main thread, before activation.
            unsafe {
                if parts.component.setState(sp.as_ptr()) != kResultOk {
                    return None;
                }
                if let Some(c) = &parts.controller {
                    s.rewind();
                    c.setComponentState(sp.as_ptr());
                    let s2 = ComWrapper::new(Stream::with(ctrl));
                    let sp2 = s2.as_com_ref::<IBStream>()?;
                    c.setState(sp2.as_ptr());
                }
            }
            Some(())
        })();
        if restored.is_none() {
            eprintln!(
                "warning: {} didn't accept its saved state; it starts fresh",
                req.desc.label()
            );
        }
    }
    let audio_ins = parts.buses(true, true);
    let audio_outs = parts.buses(true, false);
    let event_ins = parts.buses(false, true);
    let event_outs = parts.buses(false, false);
    let max_block = req.max_block.max(1);
    // SAFETY: setting up and activating an initialized component, on its
    // main thread, as VST3 orders it: buses, processing setup, active.
    unsafe {
        let audio = MediaTypes_::kAudio as i32;
        let event = MediaTypes_::kEvent as i32;
        let (input, output) = (
            BusDirections_::kInput as i32,
            BusDirections_::kOutput as i32,
        );
        for i in 0..audio_ins.len() {
            parts.component.activateBus(audio, input, i as i32, 1);
        }
        for i in 0..audio_outs.len() {
            parts.component.activateBus(audio, output, i as i32, 1);
        }
        if !event_ins.is_empty() {
            parts.component.activateBus(event, input, 0, 1);
        }
        if !event_outs.is_empty() {
            parts.component.activateBus(event, output, 0, 1);
        }
        if processor.canProcessSampleSize(SymbolicSampleSizes_::kSample32 as i32) != kResultTrue {
            return Err("it can't process 32-bit samples".into());
        }
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            maxSamplesPerBlock: i32::try_from(max_block).unwrap_or(i32::MAX),
            sampleRate: req.sample_rate,
        };
        if processor.setupProcessing(&mut setup) != kResultOk {
            return Err("it can't run at this sample rate and block size".into());
        }
        if parts.component.setActive(1) != kResultOk {
            return Err("failed to start".into());
        }
    }
    // SAFETY: a plain query of an active processor.
    let latency = unsafe { processor.getLatencySamples() };
    let have: Vec<u32> = parts.params().iter().map(|p| p.0).collect();
    let param_ids: Vec<Option<u32>> = req
        .desc
        .params
        .iter()
        .map(|p| have.contains(&p.id).then_some(p.id))
        .collect();
    let (edits_tx, edits_rx) = rtrb::RingBuffer::new(1024);
    let (to_main, from_audio) = rtrb::RingBuffer::new(1024);
    let handler = ComWrapper::new(ComponentHandler {
        to_audio: Mutex::new(edits_tx),
        edits: Mutex::new(Vec::new()),
        restart: Default::default(),
    });
    if let (Some(c), Some(h)) = (&parts.controller, handler.as_com_ref::<IComponentHandler>()) {
        // SAFETY: our handler outlives the controller's use of it: it's
        // unset before the controller goes (`Parts::terminate`).
        unsafe { c.setComponentHandler(h.as_ptr()) };
    }
    let midi = parts.controller.as_ref().and_then(midi_map);
    let requests = Arc::new(Requests::default());
    let id = registry::next_id();
    let ch = |b: &[BusInfo]| b.iter().map(|x| x.channelCount.max(0) as u32).collect();
    let proc = Vst3Processor::new(
        processor.clone(),
        id,
        requests.clone(),
        Setup {
            ins: ch(&audio_ins),
            outs: ch(&audio_outs),
            param_ids: param_ids.clone(),
            midi_map: midi,
            latency,
            max_block,
            sample_rate: req.sample_rate,
        },
        edits_rx,
        to_main,
    );
    registry::insert(Entry {
        id,
        node_key: req.node_key.to_string(),
        title: format!("{} — {}", req.desc.name, req.name),
        sockets: req.desc.params.iter().map(|p| p.socket.clone()).collect(),
        requests,
        from_audio: rtrb::RingBuffer::new(1).1,
        half: MainHalf::Vst3(Box::new(Vst3Instance {
            parts: Some(parts),
            handler,
            param_ids,
            from_audio,
            view: None,
            _module: module.clone(),
        })),
    });
    Ok(Box::new(proc))
}

impl Vst3Instance {
    fn index_of(&self, id: u32) -> Option<usize> {
        self.param_ids.iter().position(|p| *p == Some(id))
    }

    /// The state as the project stores it.
    pub fn save_state(&mut self) -> Option<Vec<u8>> {
        let parts = self.parts.as_ref()?;
        let comp = ComWrapper::new(Stream::default());
        let ctrl = ComWrapper::new(Stream::default());
        // SAFETY: valid streams for the duration of the calls, on the main
        // thread.
        unsafe {
            let cp = comp.as_com_ref::<IBStream>()?;
            if parts.component.getState(cp.as_ptr()) != kResultOk {
                return None;
            }
            if let Some(c) = &parts.controller {
                let cc = ctrl.as_com_ref::<IBStream>()?;
                let _ = c.getState(cc.as_ptr());
            }
        }
        let (a, b) = (comp.bytes(), ctrl.bytes());
        let mut out = STATE_TAG.to_vec();
        out.extend((a.len() as u64).to_le_bytes());
        out.extend(a);
        out.extend((b.len() as u64).to_le_bytes());
        out.extend(b);
        Some(out)
    }

    /// Answer the plugin: edits and output changes to report as
    /// `(description index, value)`, and whether it asked to restart or
    /// its values all changed.
    pub fn service(&mut self) -> (Vec<(usize, f64)>, bool) {
        let mut moved = Vec::new();
        let edits: Vec<(u32, f64)> = self
            .handler
            .edits
            .lock()
            .map(|mut e| std::mem::take(&mut *e))
            .unwrap_or_default();
        for (id, v) in edits {
            if let Some(i) = self.index_of(id) {
                moved.push((i, v));
            }
        }
        while let Ok((id, v)) = self.from_audio.pop() {
            if let Some(c) = self.parts.as_ref().and_then(|p| p.controller.as_ref()) {
                // SAFETY: telling the controller (main thread) a value the
                // processor changed, as hosts must.
                unsafe { c.setParamNormalized(id, v) };
            }
            if let Some(i) = self.index_of(id) {
                moved.push((i, v));
            }
        }
        let flags = self.handler.restart.swap(0, Ordering::AcqRel);
        let restart = flags
            & (RestartFlags_::kLatencyChanged
                | RestartFlags_::kIoChanged
                | RestartFlags_::kReloadComponent
                | RestartFlags_::kParamTitlesChanged) as i32
            != 0;
        if flags & RestartFlags_::kParamValuesChanged as i32 != 0
            && let Some(c) = self.parts.as_ref().and_then(|p| p.controller.as_ref())
        {
            for (i, id) in self.param_ids.iter().enumerate() {
                if let Some(id) = id {
                    // SAFETY: a plain query on the main thread.
                    moved.push((i, unsafe { c.getParamNormalized(*id) }));
                }
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(v) = &self.view {
            service_run_loop(&v.frame);
        }
        (moved, restart)
    }

    pub fn has_window(&self) -> bool {
        self.parts
            .as_ref()
            .and_then(|p| p.controller.as_ref())
            .is_some()
    }

    /// Open the plugin's window in a host window.
    pub fn open_window(&mut self, title: &str) -> Result<(), String> {
        if let Some(v) = &self.view {
            v.window.raise();
            return Ok(());
        }
        let controller = self
            .parts
            .as_ref()
            .and_then(|p| p.controller.clone())
            .ok_or("the plugin has no window")?;
        let platform = platform_type().ok_or("plugin windows aren't supported on this system")?;
        // SAFETY: asking the controller for its editor view; a returned
        // view is an owned reference.
        let raw = unsafe { controller.createView(c"editor".as_ptr()) };
        // SAFETY: as above.
        let view =
            unsafe { ComPtr::<IPlugView>::from_raw(raw) }.ok_or("the plugin has no window")?;
        // SAFETY: plain calls on a fresh view, on the main thread.
        unsafe {
            if view.isPlatformTypeSupported(platform.as_ptr()) != kResultTrue {
                return Err("the plugin's window can't open on this system".into());
            }
        }
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: 640,
            bottom: 480,
        };
        // SAFETY: an out rectangle.
        unsafe { view.getSize(&mut rect) };
        let (w, h) = (
            (rect.right - rect.left).max(16) as u32,
            (rect.bottom - rect.top).max(16) as u32,
        );
        // SAFETY: a plain query.
        let resizable = unsafe { view.canResize() } == kResultTrue;
        let window = crate::gui::HostWindow::open(title, w, h, resizable)?;
        let frame = ComWrapper::new(PlugFrame::default());
        // SAFETY: our frame outlives the view's use of it (unset before the
        // view goes, in `close_window`); the parent window outlives the
        // view (dropped after it's removed).
        unsafe {
            if let Some(f) = frame.as_com_ref::<IPlugFrame>() {
                view.setFrame(f.as_ptr());
            }
            if view.attached(window.native_parent(), platform.as_ptr()) != kResultOk {
                view.setFrame(std::ptr::null_mut());
                return Err("the plugin couldn't open in its window".into());
            }
        }
        window.show();
        self.view = Some(View {
            view,
            frame,
            window,
        });
        Ok(())
    }

    pub fn close_window(&mut self) {
        let Some(v) = self.view.take() else { return };
        // SAFETY: removing the view from its parent and frame before either
        // goes, on the main thread.
        unsafe {
            v.view.removed();
            v.view.setFrame(std::ptr::null_mut());
        }
        // The window goes after the view let go of it (as for CLAP).
        drop(v);
    }

    /// Keep an open window serviced; true when it closed.
    pub fn service_window(&mut self) -> bool {
        let Some(v) = self.view.as_mut() else {
            return false;
        };
        let packed = v.frame.resize.swap(0, Ordering::AcqRel);
        if packed != 0 {
            let (w, h) = ((packed >> 32) as u32, (packed & 0xffff_ffff) as u32);
            v.window.resize(w, h);
            let mut r = ViewRect {
                left: 0,
                top: 0,
                right: w as i32,
                bottom: h as i32,
            };
            // SAFETY: telling the view its new size, on the main thread.
            unsafe { v.view.onSize(&mut r) };
        }
        if v.window.poll_closed() {
            self.close_window();
            return true;
        }
        false
    }
}

#[cfg(target_os = "linux")]
fn service_run_loop(frame: &ComWrapper<PlugFrame>) {
    use vst3::Steinberg::Linux::{IEventHandlerTrait, ITimerHandlerTrait};
    let now = std::time::Instant::now();
    let (fds, due) = {
        let mut rl = frame.run_loop.borrow_mut();
        let fds: Vec<_> = rl.fds.iter().map(|(fd, h)| (*fd, h.clone())).collect();
        let mut due = Vec::new();
        for t in rl.timers.iter_mut() {
            if t.1 <= now {
                due.push(t.2.clone());
                t.1 = now + std::time::Duration::from_millis(t.0);
            }
        }
        (fds, due)
    };
    for h in due {
        // SAFETY: a registered timer handler, called on the main thread.
        unsafe { h.onTimer() };
    }
    if fds.is_empty() {
        return;
    }
    let mut polls: Vec<libc::pollfd> = fds
        .iter()
        .map(|(fd, _)| libc::pollfd {
            fd: *fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    // SAFETY: `polls` is a valid array of its length; a zero timeout never
    // blocks.
    let n = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, 0) };
    if n <= 0 {
        return;
    }
    for (p, (fd, h)) in polls.iter().zip(fds) {
        if p.revents != 0 {
            // SAFETY: a registered descriptor handler, on the main thread.
            unsafe { h.onFDIsSet(fd) };
        }
    }
}

fn platform_type() -> Option<CString> {
    let t = if cfg!(target_os = "macos") {
        "NSView"
    } else if cfg!(windows) {
        "HWND"
    } else if cfg!(unix) {
        "X11EmbedWindowID"
    } else {
        return None;
    };
    CString::new(t).ok()
}

impl Drop for Vst3Instance {
    fn drop(&mut self) {
        self.close_window();
        if let Some(parts) = self.parts.take() {
            // SAFETY: deactivating on the main thread; the audio half has
            // stopped processing and let go first (D132).
            unsafe { parts.component.setActive(0) };
            parts.terminate();
        }
    }
}
