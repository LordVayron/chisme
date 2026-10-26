//! Tutorial mode: click rings, a bigger and smoother cursor, auto zoom that
//! follows clicks and typing, and a banner for keyboard shortcuts. Each part
//! can go to the recording, the stream or both. This file holds the settings
//! and the motion maths (no GPU); the engine draws the result.
//!
//! Positions are fractions of the captured screen or window (0..1).

use serde::{Deserialize, Serialize};

/// Where a part shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Apply {
    #[default]
    Both,
    Recording,
    Stream,
}

impl Apply {
    pub const ALL: [Apply; 3] = [Apply::Both, Apply::Recording, Apply::Stream];

    pub fn label(self) -> &'static str {
        use crate::i18n::l;
        match self {
            Apply::Both => l("Both", "Ambos"),
            Apply::Recording => l("Recording", "Grabación"),
            Apply::Stream => l("Stream", "Stream"),
        }
    }

    fn covers(self, out: Out) -> bool {
        match out {
            Out::Recording => self != Apply::Stream,
            Out::Stream => self != Apply::Recording,
        }
    }
}

/// A picture that leaves Chisme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Recording,
    Stream,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Part {
    pub on: bool,
    pub to: Apply,
}

impl Default for Part {
    fn default() -> Self {
        Part { on: true, to: Apply::Both }
    }
}

/// Tutorial mode settings. The main switch is off by default; turning it on
/// turns on every part that is on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tutorial {
    pub enabled: bool,
    pub clicks: Part,
    pub big_cursor: Part,
    /// How much bigger the cursor is (1.2x to 3x).
    pub cursor_size: f64,
    pub zoom: Part,
    /// How far auto zoom goes in (1.2x to 3x).
    pub zoom_strength: f64,
    pub smooth_cursor: Part,
    pub keys: Part,
}

impl Default for Tutorial {
    fn default() -> Self {
        Tutorial { enabled: false, clicks: Part::default(), big_cursor: Part::default(), cursor_size: 1.6, zoom: Part::default(), zoom_strength: 1.8, smooth_cursor: Part::default(), keys: Part::default() }
    }
}

/// The parts one picture gets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Parts {
    pub clicks: bool,
    pub big_cursor: bool,
    pub zoom: bool,
    pub smooth_cursor: bool,
    pub keys: bool,
}

impl Tutorial {
    /// What a recording or a stream gets.
    pub fn parts(&self, out: Out) -> Parts {
        let on = |p: Part| self.enabled && p.on && p.to.covers(out);
        Parts { clicks: on(self.clicks), big_cursor: on(self.big_cursor), zoom: on(self.zoom), smooth_cursor: on(self.smooth_cursor), keys: on(self.keys) }
    }

    /// Every part that is on, wherever it goes (the preview while nothing
    /// records or streams).
    pub fn all(&self) -> Parts {
        let (r, s) = (self.parts(Out::Recording), self.parts(Out::Stream));
        Parts { clicks: r.clicks || s.clicks, big_cursor: r.big_cursor || s.big_cursor, zoom: r.zoom || s.zoom, smooth_cursor: r.smooth_cursor || s.smooth_cursor, keys: r.keys || s.keys }
    }

    /// Chisme draws the cursor itself (the capture's own is turned off).
    pub fn own_cursor(&self) -> bool {
        self.enabled && (self.big_cursor.on || self.smooth_cursor.on)
    }

    /// The mouse and keyboard need watching.
    pub fn needs_input(&self) -> bool {
        self.enabled && (self.clicks.on || self.big_cursor.on || self.zoom.on || self.smooth_cursor.on || self.keys.on)
    }
}

/// A critically damped spring: eases in and out, and settles on its target
/// without overshooting when it starts at rest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spring {
    pub x: f64,
    pub v: f64,
}

impl Spring {
    pub fn at(x: f64) -> Spring {
        Spring { x, v: 0.0 }
    }

    /// Move toward `target` for `dt` seconds; `w` sets the speed (it
    /// settles in about 5 / `w` seconds). Exact, so any frame rate gives the
    /// same motion.
    pub fn step(&mut self, target: f64, dt: f64, w: f64) -> f64 {
        let x0 = self.x - target;
        let c = self.v + w * x0;
        let e = (-w * dt).exp();
        self.x = target + (x0 + c * dt) * e;
        self.v = (c - w * (x0 + c * dt)) * e;
        self.x
    }
}

/// The part of a picture shown when zoomed `z` times around `c`, kept
/// inside the picture: x, y, w, h (fractions).
pub fn window(c: [f64; 2], z: f64) -> [f64; 4] {
    let s = 1.0 / z.max(1.0);
    [(c[0] - s / 2.0).clamp(0.0, 1.0 - s), (c[1] - s / 2.0).clamp(0.0, 1.0 - s), s, s]
}

