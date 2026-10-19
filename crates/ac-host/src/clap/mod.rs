//! CLAP plugins: loading a bundle, describing its plugins for the scanner,
//! and creating running instances for `Plugin` nodes.

pub(crate) mod host;
pub(crate) mod processor;

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use ac_core::{PluginDesc, PluginFormat, PluginParam, PluginPort, PluginProcessor, PluginRequest};
use clack_extensions::audio_ports::AudioPortInfoBuffer;
use clack_extensions::note_ports::{NoteDialects, NotePortInfoBuffer};
use clack_extensions::params::{ParamInfoBuffer, ParamInfoFlags};
use clack_host::entry::EntryDescriptor;
use clack_host::prelude::*;

use self::host::{AcClap, ClapMain, ClapShared, Extensions, Requests, host_info};
use self::processor::{ClapProcessor, Layout, Notes, PARAM_RING};
use crate::registry::{self, Entry, MainHalf};

/// The binary a CLAP bundle path names (on macOS a bundle folder holds
/// it; elsewhere the `.clap` file is the binary).
fn binary(path: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    if path.is_dir() {
        return crate::library::bundle_executable(path).unwrap_or_else(|| {
            let stem = path.file_stem().unwrap_or_default();
            path.join("Contents").join("MacOS").join(stem)
        });
    }
    path.to_path_buf()
}

/// Load a CLAP bundle (cached: loading one twice gives the same entry).
/// Its library stays loaded after the entry goes; the entry's `deinit`
/// still runs when the last user of it does (D139).
#[allow(unsafe_code)]
pub(crate) fn load_entry(path: &Path) -> Result<PluginEntry, String> {
    crate::gui::prepare_display();
    let bin = binary(path);
    // SAFETY: loading a plugin library runs its initialization code, which
    // can do anything; there is no safe way to host native plugins in
    // process. The path comes from the scanner or the project, and the
    // person chose to use the plugin (DECISIONS D135 on isolation).
    let resident = unsafe { crate::library::open(&bin) }?;
    // SAFETY: `clap_entry` is the CLAP entry point, a `clap_plugin_entry`
    // the library exports; the library is never unloaded (D139), so the
    // descriptor lives as long as the process.
    let entry: &'static EntryDescriptor = unsafe {
        let symbol = resident
            .library
            .get::<*const EntryDescriptor>(b"clap_entry\0")
            .map_err(|_| "it isn't a CLAP plugin (no clap_entry)".to_string())?;
        let ptr = *symbol;
        ptr.as_ref().ok_or("its clap_entry is empty")?
    };
    let bundle = CString::new(path.to_string_lossy().into_owned())
        .map_err(|_| "the path has a NUL in it")?;
    // SAFETY: as above; `entry` is the library's own descriptor.
    unsafe { PluginEntry::load_from_raw(entry, &bundle) }
        .map_err(|e| format!("couldn't start {}: {e}", path.display()))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .trim()
        .to_string()
}

/// Every plugin in a CLAP bundle, described by creating each once.
pub(crate) fn describe_bundle(path: &Path) -> Result<Vec<PluginDesc>, String> {
    let entry = load_entry(path)?;
    let factory = entry
        .get_plugin_factory()
        .ok_or("the bundle has no plugin factory")?;
    let mut out = Vec::new();
    for d in factory.plugin_descriptors() {
        let Some(id) = d.id() else { continue };
        let mut desc = PluginDesc {
            format: PluginFormat::Clap,
            id: id.to_string_lossy().into_owned(),
            name: d
                .name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            vendor: d
                .vendor()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            version: d
                .version()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            instrument: d
                .features()
                .any(|f| f.to_bytes() == b"instrument" || f.to_bytes() == b"synthesizer"),
            ..PluginDesc::default()
        };
        if desc.name.is_empty() {
            desc.name = desc.id.clone();
        }
        let mut instance = new_instance(&entry, &desc.id, Arc::default())?;
        let ext = instance.access_shared_handler(|h| h.extensions());
        let layout = layout(&mut instance, &ext);
        desc.audio_in = ports(&mut instance, &ext, true);
        desc.audio_out = ports(&mut instance, &ext, false);
        debug_assert_eq!(desc.audio_in.len(), layout.ins.len());
        desc.midi_in = layout.notes != Notes::None;
        desc.midi_out = note_port_count(&mut instance, &ext, false) > 0;
        desc.params = params(&mut instance, &ext);
        PluginDesc::assign_sockets(&mut desc.params);
        desc.repair();
        out.push(desc);
    }
    Ok(out)
}

fn new_instance(
    entry: &PluginEntry,
    id: &str,
    requests: Arc<Requests>,
) -> Result<PluginInstance<AcClap>, String> {
    let cid = CString::new(id).map_err(|_| "the plugin id has a NUL byte".to_string())?;
    let info = host_info().ok_or("bad host info")?;
    PluginInstance::<AcClap>::new(
        move |_| ClapShared::new(requests),
        |shared| ClapMain::new(shared),
        entry,
        &cid,
        &info,
    )
    .map_err(|e| format!("couldn't create it: {e}"))
}

