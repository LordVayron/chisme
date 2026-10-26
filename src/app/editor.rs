//! The layout editor. On the previews: click a layer to select it, drag it
//! to move (with snapping guides), drag a corner or side to resize, the
//! round handle to turn it, and right-click for more. In crop mode the
//! corners and sides cut the picture instead. The "Layers" card lists the
//! layers of the layout being edited (16:9 or 9:16) and the selected one's
//! settings. Arrow keys nudge, Delete removes, Ctrl+D duplicates, Ctrl+Z /
//! Ctrl+Y undo and redo.

use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

use super::theme::{self, *};
use super::App;
use crate::i18n::l as tr;
use crate::compose::Shape;
use crate::layout::{self, Layer, Layout, Source, clamp, snap};

/// What a drag on the preview is doing.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Move,
    /// 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right, 4 top,
    /// 5 right, 6 bottom, 7 left.
    Handle(usize),
    Rotate,
}

/// A move, resize, turn or crop in progress.
#[derive(Clone, Copy)]
pub struct Drag {
    layer: usize,
    kind: Kind,
    start: [f64; 4],
    rotation: f64,
    crop: [f64; 4],
    from: Pos2,
}

const HANDLE: f32 = 11.0;
/// How far the turn handle sits above the box.
const ROT_GAP: f32 = 26.0;

/// Undo history entry: both layouts.
pub type Snapshot = (Option<Layout>, Option<Layout>);

fn to_screen(r: [f64; 4], img: Rect) -> Rect {
    Rect::from_min_size(
        pos2(img.left() + r[0] as f32 * img.width(), img.top() + r[1] as f32 * img.height()),
        vec2(r[2] as f32 * img.width(), r[3] as f32 * img.height()),
    )
}

