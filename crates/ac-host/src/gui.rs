//! Plugin windows (DECISIONS D134). A plugin's interface opens in a window
//! of its own, beside the editor: a top-level window this host makes
//! (on X11, Win32, or Cocoa) with the plugin's view embedded in it, or,
//! for a plugin that only floats, the plugin's own window. Either way it
//! runs on the main thread, and [`crate::poll`] keeps it serviced: closing
//! it, resizing it when the plugin asks.

use std::ffi::CString;
use std::sync::atomic::Ordering;

use clack_extensions::gui::{GuiApiType, GuiConfiguration, GuiSize};

use crate::clap::ClapInstance;
use crate::clap::host::Requests;
use crate::registry::{self, MainHalf};

mod window;

pub(crate) use window::{HostWindow, prepare_display};

/// An open CLAP plugin window.
pub(crate) struct ClapGui {
    /// The host window the plugin's view sits in; `None` when the plugin
    /// floats its own.
    window: Option<HostWindow>,
}

/// Whether the newest instance for `node_key` has a window to show.
pub fn has_window(node_key: &str) -> bool {
    registry::with(node_key, |e| match &e.half {
        MainHalf::Clap(c) => c.ext.gui.is_some(),
    })
    .unwrap_or(false)
}

/// Whether its window is open.
pub fn window_open(node_key: &str) -> bool {
    registry::with(node_key, |e| match &e.half {
        MainHalf::Clap(c) => c.gui.is_some(),
    })
    .unwrap_or(false)
}

/// Open (or bring back) the plugin window of the newest instance for
/// `node_key` running on this thread.
pub fn show_window(node_key: &str) -> Result<(), String> {
    registry::with(node_key, |e| {
        let title = e.title.clone();
        match &mut e.half {
            MainHalf::Clap(c) => open_clap(c, &title),
        }
    })
    .unwrap_or_else(|| Err("the plugin isn't running".into()))
}

/// Close it.
pub fn close_window(node_key: &str) {
    registry::with(node_key, |e| match &mut e.half {
        MainHalf::Clap(c) => close_clap(c),
    });
}

/// Close every plugin window on this thread.
pub(crate) fn close_all() {
    for key in registry::running() {
        close_window(&key);
    }
}

fn open_clap(c: &mut ClapInstance, title: &str) -> Result<(), String> {
    let gui = c.ext.gui.ok_or("the plugin has no window")?;
    let handle = c.instance.plugin_handle();
    if let Some(open) = &c.gui {
        if let Some(w) = &open.window {
            w.raise();
        }
        let _ = gui.show(&handle);
        return Ok(());
    }
    let api = GuiApiType::default_for_current_platform()
        .ok_or("plugin windows aren't supported on this system")?;
    let embedded = GuiConfiguration {
        api_type: api,
        is_floating: false,
    };
    let floating = GuiConfiguration {
        api_type: api,
        is_floating: true,
    };
    if window::AVAILABLE && gui.is_api_supported(&handle, embedded) {
        gui.create(&handle, embedded)
            .map_err(|e| format!("the plugin couldn't make its window: {e}"))?;
        let size = gui.get_size(&handle).unwrap_or(GuiSize {
            width: 640,
            height: 480,
        });
        let resizable = gui.can_resize(&handle);
        let win = match HostWindow::open(title, size.width, size.height, resizable) {
            Ok(w) => w,
            Err(e) => {
                gui.destroy(&handle);
                return Err(e);
            }
        };
        // SAFETY: the host window outlives the plugin's view: `ClapGui`
        // keeps it, and `close_clap` destroys the view before dropping it.
        // Handing a plugin a native parent window is inherently unsafe.
        #[allow(unsafe_code)]
        let parented = unsafe { gui.set_parent(&handle, win.clap_window()) };
        if let Err(e) = parented {
            gui.destroy(&handle);
            return Err(format!("the plugin couldn't open in its window: {e}"));
        }
        let _ = gui.show(&handle);
        win.show();
        c.gui = Some(ClapGui { window: Some(win) });
        Ok(())
    } else if gui.is_api_supported(&handle, floating) {
        gui.create(&handle, floating)
            .map_err(|e| format!("the plugin couldn't make its window: {e}"))?;
        if let Ok(t) = CString::new(title) {
            gui.suggest_title(&handle, &t);
        }
        if let Err(e) = gui.show(&handle) {
            gui.destroy(&handle);
            return Err(format!("the plugin couldn't show its window: {e}"));
        }
        c.gui = Some(ClapGui { window: None });
        Ok(())
    } else {
        Err("the plugin's window can't open on this system".into())
    }
}

pub(crate) fn close_clap(c: &mut ClapInstance) {
    let Some(open) = c.gui.take() else {
        return;
    };
    if let Some(gui) = c.ext.gui {
        let handle = c.instance.plugin_handle();
        let _ = gui.hide(&handle);
        gui.destroy(&handle);
    }
    // The host window goes only now, after the plugin let go of it: a
    // toolkit still looking at its parent would fail on a window that's
    // gone.
    drop(open);
}

/// Keep an open window serviced; true when it closed.
pub(crate) fn service_clap(c: &mut ClapInstance, r: &Requests) -> bool {
    let Some(open) = c.gui.as_mut() else {
        r.gui_closed.store(false, Ordering::Release);
        r.resize.store(0, Ordering::Release);
        return false;
    };
    let packed = r.resize.swap(0, Ordering::AcqRel);
    if packed != 0
        && let Some(w) = &open.window
    {
        let size = GuiSize::unpack_from_u64(packed);
        w.resize(size.width, size.height);
        if let Some(gui) = c.ext.gui {
            let _ = gui.set_size(&c.instance.plugin_handle(), size);
        }
    }
    if r.show.swap(false, Ordering::AcqRel)
        && let Some(w) = &open.window
    {
        w.show();
    }
    if r.hide.swap(false, Ordering::AcqRel)
        && let Some(w) = &open.window
    {
        w.hide();
    }
    let closed_by_person = open.window.as_mut().is_some_and(|w| w.poll_closed());
    let closed_by_plugin = r.gui_closed.swap(false, Ordering::AcqRel);
    if closed_by_person || closed_by_plugin {
        close_clap(c);
        return true;
    }
    false
}
