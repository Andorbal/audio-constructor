//! An X11 top-level window for a plugin's view (Linux and the BSDs;
//! Wayland sessions run it through XWayland, as CLAP has no Wayland API).
//! Its own connection, polled without blocking from `crate::poll`.

use clack_extensions::gui::Window;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux,
    EventMask, PropMode, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub(crate) const AVAILABLE: bool = true;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
        WM_NORMAL_HINTS,
        _NET_WM_NAME,
        UTF8_STRING,
    }
}

/// Atoms a window manager creates, which plugin toolkits look up without
/// creating them and then use as if they existed: on a server with no
/// window manager (a virtual display, a bare session) a property set with
/// the missing atom is an X error, which Xlib's default handler turns into
/// an exit. Interning them first makes them exist; on a desktop they
/// already do.
const WM_ATOMS: &[&[u8]] = &[
    b"CLIPBOARD",
    b"TARGETS",
    b"UTF8_STRING",
    b"WM_CHANGE_STATE",
    b"WM_DELETE_WINDOW",
    b"WM_PROTOCOLS",
    b"WM_STATE",
    b"WM_TAKE_FOCUS",
    b"XdndActionCopy",
    b"XdndActionList",
    b"XdndAware",
    b"XdndDrop",
    b"XdndEnter",
    b"XdndFinished",
    b"XdndLeave",
    b"XdndPosition",
    b"XdndSelection",
    b"XdndStatus",
    b"XdndTypeList",
    b"_KDE_NET_WM_WINDOW_TYPE_OVERRIDE",
    b"_MOTIF_WM_HINTS",
    b"_NET_ACTIVE_WINDOW",
    b"_NET_FRAME_EXTENTS",
    b"_NET_SUPPORTED",
    b"_NET_WM_ACTION_CLOSE",
    b"_NET_WM_ACTION_FULLSCREEN",
    b"_NET_WM_ACTION_MINIMIZE",
    b"_NET_WM_ACTION_MOVE",
    b"_NET_WM_ACTION_RESIZE",
    b"_NET_WM_ALLOWED_ACTIONS",
    b"_NET_WM_ICON",
    b"_NET_WM_MOVERESIZE",
    b"_NET_WM_NAME",
    b"_NET_WM_PID",
    b"_NET_WM_PING",
    b"_NET_WM_STATE",
    b"_NET_WM_STATE_ABOVE",
    b"_NET_WM_STATE_FULLSCREEN",
    b"_NET_WM_STATE_HIDDEN",
    b"_NET_WM_STATE_MAXIMIZED_HORZ",
    b"_NET_WM_STATE_MAXIMIZED_VERT",
    b"_NET_WM_STATE_SKIP_TASKBAR",
    b"_NET_WM_USER_TIME",
    b"_NET_WM_WINDOW_TYPE",
    b"_NET_WM_WINDOW_TYPE_COMBO",
    b"_NET_WM_WINDOW_TYPE_DIALOG",
    b"_NET_WM_WINDOW_TYPE_NORMAL",
    b"_NET_WM_WINDOW_TYPE_POPUP_MENU",
    b"_NET_WORKAREA",
    b"_XEMBED",
    b"_XEMBED_INFO",
    b"_XSETTINGS_SETTINGS",
];

/// Make the atoms in [`WM_ATOMS`] exist on the display, once per
/// process, before any plugin library loads (toolkits look them up when
/// they start, and keep what they found). Without a display it does
/// nothing.
pub(crate) fn prepare_display() {
    // The connection stays open: an X server with no clients left resets,
    // and forgets atoms with it.
    static KEEP: std::sync::OnceLock<Option<RustConnection>> = std::sync::OnceLock::new();
    KEEP.get_or_init(|| {
        std::env::var_os("DISPLAY")?;
        let (conn, _) = x11rb::connect(None).ok()?;
        intern_wm_atoms(&conn);
        Some(conn)
    });
}

fn intern_wm_atoms(conn: &RustConnection) {
    let cookies: Vec<_> = WM_ATOMS
        .iter()
        .filter_map(|name| conn.intern_atom(false, name).ok())
        .collect();
    for c in cookies {
        let _ = c.reply();
    }
}