fn rot(v: Vec2, deg: f64) -> Vec2 {
    let (s, c) = (deg.to_radians() as f32).sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// Handle signs (x, y): -1 left/top, 1 right/bottom, 0 the middle.
fn signs(h: usize) -> (f32, f32) {
    match h {
        0 => (-1.0, -1.0),
        1 => (1.0, -1.0),
        2 => (-1.0, 1.0),
        3 => (1.0, 1.0),
        4 => (0.0, -1.0),
        5 => (1.0, 0.0),
        6 => (0.0, 1.0),
        _ => (-1.0, 0.0),
    }
}

/// The 8 handles of a (turned) box, in screen pixels.
fn handles(r: Rect, deg: f64) -> [Pos2; 8] {
    let c = r.center();
    let (hw, hh) = (r.width() / 2.0, r.height() / 2.0);
    let mut out = [Pos2::ZERO; 8];
    for (i, o) in out.iter_mut().enumerate() {
        let (sx, sy) = signs(i);
        *o = c + rot(vec2(sx * hw, sy * hh), deg);
    }
    out
}

fn rot_handle(r: Rect, deg: f64) -> Pos2 {
    r.center() + rot(vec2(0.0, -r.height() / 2.0 - ROT_GAP), deg)
}

/// Is `p` inside the (turned) box?
fn inside(r: Rect, deg: f64, p: Pos2) -> bool {
    let l = rot(p - r.center(), -deg);
    l.x.abs() <= r.width() / 2.0 && l.y.abs() <= r.height() / 2.0
}

/// The topmost visible layer under `p`.
fn hit(layout: &Layout, img: Rect, p: Pos2) -> Option<usize> {
    layout.layers.iter().enumerate().rev().find(|(_, l)| l.visible && inside(to_screen(l.rect, img), l.rotation, p)).map(|(i, _)| i)
}

/// The 4 corners of a turned box, for its outline.
fn outline(r: Rect, deg: f64) -> Vec<Pos2> {
    let h = handles(r, deg);
    vec![h[0], h[1], h[3], h[2]]
}

/// Quick places for a layer: (name, rect). Sizes are kept for the corners
/// and the center.
fn place(kind: usize, r: [f64; 4], frame: (f64, f64)) -> [f64; 4] {
    let (mx, my) = (0.025, 0.025 * frame.0 / frame.1);
    let [_, _, w, h] = r;
    match kind {
        0 => [0.0, 0.0, 1.0, 1.0],
        1 => [0.0, 0.0, 0.5, 1.0],
        2 => [0.5, 0.0, 0.5, 1.0],
        3 => [0.0, 0.0, 1.0, 0.5],
        4 => [0.0, 0.5, 1.0, 0.5],
        5 => [(1.0 - w) / 2.0, (1.0 - h) / 2.0, w, h],
        6 => [mx, my, w, h],
        7 => [1.0 - w - mx, my, w, h],
        8 => [mx, 1.0 - h - my, w, h],
        _ => [1.0 - w - mx, 1.0 - h - my, w, h],
    }
}

const PLACES: [(&str, &str); 10] = [
    ("Full frame", "Pantalla completa"),
    ("Left half", "Mitad izquierda"),
    ("Right half", "Mitad derecha"),
    ("Top half", "Mitad de arriba"),
    ("Bottom half", "Mitad de abajo"),
    ("Center", "Centro"),
    ("Top left", "Arriba a la izquierda"),
    ("Top right", "Arriba a la derecha"),
    ("Bottom left", "Abajo a la izquierda"),
    ("Bottom right", "Abajo a la derecha"),
];

impl App {
    /// Send the edited layout to the engine (instant) without saving yet.
    pub fn layout_live(&mut self) {
        if let Some(e) = &self.engine {
            e.set_live(self.settings.live());
        }
    }

    /// Put the brand logo in the top-right corner of the (custom) layout.
    pub fn add_logo(&mut self) {
        let Some(logo) = self.settings.brand.logo.clone() else { return };
        self.customize_layout();
        let (w, h) = (self.status.out_size.0.max(16) as f64, self.status.out_size.1.max(9) as f64);
        let aspect = image::image_dimensions(&logo).map_or(1.0, |(lw, lh)| lw as f64 / lh.max(1) as f64);
        if let Some(l) = self.settings.layout.as_mut() {
            l.layers.push(Layer::image(&logo, aspect, w / h));
            self.sel_layer = Some(l.layers.len() - 1);
            self.edit_v = false;
        }
        self.settings_changed();
        self.page = super::Page::Studio;
    }

    /// Start editing: the current scene as layers.
    pub fn customize_layout(&mut self) {
        // A template becomes the starting point of the custom layout, with
        // the 9:16 layout it had.
        let v = self.settings.vertical_layout();
        self.settings.keep_vertical();
        self.settings.template = None;
        if self.settings.layout.is_none() {
            let (w, h) = (self.status.out_size.0.max(16) as f64, self.status.out_size.1.max(9) as f64);
            let cam_aspect = self.status.cam.as_ref().map_or(16.0 / 9.0, |c| c.1.0 as f64 / c.1.1.max(1) as f64);
            self.settings.layout = Some(Layout::from_scene(self.settings.scene, &self.settings.pip(), w, h, cam_aspect, w / h));
        }
        self.settings.layout_v = Some(v);
        self.settings.keep_vertical();
        self.sel_layer = None;
        self.crop_mode = false;
        self.settings_changed();
    }

    /// The layout being edited: 16:9, or (`vertical`) the 9:16 one.
    fn edited(&mut self, vertical: bool) -> Option<&mut Layout> {
        if vertical {
            let auto = self.settings.vertical_layout();
            Some(self.settings.layout_v.get_or_insert(auto))
        } else {
            self.settings.layout.as_mut()
        }
    }

    /// The frame size of the layout being edited.
    fn frame(&self, vertical: bool) -> (f64, f64) {
        if vertical { (1080.0, 1920.0) } else { (self.status.out_size.0.max(16) as f64, self.status.out_size.1.max(9) as f64) }
    }

    /// A layer's picture size (for crops), when known.
    fn source_aspect(&self, l: &Layer) -> f64 {
        let ratio = |s: (u32, u32)| s.0 as f64 / s.1.max(1) as f64;
        match l.source {
            Source::Screen if self.status.content.0 > 0 => ratio(self.status.content),
            Source::Camera => self.status.cam.as_ref().map_or(16.0 / 9.0, |c| ratio(c.1)),
            Source::Image => l.image.as_deref().and_then(|p| image::image_dimensions(p).ok()).map_or(1.0, ratio),
            _ => 16.0 / 9.0,
        }
    }

    /// Undo history: a step is kept once the mouse is up and something
    /// changed (a whole drag or slider move is one step).
    fn track_undo(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let cur: Snapshot = (self.settings.layout.clone(), self.settings.layout_v.clone());
        match &self.undo_base {
            Some(b) if *b != cur => {
                self.undo.push(b.clone());
                if self.undo.len() > 60 {
                    self.undo.remove(0);
                }
                self.redo.clear();
                self.undo_base = Some(cur);
            }
            None => self.undo_base = Some(cur),
            _ => {}
        }
    }

    pub fn undo_layout(&mut self, back: bool) {
        let (from, to) = if back { (&mut self.undo, &mut self.redo) } else { (&mut self.redo, &mut self.undo) };
        let Some(prev) = from.pop() else { return };
        to.push((self.settings.layout.clone(), self.settings.layout_v.clone()));
        self.settings.layout = prev.0.clone();
        self.settings.layout_v = prev.1.clone();
        self.undo_base = Some(prev);
        let n = |l: &Option<Layout>| l.as_ref().map_or(0, |l| l.layers.len());
        if self.sel_layer.is_some_and(|i| i >= if self.edit_v { n(&self.settings.layout_v) } else { n(&self.settings.layout) }) {
            self.sel_layer = None;
        }
        self.settings.keep_vertical();
        self.settings_changed();
    }

    /// Edits on the selected layer of the layout being edited.
    fn layer_op(&mut self, op: LayerOp) {
        let v = self.edit_v;
        let frame = self.frame(v);
        let sel = self.sel_layer;
        let Some(layout) = self.edited(v) else { return };
        let Some(i) = sel.filter(|&i| i < layout.layers.len()) else { return };
        let n = layout.layers.len();
        let mut new_sel = Some(i);
        match op {
            LayerOp::Delete => {
                layout.layers.remove(i);
                new_sel = None;
            }
            LayerOp::Duplicate => {
                let mut l = layout.layers[i].clone();
                l.rect = clamp([l.rect[0] + 0.03, l.rect[1] + 0.03 * frame.0 / frame.1, l.rect[2], l.rect[3]]);
                layout.layers.insert(i + 1, l);
                new_sel = Some(i + 1);
            }
            LayerOp::Front => {
                let l = layout.layers.remove(i);
                layout.layers.push(l);
                new_sel = Some(n - 1);
            }
            LayerOp::Back => {
                let l = layout.layers.remove(i);
                layout.layers.insert(0, l);
                new_sel = Some(0);
            }
            LayerOp::Up if i + 1 < n => {
                layout.layers.swap(i, i + 1);
                new_sel = Some(i + 1);
            }
            LayerOp::Down if i > 0 => {
                layout.layers.swap(i, i - 1);
                new_sel = Some(i - 1);
            }
            LayerOp::Nudge(dx, dy) => {
                let r = layout.layers[i].rect;
                layout.layers[i].rect = clamp([r[0] + dx / frame.0, r[1] + dy / frame.1, r[2], r[3]]);
            }
            LayerOp::Place(k) => {
                let l = &mut layout.layers[i];
                l.rect = clamp(place(k, l.rect, frame));
                l.rotation = 0.0;
            }
            LayerOp::Toggle => layout.layers[i].visible = !layout.layers[i].visible,
            LayerOp::Fill(f) => layout.layers[i].fill = f,
            LayerOp::ResetCrop => layout.layers[i].crop = [0.0, 0.0, 1.0, 1.0],
            LayerOp::ResetTurn => layout.layers[i].rotation = 0.0,
            _ => {}
        }
        self.sel_layer = new_sel;
        if matches!(op, LayerOp::Delete | LayerOp::ResetCrop) {
            self.crop_mode = false;
        }
        self.settings.keep_vertical();
        self.settings_changed();
    }

    /// Turn crop mode on for the selected layer. A filled layer's visible
    /// part becomes its crop first, so cutting starts from what you see.
    fn start_crop(&mut self) {
        let v = self.edit_v;
        let frame = self.frame(v);
        let Some(i) = self.sel_layer else { return };
        let aspect = {
            let Some(l) = self.edited(v).and_then(|l| l.layers.get(i)).cloned() else { return };
            if l.source == Source::Text {
                return;
            }
            self.source_aspect(&l)
        };
        if let Some(l) = self.edited(v).and_then(|l| l.layers.get_mut(i)) {
            let c = l.crop_rect();
            let shown = aspect * c[2] / c[3];
            let boxa = l.rect[2] * frame.0 / (l.rect[3] * frame.1);
            if l.fill && (shown - boxa).abs() > 0.01 {
                // Cover crop of the current crop to the box's shape.
                l.crop = if shown > boxa {
                    let w = c[2] * boxa / shown;
                    [c[0] + (c[2] - w) / 2.0, c[1], w, c[3]]
                } else {
                    let h = c[3] * shown / boxa;
                    [c[0], c[1] + (c[3] - h) / 2.0, c[2], h]
                };
            } else if !l.fill && (shown - boxa).abs() > 0.01 {
                // Fit: the box becomes the picture's shape (what you see).
                let [x, y, w, h] = l.rect;
                let (cx, cy) = (x + w / 2.0, y + h / 2.0);
                let (nw, nh) = if shown > boxa { (w, w * frame.0 / shown / frame.1) } else { (h * frame.1 * shown / frame.0, h) };
                l.rect = [cx - nw / 2.0, cy - nh / 2.0, nw, nh];
            }
        }
        self.crop_mode = true;
        self.settings_changed();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum LayerOp {
    Delete,
    Duplicate,
    Front,
    Back,
    Up,
    Down,
    Nudge(f64, f64),
    Place(usize),
    Toggle,
    Fill(bool),
    ResetCrop,
    ResetTurn,
    #[allow(dead_code)]
    None,
}

/// Keyboard shortcuts while editing (not while typing in a field).
fn keys(app: &mut App, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let (ops, undo, redo, esc) = ctx.input(|i| {
        let mut ops = vec![];
        let step = if i.modifiers.shift { 10.0 } else { 1.0 };
        for (k, dx, dy) in [(egui::Key::ArrowLeft, -step, 0.0), (egui::Key::ArrowRight, step, 0.0), (egui::Key::ArrowUp, 0.0, -step), (egui::Key::ArrowDown, 0.0, step)] {
            if i.key_pressed(k) {
                ops.push(LayerOp::Nudge(dx, dy));
            }
        }
        if i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace) {
            ops.push(LayerOp::Delete);
        }
        if i.modifiers.command && i.key_pressed(egui::Key::D) {
            ops.push(LayerOp::Duplicate);
        }
        let undo = i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z);
        let redo = i.modifiers.command && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z)));
        (ops, undo, redo, i.key_pressed(egui::Key::Escape))
    });
    if esc {
        if app.crop_mode {
            app.crop_mode = false;
        } else {
            app.sel_layer = None;
        }
    }
    if undo {
        app.undo_layout(true);
    }
    if redo {
        app.undo_layout(false);
    }
    if app.sel_layer.is_some() {
        for op in ops {
            app.layer_op(op);
        }
    }
}