/// A layer's `crop` (fractions of the picture) zoomed `z` times around
/// `focus` (a point of the picture), staying inside the crop.
pub fn zoom_crop(crop: [f64; 4], focus: [f64; 2], z: f64) -> [f64; 4] {
    let f = [(focus[0] - crop[0]) / crop[2].max(1e-6), (focus[1] - crop[1]) / crop[3].max(1e-6)];
    let w = window(f, z);
    [crop[0] + w[0] * crop[2], crop[1] + w[1] * crop[3], w[2] * crop[2], w[3] * crop[3]]
}

/// Something the mouse or keyboard did.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Click([f64; 2]),
    /// A key without Ctrl, Alt or Win; where the text cursor is, if known.
    Typing(Option<[f64; 2]>),
    /// A shortcut's name ("Ctrl + Shift + S").
    Shortcut(String),
}

/// What to draw this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// Zoom (1 = none) and its center.
    pub zoom: f64,
    pub center: [f64; 2],
    /// The cursor where it is and smoothed.
    pub cursor: Option<[f64; 2]>,
    pub smooth: Option<[f64; 2]>,
    /// 0 once the mouse rests for a while (the big cursor hides).
    pub cursor_alpha: f64,
    /// Clicks being drawn: where, and progress 0..1.
    pub clicks: Vec<([f64; 2], f64)>,
    /// The shortcut banner and its opacity.
    pub banner: Option<(String, f64)>,
}

/// Seconds the zoom stays in after a click, or after typing.
pub const HOLD_CLICK: f64 = 2.5;
pub const HOLD_TYPING: f64 = 3.0;
/// A click ring's length.
pub const RING_SECS: f64 = 0.6;
/// The banner's time on screen (and its fade).
pub const BANNER_SECS: f64 = 1.6;
const BANNER_FADE: f64 = 0.3;
/// The big cursor hides after resting this long.
pub const IDLE_SECS: f64 = 2.0;
/// Spring speeds: the zoom, and the cursor.
const ZOOM_W: f64 = 6.5;
const CURSOR_W: f64 = 28.0;
/// While zoomed in, the view follows the cursor when it gets this close to
/// an edge (a fraction of the view).
const EDGE: f64 = 0.15;

/// Tutorial motion over time.
#[derive(Clone, Debug)]
pub struct State {
    zoom: Spring,
    cx: Spring,
    cy: Spring,
    target_z: f64,
    focus: [f64; 2],
    until: f64,
    cur: Option<(Spring, Spring)>,
    pos: Option<[f64; 2]>,
    last_move: f64,
    clicks: Vec<(f64, [f64; 2])>,
    banner: Option<(String, f64)>,
    last_t: Option<f64>,
}

impl Default for State {
    fn default() -> Self {
        State { zoom: Spring::at(1.0), cx: Spring::at(0.5), cy: Spring::at(0.5), target_z: 1.0, focus: [0.5, 0.5], until: 0.0, cur: None, pos: None, last_move: f64::NEG_INFINITY, clicks: vec![], banner: None, last_t: None }
    }
}

impl State {
    /// Something happened at `p`: zoom in there, or keep the view if `p`
    /// is already well inside it.
    fn activity(&mut self, p: [f64; 2], hold: f64, t: f64, strength: f64) {
        let w = window(self.focus, strength);
        let inside = |v: f64, a: f64, s: f64| v >= a + s * EDGE && v <= a + s * (1.0 - EDGE);
        if self.target_z <= 1.0 || !(inside(p[0], w[0], w[2]) && inside(p[1], w[1], w[3])) {
            self.focus = p;
        }
        if self.zoom.x < 1.02 {
            // Starting from the whole picture: head straight for the point.
            self.cx = Spring::at(self.focus[0]);
            self.cy = Spring::at(self.focus[1]);
        }
        self.target_z = strength.max(1.0);
        self.until = self.until.max(t + hold);
    }