pub(crate) struct HostWindow {
    conn: RustConnection,
    window: u32,
    atoms: Atoms,
    closed: bool,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("couldn't open a window: {e}")
}

impl HostWindow {
    pub fn open(title: &str, width: u32, height: u32, resizable: bool) -> Result<Self, String> {
        let (conn, screen_num) = x11rb::connect(None).map_err(err)?;
        let screen = conn
            .setup()
            .roots
            .get(screen_num)
            .ok_or_else(|| err("no screen"))?
            .clone();
        let atoms = Atoms::new(&conn).map_err(err)?.reply().map_err(err)?;
        intern_wm_atoms(&conn);
        let window = conn.generate_id().map_err(err)?;
        let (w, h) = (clamp_size(width), clamp_size(height));
        conn.create_window(
            screen.root_depth,
            window,
            screen.root,
            0,
            0,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .event_mask(EventMask::STRUCTURE_NOTIFY)
                .background_pixel(screen.black_pixel),
        )
        .map_err(err)?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            title.as_bytes(),
        )
        .map_err(err)?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_NAME,
            atoms.UTF8_STRING,
            title.as_bytes(),
        )
        .map_err(err)?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms.WM_PROTOCOLS,
            AtomEnum::ATOM,
            &[atoms.WM_DELETE_WINDOW],
        )
        .map_err(err)?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"audio-constructor-plugin\0Audio Constructor\0",
        )
        .map_err(err)?;
        let this = HostWindow {
            conn,
            window,
            atoms,
            closed: false,
        };
        this.size_hints(w, h, resizable);
        // The plugin connects on its own and reparents into this window,
        // so it must exist on the server before its id is handed over.
        this.conn.sync().map_err(err)?;
        Ok(this)
    }

    /// WM_NORMAL_HINTS: a fixed size unless the plugin can resize.
    fn size_hints(&self, w: u16, h: u16, resizable: bool) {
        // flags: PMinSize (16) | PMaxSize (32) when fixed.
        let mut hints = [0u32; 18];
        if !resizable {
            hints[0] = 16 | 32;
            hints[5] = u32::from(w);
            hints[6] = u32::from(h);
            hints[7] = u32::from(w);
            hints[8] = u32::from(h);
        }
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            self.window,
            self.atoms.WM_NORMAL_HINTS,
            AtomEnum::WM_SIZE_HINTS,
            &hints,
        );
    }

    pub fn clap_window(&self) -> Window<'static, 'static> {
        Window::from_x11_handle(self.window.into())
    }

    /// The window as VST3's `X11EmbedWindowID` parent (its XID).
    pub fn native_parent(&self) -> *mut std::ffi::c_void {
        self.window as usize as *mut std::ffi::c_void
    }

    pub fn show(&self) {
        let _ = self.conn.map_window(self.window);
        let _ = self.conn.flush();
    }

    pub fn hide(&self) {
        let _ = self.conn.unmap_window(self.window);
        let _ = self.conn.flush();
    }

    pub fn raise(&self) {
        let _ = self.conn.map_window(self.window);
        let _ = self.conn.configure_window(
            self.window,
            &ConfigureWindowAux::new().stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
        );
        let _ = self.conn.flush();
    }

    pub fn resize(&self, width: u32, height: u32) {
        let _ = self.conn.configure_window(
            self.window,
            &ConfigureWindowAux::new()
                .width(u32::from(clamp_size(width)))
                .height(u32::from(clamp_size(height))),
        );
        let _ = self.conn.flush();
    }

    /// Drain this window's events; true once the person asked to close it
    /// (or the connection went away).
    pub fn poll_closed(&mut self) -> bool {
        loop {
            match self.conn.poll_for_event() {
                Ok(Some(Event::ClientMessage(ClientMessageEvent { data, format, .. })))
                    if format == 32 && data.as_data32()[0] == self.atoms.WM_DELETE_WINDOW =>
                {
                    self.closed = true;
                }
                Ok(Some(Event::DestroyNotify(_))) => self.closed = true,
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {
                    self.closed = true;
                    break;
                }
            }
        }
        self.closed
    }
}

fn clamp_size(v: u32) -> u16 {
    v.clamp(16, 8192) as u16
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}
