//! The top-level window a plugin's view is embedded in, per platform.

#[cfg(all(unix, not(target_os = "macos")))]
mod x11;
#[cfg(all(unix, not(target_os = "macos")))]
use x11 as imp;

#[cfg(target_os = "windows")]
mod win32;
#[cfg(target_os = "windows")]
use win32 as imp;

#[cfg(target_os = "macos")]
mod cocoa;
#[cfg(target_os = "macos")]
use cocoa as imp;

#[cfg(not(any(unix, target_os = "windows")))]
mod imp {
    use clack_extensions::gui::Window;

    pub(crate) const AVAILABLE: bool = false;

    pub(crate) struct HostWindow;

    impl HostWindow {
        pub fn open(_: &str, _: u32, _: u32, _: bool) -> Result<HostWindow, String> {
            Err("plugin windows aren't supported on this system".into())
        }
        pub fn clap_window(&self) -> Window<'static, 'static> {
            Window::from_x11_handle(0)
        }
        pub fn show(&self) {}
        pub fn hide(&self) {}
        pub fn raise(&self) {}
        pub fn resize(&self, _: u32, _: u32) {}
        pub fn poll_closed(&mut self) -> bool {
            true
        }
    }
}

pub(crate) use imp::{AVAILABLE, HostWindow};

/// Get the display ready for plugin toolkits, before the first plugin
/// library loads (X11: see `x11::prepare_display`).
pub(crate) fn prepare_display() {
    #[cfg(all(unix, not(target_os = "macos")))]
    x11::prepare_display();
}
