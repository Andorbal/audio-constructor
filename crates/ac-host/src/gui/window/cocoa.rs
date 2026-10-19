//! A Cocoa window for a plugin's view (macOS), on the process's main
//! thread as AppKit requires. Closing it only orders it out (it isn't
//! released when closed), and `poll_closed` notices.

#![allow(unsafe_code)]

use std::cell::Cell;
use std::ffi::c_void;

use clack_extensions::gui::Window;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub(crate) const AVAILABLE: bool = true;

pub(crate) struct HostWindow {
    window: Retained<NSWindow>,
    shown: Cell<bool>,
}

fn size(width: u32, height: u32) -> NSSize {
    NSSize::new(
        f64::from(width.clamp(16, 8192)),
        f64::from(height.clamp(16, 8192)),
    )
}

impl HostWindow {
    pub fn open(title: &str, width: u32, height: u32, resizable: bool) -> Result<Self, String> {
        let mtm = MainThreadMarker::new()
            .ok_or("plugin windows open only on the main thread on macOS")?;
        let mut style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;
        if resizable {
            style |= NSWindowStyleMask::Resizable;
        }
        let rect = NSRect::new(NSPoint::new(200.0, 200.0), size(width, height));
        // SAFETY: a window made on the main thread (the marker proves it)
        // with a valid frame; it must not be released when closed, since
        // this value keeps a strong reference and closes it in `drop`.
        let window = unsafe {
            let w = NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            );
            w.setReleasedWhenClosed(false);
            w
        };
        window.setTitle(&NSString::from_str(title));
        window.center();
        Ok(HostWindow {
            window,
            shown: Cell::new(false),
        })
    }

    pub fn clap_window(&self) -> Window<'static, 'static> {
        let view = self.window.contentView().map_or(std::ptr::null_mut(), |v| {
            Retained::as_ptr(&v) as *mut c_void
        });
        // SAFETY: the content view lives as long as the window, which this
        // value owns until the plugin has let go of it.
        unsafe { Window::from_cocoa_nsview(view) }
    }

    /// The content view as VST3's `NSView` parent.
    pub fn native_parent(&self) -> *mut c_void {
        self.window.contentView().map_or(std::ptr::null_mut(), |v| {
            Retained::as_ptr(&v) as *mut c_void
        })
    }

    pub fn show(&self) {
        self.window.makeKeyAndOrderFront(None);
        self.shown.set(true);
    }

    pub fn hide(&self) {
        self.shown.set(false);
        self.window.orderOut(None);
    }

    pub fn raise(&self) {
        self.show();
    }

    pub fn resize(&self, width: u32, height: u32) {
        self.window.setContentSize(size(width, height));
    }

    /// True once the person closed it.
    pub fn poll_closed(&mut self) -> bool {
        self.shown.get() && !self.window.isVisible()
    }
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        self.window.close();
    }
}