/// Mouse editing of the layout on the preview image `img`: the 16:9 one,
/// or (`vertical`) the 9:16 one, which starts from the automatic layout.
pub fn layout_edit(app: &mut App, ui: &mut Ui, img: Rect, vertical: bool) {
    let locked = (!vertical && app.settings.layout_locked) || (vertical && app.settings.layout_locked_v);
    let layout = if vertical {
        app.settings.layout_v.clone().unwrap_or_else(|| app.settings.vertical_layout())
    } else {
        let Some(l) = app.settings.layout.clone() else { return };
        l
    };
    let resp = ui.interact(img, Id::new(("layout-edit", vertical)), if locked { Sense::hover() } else { Sense::click_and_drag() });
    if locked {
        // Locked: nothing moves by accident. Say so on hover.
        if resp.hovered() {
            let p = ui.painter_at(img);
            let r = Rect::from_center_size(img.center_bottom() - vec2(0.0, 22.0), vec2(150.0, 24.0));
            p.rect_filled(r, CornerRadius::same(12), Color32::from_black_alpha(170));
            p.text(r.center(), Align2::CENTER_CENTER, tr("Locked: unlock to edit", "Bloqueado: desbloquea para editar"), body(11.5), TEXT);
        }
        return;
    }
    // Selection belongs to one picture at a time.
    let pressed = resp.drag_started() || resp.clicked() || resp.secondary_clicked();
    if pressed && app.edit_v != vertical {
        app.edit_v = vertical;
        app.sel_layer = None;
        app.drag = None;
        app.crop_mode = false;
    }
    let mine = app.edit_v == vertical;
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let shift = ui.input(|i| i.modifiers.shift);
    let sel = app.sel_layer.filter(|_| mine).and_then(|i| layout.layers.get(i).map(|l| (i, l.clone())));
    let crop = app.crop_mode && mine;

    // What's under the pointer: a handle of the selected layer, its turn
    // handle, or a layer.
    let grab = |p: Pos2| -> Option<Kind> {
        let (_, l) = sel.as_ref()?;
        let r = to_screen(l.rect, img);
        if !crop && (rot_handle(r, l.rotation) - p).length() <= HANDLE {
            return Some(Kind::Rotate);
        }
        handles(r, l.rotation).iter().position(|h| (*h - p).length() <= HANDLE * 0.9).map(Kind::Handle)
    };
    let hovered = resp.hover_pos().and_then(|p| hit(&layout, img, p));
    if let Some(p) = resp.hover_pos() {
        let cursor = match grab(p) {
            Some(Kind::Rotate) => CursorIcon::Alias,
            Some(Kind::Handle(h)) => {
                // The resize arrow that matches the handle as turned.
                let a = sel.as_ref().map_or(0.0, |(_, l)| l.rotation);
                let (sx, sy) = signs(h);
                let d = rot(vec2(sx, sy), a);
                let ang = d.y.atan2(d.x).to_degrees().rem_euclid(180.0);
                match ((ang + 22.5) / 45.0) as i32 % 4 {
                    0 => CursorIcon::ResizeHorizontal,
                    1 => CursorIcon::ResizeNwSe,
                    2 => CursorIcon::ResizeVertical,
                    _ => CursorIcon::ResizeNeSw,
                }
            }
            _ if hovered.is_some() => CursorIcon::Grab,
            _ => CursorIcon::Default,
        };
        ui.ctx().set_cursor_icon(cursor);
    }

    if resp.drag_started()
        && let Some(p) = pointer
    {
        let kind = if mine { grab(p) } else { None };
        let target = if kind.is_some() { app.sel_layer } else { hit(&layout, img, p) };
        if target != app.sel_layer {
            app.crop_mode = false;
        }
        app.sel_layer = target;
        app.drag = target.and_then(|i| layout.layers.get(i)).map(|l| Drag { layer: target.unwrap_or(0), kind: kind.unwrap_or(Kind::Move), start: l.rect, rotation: l.rotation, crop: l.crop_rect(), from: p });
    }
    if (resp.clicked() || resp.secondary_clicked())
        && let Some(p) = pointer
    {
        let t = hit(&layout, img, p);
        if t != app.sel_layer {
            app.crop_mode = false;
        }
        app.sel_layer = t;
    }
    // A double click on a layer crops it.
    if resp.double_clicked() && app.sel_layer.is_some() {
        app.start_crop();
    }
    app.guides = (None, None);
    if resp.dragged()
        && let (Some(d), Some(p)) = (app.drag, pointer)
    {
        let (iw, ih) = (img.width() as f64, img.height() as f64);
        let r0 = to_screen(d.start, img);
        let c0 = r0.center();
        let mut new_rot = d.rotation;
        let mut new_crop = d.crop;
        let mirror = layout.layers.get(d.layer).is_some_and(|l| l.mirror && l.source == Source::Camera);
        let mut r = match d.kind {
            Kind::Move => {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                let (dx, dy) = (((p.x - d.from.x) as f64) / iw, ((p.y - d.from.y) as f64) / ih);
                [d.start[0] + dx, d.start[1] + dy, d.start[2], d.start[3]]
            }
            Kind::Rotate => {
                ui.ctx().set_cursor_icon(CursorIcon::Alias);
                let a0 = (d.from - c0).angle() as f64;
                let a1 = (p - c0).angle() as f64;
                let mut a = d.rotation + (a1 - a0).to_degrees();
                a = (a + 180.0).rem_euclid(360.0) - 180.0;
                // Shift: steps of 15°; otherwise it settles on straight
                // angles when close.
                if shift {
                    a = (a / 15.0).round() * 15.0;
                } else {
                    let s = (a / 90.0).round() * 90.0;
                    if (a - s).abs() < 3.0 {
                        a = s;
                    }
                }
                new_rot = a;
                d.start
            }
            Kind::Handle(h) => {
                let (sx, sy) = signs(h);
                // The pointer's move in the box's own (turned) frame.
                let dl = rot(p - d.from, -d.rotation);
                let (w0, h0) = (r0.width(), r0.height());
                let mut nw = if sx != 0.0 { (w0 + sx * dl.x).max(12.0) } else { w0 };
                let mut nh = if sy != 0.0 { (h0 + sy * dl.y).max(12.0) } else { h0 };
                if crop {
                    // Cut the picture: the content stays where it is.
                    let [cx, cy, cw, ch] = d.crop;
                    let (sxc, _) = if mirror { (-sx, sy) } else { (sx, sy) };
                    let mut kw = (nw / w0) as f64;
                    let mut kh = (nh / h0) as f64;
                    // Not past the picture's edges.
                    if sxc > 0.0 {
                        kw = kw.min((1.0 - cx) / cw);
                    } else if sxc < 0.0 {
                        kw = kw.min((cx + cw) / cw);
                    }
                    if sy > 0.0 {
                        kh = kh.min((1.0 - cy) / ch);
                    } else if sy < 0.0 {
                        kh = kh.min((cy + ch) / ch);
                    }
                    let (ncw, nch) = (cw * kw, ch * kh);
                    let ncx = if sxc < 0.0 { cx + cw - ncw } else { cx };
                    let ncy = if sy < 0.0 { cy + ch - nch } else { cy };
                    new_crop = [ncx, ncy, ncw, nch];
                    nw = w0 * kw as f32;
                    nh = h0 * kh as f32;
                } else if shift && sx != 0.0 && sy != 0.0 {
                    // Keep the shape: follow the larger change.
                    let k = (nw / w0).max(nh / h0);
                    nw = w0 * k;
                    nh = h0 * k;
                }
                // The opposite side stays put.
                let shift_l = vec2(sx * (nw - w0) / 2.0, sy * (nh - h0) / 2.0);
                let c = c0 + rot(shift_l, d.rotation);
                let rr = Rect::from_center_size(c, vec2(nw, nh));
                [((rr.left() - img.left()) as f64) / iw, ((rr.top() - img.top()) as f64) / ih, nw as f64 / iw, nh as f64 / ih]
            }
        };
        if d.kind == Kind::Move {
            let others: Vec<[f64; 4]> = layout.layers.iter().enumerate().filter(|(i, l)| *i != d.layer && l.visible).map(|(_, l)| l.rect).collect();
            let s = snap(r, &others, 0.012);
            r[0] += s.dx;
            r[1] += s.dy;
            app.guides = (s.vx, s.hy);
        }
        if let Some(l) = app.edited(vertical).and_then(|l| l.layers.get_mut(d.layer)) {
            l.rect = if d.rotation.abs() > 0.01 || new_rot.abs() > 0.01 { soft_clamp(r) } else { clamp(r) };
            l.rotation = new_rot;
            l.crop = new_crop;
        }
        app.layout_live();
    }
    if resp.drag_stopped() {
        app.drag = None;
        app.settings.keep_vertical();
        app.settings_changed();
    }
    // Right-click: the layer's menu.
    let menu_layer = app.sel_layer.filter(|_| app.edit_v == vertical).and_then(|i| layout.layers.get(i).cloned());
    resp.context_menu(|ui| {
        let Some(l) = &menu_layer else {
            ui.label(egui::RichText::new(tr("Right-click a layer for its options", "Clic derecho en una capa para sus opciones")).color(MUTED));
            return;
        };
        ui.label(egui::RichText::new(l.name()).strong());
        ui.separator();
        let mut op = None;
        if ui.button(tr("Bring to front", "Traer al frente")).clicked() {
            op = Some(LayerOp::Front);
        }
        if ui.button(tr("Send to back", "Enviar al fondo")).clicked() {
            op = Some(LayerOp::Back);
        }
        if ui.button(tr("Duplicate   Ctrl+D", "Duplicar   Ctrl+D")).clicked() {
            op = Some(LayerOp::Duplicate);
        }
        ui.separator();
        if l.source != Source::Text {
            if ui.button(tr("Crop…", "Recortar…")).clicked() {
                app.start_crop();
                ui.close();
            }
            if l.cropped() && ui.button(tr("Reset crop", "Quitar recorte")).clicked() {
                op = Some(LayerOp::ResetCrop);
            }
            if ui.button(if l.fill { tr("Fit the whole picture", "Ajustar la imagen completa") } else { tr("Fill the box", "Llenar el cuadro") }).clicked() {
                op = Some(LayerOp::Fill(!l.fill));
            }
        }
        if l.rotation.abs() > 0.01 && ui.button(tr("Straighten", "Enderezar")).clicked() {
            op = Some(LayerOp::ResetTurn);
        }
        ui.menu_button(tr("Place", "Colocar"), |ui| {
            for (k, (en, es)) in PLACES.iter().enumerate() {
                if ui.button(tr(en, es)).clicked() {
                    op = Some(LayerOp::Place(k));
                }
            }
        });
        ui.separator();
        if ui.button(tr("Hide", "Ocultar")).clicked() {
            op = Some(LayerOp::Toggle);
        }
        if ui.button(egui::RichText::new(tr("Delete   Del", "Eliminar   Supr")).color(RED)).clicked() {
            op = Some(LayerOp::Delete);
        }
        if let Some(op) = op {
            app.layer_op(op);
            ui.close();
        }
    });

    // Hover outline, selection, handles and guides.
    let painter = ui.painter_at(img.expand(ROT_GAP + HANDLE));
    if let Some(i) = hovered.filter(|h| Some(*h) != app.sel_layer.filter(|_| mine) && app.drag.is_none())
        && let Some(l) = layout.layers.get(i)
    {
        let r = to_screen(l.rect, img);
        painter.add(egui::Shape::closed_line(outline(r, l.rotation), Stroke::new(1.0, Color32::from_white_alpha(150))));
        painter.text(r.left_top() + vec2(4.0, 4.0), Align2::LEFT_TOP, l.source.label(), semibold(11.0), Color32::from_white_alpha(200));
    }
    if let Some(l) = app.sel_layer.filter(|_| app.edit_v == vertical).and_then(|i| app.edited(vertical).and_then(|lay| lay.layers.get(i)).cloned()) {
        let r = to_screen(l.rect, img);
        let col = if app.crop_mode { Color32::from_rgb(80, 200, 255) } else { ORANGE };
        painter.add(egui::Shape::closed_line(outline(r, l.rotation), Stroke::new(1.5, col)));
        let hs = handles(r, l.rotation);
        for (k, h) in hs.iter().enumerate() {
            if app.crop_mode {
                // Crop marks: L shapes on the corners, bars on the sides.
                let (sx, sy) = signs(k);
                let len = 10.0;
                let a = rot(vec2(-sx, 0.0), l.rotation) * len;
                let b = rot(vec2(0.0, -sy), l.rotation) * len;
                let st = Stroke::new(3.5, col);
                if sx != 0.0 && sy != 0.0 {
                    painter.line_segment([*h, *h + a], st);
                    painter.line_segment([*h, *h + b], st);
                } else {
                    let along = if sx == 0.0 { rot(vec2(1.0, 0.0), l.rotation) } else { rot(vec2(0.0, 1.0), l.rotation) };
                    painter.line_segment([*h - along * len, *h + along * len], st);
                }
            } else {
                let size = if k < 4 { HANDLE } else { HANDLE * 0.8 };
                let hr = Rect::from_center_size(*h, Vec2::splat(size));
                painter.rect_filled(hr, CornerRadius::same(3), Color32::WHITE);
                painter.rect_stroke(hr, CornerRadius::same(3), Stroke::new(1.5, col), StrokeKind::Inside);
            }
        }
        if !app.crop_mode {
            // The turn handle on a short stem above the box.
            let top = r.center() + rot(vec2(0.0, -r.height() / 2.0), l.rotation);
            let rh = rot_handle(r, l.rotation);
            painter.line_segment([top, rh], Stroke::new(1.5, col));
            painter.circle_filled(rh, 6.5, Color32::WHITE);
            painter.circle_stroke(rh, 6.5, Stroke::new(1.5, col));
        }
        let tag = if app.crop_mode { format!("{} · {}", l.source.label(), tr("crop: drag the edges, Esc when done", "recorte: arrastra los bordes, Esc al terminar")) } else { l.source.label().to_string() };
        let tp = r.left_top() + vec2(4.0, -16.0);
        painter.text(tp, Align2::LEFT_TOP, tag, semibold(11.0), col);
        if l.rotation.abs() > 0.01 && app.drag.is_some_and(|d| d.kind == Kind::Rotate) {
            painter.text(r.center(), Align2::CENTER_CENTER, format!("{:.0}°", l.rotation), semibold(14.0), Color32::WHITE);
        }
    }
    let guide = Stroke::new(1.0, ORANGE.gamma_multiply(0.9));
    if let Some(x) = app.guides.0 {
        let gx = img.left() + x as f32 * img.width();
        painter.line_segment([pos2(gx, img.top()), pos2(gx, img.bottom())], guide);
    }
    if let Some(y) = app.guides.1 {
        let gy = img.top() + y as f32 * img.height();
        painter.line_segment([pos2(img.left(), gy), pos2(img.right(), gy)], guide);
    }
}

