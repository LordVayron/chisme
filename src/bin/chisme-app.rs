//! The Chisme app without a console window (what the Start menu opens).
#![windows_subsystem = "windows"]

use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::HSTRING;

fn main() {
    if let Err(e) = chisme::app::run() {
        unsafe {
            MessageBoxW(None, &HSTRING::from(format!("{e:#}")), &HSTRING::from("Chisme"), MB_OK | MB_ICONERROR);
        }
    }
}