    /// Advance to host time `t` with the cursor at `cursor` and the events
    /// since the last call. `strength`: how far the zoom goes in.
    pub fn update(&mut self, t: f64, cursor: Option<[f64; 2]>, events: &[(f64, Event)], strength: f64) -> Frame {
        let dt = self.last_t.map_or(0.0, |l| (t - l).clamp(0.0, 0.1));
        self.last_t = Some(t);
        if let Some(p) = cursor {
            let moved = self.pos.is_none_or(|q| (p[0] - q[0]).abs() + (p[1] - q[1]).abs() > 1e-4);
            if moved {
                self.last_move = t;
            }
            match &mut self.cur {
                // A long jump (another screen, a fresh start): no glide.
                Some((x, y)) if (x.x - p[0]).abs() + (y.x - p[1]).abs() < 0.35 => {
                    x.step(p[0], dt, CURSOR_W);
                    y.step(p[1], dt, CURSOR_W);
                }
                _ => self.cur = Some((Spring::at(p[0]), Spring::at(p[1]))),
            }
        }
        self.pos = cursor;
        for (at, e) in events {
            match e {
                Event::Click(p) => {
                    self.clicks.push((*at, *p));
                    self.activity(*p, HOLD_CLICK, *at, strength);
                }
                Event::Typing(p) => {
                    let p = p.or(cursor).unwrap_or(self.focus);
                    self.activity(p, HOLD_TYPING, *at, strength);
                }
                Event::Shortcut(s) => self.banner = Some((s.clone(), *at)),
            }
        }
        if t > self.until {
            self.target_z = 1.0;
        } else if let Some(p) = cursor {
            // Zoomed in: keep the cursor in view, away from the edges.
            let w = window(self.focus, self.target_z);
            let mut c = [w[0] + w[2] / 2.0, w[1] + w[3] / 2.0];
            for k in 0..2 {
                let (lo, hi) = (w[k] + w[2 + k] * EDGE, w[k] + w[2 + k] * (1.0 - EDGE));
                if p[k] < lo {
                    c[k] -= lo - p[k];
                } else if p[k] > hi {
                    c[k] += p[k] - hi;
                }
            }
            self.focus = c;
        }
        // The center moves to the kept-inside window's center, so the
        // view never pushes against an edge.
        let w = window(self.focus, self.target_z.max(self.zoom.x));
        let center = [w[0] + w[2] / 2.0, w[1] + w[3] / 2.0];
        let zoom = self.zoom.step(self.target_z, dt, ZOOM_W).max(1.0);
        let cx = self.cx.step(center[0], dt, ZOOM_W);
        let cy = self.cy.step(center[1], dt, ZOOM_W);
        self.clicks.retain(|(at, _)| t - at < RING_SECS);
        if self.banner.as_ref().is_some_and(|(_, at)| t - at > BANNER_SECS) {
            self.banner = None;
        }
        let rest = t - self.last_move;
        Frame {
            zoom,
            center: [cx, cy],
            cursor,
            smooth: self.cur.map(|(x, y)| [x.x, y.x]),
            cursor_alpha: (1.0 - (rest - IDLE_SECS) / 0.3).clamp(0.0, 1.0),
            clicks: self.clicks.iter().map(|(at, p)| (*p, ((t - at) / RING_SECS).clamp(0.0, 1.0))).collect(),
            banner: self.banner.as_ref().map(|(s, at)| (s.clone(), ((BANNER_SECS - (t - at)) / BANNER_FADE).clamp(0.0, 1.0))),
        }
    }
}

