//! The host's side of the VST3 conversation: the COM objects a plugin
//! calls back into. Each is a Rust value behind the `vst3` crate's
//! `ComWrapper` (reference counted, its interfaces laid out for C).
//!
//! - [`HostApp`]: the host's name, and messages for plugins whose
//!   processor and controller talk through connection points.
//! - [`ComponentHandler`]: edits from the plugin's window and restart
//!   requests, as atomics and rings answered by `crate::poll`.
//! - [`Stream`]: a memory stream for saving and restoring state.
//! - [`PlugFrame`]: the window's frame (resize requests) and, on Linux,
//!   the run loop a plugin's window registers its descriptors and timers
//!   with.
//!
//! Every method is called through a C ABI by the plugin, so each is
//! `unsafe` by the trait's definition; the ones that read the plugin's
//! pointers say what they rely on.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use vst3::Steinberg::Vst::{
    IAttributeList, IAttributeListTrait, IComponentHandler, IComponentHandlerTrait,
    IHostApplication, IHostApplicationTrait, IMessage, IMessageTrait, String128, TChar,
};
use vst3::Steinberg::{
    FIDString, IBStream, IBStreamTrait, IPlugFrame, IPlugFrameTrait, IPlugView, TUID, ViewRect,
    int32, int64, kInvalidArgument, kNoInterface, kResultFalse, kResultOk, tresult, uint32,
};
use vst3::{Class, ComWrapper, Interface};

/// Write `s` into a VST3 UTF-16 string, NUL-terminated and cut to fit.
pub(crate) fn write_string128(dst: &mut String128, s: &str) {
    let mut n = 0;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16().take(127)) {
        *d = c;
        n += 1;
    }
    if let Some(d) = dst.get_mut(n) {
        *d = 0;
    }
}

/// A VST3 UTF-16 string, to its NUL.
pub(crate) fn read_string128(src: &[TChar]) -> String {
    let len = src.iter().position(|c| *c == 0).unwrap_or(src.len());
    String::from_utf16_lossy(&src[..len]).trim().to_string()
}

fn same_iid(iid: *const TUID, guid: &[u8; 16]) -> bool {
    if iid.is_null() {
        return false;
    }
    // SAFETY: a non-null TUID pointer from the plugin points at 16 bytes.
    let iid = unsafe { &*iid };
    iid.iter().zip(guid).all(|(a, b)| *a as u8 == *b)
}

/// The host application object plugins get in `initialize`.
pub(crate) struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: a non-null String128 pointer the plugin owns.
        write_string128(unsafe { &mut *name }, "Audio Constructor");
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        _cid: *mut TUID,
        iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        if obj.is_null() {
            return kInvalidArgument;
        }
        let made: Option<*mut c_void> = if same_iid(iid, &IMessage::IID) {
            ComWrapper::new(Message::default())
                .to_com_ptr::<IMessage>()
                .map(|p| p.into_raw().cast())
        } else if same_iid(iid, &IAttributeList::IID) {
            ComWrapper::new(Attributes::default())
                .to_com_ptr::<IAttributeList>()
                .map(|p| p.into_raw().cast())
        } else {
            None
        };
        match made {
            Some(p) => {
                // SAFETY: `obj` is the plugin's out pointer; the object's
                // one reference passes to it.
                unsafe { *obj = p };
                kResultOk
            }
            None => {
                // SAFETY: as above.
                unsafe { *obj = std::ptr::null_mut() };
                kNoInterface
            }
        }
    }
}

/// A message between a plugin's processor and controller.
#[derive(Default)]
pub(crate) struct Message {
    id: Mutex<Option<std::ffi::CString>>,
    attributes: Mutex<Option<ComWrapper<Attributes>>>,
}

