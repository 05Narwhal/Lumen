//! The strip below a maximized window (Windows only).
//!
//! tao keeps 1 px of a maximized borderless window outside the client area when the taskbar
//! auto-hides on that edge, so the mouse can still reach the taskbar. Nothing paints that
//! non-client strip, and DWM shows it pure black: a line under the whole app. We subclass the
//! window and fill everything between the window and client rectangles with the theme's base
//! colour after the default `WM_NCPAINT`. `set_background_color` never reaches it.

use tauri::{App, AppHandle};

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        ClientToScreen, CreateSolidBrush, DeleteObject, FillRect, GetWindowDC, ReleaseDC, HGDIOBJ,
    };
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, IsZoomed, SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_ACTIVATE, WM_NCACTIVATE, WM_NCPAINT, WM_SIZE,
    };

    /// COLORREF (0x00BBGGRR); starts as the dark boot colour until the page sends the theme's.
    static COLOUR: AtomicU32 = AtomicU32::new(0x0021_1814);
    const SUBCLASS_ID: usize = 0x4C75_6D65;

    pub fn install(hwnd: isize) {
        // SAFETY: a valid top-level HWND owned by this process; the proc lives for 'static.
        unsafe {
            SetWindowSubclass(hwnd as HWND, Some(proc), SUBCLASS_ID, 0);
        }
    }

    pub fn set_colour(hwnd: isize, r: u8, g: u8, b: u8) {
        COLOUR.store(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16), Ordering::Relaxed);
        // SAFETY: asks Windows to recompute and repaint the frame; no pointers are passed.
        unsafe {
            SetWindowPos(
                hwnd as HWND,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    unsafe extern "system" fn proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        let result = DefSubclassProc(hwnd, msg, wparam, lparam);
        if matches!(msg, WM_NCPAINT | WM_NCACTIVATE | WM_ACTIVATE | WM_SIZE) && IsZoomed(hwnd) != 0 {
            paint(hwnd);
        }
        result
    }

    unsafe fn paint(hwnd: HWND) {
        let mut wr: RECT = std::mem::zeroed();
        let mut cr: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut wr) == 0 || GetClientRect(hwnd, &mut cr) == 0 {
            return;
        }
        let mut origin = POINT { x: 0, y: 0 };
        ClientToScreen(hwnd, &mut origin);
        // The client rectangle in window coordinates.
        let (l, t) = (origin.x - wr.left, origin.y - wr.top);
        let (r, b) = (l + cr.right, t + cr.bottom);
        let (w, h) = (wr.right - wr.left, wr.bottom - wr.top);
        let strips = [
            RECT { left: 0, top: b, right: w, bottom: h },
            RECT { left: 0, top: 0, right: w, bottom: t },
            RECT { left: 0, top: t, right: l, bottom: b },
            RECT { left: r, top: t, right: w, bottom: b },
        ];
        let dc = GetWindowDC(hwnd);
        if dc.is_null() {
            return;
        }
        let brush = CreateSolidBrush(COLOUR.load(Ordering::Relaxed));
        for s in strips.iter().filter(|s| s.right > s.left && s.bottom > s.top) {
            FillRect(dc, s, brush);
        }
        DeleteObject(brush as HGDIOBJ);
        ReleaseDC(hwnd, dc);
    }
}

/// Subclass the main window so its non-client strips take the theme colour.
pub fn install(app: &App) {
    #[cfg(windows)]
    {
        use tauri::Manager;
        if let Some(hwnd) = app.get_webview_window("main").and_then(|w| w.hwnd().ok()) {
            imp::install(hwnd.0 as isize);
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// The page's `--bg-base`, sent on every theme or scheme change.
pub fn set_colour(app: &AppHandle, r: u8, g: u8, b: u8) {
    #[cfg(windows)]
    {
        use tauri::Manager;
        if let Some(hwnd) = app.get_webview_window("main").and_then(|w| w.hwnd().ok()) {
            imp::set_colour(hwnd.0 as isize, r, g, b);
        }
    }
    #[cfg(not(windows))]
    let _ = (app, r, g, b);
}