/// Turned layers may reach past the frame a little (their corners would
/// otherwise stop them early); the center stays inside.
fn soft_clamp(r: [f64; 4]) -> [f64; 4] {
    let w = r[2].clamp(0.04, 2.0);
    let h = r[3].clamp(0.04, 2.0);
    let cx = (r[0] + w / 2.0).clamp(0.0, 1.0);
    let cy = (r[1] + h / 2.0).clamp(0.0, 1.0);
    [cx - w / 2.0, cy - h / 2.0, w, h]
}

/// A small heading inside the card.
fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).color(FAINT).size(11.5).strong());
}

/// A small icon of a place in the frame.
fn place_icon(ui: &mut Ui, k: usize, tip: &str, vertical: bool) -> bool {
    let size = if vertical { vec2(18.0, 28.0) } else { vec2(30.0, 20.0) };
    let (r, resp) = ui.allocate_exact_size(size + vec2(8.0, 8.0), Sense::click());
    let resp = resp.on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand);
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(r, CornerRadius::same(6), CONTROL);
    }
    let f = Rect::from_center_size(r.center(), size);
    p.rect_stroke(f, CornerRadius::same(3), Stroke::new(1.2, MUTED), StrokeKind::Inside);
    let q = place(k, [0.0, 0.0, 0.4, 0.4], (size.x as f64, size.y as f64));
    let b = Rect::from_min_size(pos2(f.left() + q[0] as f32 * f.width(), f.top() + q[1] as f32 * f.height()), vec2(q[2] as f32 * f.width(), q[3] as f32 * f.height())).shrink(1.5);
    p.rect_filled(b, CornerRadius::same(2), if resp.hovered() { ORANGE } else { TEXT });
    resp.clicked()
}

