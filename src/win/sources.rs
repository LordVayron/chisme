//! Displays and windows that can be recorded.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows::core::BOOL;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible, WS_EX_TOOLWINDOW,
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