/// A click ring at progress `k` (0..1): radius as a fraction of the
/// largest, and opacity.
pub fn ring(k: f64) -> (f64, f64) {
    let k = k.clamp(0.0, 1.0);
    let ease = 1.0 - (1.0 - k).powi(3);
    (0.35 + 0.65 * ease, (1.0 - k) * (1.0 - k * 0.3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn spring_eases_without_overshoot() {
        let mut s = Spring::at(1.0);
        let mut last = 1.0;
        let mut steps = vec![];
        for _ in 0..120 {
            let x = s.step(2.0, 1.0 / 60.0, ZOOM_W);
            assert!(x >= last - 1e-12 && x <= 2.0 + 1e-12, "moves one way and never passes the target");
            steps.push(x - last);
            last = x;
        }
        // Eases in (speeds up) then out (slows down).
        assert!(steps[0] < steps[5]);
        assert!(steps[60] < steps[5]);
        assert!((2.0 - last).abs() < 0.01, "settles within two seconds");
        // Frame rate doesn't change the motion.
        let (mut a, mut b) = (Spring::at(0.0), Spring::at(0.0));
        for _ in 0..30 {
            a.step(1.0, 1.0 / 30.0, 6.0);
        }
        for _ in 0..60 {
            b.step(1.0, 1.0 / 60.0, 6.0);
        }
        assert!((a.x - b.x).abs() < 1e-9);
    }

    #[test]
    fn window_stays_inside() {
        assert_eq!(window([0.5, 0.5], 1.0), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(window([0.5, 0.5], 2.0), [0.25, 0.25, 0.5, 0.5]);
        // Near a corner it slides in rather than showing past the edge.
        assert_eq!(window([0.95, 0.02], 2.0), [0.5, 0.0, 0.5, 0.5]);
        // Below 1x is 1x.
        assert_eq!(window([0.1, 0.9], 0.5), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn zoom_inside_a_crop() {
        // No crop: the same as `window`.
        assert_eq!(zoom_crop([0.0, 0.0, 1.0, 1.0], [0.5, 0.5], 2.0), [0.25, 0.25, 0.5, 0.5]);
        // A layer showing the right half: zooming stays in that half.
        let c = zoom_crop([0.5, 0.0, 0.5, 1.0], [0.1, 0.5], 2.0);
        assert!(close(c[0], 0.5) && close(c[2], 0.25) && close(c[1], 0.25) && close(c[3], 0.5));
        let c = zoom_crop([0.5, 0.0, 0.5, 1.0], [0.75, 0.5], 4.0);
        assert!(close(c[0] + c[2] / 2.0, 0.75) && close(c[2], 0.125));
    }

    #[test]
    fn parts_follow_targets() {
        let mut t = Tutorial::default();
        assert_eq!(t.parts(Out::Recording), Parts::default(), "off by default");
        t.enabled = true;
        t.keys.to = Apply::Stream;
        t.zoom.to = Apply::Recording;
        t.clicks.on = false;
        let r = t.parts(Out::Recording);
        let s = t.parts(Out::Stream);
        assert!(r.zoom && !r.keys && !r.clicks);
        assert!(s.keys && !s.zoom);
        assert!(t.all().zoom && t.all().keys && !t.all().clicks);
        assert!(t.own_cursor());
    }

    #[test]
    fn zooms_on_click_and_back_when_idle() {
        let mut s = State::default();
        let f = s.update(0.0, Some([0.8, 0.8]), &[], 2.0);
        assert!(close(f.zoom, 1.0));
        s.update(0.1, Some([0.8, 0.8]), &[(0.1, Event::Click([0.8, 0.8]))], 2.0);
        let mut t = 0.1;
        let mut f = Frame::default();
        while t < 1.5 {
            t += 1.0 / 60.0;
            f = s.update(t, Some([0.8, 0.8]), &[], 2.0);
        }
        assert!((f.zoom - 2.0).abs() < 0.02, "zoomed in: {}", f.zoom);
        // The view is kept inside the picture: centered at 0.75, not 0.8.
        assert!((f.center[0] - 0.75).abs() < 0.01 && (f.center[1] - 0.75).abs() < 0.01);
        while t < 6.0 {
            t += 1.0 / 60.0;
            f = s.update(t, Some([0.8, 0.8]), &[], 2.0);
        }
        assert!((f.zoom - 1.0).abs() < 0.01, "zoomed back out: {}", f.zoom);
    }

    #[test]
    fn follows_cursor_and_typing_keeps_zoom() {
        let mut s = State::default();
        s.update(0.0, Some([0.3, 0.3]), &[(0.0, Event::Click([0.3, 0.3]))], 2.0);
        let mut t = 0.0;
        let mut f = Frame::default();
        // The cursor drifts right, past the view's edge.
        while t < 2.0 {
            t += 1.0 / 60.0;
            let x = 0.3 + 0.3 * (t / 2.0);
            f = s.update(t, Some([x, 0.3]), &[], 2.0);
        }
        let w = window(f.center, f.zoom);
        assert!(0.6 < w[0] + w[2], "the view followed the cursor");
        // Typing keeps it zoomed past the click's hold.
        while t < 5.0 {
            t += 1.0 / 60.0;
            let ev = if (t * 60.0) as i64 % 20 == 0 { vec![(t, Event::Typing(None))] } else { vec![] };
            f = s.update(t, Some([0.6, 0.3]), &ev, 2.0);
        }
        assert!(f.zoom > 1.9);
    }

    #[test]
    fn rings_banner_and_idle_cursor() {
        let mut s = State::default();
        let f = s.update(1.0, Some([0.5, 0.5]), &[(1.0, Event::Click([0.5, 0.5])), (1.0, Event::Shortcut("Ctrl + S".into()))], 1.5);
        assert_eq!(f.clicks.len(), 1);
        assert_eq!(f.banner, Some(("Ctrl + S".into(), 1.0)));
        assert!(close(f.cursor_alpha, 1.0));
        let f = s.update(1.0 + RING_SECS + 0.01, Some([0.5, 0.5]), &[], 1.5);
        assert!(f.clicks.is_empty());
        let f = s.update(1.0 + BANNER_SECS + 0.01, Some([0.5, 0.5]), &[], 1.5);
        assert!(f.banner.is_none());
        let f = s.update(1.0 + IDLE_SECS + 0.5, Some([0.5, 0.5]), &[], 1.5);
        assert!(close(f.cursor_alpha, 0.0), "hidden after resting");
        let f = s.update(1.0 + IDLE_SECS + 0.6, Some([0.6, 0.5]), &[], 1.5);
        assert!(close(f.cursor_alpha, 1.0), "back as soon as it moves");
        let (r0, a0) = ring(0.0);
        let (r1, a1) = ring(1.0);
        assert!(r0 < r1 && a0 > a1 && close(a1, 0.0) && close(r1, 1.0));
    }
}
