//! Watches the mouse and keyboard for Tutorial mode on a worker thread:
//! where the cursor is (as a fraction of the captured display or window),
//! its picture, clicks, and key presses. Keys are never recorded as text:
//! a press is either "typing" (no Ctrl, Alt or Win held) or a shortcut's
//! name.
//!
//! Tests can replace the real mouse and keyboard with a script:
//! `CHISME_FAKE_INPUT="secs:x,y[,click|,type|,key=Ctrl+S];…"`, with x and y
//! as fractions of the captured picture, timed from when watching starts.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::now;
use crate::tutorial::Event;

/// What is captured, as plain numbers (handles can't cross threads).
#[derive(Clone, Copy, Debug)]
pub enum Area {
    Display(isize),
    Window(isize),
}

/// The cursor's picture, straight-alpha RGBA, drawn at twice its size so
/// it stays sharp when shown bigger.
#[derive(Debug)]
pub struct Sprite {
    pub id: u64,
    pub size: (u32, u32),
    pub px: Vec<u8>,
    /// Its size on screen at 1x, in screen pixels.
    pub base: (f64, f64),
    /// The hot spot, as a fraction of the picture.
    pub hot: (f64, f64),
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// The cursor, as a fraction of the captured area (None: outside it
    /// or hidden).
    pub pos: Option<[f64; 2]>,
    /// The captured area's width in screen pixels.
    pub area_w: f64,
    pub sprite: Option<Arc<Sprite>>,
}

#[derive(Default)]
struct Inner {
    snap: Snapshot,
    events: Vec<(f64, Event)>,
}

pub struct Tracker {
    inner: Arc<Mutex<Inner>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Tracker {
    pub fn start(area: Area) -> Tracker {
        let inner = Arc::new(Mutex::new(Inner::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (i2, s2) = (inner.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("tutorial-input".into())
            .spawn(move || match std::env::var("CHISME_FAKE_INPUT") {
                Ok(script) => fake(&script, &i2, &s2),
                Err(_) => watch(area, &i2, &s2),
            })
            .ok();
        Tracker { inner, stop, thread }
    }

    /// The cursor now, and the events since the last call.
    pub fn poll(&self) -> (Snapshot, Vec<(f64, Event)>) {
        let mut i = self.inner.lock().unwrap();
        (i.snap.clone(), std::mem::take(&mut i.events))
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The captured area on screen (physical pixels).
fn area_rect(area: Area) -> Option<RECT> {
    unsafe {
        match area {
            Area::Display(h) => {
                let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                GetMonitorInfoW(HMONITOR(h as _), &mut mi).as_bool().then_some(mi.rcMonitor)
            }
            Area::Window(h) => {
                let mut r = RECT::default();
                let hwnd = HWND(h as _);
                if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as _, size_of::<RECT>() as u32).is_err() {
                    GetWindowRect(hwnd, &mut r).ok()?;
                }
                Some(r)
            }
        }
    }
}

fn frac(p: POINT, r: &RECT) -> [f64; 2] {
    let (w, h) = ((r.right - r.left).max(1) as f64, (r.bottom - r.top).max(1) as f64);
    [(p.x - r.left) as f64 / w, (p.y - r.top) as f64 / h]
}

fn inside(f: [f64; 2]) -> bool {
    (0.0..=1.0).contains(&f[0]) && (0.0..=1.0).contains(&f[1])
}

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

/// Keys that are never shortcuts by themselves.
fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C | 0x14 | 0x90 | 0x91) || vk < 0x08
}

/// A key's name for the banner.
fn key_name(vk: u16) -> Option<String> {
    Some(match vk {
        0x41..=0x5A | 0x30..=0x39 => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x08 => "Backspace".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x1B => "Esc".into(),
        0x20 => "Space".into(),
        0x21 => "PgUp".into(),
        0x22 => "PgDn".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "←".into(),
        0x26 => "↑".into(),
        0x27 => "→".into(),
        0x28 => "↓".into(),
        0x2C => "PrtSc".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x6A => "*".into(),
        0x6B => "+".into(),
        0x6D => "-".into(),
        0x6F => "/".into(),
        _ => {
            let c = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_CHAR) } & 0x7fff;
            char::from_u32(c).filter(|c| !c.is_control() && *c != '\0')?.to_uppercase().to_string()
        }
    })
}

/// Keys that type a character (letters, digits, punctuation, space).
fn types_text(vk: u16) -> bool {
    matches!(vk, 0x20 | 0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2)
}

/// The text cursor of the window in front, if it has a standard one.
fn caret(r: &RECT) -> Option<[f64; 2]> {
    unsafe {
        let tid = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let mut gi = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(tid, &mut gi).ok()?;
        if gi.hwndCaret.is_invalid() {
            return None;
        }
        let mut p = POINT { x: gi.rcCaret.left, y: gi.rcCaret.bottom };
        if !ClientToScreen(gi.hwndCaret, &mut p).as_bool() {
            return None;
        }
        Some(frac(p, r)).filter(|f| inside(*f))
    }
}

static SPRITE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Draw a cursor into RGBA: once on black and once on white, which gives
/// both its colors and its transparency (also for old black-and-white
/// cursors). Pixels that invert what's behind them come out black.
pub fn sprite(cursor: HCURSOR) -> Option<Sprite> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(HICON(cursor.0), &mut ii).ok()?;
        let mut bm = BITMAP::default();
        let src = if !ii.hbmColor.is_invalid() { ii.hbmColor } else { ii.hbmMask };
        GetObjectW(src.into(), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _));
        let (bw, bh) = (bm.bmWidth.max(1), if ii.hbmColor.is_invalid() { bm.bmHeight / 2 } else { bm.bmHeight }.max(1));
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(ii.hbmColor.into());
        }
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(ii.hbmMask.into());
        }
        let (w, h) = (bw * 2, bh * 2);
        let dc = CreateCompatibleDC(None);
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(dib) = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let old = SelectObject(dc, dib.into());
        let n = (w * h * 4) as usize;
        let pass = |fill: u8| {
            std::ptr::write_bytes(bits as *mut u8, fill, n);
            let _ = DrawIconEx(dc, 0, 0, HICON(cursor.0), w, h, 0, None, DI_NORMAL);
            let _ = GdiFlush();
            std::slice::from_raw_parts(bits as *const u8, n).to_vec()
        };
        let (black, white) = (pass(0), pass(255));
        SelectObject(dc, old);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(dc);
        let mut px = vec![0u8; n];
        for i in 0..(w * h) as usize {
            let (b, wh) = (&black[i * 4..i * 4 + 3], &white[i * 4..i * 4 + 3]);
            let sb: i32 = b.iter().map(|&v| v as i32).sum();
            let sw: i32 = wh.iter().map(|&v| v as i32).sum();
            let o = &mut px[i * 4..i * 4 + 4];
            if sw < sb - 96 {
                o.copy_from_slice(&[0, 0, 0, 255]);
                continue;
            }
            let a = (255 - (sw - sb) / 3).clamp(0, 255);
            if a > 0 {
                // BGR on black is the color times alpha.
                for k in 0..3 {
                    o[k] = ((b[2 - k] as i32 * 255) / a).min(255) as u8;
                }
            }
            o[3] = a as u8;
        }
        Some(Sprite { id: SPRITE_ID.fetch_add(1, Ordering::Relaxed), size: (w as u32, h as u32), px, base: (bw as f64, bh as f64), hot: (ii.xHotspot as f64 / bw as f64, ii.yHotspot as f64 / bh as f64) })
    }
}