/// The "Layers" card (in place of the camera card while a layout is custom).
pub fn layers_card(app: &mut App, ui: &mut Ui, card: Rect) {
    let ctx = ui.ctx().clone();
    app.track_undo(&ctx);
    keys(app, &ctx);
    let p = ui.painter().clone();
    theme::card(&p, card);
    let folded = app.settings.fold_camera;
    if super::ui::fold_title(ui, card, tr("Layers", "Capas"), folded, "layers") {
        app.settings.fold_camera = !folded;
        app.settings_changed();
    }
    if folded {
        return;
    }
    let mut save = false;
    {
        let label = tr("Save layout", "Guardar layout");
        let bw = 34.0 + ui.painter().layout_no_wrap(label.to_string(), semibold(13.5), TEXT).size().x;
        let b = Rect::from_min_size(pos2(card.right() - 18.0 - bw, card.top() + 11.0), vec2(bw, 28.0));
        if super::media_ui::pill(ui, b, "layers-save", label, true) {
            save = true;
        }
        // Undo / redo.
        for (k, (sym, tip, on)) in [("↶", tr("Undo (Ctrl+Z)", "Deshacer (Ctrl+Z)"), !app.undo.is_empty()), ("↷", tr("Redo (Ctrl+Y)", "Rehacer (Ctrl+Y)"), !app.redo.is_empty())].into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(b.left() - 8.0 - 34.0 * (2 - k) as f32, b.top()), vec2(28.0, 28.0));
            let resp = ui.interact(r, Id::new(("layers-undo", k)), if on { Sense::click() } else { Sense::hover() }).on_hover_text(tip);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, on, tip));
            ui.painter().rect_filled(r, CornerRadius::same(14), if on && resp.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 120) } else { Color32::from_rgba_premultiplied(40, 40, 44, 100) });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, sym, semibold(15.0), if on { TEXT } else { FAINT });
            if on && resp.clicked() {
                app.undo_layout(k == 0);
            }
        }
    }
    let inner = Rect::from_min_max(pos2(card.left() + 18.0, card.top() + 50.0), pos2(card.right() - 18.0, card.bottom() - 12.0));
    let cam_aspect = app.status.cam.as_ref().map_or(16.0 / 9.0, |c| c.1.0 as f64 / c.1.1.max(1) as f64);
    let has_cam = app.status.cam.is_some();
    let pip = app.settings.pip();
    let mut changed = false;
    let mut op: Option<LayerOp> = None;
    let mut crop_now = false;
    let show_v = app.settings.vertical_preview;
    let show_h = app.settings.horizontal_preview || !show_v;
    // Editing the picture that's shown (both shown: the one picked here).
    if !show_v && app.edit_v {
        app.edit_v = false;
        app.sel_layer = None;
    } else if !show_h && !app.edit_v {
        app.edit_v = true;
        app.sel_layer = None;
    }
    let v = app.edit_v;
    let (ow, oh) = app.frame(v);
    let content_aspect = app.source_aspect(&Layer::default());
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical().max_height(inner.height()).auto_shrink([false; 2]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show(ui, |ui| {
            // Which layout: 16:9 or 9:16.
            if show_h && show_v {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr("Editing", "Editando")).color(MUTED).size(13.0));
                    for (vert, label) in [(false, "16:9"), (true, "9:16")] {
                        if ui.selectable_label(app.edit_v == vert, egui::RichText::new(label).strong()).on_hover_text(tr("Its own layers and settings (or click its preview)", "Sus propias capas y ajustes (o haz clic en su vista)")).clicked() && app.edit_v != vert {
                            app.edit_v = vert;
                            app.sel_layer = None;
                            app.crop_mode = false;
                        }
                    }
                });
            }
            let sel = app.sel_layer;
            let crop_mode = app.crop_mode;
            let layout = if v {
                let auto = app.settings.vertical_layout();
                app.settings.layout_v.get_or_insert(auto)
            } else {
                match app.settings.layout.as_mut() {
                    Some(l) => l,
                    None => return,
                }
            };
            // Add layers.
            heading(ui, tr("ADD", "AGREGAR"));
            let mut added = None;
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr("+ Screen", "+ Pantalla")).clicked() {
                    let a = content_aspect / (ow / oh);
                    let (w, h) = if a > 1.0 { (0.5, 0.5 / a) } else { (0.5 * a, 0.5) };
                    added = Some(Layer { source: Source::Screen, rect: [(1.0 - w) / 2.0, (1.0 - h) / 2.0, w, h], fill: false, ..Default::default() });
                }
                if ui.add_enabled(has_cam, egui::Button::new(tr("+ Camera", "+ Cámara"))).on_disabled_hover_text(tr("Pick a camera in Settings", "Elige una cámara en Ajustes")).clicked() {
                    added = Some(layout::new_camera(&pip, ow, oh, cam_aspect));
                }
                if ui.button(tr("+ Image", "+ Imagen")).on_hover_text(tr("PNG or JPG: logo, frame, background…", "PNG o JPG: logo, marco, fondo…")).clicked()
                    && let Some(path) = pick_image()
                {
                    let aspect = image::image_dimensions(&path).map_or(1.0, |(w, h)| w as f64 / h.max(1) as f64);
                    added = Some(Layer::image(&path.to_string_lossy(), aspect, ow / oh));
                }
                if ui.button(tr("+ Guest", "+ Invitado")).on_hover_text(tr("A guest from the Guests page, once on stage", "Un invitado de la página Invitados, cuando está en vivo")).clicked() {
                    let seat = layout.layers.iter().filter(|l| l.source == Source::Guest).count().min(7) as u8;
                    added = Some(Layer { source: Source::Guest, guest: seat, rect: [0.55, 0.3, 0.4, 0.4], fill: true, roundness: 0.06, ..Default::default() });
                }
                if ui.button(tr("+ Text", "+ Texto")).on_hover_text(tr("Lower third: name and title", "Lower third: nombre y título")).clicked() {
                    added = Some(Layer::lower_third(tr("Your name", "Tu nombre"), tr("Your title", "Tu título"), 1));
                }
            });
            let mut new_sel = None;
            if let Some(l) = added {
                layout.layers.push(l);
                new_sel = Some(layout.layers.len() - 1);
                changed = true;
            }
            // The list, top layer first.
            heading(ui, tr("LAYERS (TOP FIRST)", "CAPAS (LA DE ARRIBA PRIMERO)"));
            let n = layout.layers.len();
            if n == 0 {
                ui.label(egui::RichText::new(tr("No layers yet: add one above.", "Aún no hay capas: agrega una arriba.")).color(FAINT).size(12.0));
            }
            let mut pick = None;
            let mut cam_sel = false;
            for i in (0..n).rev() {
                let is_sel = sel == Some(i);
                let l = &mut layout.layers[i];
                let frame = egui::Frame::new()
                    .fill(if is_sel { Color32::from_rgba_premultiplied(70, 34, 0, 90) } else { Color32::from_rgba_premultiplied(30, 30, 33, 45) })
                    .stroke(if is_sel { Stroke::new(1.0, ORANGE) } else { Stroke::NONE })
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(8, 3));
                // The name selects the layer; the buttons stay clickable (a
                // click area over the whole row used to swallow them).
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if icon_button(ui, if l.visible { Icon::Eye } else { Icon::EyeOff }, tr("Show / hide", "Mostrar / ocultar")) {
                            l.visible = !l.visible;
                            changed = true;
                        }
                        let mut name = l.name();
                        if name.chars().count() > 24 {
                            name = format!("{}…", name.chars().take(23).collect::<String>());
                        }
                        let label = ui.add(egui::Label::new(egui::RichText::new(name).size(13.0).color(if l.visible { TEXT } else { FAINT })).sense(Sense::click()));
                        if label.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tr("Select (or click it on the preview)", "Seleccionar (o haz clic en ella en la vista)")).clicked() {
                            pick = Some(i);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, Icon::Remove, tr("Delete layer", "Eliminar capa")) {
                                pick = Some(i);
                                op = Some(LayerOp::Delete);
                            }
                            if icon_button(ui, Icon::Down, tr("Send backward", "Enviar atrás")) {
                                pick = Some(i);
                                op = Some(LayerOp::Down);
                            }
                            if icon_button(ui, Icon::Up, tr("Bring forward", "Traer al frente")) {
                                pick = Some(i);
                                op = Some(LayerOp::Up);
                            }
                        });
                    });
                });
            }
            // The selected layer's settings.
            if let Some(i) = sel.filter(|&i| i < n) {
                let l = &mut layout.layers[i];
                ui.add_space(8.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(l.name()).strong().size(14.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(tr("Duplicate", "Duplicar")).on_hover_text("Ctrl+D").clicked() {
                            op = Some(LayerOp::Duplicate);
                        }
                    });
                });
                heading(ui, tr("PLACE", "COLOCAR"));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (k, (en, es)) in PLACES.iter().enumerate() {
                        if place_icon(ui, k, tr(en, es), v) {
                            op = Some(LayerOp::Place(k));
                        }
                    }
                });
                match l.source {
                    Source::Text => {
                        heading(ui, tr("TEXT", "TEXTO"));
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(tr("Name", "Nombre")).color(MUTED).size(13.0));
                            changed |= ui.add(egui::TextEdit::singleline(&mut l.text).desired_width(200.0)).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(tr("Title", "Título")).color(MUTED).size(13.0));
                            changed |= ui.add(egui::TextEdit::singleline(&mut l.subtitle).desired_width(200.0)).changed();
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(tr("Style", "Estilo")).color(MUTED).size(13.0));
                            for s in 0..crate::brand::STYLES.len() {
                                if ui.selectable_label(l.style == s as u8, crate::brand::style_name(s as u8)).clicked() {
                                    l.style = s as u8;
                                    changed = true;
                                }
                            }
                        });
                        ui.label(egui::RichText::new(tr("Colors and font: Settings → Brand.", "Colores y tipografía: Ajustes → Marca.")).color(FAINT).size(11.5));
                    }
                    Source::Guest => {
                        heading(ui, tr("GUEST", "INVITADO"));
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(tr("Seat", "Lugar")).color(MUTED).size(13.0));
                            for s in 0..4u8 {
                                changed |= ui.selectable_value(&mut l.guest, s, format!("{}", s + 1)).on_hover_text(tr("Who was brought on stage 1st, 2nd…", "Quién subió 1.º, 2.º…")).changed();
                            }
                        });
                    }
                    Source::Image => {
                        if ui.button(tr("Change image…", "Cambiar imagen…")).clicked()
                            && let Some(path) = pick_image()
                        {
                            l.image = Some(path.to_string_lossy().into_owned());
                            changed = true;
                        }
                    }
                    _ => {}
                }
                if l.source != Source::Text {
                    heading(ui, tr("PICTURE", "IMAGEN"));
                    ui.horizontal_wrapped(|ui| {
                        changed |= ui.selectable_value(&mut l.fill, true, tr("Fill", "Llenar")).on_hover_text(tr("Fill the box (the edges may be cut)", "Llenar el cuadro (se pueden cortar los bordes)")).changed();
                        changed |= ui.selectable_value(&mut l.fill, false, tr("Fit", "Ajustar")).on_hover_text(tr("Show the whole picture", "Mostrar la imagen completa")).changed();
                        ui.add_space(8.0);
                        let crop_label = if crop_mode { tr("Done cropping", "Listo") } else { tr("Crop", "Recortar") };
                        if ui.selectable_label(crop_mode, crop_label).on_hover_text(tr("Drag the edges on the preview to cut the picture (or double-click the layer)", "Arrastra los bordes en la vista para recortar (o doble clic en la capa)")).clicked() {
                            crop_now = true;
                        }
                        if l.cropped() && ui.button(tr("Reset crop", "Quitar recorte")).clicked() {
                            op = Some(LayerOp::ResetCrop);
                        }
                    });
                }
                heading(ui, tr("LOOK", "ESTILO"));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr("Shape", "Forma")).color(MUTED).size(13.0));
                    for s in [Shape::Rect, Shape::Rounded, Shape::Circle, Shape::Pill, Shape::Squircle] {
                        let (r, resp) = ui.allocate_exact_size(vec2(34.0, 28.0), Sense::click());
                        let on = l.shape == s;
                        let fill = if on { Color32::from_rgba_premultiplied(62, 62, 68, 120) } else if resp.hovered() { CONTROL } else { Color32::TRANSPARENT };
                        ui.painter().rect_filled(r, CornerRadius::same(8), fill);
                        super::ui::shape_icon(ui.painter(), s, r.center(), if on { TEXT } else { MUTED });
                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            l.shape = s;
                            changed = true;
                        }
                    }
                });
                let wide = (ui.available_width() - 170.0).max(70.0);
                ui.spacing_mut().slider_width = wide;
                if matches!(l.shape, Shape::Rounded | Shape::Squircle | Shape::Square) {
                    changed |= ui.add(egui::Slider::new(&mut l.roundness, 0.0..=1.0).text(tr("Roundness", "Redondez"))).changed();
                }
                let mut op_pct = l.opacity * 100.0;
                if ui.add(egui::Slider::new(&mut op_pct, 0.0..=100.0).suffix(" %").max_decimals(0).text(tr("Opacity", "Opacidad"))).changed() {
                    l.opacity = op_pct / 100.0;
                    changed = true;
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().slider_width = (wide - 60.0).max(60.0);
                    changed |= ui.add(egui::Slider::new(&mut l.rotation, -180.0..=180.0).suffix("°").max_decimals(0).text(tr("Turn", "Girar"))).on_hover_text(tr("Or drag the round handle above the layer (Shift: steps of 15°)", "O arrastra el círculo arriba de la capa (Shift: pasos de 15°)")).changed();
                    if l.rotation.abs() > 0.01 && ui.small_button("0°").on_hover_text(tr("Straighten", "Enderezar")).clicked() {
                        l.rotation = 0.0;
                        changed = true;
                    }
                });
                changed |= ui.add(egui::Slider::new(&mut l.border, 0.0..=24.0).text(tr("Border", "Borde"))).changed();
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut l.shadow, tr("Shadow", "Sombra")).changed();
                    if l.source == Source::Camera {
                        changed |= ui.checkbox(&mut l.mirror, tr("Mirror", "Espejo")).changed();
                    }
                    if l.border > 0.0 {
                        for c in [[255, 255, 255], [255, 102, 0], [255, 196, 0], [36, 84, 255]] {
                            let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                            ui.painter().circle_filled(r.center(), 7.0, Color32::from_rgb(c[0], c[1], c[2]));
                            if l.border_color == c {
                                ui.painter().circle_stroke(r.center(), 9.0, Stroke::new(1.5, TEXT));
                            }
                            if resp.clicked() {
                                l.border_color = c;
                                changed = true;
                            }
                        }
                        changed |= ui.color_edit_button_srgb(&mut l.border_color).on_hover_text(tr("Any color", "Cualquier color")).changed();
                    }
                });
                cam_sel = l.source == Source::Camera;
            } else {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(tr(
                    "Click a layer on the preview (or in the list) to edit it. Drag to move it, corners and sides to resize (Shift keeps its shape), the round handle to turn it. Right-click for more. Double-click to crop.",
                    "Haz clic en una capa de la vista (o en la lista) para editarla. Arrástrala para moverla, las esquinas y lados para cambiar su tamaño (Shift mantiene la forma), el círculo para girarla. Clic derecho para más. Doble clic para recortar.",
                ))
                .color(FAINT)
                .size(12.0));
            }
            // A camera layer: the camera itself (which one, its background,
            // zoom, focus, light), like the Camera card.
            if cam_sel {
                heading(ui, tr("CAMERA", "CÁMARA"));
                if app.devices.cameras.is_empty() {
                    app.refresh_devices();
                }
                let cams = app.devices.cameras.clone();
                let cur = app.settings.camera.clone();
                let mut pick_cam = None;
                egui::ComboBox::from_id_salt("layer-cam")
                    .width((ui.available_width() - 8.0).min(300.0))
                    .selected_text(cur.clone().unwrap_or_else(|| tr("No camera", "Sin cámara").to_string()))
                    .show_ui(ui, |ui| {
                        for c in &cams {
                            if ui.selectable_label(cur.as_deref() == Some(c.as_str()), c).clicked() {
                                pick_cam = Some(c.clone());
                            }
                        }
                    });
                if let Some(c) = pick_cam {
                    app.settings.camera = Some(c);
                    changed = true;
                }
                if has_cam {
                    if super::ui::vbg_row(ui, &mut app.settings.vbg, 180.0) {
                        changed = true;
                    }
                    egui::CollapsingHeader::new(egui::RichText::new(tr("Zoom, focus and light", "Zoom, enfoque y luz")).color(MUTED).size(13.0))
                        .id_salt("layer-cam-controls")
                        .show(ui, |ui| super::ui::cam_controls(app, ui));
                }
            }
            let layout = if v {
                let auto = app.settings.vertical_layout();
                app.settings.layout_v.get_or_insert(auto)
            } else {
                match app.settings.layout.as_mut() {
                    Some(l) => l,
                    None => return,
                }
            };
            ui.add_space(6.0);
            ui.separator();
            // Background.
            heading(ui, tr("BACKGROUND", "FONDO"));
            ui.horizontal_wrapped(|ui| {
                let b = &app.settings.brand;
                let mut colors: Vec<[u8; 3]> = vec![];
                for c in [b.primary, b.secondary, [0, 0, 0], [18, 18, 20], [40, 18, 70], [36, 84, 255], [255, 255, 255]] {
                    if !colors.contains(&c) {
                        colors.push(c);
                    }
                }
                for c in colors {
                    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 7.5, Color32::from_rgb(c[0], c[1], c[2]));
                    ui.painter().circle_stroke(r.center(), 7.5, Stroke::new(1.0, Color32::from_white_alpha(40)));
                    if layout.background == c && layout.background_image.is_none() {
                        ui.painter().circle_stroke(r.center(), 9.5, Stroke::new(1.5, TEXT));
                    }
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        layout.background = c;
                        layout.background_image = None;
                        changed = true;
                    }
                }
                if ui.color_edit_button_srgb(&mut layout.background).on_hover_text(tr("Any color", "Cualquier color")).changed() {
                    layout.background_image = None;
                    changed = true;
                }
                if ui.button(tr("Image…", "Imagen…")).on_hover_text(tr("Your own background picture", "Tu propia imagen de fondo")).clicked()
                    && let Some(path) = pick_image()
                {
                    layout.background_image = Some(path.to_string_lossy().into_owned());
                    changed = true;
                }
                if let Some(bg) = layout.background_image.clone() {
                    let name = std::path::Path::new(&bg).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(egui::RichText::new(name).size(12.0).color(ORANGE));
                    if icon_button(ui, Icon::Remove, tr("Remove the background picture", "Quitar la imagen de fondo")) {
                        layout.background_image = None;
                        changed = true;
                    }
                }
            });
            if let Some(s) = new_sel {
                app.sel_layer = Some(s);
                app.crop_mode = false;
            }
            if let Some(i) = pick {
                if app.sel_layer != Some(i) {
                    app.crop_mode = false;
                }
                app.sel_layer = Some(i);
            }
        });
    });
    if crop_now {
        if app.crop_mode {
            app.crop_mode = false;
        } else {
            app.start_crop();
        }
    }
    if let Some(op) = op {
        app.layer_op(op);
    }
    if changed {
        app.settings.keep_vertical();
        app.settings_changed();
    }
    if save {
        app.save_dialog = Some(super::SaveDialog::default());
    }
}

