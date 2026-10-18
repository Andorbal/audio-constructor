//! Finding the plugin library cargo built next to the test binaries.

use std::path::PathBuf;

/// The plugin's dynamic library (`libac_plugin.so`, `libac_plugin.dylib`,
/// `ac_plugin.dll`), which cargo builds with the crate's rlib before it
/// links the integration tests: next to the test binary (`deps/`) or one
/// folder up (the profile folder). `AC_PLUGIN_LIBRARY` names another one
/// instead, such as the binary inside a release bundle (CI tests those).
pub fn plugin_library() -> PathBuf {
    if let Some(path) = std::env::var_os("AC_PLUGIN_LIBRARY") {
        let path = PathBuf::from(path);
        assert!(path.is_file(), "AC_PLUGIN_LIBRARY: {} isn't a file", path.display());
        return path;
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
        "{name} not found next to {}: build the plugin first (`cargo build -p ac-plugin`)",
        exe.display()
    );
}