impl Class for Message {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        match self.id.lock() {
            Ok(id) => id.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            Err(_) => std::ptr::null(),
        }
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        let value = if id.is_null() {
            None
        } else {
            // SAFETY: a non-null id is a NUL-terminated string.
            Some(unsafe { CStr::from_ptr(id) }.to_owned())
        };
        if let Ok(mut m) = self.id.lock() {
            *m = value;
        }
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        let Ok(mut a) = self.attributes.lock() else {
            return std::ptr::null_mut();
        };
        let list = a.get_or_insert_with(|| ComWrapper::new(Attributes::default()));
        // Not a new reference: the message keeps the list.
        list.as_com_ref::<IAttributeList>()
            .map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
}

#[derive(Clone)]
enum Attr {
    Int(i64),
    Float(f64),
    Text(Vec<u16>),
    Binary(Vec<u8>),
}

/// The attributes of a [`Message`].
#[derive(Default)]
pub(crate) struct Attributes {
    values: Mutex<HashMap<Vec<u8>, Attr>>,
}

impl Class for Attributes {
    type Interfaces = (IAttributeList,);
}

fn key(id: *const c_char) -> Option<Vec<u8>> {
    if id.is_null() {
        return None;
    }
    // SAFETY: a non-null attribute id is a NUL-terminated string.
    Some(unsafe { CStr::from_ptr(id) }.to_bytes().to_vec())
}