fn ports(instance: &mut PluginInstance<AcClap>, ext: &Extensions, input: bool) -> Vec<PluginPort> {
    let Some(ap) = ext.audio_ports else {
        return Vec::new();
    };
    let handle = instance.plugin_handle();
    let mut buf = AudioPortInfoBuffer::new();
    (0..ap.count(&handle, input))
        .filter_map(|i| {
            ap.get(&handle, i, input, &mut buf).map(|p| PluginPort {
                name: text(p.name),
                channels: p.channel_count.max(1),
            })
        })
        .take(ac_core::plugin::MAX_PLUGIN_PORTS)
        .collect()
}

fn note_port_count(instance: &mut PluginInstance<AcClap>, ext: &Extensions, input: bool) -> u32 {
    ext.note_ports
        .map_or(0, |np| np.count(&instance.plugin_handle(), input))
}

/// How the plugin's first note input port wants notes.
fn notes(instance: &mut PluginInstance<AcClap>, ext: &Extensions) -> Notes {
    let Some(np) = ext.note_ports else {
        return Notes::None;
    };
    let handle = instance.plugin_handle();
    if np.count(&handle, true) == 0 {
        return Notes::None;
    }
    let mut buf = NotePortInfoBuffer::new();
    let Some(info) = np.get(&handle, 0, true, &mut buf) else {
        return Notes::None;
    };
    let d = info.supported_dialects;
    if d.contains(NoteDialects::CLAP) {
        Notes::Clap {
            midi: d.contains(NoteDialects::MIDI),
        }
    } else if d.intersects(NoteDialects::MIDI | NoteDialects::MIDI_MPE) {
        Notes::Midi
    } else {
        Notes::None
    }
}

fn layout(instance: &mut PluginInstance<AcClap>, ext: &Extensions) -> Layout {
    let ch = |v: Vec<PluginPort>| v.into_iter().map(|p| p.channels).collect();
    let ins = ch(ports(instance, ext, true));
    let outs = ch(ports(instance, ext, false));
    Layout {
        ins,
        outs,
        notes: notes(instance, ext),
    }
}

/// The parameters a node gets sockets for: visible, writable ones, in the
/// plugin's order, up to the cap.
fn params(instance: &mut PluginInstance<AcClap>, ext: &Extensions) -> Vec<PluginParam> {
    let Some(p) = ext.params else {
        return Vec::new();
    };
    let handle = instance.plugin_handle();
    let mut buf = ParamInfoBuffer::new();
    let mut out = Vec::new();
    for i in 0..p.count(&handle) {
        let Some(info) = p.get_info(&handle, i, &mut buf) else {
            continue;
        };
        if info
            .flags
            .intersects(ParamInfoFlags::IS_HIDDEN | ParamInfoFlags::IS_READONLY)
        {
            continue;
        }
        out.push(PluginParam {
            id: info.id.get(),
            name: text(info.name),
            socket: String::new(),
            min: info.min_value,
            max: info.max_value,
            default: info.default_value,
            stepped: info.flags.contains(ParamInfoFlags::IS_STEPPED),
        });
        if out.len() >= ac_core::plugin::MAX_PLUGIN_PARAMS {
            break;
        }
    }
    out
}

/// The main-thread half of a running CLAP instance, kept in
/// [`crate::registry`] on the thread that created it.
pub(crate) struct ClapInstance {
    pub instance: PluginInstance<AcClap>,
    pub ext: Extensions,
    /// The plugin's id for each parameter of the node's description.
    pub param_ids: Vec<Option<ClapId>>,
    pub gui: Option<crate::gui::ClapGui>,
}

/// Create, restore, and activate an instance for a node, register its
/// main half, and return its audio half.
pub(crate) fn instantiate(
    entry: &PluginEntry,
    req: &PluginRequest<'_>,
    state: Option<&[u8]>,
) -> Result<Box<dyn PluginProcessor>, String> {
    let has_it = entry.get_plugin_factory().is_some_and(|f| {
        f.plugin_descriptors().any(|d| {
            d.id()
                .is_some_and(|id| id.to_bytes() == req.desc.id.as_bytes())
        })
    });
    if !has_it {
        return Err("isn't installed on this computer".into());
    }
    let requests = Arc::new(Requests::default());
    let mut instance = new_instance(entry, &req.desc.id, requests.clone())?;
    let ext = instance.access_shared_handler(|h| h.extensions());
    if let (Some(bytes), Some(st)) = (state, ext.state) {
        let mut r = std::io::Cursor::new(bytes);
        if st.load(&instance.plugin_handle(), &mut r).is_err() {
            eprintln!(
                "warning: {} didn't accept its saved state; it starts fresh",
                req.desc.label()
            );
        }
        // Loading a state is not a change to the project.
        requests.dirty.store(false, Ordering::Release);
    }
    let layout = layout(&mut instance, &ext);
    let param_ids = param_ids(&mut instance, &ext, req.desc);
    let config = PluginAudioConfiguration {
        sample_rate: req.sample_rate,
        min_frames_count: 1,
        max_frames_count: u32::try_from(req.max_block.max(1)).unwrap_or(u32::MAX),
    };
    let stopped = instance
        .activate(|_, _| (), config)
        .map_err(|e| format!("failed to start: {e}"))?;
    // Latency is read after activation; a change reported during it is
    // already in that figure.
    let latency = ext.latency.map_or(0, |l| l.get(&instance.plugin_handle()));
    requests.latency.store(false, Ordering::Release);
    requests.restart.store(false, Ordering::Release);
    let (tx, rx) = rtrb::RingBuffer::new(PARAM_RING);
    let id = registry::next_id();
    let sockets = req.desc.params.iter().map(|p| p.socket.clone()).collect();
    let proc = ClapProcessor::new(
        stopped,
        id,
        requests.clone(),
        &layout,
        param_ids.clone(),
        latency,
        req.max_block,
        tx,
    );
    registry::insert(Entry {
        id,
        node_key: req.node_key.to_string(),
        title: format!("{} — {}", req.desc.name, req.name),
        sockets,
        requests,
        from_audio: rx,
        half: MainHalf::Clap(Box::new(ClapInstance {
            instance,
            ext,
            param_ids,
            gui: None,
        })),
    });
    Ok(Box::new(proc))
}

