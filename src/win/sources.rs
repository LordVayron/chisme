//! Displays and windows that can be recorded.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows::core::BOOL;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindow, IsWindowVisible, WS_EX_TOOLWINDOW,
};

pub struct Display {
    pub hmon: HMONITOR,
    pub name: String,
    pub rect: RECT,
    pub primary: bool,
}

/// All displays, the primary one first.
pub fn displays() -> Vec<Display> {
    unsafe extern "system" fn cb(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        unsafe { (*(data.0 as *mut Vec<HMONITOR>)).push(h) };
        TRUE
    }
    let mut mons: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut mons as *mut _ as isize));
    }
    let mut out: Vec<Display> = mons
        .into_iter()
        .filter_map(|hmon| {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            unsafe { GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO) }.as_bool().then(|| Display {
                hmon,
                name: String::from_utf16_lossy(&info.szDevice).trim_end_matches('\0').to_string(),
                rect: info.monitorInfo.rcMonitor,
                primary: info.monitorInfo.dwFlags & 1 != 0,
            })
        })
        .collect();
    out.sort_by_key(|d| !d.primary);
    out
}

/// Whether any part of window `hwnd` is on display `n` (1-based, primary
/// first, as in `displays()`). True when unsure, so callers stay safe.
pub fn window_on_display(hwnd: isize, n: usize) -> bool {
    let Some(d) = n.checked_sub(1).and_then(|i| displays().into_iter().nth(i)) else { return true };
    let mut r = RECT::default();
    let ok = unsafe { DwmGetWindowAttribute(HWND(hwnd as _), DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut c_void, size_of::<RECT>() as u32) }.is_ok();
    if !ok {
        return true;
    }
    let m = d.rect;
    r.left < m.right && r.right > m.left && r.top < m.bottom && r.bottom > m.top
}

pub struct Window {
    pub hwnd: HWND,
    pub title: String,
}

/// Visible top-level windows with a title, in z-order.
pub fn windows() -> Vec<Window> {
    unsafe extern "system" fn cb(h: HWND, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<Window>) };
        unsafe {
            if !IsWindowVisible(h).as_bool() || GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
                return TRUE;
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut c_void, 4);
            if cloaked != 0 {
                return TRUE;
            }
            let len = GetWindowTextLengthW(h);
            if len > 0 {
                let mut buf = vec![0u16; len as usize + 1];
                let n = GetWindowTextW(h, &mut buf);
                list.push(Window { hwnd: h, title: String::from_utf16_lossy(&buf[..n as usize]) });
            }
        }
        TRUE
    }
    let mut out = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// A window by handle (decimal or 0x hex) or by a case-insensitive part of
/// its title.
pub fn find_window(query: &str) -> Option<Window> {
    let handle = query
        .strip_prefix("0x")
        .map(|h| usize::from_str_radix(h, 16).ok())
        .unwrap_or_else(|| query.parse::<usize>().ok());
    let q = query.to_lowercase();
    windows()
        .into_iter()
        .find(|w| handle.is_some_and(|h| w.hwnd.0 as usize == h) || w.title.to_lowercase().contains(&q))
}

/// A window's size on screen, without its shadow (what capture gets).
pub fn frame_size(hwnd: usize) -> Option<(u32, u32)> {
    let mut r = RECT::default();
    unsafe { DwmGetWindowAttribute(HWND(hwnd as _), DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut c_void, size_of::<RECT>() as u32) }.ok()?;
    Some(((r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32))
}

/// Visible windows with their app's name, leaving out Chisme's own.
pub fn app_windows() -> Vec<(Window, String)> {
    let procs = crate::win::apps::processes();
    let me = std::process::id();
    windows()
        .into_iter()
        .filter_map(|w| {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(w.hwnd, Some(&mut pid)) };
            (pid != me).then(|| {
                let app = procs.iter().find(|p| p.pid == pid).map(|p| p.name.clone()).unwrap_or_default();
                (w, app)
            })
        })
        .collect()
}

/// Whether a window still exists.
pub fn alive(hwnd: usize) -> bool {
    unsafe { IsWindow(Some(HWND(hwnd as _))).as_bool() }
}

/// The window a Window layer means: the one with this exact title, else one
/// of the same app (titles change, like a browser's with each tab), else
/// one whose title contains it.
pub fn match_window(title: &str, app: &str) -> Option<Window> {
    let list = windows();
    if let Some(i) = list.iter().position(|w| w.title == title) {
        return list.into_iter().nth(i);
    }
    if !app.is_empty() {
        let procs = crate::win::apps::processes();
        let owner = |w: &Window| {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(w.hwnd, Some(&mut pid)) };
            procs.iter().find(|p| p.pid == pid).is_some_and(|p| p.name.eq_ignore_ascii_case(app))
        };
        if let Some(i) = list.iter().position(owner) {
            return list.into_iter().nth(i);
        }
    }
    let q = title.to_lowercase();
    list.into_iter().find(|w| !q.is_empty() && w.title.to_lowercase().contains(&q))
}
