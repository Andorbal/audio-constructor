//! Loading a VST3 module: the binary inside a bundle for this system, its
//! entry and exit functions (`ModuleEntry` on Linux, `InitDll` on Windows,
//! `bundleEntry` on macOS), and its plugin factory. The library itself is
//! never unloaded (D139).

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use vst3::ComPtr;
use vst3::Steinberg::IPluginFactory;

/// A loaded module. Dropping it releases the factory and calls the
/// module's exit function; its library stays loaded (D139).
pub(crate) struct Module {
    factory: ManuallyDrop<ComPtr<IPluginFactory>>,
    exit: Option<unsafe extern "C" fn() -> bool>,
    #[cfg(target_os = "macos")]
    bundle: *const c_void,
}

// SAFETY: a VST3 factory is safe to call from any thread (the SDK's
// threading rules put no restriction on it); the module's entry and exit
// are called once each, from the threads that load and drop it.
unsafe impl Send for Module {}
// SAFETY: as above; shared use only calls the factory.
unsafe impl Sync for Module {}

/// The architecture folder a bundle keeps this system's binary in.
fn arch_dir() -> String {
    let arch = std::env::consts::ARCH;
    if cfg!(windows) {
        let a = match arch {
            "aarch64" => "arm64",
            other => other,
        };
        format!("{a}-win")
    } else {
        format!("{arch}-linux")
    }
}

/// The binary a bundle path holds for this system (a single-file `.vst3`
/// is its own binary).
pub(crate) fn binary(bundle: &Path) -> PathBuf {
    if bundle.is_file() {
        return bundle.to_path_buf();
    }
    let stem = bundle
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let contents = bundle.join("Contents");
    if cfg!(target_os = "macos") {
        #[cfg(target_os = "macos")]
        if let Some(exe) = crate::library::bundle_executable(bundle) {
            return exe;
        }
        contents.join("MacOS").join(stem)
    } else if cfg!(windows) {
        contents.join(arch_dir()).join(format!("{stem}.vst3"))
    } else {
        contents.join(arch_dir()).join(format!("{stem}.so"))
    }
}

type Exit = unsafe extern "C" fn() -> bool;

impl Module {
    /// Load a bundle's module and get its factory.
    pub fn load(bundle: &Path) -> Result<Arc<Module>, String> {
        crate::gui::prepare_display();
        let bin = binary(bundle);
        if !bin.exists() {
            return Err(format!("no binary for this system at {}", bin.display()));
        }
        // SAFETY: loading a plugin library runs its initialization code,
        // which can do anything; there is no safe way to host native
        // plugins in process (DECISIONS D135).
        let resident = unsafe { crate::library::open(&bin) }?;
        let library = resident.library;
        #[cfg(target_os = "macos")]
        let cf_bundle = cf::bundle(bundle);
        // SAFETY: the VST3 module entry points, with the SDK's signatures,
        // called once after loading as the SDK's hosting code does.
        let exit: Option<Exit> = unsafe {
            #[cfg(all(unix, not(target_os = "macos")))]
            let (entry, exit) = (
                library.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"ModuleEntry\0"),
                library.get::<Exit>(b"ModuleExit\0").ok().map(|s| *s),
            );
            #[cfg(windows)]
            let (entry, exit) = (
                library.get::<unsafe extern "C" fn() -> bool>(b"InitDll\0"),
                library.get::<Exit>(b"ExitDll\0").ok().map(|s| *s),
            );
            #[cfg(target_os = "macos")]
            let (entry, exit) = (
                library.get::<unsafe extern "C" fn(*const c_void) -> bool>(b"bundleEntry\0"),
                library.get::<Exit>(b"bundleExit\0").ok().map(|s| *s),
            );
            #[cfg(all(unix, not(target_os = "macos")))]
            let started = entry.map_or(true, |e| e(resident.handle as *mut c_void));
            #[cfg(windows)]
            let started = entry.map_or(true, |e| e());
            #[cfg(target_os = "macos")]
            let started = entry.map_or(true, |e| e(cf_bundle));
            if !started {
                #[cfg(target_os = "macos")]
                cf::release(cf_bundle);
                return Err("its module refused to start".into());
            }
            exit
        };
        // SAFETY: `GetPluginFactory` is the VST3 entry point, returning an
        // owned factory reference.
        let factory = unsafe {
            library
                .get::<unsafe extern "system" fn() -> *mut c_void>(b"GetPluginFactory\0")
                .ok()
                .and_then(|get| ComPtr::<IPluginFactory>::from_raw(get().cast()))
        };
        let Some(factory) = factory else {
            if let Some(exit) = exit {
                // SAFETY: the module started; it's stopped before unloading.
                unsafe { exit() };
            }
            #[cfg(target_os = "macos")]
            cf::release(cf_bundle);
            return Err("it isn't a VST3 plugin (no plugin factory)".into());
        };
        Ok(Arc::new(Module {
            factory: ManuallyDrop::new(factory),
            exit,
            #[cfg(target_os = "macos")]
            bundle: cf_bundle,
        }))
    }

    pub fn factory(&self) -> &ComPtr<IPluginFactory> {
        &self.factory
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: each dropped once, here, in the order the module needs:
        // the factory, then the module's exit. The library stays mapped.
        unsafe {
            ManuallyDrop::drop(&mut self.factory);
            if let Some(exit) = self.exit.take() {
                exit();
            }
        }
        #[cfg(target_os = "macos")]
        cf::release(self.bundle);
    }
}

/// The CoreFoundation calls `bundleEntry` needs.
#[cfg(target_os = "macos")]
mod cf {
    use std::ffi::c_void;
    use std::path::Path;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: *const c_void,
            buffer: *const u8,
            len: isize,
            is_directory: u8,
        ) -> *const c_void;
        fn CFBundleCreate(allocator: *const c_void, url: *const c_void) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    /// A CFBundle for the bundle folder (null if it can't be made).
    pub fn bundle(path: &Path) -> *const c_void {
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        // SAFETY: CoreFoundation calls with a valid byte buffer of the
        // given length; the URL is released once the bundle holds it.
        unsafe {
            let url = CFURLCreateFromFileSystemRepresentation(
                std::ptr::null(),
                bytes.as_ptr(),
                bytes.len() as isize,
                1,
            );
            if url.is_null() {
                return std::ptr::null();
            }
            let b = CFBundleCreate(std::ptr::null(), url);
            CFRelease(url);
            b
        }
    }

    pub fn release(b: *const c_void) {
        if !b.is_null() {
            // SAFETY: a bundle `bundle` created, released once.
            unsafe { CFRelease(b) };
        }
    }
}