fn watch(area: Area, inner: &Mutex<Inner>, stop: &AtomicBool) {
    let mut held = [false; 256];
    let mut sprites: Vec<(isize, Arc<Sprite>)> = vec![];
    let mut rect = area_rect(area).unwrap_or_default();
    let mut tick = 0u32;
    while !stop.load(Ordering::Relaxed) {
        // The window may move: look again every quarter second.
        tick += 1;
        if tick % 30 == 0
            && let Some(r) = area_rect(area)
        {
            rect = r;
        }
        let mut ci = CURSORINFO { cbSize: size_of::<CURSORINFO>() as u32, ..Default::default() };
        let got = unsafe { GetCursorInfo(&mut ci) }.is_ok();
        let showing = got && ci.flags.0 & CURSOR_SHOWING.0 != 0 && !ci.hCursor.is_invalid();
        let f = frac(ci.ptScreenPos, &rect);
        let sprite = if showing {
            let key = ci.hCursor.0 as isize;
            match sprites.iter().find(|s| s.0 == key) {
                Some(s) => Some(s.1.clone()),
                None => sprite(ci.hCursor).map(Arc::new).inspect(|s| {
                    if sprites.len() > 24 {
                        sprites.remove(0);
                    }
                    sprites.push((key, s.clone()));
                }),
            }
        } else {
            None
        };
        let t = now();
        let mut events = vec![];
        for vk in [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON] {
            let d = down(vk);
            if d && !held[vk.0 as usize] && got && inside(f) {
                events.push((t, Event::Click(f)));
            }
            held[vk.0 as usize] = d;
        }
        for vk in 0x08u16..0xFF {
            if matches!(vk, 0x01 | 0x02 | 0x04) {
                continue;
            }
            let d = down(VIRTUAL_KEY(vk));
            let pressed = d && !held[vk as usize];
            held[vk as usize] = d;
            if !pressed || is_modifier(vk) {
                continue;
            }
            let (ctrl, alt, shift, win) = (down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT), down(VK_LWIN) || down(VK_RWIN));
            // AltGr (Ctrl + right Alt) types characters on many layouts.
            let altgr = down(VK_RMENU) && ctrl && types_text(vk);
            if (ctrl || alt || win) && !altgr {
                if let Some(name) = key_name(vk) {
                    let mods = [(ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift"), (win, "Win")];
                    let mut s: Vec<&str> = mods.iter().filter(|m| m.0).map(|m| m.1).collect();
                    s.push(&name);
                    events.push((t, Event::Shortcut(s.join(" + "))));
                }
            } else {
                events.push((t, Event::Typing(caret(&rect))));
            }
        }
        {
            let mut i = inner.lock().unwrap();
            i.snap = Snapshot { pos: (showing && inside(f)).then_some(f), area_w: (rect.right - rect.left).max(1) as f64, sprite };
            i.events.extend(events);
            // Nobody is reading: don't pile up.
            if i.events.len() > 256 {
                i.events.drain(..128);
            }
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

/// One step of a fake input script.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub at: f64,
    pub pos: [f64; 2],
    pub event: Option<Event>,
}

/// Parse `secs:x,y[,click|,type|,key=Name];…`.
pub fn parse_script(s: &str) -> Vec<Step> {
    s.split(';')
        .filter_map(|part| {
            let (at, rest) = part.trim().split_once(':')?;
            let mut it = rest.split(',');
            let x = it.next()?.trim().parse().ok()?;
            let y = it.next()?.trim().parse().ok()?;
            let event = it.next().map(str::trim).and_then(|e| match e {
                "click" => Some(Event::Click([x, y])),
                "type" => Some(Event::Typing(None)),
                k => k.strip_prefix("key=").map(|n| Event::Shortcut(n.split('+').map(str::trim).collect::<Vec<_>>().join(" + "))),
            });
            Some(Step { at: at.trim().parse().ok()?, pos: [x, y], event })
        })
        .collect()
}

fn fake(script: &str, inner: &Mutex<Inner>, stop: &AtomicBool) {
    let steps = parse_script(script);
    let arrow = unsafe { LoadCursorW(None, IDC_ARROW) }.ok().and_then(sprite).map(Arc::new);
    let t0 = now();
    let mut next = 0;
    while !stop.load(Ordering::Relaxed) {
        let t = now() - t0;
        // The cursor glides from one step to the next.
        let i = steps.iter().rposition(|s| s.at <= t);
        let pos = match (i.map(|i| &steps[i]), i.map_or(steps.first(), |i| steps.get(i + 1))) {
            (Some(a), Some(b)) if b.at > a.at => {
                let k = ((t - a.at) / (b.at - a.at)).clamp(0.0, 1.0);
                Some([a.pos[0] + (b.pos[0] - a.pos[0]) * k, a.pos[1] + (b.pos[1] - a.pos[1]) * k])
            }
            (Some(a), _) => Some(a.pos),
            (None, _) => None,
        };
        let mut i = inner.lock().unwrap();
        while next < steps.len() && steps[next].at <= t {
            if let Some(e) = &steps[next].event {
                i.events.push((now(), e.clone()));
            }
            next += 1;
        }
        i.snap = Snapshot { pos, area_w: 0.0, sprite: arrow.clone() };
        drop(i);
        std::thread::sleep(Duration::from_millis(8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_script() {
        let s = parse_script("0:0.1,0.2; 1.5:0.5,0.5,click;2:0.5,0.5,key=Ctrl+Shift+S;3:0.6,0.5,type");
        assert_eq!(s.len(), 4);
        assert_eq!(s[1].event, Some(Event::Click([0.5, 0.5])));
        assert_eq!(s[2].event, Some(Event::Shortcut("Ctrl + Shift + S".into())));
        assert_eq!(s[3].event, Some(Event::Typing(None)));
        assert!(s[0].event.is_none() && s[1].at == 1.5);
    }

    #[test]
    fn typed_text_is_never_named_alone() {
        // Letters and digits type text; F-keys and arrows don't.
        assert!(types_text(0x41) && types_text(0x35) && types_text(0x20));
        assert!(!types_text(0x70) && !types_text(0x25));
        assert!(is_modifier(0x11) && is_modifier(0xA0) && !is_modifier(0x41));
        assert_eq!(key_name(0x53).as_deref(), Some("S"));
        assert_eq!(key_name(0x74).as_deref(), Some("F5"));
    }
}
