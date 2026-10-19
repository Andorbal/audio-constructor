//! Plugin libraries, loaded once and kept for the life of the process
//! (DECISIONS D139).
//!
//! A plugin's library is never unloaded, in the host or in a scanner
//! child: its exit handlers (`atexit`), thread-local destructors, and
//! threads can outlive the formats' own deinit calls, and calling into an
//! unmapped library crashes the process, usually at exit. Each format's
//! deinit (`clap_entry.deinit`, `ModuleExit`/`ExitDll`/`bundleExit`) is
//! still called when the last user of a module goes; only the unmapping
//! is skipped. Opening a library again returns the copy already loaded.

#![allow(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A loaded plugin library. It's never closed, so `'static` is true.
pub(crate) struct Resident {
    pub library: &'static libloading::Library,
    /// The `dlopen` handle (VST3's `ModuleEntry` on Linux takes it).
    #[cfg(all(unix, not(target_os = "macos")))]
    pub handle: usize,
}

static RESIDENT: Mutex<Vec<(PathBuf, &'static Resident)>> = Mutex::new(Vec::new());

/// The `dlopen` flags: resolve every symbol now (a plugin missing one fails
/// here, not mid-performance), keep its symbols to itself, and never unmap
/// it even if a handle to it is closed.
#[cfg(unix)]
pub(crate) const FLAGS: std::ffi::c_int = libc::RTLD_NOW | libc::RTLD_LOCAL | libc::RTLD_NODELETE;

/// Open the library at `path` (the binary itself, not a bundle folder), or
/// give the copy this process already has.
///
/// # Safety
///
/// Loading a library runs its initialization code, which can do anything;
/// there is no safe way to host native plugins in process (D135).
pub(crate) unsafe fn open(path: &Path) -> Result<&'static Resident, String> {
    let mut list = RESIDENT.lock().map_err(|_| "plugin libraries poisoned")?;
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some((_, r)) = list.iter().find(|(p, _)| *p == key) {
        return Ok(r);
    }
    // SAFETY: as above, the caller's.
    let resident =
        unsafe { load(path) }.map_err(|e| format!("couldn't load {}: {e}", path.display()))?;
    let resident: &'static Resident = Box::leak(Box::new(resident));
    list.push((key, resident));
    Ok(resident)
}

#[cfg(unix)]
unsafe fn load(path: &Path) -> Result<Resident, libloading::Error> {
    // SAFETY: as for `open`; the flags only change when symbols resolve and
    // whether the library can be unmapped.
    let lib = unsafe { libloading::os::unix::Library::open(Some(path), FLAGS) }?;
    let handle = lib.into_raw();
    // SAFETY: `handle` came from `into_raw` just above.
    let lib = unsafe { libloading::os::unix::Library::from_raw(handle) };
    Ok(Resident {
        library: Box::leak(Box::new(lib.into())),
        #[cfg(not(target_os = "macos"))]
        handle: handle as usize,
    })
}

#[cfg(windows)]
unsafe fn load(path: &Path) -> Result<Resident, libloading::Error> {
    // SAFETY: as for `open`. Leaked, so `FreeLibrary` is never called.
    let lib = unsafe { libloading::Library::new(path) }?;
    Ok(Resident {
        library: Box::leak(Box::new(lib)),
    })
}

/// Whether this process has loaded the library at `path` (it stays loaded
/// until the process exits).
pub fn is_resident(path: &Path) -> bool {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    RESIDENT
        .lock()
        .map(|l| l.iter().any(|(p, _)| *p == key))
        .unwrap_or(false)
}

/// The executable a macOS bundle names in its `Info.plist`.
#[cfg(target_os = "macos")]
pub(crate) fn bundle_executable(bundle: &Path) -> Option<PathBuf> {
    use objc2_foundation::{NSBundle, NSURL};
    let url = NSURL::from_directory_path(bundle)?;
    let b = NSBundle::bundleWithURL(&url)?;
    b.executableURL()?.absoluteURL()?.to_file_path()
}