/// Map the description's parameters onto the plugin's by id; one the
/// plugin no longer has is left unmapped (its socket does nothing).
fn param_ids(
    instance: &mut PluginInstance<AcClap>,
    ext: &Extensions,
    desc: &PluginDesc,
) -> Vec<Option<ClapId>> {
    let mut have = Vec::new();
    if let Some(p) = ext.params {
        let handle = instance.plugin_handle();
        let mut buf = ParamInfoBuffer::new();
        for i in 0..p.count(&handle) {
            if let Some(info) = p.get_info(&handle, i, &mut buf) {
                have.push(info.id.get());
            }
        }
    }
    desc.params
        .iter()
        .map(|p| {
            have.contains(&p.id)
                .then(|| ClapId::from_raw(p.id))
                .flatten()
        })
        .collect()
}

impl Drop for ClapInstance {
    fn drop(&mut self) {
        // The plugin's view goes before the plugin, and the host window
        // after the view (CLAP: destroy the GUI before the instance).
        crate::gui::close_clap(self);
    }
}

impl ClapInstance {
    /// The plugin's state, as it saves it.
    pub fn save_state(&mut self) -> Option<Vec<u8>> {
        let st = self.ext.state?;
        let mut out = Vec::new();
        st.save(&self.instance.plugin_handle(), &mut out).ok()?;
        Some(out)
    }

    /// Current values of the description's parameters, by index.
    pub fn values(&mut self) -> Vec<(usize, f64)> {
        let Some(p) = self.ext.params else {
            return Vec::new();
        };
        let handle = self.instance.plugin_handle();
        self.param_ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| Some((i, p.get_value(&handle, (*id)?)?)))
            .collect()
    }

    /// Run timers that are due and file descriptors that are ready.
    pub fn service(&mut self) {
        let now = std::time::Instant::now();
        let due: Vec<u32> = self.instance.access_handler(|h| {
            let mut timers = h.timers.borrow_mut();
            let mut due = Vec::new();
            for t in timers.iter_mut() {
                if t.due <= now {
                    due.push(t.id);
                    let period = std::time::Duration::from_millis(t.period_ms.into());
                    t.due = (t.due + period).max(now);
                }
            }
            due
        });
        if let Some(timer) = self.ext.timer {
            for id in due {
                timer.on_timer(
                    &self.instance.plugin_handle(),
                    clack_extensions::timer::TimerId(id),
                );
            }
        }
        #[cfg(unix)]
        self.service_fds();
    }

    #[cfg(unix)]
    #[allow(unsafe_code)]
    fn service_fds(&mut self) {
        use clack_extensions::posix_fd::FdFlags;
        let Some(ext) = self.ext.posix_fd else { return };
        let fds: Vec<(i32, FdFlags)> = self.instance.access_handler(|h| h.fds.borrow().clone());
        if fds.is_empty() {
            return;
        }
        let mut polls: Vec<libc::pollfd> = fds
            .iter()
            .map(|&(fd, f)| libc::pollfd {
                fd,
                events: (if f.contains(FdFlags::READ) {
                    libc::POLLIN
                } else {
                    0
                }) | (if f.contains(FdFlags::WRITE) {
                    libc::POLLOUT
                } else {
                    0
                }),
                revents: 0,
            })
            .collect();
        // SAFETY: `polls` is a valid, exclusively borrowed array of
        // `pollfd`s whose length is passed with it; a zero timeout never
        // blocks. `poll` is the only way to ask the OS about descriptors.
        let n = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, 0) };
        if n <= 0 {
            return;
        }
        for p in polls {
            if p.revents == 0 {
                continue;
            }
            let mut flags = FdFlags::empty();
            if p.revents & libc::POLLIN != 0 {
                flags |= FdFlags::READ;
            }
            if p.revents & libc::POLLOUT != 0 {
                flags |= FdFlags::WRITE;
            }
            if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                flags |= FdFlags::ERROR;
            }
            ext.on_fd(&self.instance.plugin_handle(), p.fd, flags);
        }
    }
}