impl Attributes {
    fn set(&self, id: *const c_char, v: Attr) -> tresult {
        match (key(id), self.values.lock()) {
            (Some(k), Ok(mut m)) => {
                m.insert(k, v);
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    fn get(&self, id: *const c_char) -> Option<Attr> {
        let k = key(id)?;
        self.values.lock().ok()?.get(&k).cloned()
    }
}

impl IAttributeListTrait for Attributes {
    unsafe fn setInt(&self, id: *const c_char, value: int64) -> tresult {
        self.set(id, Attr::Int(value))
    }

    unsafe fn getInt(&self, id: *const c_char, value: *mut int64) -> tresult {
        match (self.get(id), value.is_null()) {
            (Some(Attr::Int(v)), false) => {
                // SAFETY: a non-null out pointer from the plugin.
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: *const c_char, value: f64) -> tresult {
        self.set(id, Attr::Float(value))
    }

    unsafe fn getFloat(&self, id: *const c_char, value: *mut f64) -> tresult {
        match (self.get(id), value.is_null()) {
            (Some(Attr::Float(v)), false) => {
                // SAFETY: a non-null out pointer from the plugin.
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: *const c_char, string: *const TChar) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut v = Vec::new();
        let mut p = string;
        // SAFETY: a non-null string is NUL-terminated UTF-16; the cap stops
        // a string that isn't from running on.
        unsafe {
            while *p != 0 && v.len() < 1 << 20 {
                v.push(*p);
                p = p.add(1);
            }
        }
        self.set(id, Attr::Text(v))
    }

    unsafe fn getString(&self, id: *const c_char, string: *mut TChar, size: uint32) -> tresult {
        let Some(Attr::Text(v)) = self.get(id) else {
            return kResultFalse;
        };
        let room = size as usize / 2;
        if string.is_null() || room == 0 {
            return kInvalidArgument;
        }
        let n = v.len().min(room - 1);
        // SAFETY: the plugin gave `size` bytes at `string`; `n + 1` UTF-16
        // units fit in them.
        unsafe {
            std::ptr::copy_nonoverlapping(v.as_ptr(), string, n);
            *string.add(n) = 0;
        }
        kResultOk
    }

    unsafe fn setBinary(&self, id: *const c_char, data: *const c_void, size: uint32) -> tresult {
        let bytes = if data.is_null() || size == 0 {
            Vec::new()
        } else {
            // SAFETY: the plugin gave `size` bytes at `data`.
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) }.to_vec()
        };
        self.set(id, Attr::Binary(bytes))
    }

    unsafe fn getBinary(
        &self,
        id: *const c_char,
        data: *mut *const c_void,
        size: *mut uint32,
    ) -> tresult {
        // The pointer handed out must stay valid while the message lives,
        // so it points into the stored value, which isn't replaced while a
        // reader holds it (a reader copies at once, by convention).
        let Ok(m) = self.values.lock() else {
            return kResultFalse;
        };
        let Some(k) = key(id) else {
            return kInvalidArgument;
        };
        match (m.get(&k), data.is_null() || size.is_null()) {
            (Some(Attr::Binary(b)), false) => {
                // SAFETY: non-null out pointers from the plugin.
                unsafe {
                    *data = b.as_ptr().cast();
                    *size = b.len() as uint32;
                }
                kResultOk
            }
            _ => kResultFalse,
        }
    }
}

/// What a plugin's controller asks of the host, from its window: edits,
/// for the processor and the editor, and restarts.
pub(crate) struct ComponentHandler {
    /// Edits on their way to the processor (main thread → audio thread).
    pub to_audio: Mutex<rtrb::Producer<(u32, f64)>>,
    /// Edits for the editor to follow (read by `poll`).
    pub edits: Mutex<Vec<(u32, f64)>>,
    /// `restartComponent` flags not yet answered.
    pub restart: AtomicI32,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, _id: u32) -> tresult {
        kResultOk
    }

    unsafe fn performEdit(&self, id: u32, value: f64) -> tresult {
        if let Ok(mut p) = self.to_audio.lock() {
            let _ = p.push((id, value));
        }
        if let Ok(mut e) = self.edits.lock() {
            e.retain(|(i, _)| *i != id);
            e.push((id, value));
        }
        kResultOk
    }

    unsafe fn endEdit(&self, _id: u32) -> tresult {
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        self.restart.fetch_or(flags, Ordering::AcqRel);
        kResultOk
    }
}

/// A growable stream in memory, for `getState` and `setState`.
#[derive(Default)]
pub(crate) struct Stream {
    data: RefCell<Vec<u8>>,
    pos: RefCell<usize>,
}

impl Class for Stream {
    type Interfaces = (IBStream,);
}

impl Stream {
    pub fn with(bytes: &[u8]) -> Stream {
        Stream {
            data: RefCell::new(bytes.to_vec()),
            pos: RefCell::new(0),
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.data.borrow().clone()
    }

    pub fn rewind(&self) {
        *self.pos.borrow_mut() = 0;
    }
}

impl IBStreamTrait for Stream {
    unsafe fn read(&self, buffer: *mut c_void, num: int32, read: *mut int32) -> tresult {
        let data = self.data.borrow();
        let mut pos = self.pos.borrow_mut();
        let n = (num.max(0) as usize).min(data.len().saturating_sub(*pos));
        if n > 0 && !buffer.is_null() {
            // SAFETY: the plugin gave room for `num` bytes at `buffer`.
            unsafe { std::ptr::copy_nonoverlapping(data[*pos..].as_ptr(), buffer.cast(), n) };
        }
        *pos += n;
        if !read.is_null() {
            // SAFETY: a non-null out pointer.
            unsafe { *read = n as int32 };
        }
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, num: int32, written: *mut int32) -> tresult {
        if buffer.is_null() || num < 0 {
            return kInvalidArgument;
        }
        let mut data = self.data.borrow_mut();
        let mut pos = self.pos.borrow_mut();
        let n = num as usize;
        // SAFETY: the plugin gave `num` bytes at `buffer`.
        let src = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), n) };
        let end = *pos + n;
        if data.len() < end {
            data.resize(end, 0);
        }
        data[*pos..end].copy_from_slice(src);
        *pos = end;
        if !written.is_null() {
            // SAFETY: a non-null out pointer.
            unsafe { *written = n as int32 };
        }
        kResultOk
    }

    unsafe fn seek(&self, offset: int64, mode: int32, result: *mut int64) -> tresult {
        let len = self.data.borrow().len() as i64;
        let mut pos = self.pos.borrow_mut();
        let base = match mode {
            0 => 0,
            1 => *pos as i64,
            2 => len,
            _ => return kInvalidArgument,
        };
        let to = (base + offset).clamp(0, len);
        *pos = to as usize;
        if !result.is_null() {
            // SAFETY: a non-null out pointer.
            unsafe { *result = to };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: a non-null out pointer.
        unsafe { *pos = *self.pos.borrow() as int64 };
        kResultOk
    }
}

/// The frame of a plugin's view: where its resize requests go, and (on
/// Linux) the run loop its window's toolkit uses.
#[derive(Default)]
pub(crate) struct PlugFrame {
    /// A size the view asked for, packed `w << 32 | h` (0: none).
    pub resize: AtomicU64,
    #[cfg(target_os = "linux")]
    pub run_loop: RefCell<RunLoop>,
}

#[cfg(target_os = "linux")]
impl Class for PlugFrame {
    type Interfaces = (IPlugFrame, vst3::Steinberg::Linux::IRunLoop);
}

#[cfg(not(target_os = "linux"))]
impl Class for PlugFrame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, _view: *mut IPlugView, size: *mut ViewRect) -> tresult {
        if size.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: a non-null rectangle from the plugin.
        let r = unsafe { *size };
        let (w, h) = (
            (r.right - r.left).max(1) as u64,
            (r.bottom - r.top).max(1) as u64,
        );
        self.resize.store((w << 32) | h, Ordering::Release);
        kResultOk
    }
}

