//! Finding our own plugin build (the `ac-plugin` library cargo builds
//! with this crate's dev-dependencies) and dressing it as a `.clap` and a
//! `.vst3` bundle in a temporary folder, the way it's installed.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The plugin library (`libac_plugin.so`, `.dylib`, or `ac_plugin.dll`)
/// next to the test binary, or the file `AC_PLUGIN_LIBRARY` names.
pub fn plugin_library() -> PathBuf {
    if let Some(path) = std::env::var_os("AC_PLUGIN_LIBRARY") {
        return PathBuf::from(path);
    }
    let name = format!(
        "{}ac_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let exe = std::env::current_exe().expect("the test binary's path");
    let deps = exe.parent().expect("the test binary's folder");
    for dir in [Some(deps), deps.parent()].into_iter().flatten() {
        let candidate = dir.join(&name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "{name} not found next to {}: build it with `cargo build -p ac-plugin`",
        exe.display()
    );
}

fn link_or_copy(src: &Path, dst: &Path) {
    let _ = std::fs::remove_file(dst);
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(src, dst).is_ok() {
            return;
        }
    }
    std::fs::copy(src, dst).expect("copy the plugin library");
}

/// A folder holding `Audio Constructor.clap` and `Audio Constructor.vst3`
/// built from the library (made once per test binary).
pub fn plugin_folder() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let lib = plugin_library();
        let dir = std::env::temp_dir().join(format!("ac-host-tests-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp folder");
        link_or_copy(&lib, &dir.join("Audio Constructor.clap"));
        let arch = std::env::consts::ARCH;
        let (sub, file) = if cfg!(target_os = "macos") {
            ("MacOS".to_string(), "Audio Constructor".to_string())
        } else if cfg!(windows) {
            (format!("{arch}-win"), "Audio Constructor.vst3".to_string())
        } else {
            (format!("{arch}-linux"), "Audio Constructor.so".to_string())
        };
        let contents = dir
            .join("Audio Constructor.vst3")
            .join("Contents")
            .join(sub);
        std::fs::create_dir_all(&contents).expect("vst3 bundle folder");
        link_or_copy(&lib, &contents.join(file));
        dir
    })
}

pub fn clap_bundle() -> PathBuf {
    plugin_folder().join("Audio Constructor.clap")
}

pub fn vst3_bundle() -> PathBuf {
    plugin_folder().join("Audio Constructor.vst3")
}

/// A `.clap` file and a `.vst3` bundle in a new folder named `name`, each
/// its own copy of the library (not a link), so the system loads each as a
/// library of its own. Returns the `.clap`, the `.vst3` bundle, and the
/// bundle's binary.
pub fn copied_bundles(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let lib = plugin_library();
    let dir = std::env::temp_dir().join(format!("ac-host-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp folder");
    let clap = dir.join("Copied.clap");
    std::fs::copy(&lib, &clap).expect("copy the plugin library");
    let arch = std::env::consts::ARCH;
    let (sub, file) = if cfg!(target_os = "macos") {
        ("MacOS".to_string(), "Copied".to_string())
    } else if cfg!(windows) {
        (format!("{arch}-win"), "Copied.vst3".to_string())
    } else {
        (format!("{arch}-linux"), "Copied.so".to_string())
    };
    let vst3 = dir.join("Copied.vst3");
    let contents = vst3.join("Contents").join(sub);
    std::fs::create_dir_all(&contents).expect("vst3 bundle folder");
    let binary = contents.join(file);
    std::fs::copy(&lib, &binary).expect("copy the plugin library");
    (clap, vst3, binary)
}

/// Whether the library at `path` is still loaded in this process (asked
/// without loading it).
#[cfg(unix)]
#[allow(unsafe_code)]
pub fn still_loaded(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("a path");
    // SAFETY: a valid path string; RTLD_NOLOAD only looks a library up.
    let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_LAZY | libc::RTLD_NOLOAD) };
    if h.is_null() {
        return false;
    }
    // SAFETY: the reference NOLOAD just took, given back.
    unsafe { libc::dlclose(h) };
    true
}