#[derive(Clone, Copy)]
pub enum Icon {
    Up,
    Down,
    Remove,
    Eye,
    EyeOff,
}

/// A small square button with a drawn icon (font glyphs for these aren't
/// in every font).
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::click());
    let resp = resp.on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand);
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(7), if resp.hovered() { Color32::from_rgba_premultiplied(70, 70, 76, 140) } else { Color32::from_rgba_premultiplied(40, 40, 44, 90) });
    let c = r.center();
    let col = TEXT;
    match icon {
        Icon::Up => {
            p.add(egui::Shape::convex_polygon(vec![c + vec2(0.0, -5.0), c + vec2(5.0, 3.0), c + vec2(-5.0, 3.0)], col, Stroke::NONE));
        }
        Icon::Down => {
            p.add(egui::Shape::convex_polygon(vec![c + vec2(-5.0, -3.0), c + vec2(5.0, -3.0), c + vec2(0.0, 5.0)], col, Stroke::NONE));
        }
        Icon::Remove => {
            let red = Color32::from_rgb(255, 90, 80);
            p.rect_filled(Rect::from_center_size(c + vec2(0.0, 1.5), vec2(8.0, 9.0)), CornerRadius::same(1), red);
            p.line_segment([c + vec2(-6.0, -4.5), c + vec2(6.0, -4.5)], Stroke::new(1.6, red));
            p.line_segment([c + vec2(-2.0, -6.0), c + vec2(2.0, -6.0)], Stroke::new(1.6, red));
        }
        Icon::Eye | Icon::EyeOff => {
            let pts: Vec<Pos2> = (0..=16).map(|k| { let a = k as f32 / 16.0 * std::f32::consts::TAU; c + vec2(a.cos() * 7.0, a.sin() * 4.0) }).collect();
            let dim = if matches!(icon, Icon::EyeOff) { FAINT } else { col };
            p.add(egui::Shape::closed_line(pts, Stroke::new(1.4, dim)));
            p.circle_filled(c, 2.2, dim);
            if matches!(icon, Icon::EyeOff) {
                p.line_segment([c + vec2(-7.0, 5.0), c + vec2(7.0, -5.0)], Stroke::new(1.5, dim));
            }
        }
    }
    resp.clicked()
}

/// Windows' file picker for a picture.
pub fn pick_image() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().set_title(tr("Choose an image", "Elige una imagen")).add_filter(tr("Image", "Imagen"), &["png", "jpg", "jpeg"]).pick_file()
}

/// The tile that switches to the custom layout, drawn like the scene icons.
pub fn custom_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let st = Stroke::new(1.4, color);
    p.rect_stroke(Rect::from_center_size(c, vec2(26.0, 16.0)), CornerRadius::same(2), st, StrokeKind::Inside);
    p.rect_stroke(Rect::from_min_size(c + vec2(-9.0, -5.0), vec2(10.0, 7.0)), CornerRadius::same(1), Stroke::new(1.2, color), StrokeKind::Inside);
    p.rect_filled(Rect::from_min_size(c + vec2(3.0, 0.0), vec2(6.0, 4.0)), CornerRadius::same(1), color);
    p.line_segment([c + vec2(4.0, -5.0), c + vec2(9.0, -5.0)], Stroke::new(1.2, ORANGE));
}