/// Descriptors and timers a plugin's window registered (Linux), serviced
/// by `crate::poll` on the main thread.
#[cfg(target_os = "linux")]
#[derive(Default)]
pub(crate) struct RunLoop {
    pub fds: Vec<(i32, vst3::ComPtr<vst3::Steinberg::Linux::IEventHandler>)>,
    pub timers: Vec<(
        u64,
        std::time::Instant,
        vst3::ComPtr<vst3::Steinberg::Linux::ITimerHandler>,
    )>,
}

#[cfg(target_os = "linux")]
impl vst3::Steinberg::Linux::IRunLoopTrait for PlugFrame {
    unsafe fn registerEventHandler(
        &self,
        handler: *mut vst3::Steinberg::Linux::IEventHandler,
        fd: i32,
    ) -> tresult {
        // SAFETY: a handler pointer from the plugin; taking a reference
        // keeps it alive while it's registered.
        let Some(h) = (unsafe { vst3::ComRef::from_raw(handler) }).map(|r| r.to_com_ptr()) else {
            return kInvalidArgument;
        };
        self.run_loop.borrow_mut().fds.push((fd, h));
        kResultOk
    }

    unsafe fn unregisterEventHandler(
        &self,
        handler: *mut vst3::Steinberg::Linux::IEventHandler,
    ) -> tresult {
        self.run_loop
            .borrow_mut()
            .fds
            .retain(|(_, h)| h.as_ptr() != handler);
        kResultOk
    }

    unsafe fn registerTimer(
        &self,
        handler: *mut vst3::Steinberg::Linux::ITimerHandler,
        ms: u64,
    ) -> tresult {
        // SAFETY: as for event handlers.
        let Some(h) = (unsafe { vst3::ComRef::from_raw(handler) }).map(|r| r.to_com_ptr()) else {
            return kInvalidArgument;
        };
        let ms = ms.max(10);
        self.run_loop.borrow_mut().timers.push((
            ms,
            std::time::Instant::now() + std::time::Duration::from_millis(ms),
            h,
        ));
        kResultOk
    }

    unsafe fn unregisterTimer(
        &self,
        handler: *mut vst3::Steinberg::Linux::ITimerHandler,
    ) -> tresult {
        self.run_loop
            .borrow_mut()
            .timers
            .retain(|(_, _, h)| h.as_ptr() != handler);
        kResultOk
    }
}
