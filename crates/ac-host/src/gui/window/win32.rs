//! A Win32 top-level window for a plugin's view. It's made on the main
//! thread, whose message loop (the editor's) delivers its messages; closing
//! it only hides it, and `poll_closed` notices.

#![allow(unsafe_code)]

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::null_mut;

use clack_extensions::gui::Window;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
    IsWindowVisible, RegisterClassW, SW_HIDE, SW_SHOW, SWP_NOMOVE, SWP_NOZORDER,
    SetForegroundWindow, SetWindowPos, ShowWindow, WM_CLOSE, WNDCLASSW, WS_MAXIMIZEBOX,
    WS_OVERLAPPEDWINDOW, WS_THICKFRAME,
};

pub(crate) const AVAILABLE: bool = true;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

const CLASS: &str = "AudioConstructorPluginWindow";

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CLOSE {
        // SAFETY: `hwnd` is the window this procedure was called for.
        unsafe { ShowWindow(hwnd, SW_HIDE) };
        return 0;
    }
    // SAFETY: forwarding the arguments Windows gave us, unchanged.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub(crate) struct HostWindow {
    hwnd: HWND,
    style: u32,
    shown: Cell<bool>,
}

impl HostWindow {
    pub fn open(title: &str, width: u32, height: u32, resizable: bool) -> Result<Self, String> {
        let class = wide(CLASS);
        let title = wide(title);
        // SAFETY: Win32 calls with valid, NUL-terminated wide strings that
        // outlive the calls; a null module name means this executable. A
        // second RegisterClassW for the same class fails harmlessly.
        unsafe {
            let instance = GetModuleHandleW(null_mut());
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: null_mut(),
                hCursor: null_mut(),
                hbrBackground: null_mut(),
                lpszMenuName: null_mut(),
                lpszClassName: class.as_ptr(),
            };
            RegisterClassW(&wc);
            let mut style = WS_OVERLAPPEDWINDOW;
            if !resizable {
                style &= !(WS_THICKFRAME | WS_MAXIMIZEBOX);
            }
            let (w, h) = outer_size(style, width, height);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                w,
                h,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            if hwnd.is_null() {
                return Err("couldn't open a window".into());
            }
            Ok(HostWindow {
                hwnd,
                style,
                shown: Cell::new(false),
            })
        }
    }

    pub fn clap_window(&self) -> Window<'static, 'static> {
        // SAFETY: `hwnd` is a live window owned by this value.
        unsafe { Window::from_win32_hwnd(self.hwnd as *mut c_void) }
    }

    pub fn show(&self) {
        // SAFETY: `hwnd` is a live window owned by this value.
        unsafe { ShowWindow(self.hwnd, SW_SHOW) };
        self.shown.set(true);
    }

    pub fn hide(&self) {
        self.shown.set(false);
        // SAFETY: as above.
        unsafe { ShowWindow(self.hwnd, SW_HIDE) };
    }

    pub fn raise(&self) {
        self.show();
        // SAFETY: as above.
        unsafe { SetForegroundWindow(self.hwnd) };
    }

    pub fn resize(&self, width: u32, height: u32) {
        let (w, h) = outer_size(self.style, width, height);
        // SAFETY: as above.
        unsafe { SetWindowPos(self.hwnd, null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER) };
    }

    /// True once the person closed it (which hid it).
    pub fn poll_closed(&mut self) -> bool {
        // SAFETY: as above.
        self.shown.get() && unsafe { IsWindowVisible(self.hwnd) } == 0
    }
}

/// The window size that gives a client area of `width` × `height`.
fn outer_size(style: u32, width: u32, height: u32) -> (i32, i32) {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: width.clamp(16, 8192) as i32,
        bottom: height.clamp(16, 8192) as i32,
    };
    // SAFETY: `r` is a valid, exclusively borrowed RECT.
    unsafe { AdjustWindowRectEx(&mut r, style, 0, 0) };
    (r.right - r.left, r.bottom - r.top)
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        // SAFETY: `hwnd` is a live window owned by this value, destroyed
        // once, on the thread that made it.
        unsafe { DestroyWindow(self.hwnd) };
    }
}
