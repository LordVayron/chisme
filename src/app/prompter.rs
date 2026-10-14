//! Immersive teleprompter: a borderless window that floats above
//! everything and never shows up in a recording. Place it right under the
//! camera so you read while looking (almost) at the lens. The script
//! scrolls past a reading line at the chosen speed; it can be mirrored for
//! a beam-splitter glass. The "Guion" page edits the script.

use std::time::Instant;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, ViewportBuilder, ViewportCommand, ViewportId,
    pos2, text::LayoutJob, vec2,
};

use super::App;
use crate::i18n::l;
use crate::t;
use super::theme::*;

const TITLE: &str = "Chisme Teleprompter";
const BAR_H: f32 = 44.0;

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub playing: bool,
    /// How far the script has scrolled, in points.
    pub offset: f32,
    /// The script's laid-out height (to stop at the end).
    height: f32,
    last: Option<Instant>,
    /// 3-2-1 before scrolling from the top.
    countdown: Option<Instant>,
    excluded: bool,
}

impl App {
    pub fn toggle_prompter(&mut self) {
        let s = &mut self.prompter;
        s.open = !s.open;
        s.playing = false;
        s.countdown = None;
        s.excluded = false;
    }

    pub fn prompter_play(&mut self) {
        let s = &mut self.prompter;
        if !s.open {
            s.open = true;
            s.excluded = false;
        }
        if s.playing || s.countdown.is_some() {
            s.playing = false;
            s.countdown = None;
        } else if s.offset <= 1.0 || s.offset >= s.height {
            s.offset = 0.0;
            s.countdown = Some(Instant::now());
        } else {
            s.playing = true;
        }
        s.last = None;
    }

    fn prompter_save(&mut self) {
        let _ = self.settings.save();
    }
}

/// Shows the teleprompter window while it is open (call every frame).
pub fn window(app: &mut App, ctx: &egui::Context) {
    if !app.prompter.open {
        return;
    }
    // Opens centered at the top of the main screen, where webcams sit.
    let screen_w = unsafe { windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN) } as f32
        / ctx.pixels_per_point();
    let builder = ViewportBuilder::default()
        .with_title(TITLE)
        .with_inner_size([620.0, 300.0])
        .with_position([(screen_w - 620.0) / 2.0, 12.0])
        .with_min_inner_size([320.0, 160.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false);
    ctx.show_viewport_immediate(ViewportId::from_hash_of("teleprompter"), builder, |ui, _| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            app.prompter.open = false;
            return;
        }
        if !app.prompter.excluded {
            app.prompter.excluded = exclude(TITLE);
        }
        render(app, ui);
    });
    if app.prompter.playing || app.prompter.countdown.is_some() {
        ctx.request_repaint();
    }
}

/// Readies the window (found by title: an immediate viewport has no handle
/// of its own here): see-through, and out of every capture.
///
/// The picture is a DirectComposition visual with an alpha channel, but
/// Windows still paints the window's own surface white underneath; a
/// layered window keyed on white makes that surface invisible.
fn exclude(title: &str) -> bool {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::core::HSTRING;
    unsafe {
        let Ok(h) = FindWindowW(None, &HSTRING::from(title)) else { return false };
        if h.is_invalid() {
            return false;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE);
        SetWindowLongW(h, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as i32);
        let _ = SetLayeredWindowAttributes(h, COLORREF(0x00FF_FFFF), 255, LWA_COLORKEY);
        std::env::var_os("CHISME_SHOW_IN_CAPTURE").is_some() || SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE).is_ok()
    }
}

fn render(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let full = ui.max_rect();
    let p = ui.painter().clone();
    let cfg = app.settings.prompter.clone();
    let s = &mut app.prompter;

    // Time step.
    let now = Instant::now();
    let dt = s.last.map_or(0.0, |t| (now - t).as_secs_f32()).min(0.1);
    s.last = Some(now);
    if let Some(t) = s.countdown
        && t.elapsed().as_secs_f32() >= 3.0
    {
        s.countdown = None;
        s.playing = true;
    }
    if s.playing {
        s.offset += cfg.speed * dt;
        if s.offset >= s.height {
            s.offset = s.height;
            s.playing = false;
        }
    }

    // Background (translucent) and the whole-window drag area.
    let alpha = (cfg.opacity.clamp(0.0, 1.0) * 255.0) as u8;
    p.rect_filled(full, CornerRadius::same(18), Color32::from_rgba_unmultiplied(8, 8, 9, alpha));
    p.rect_stroke(full.shrink(0.5), CornerRadius::same(18), Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 28)), egui::StrokeKind::Inside);
    let bg = ui.interact(full, Id::new("prompter_bg"), Sense::click_and_drag());
    if bg.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if bg.double_clicked() {
        app_play(app);
    }
    let s = &mut app.prompter;

    // The script, laid out centered and scrolled past the reading line.
    let margin = 28.0;
    let wrap = (full.width() - margin * 2.0).max(50.0);
    let mut job = LayoutJob::single_section(
        cfg.script.clone(),
        egui::TextFormat { font_id: semibold(cfg.font), color: Color32::WHITE, line_height: Some(cfg.font * 1.35), ..Default::default() },
    );
    job.wrap.max_width = wrap;
    job.halign = Align::Center;
    let galley = ctx.fonts_mut(|f| f.layout_job(job));
    s.height = galley.rect.height();
    let line_y = full.top() + full.height() * cfg.line.clamp(0.1, 0.9);
    let origin = pos2(full.center().x, line_y - s.offset);
    let clip = full.shrink2(vec2(4.0, 2.0));
    let cx = full.center().x;
    // Row meshes keep texel UVs; the tessellator normally normalizes them.
    let tex = ctx.fonts(|f| f.font_image_size());
    let uv_scale = vec2(1.0 / tex[0] as f32, 1.0 / tex[1] as f32);
    for row in &galley.rows {
        let top = origin.y + row.rect().top();
        let bottom = origin.y + row.rect().bottom();
        if bottom < clip.top() || top > clip.bottom() {
            continue;
        }
        // Lines already read fade out; the ones far below too.
        let mid = (top + bottom) / 2.0;
        let read = if mid < line_y - cfg.font * 0.6 { 0.38 } else { 1.0 };
        let edge = ((mid - clip.top()) / 40.0).min((clip.bottom() - mid) / 40.0).clamp(0.0, 1.0);
        let fade = read * edge;
        let mut mesh = row.visuals.mesh.clone();
        for v in &mut mesh.vertices {
            let mut x = origin.x + row.pos.x + v.pos.x;
            if cfg.mirror {
                x = 2.0 * cx - x;
            }
            v.pos = pos2(x, origin.y + row.pos.y + v.pos.y);
            v.uv = pos2(v.uv.x * uv_scale.x, v.uv.y * uv_scale.y);
            v.color = v.color.gamma_multiply(fade);
        }
        p.with_clip_rect(clip).add(Shape::mesh(mesh));
    }

    // Reading line: two small orange arrows at the edges.
    for (x, d) in [(full.left() + 10.0, 1.0f32), (full.right() - 10.0, -1.0)] {
        p.add(Shape::convex_polygon(vec![pos2(x, line_y - 7.0), pos2(x + d * 10.0, line_y), pos2(x, line_y + 7.0)], ORANGE, Stroke::NONE));
    }
    // Progress along the bottom.
    let prog = if s.height > 0.0 { (s.offset / s.height).clamp(0.0, 1.0) } else { 0.0 };
    let track = Rect::from_min_max(pos2(full.left() + 18.0, full.bottom() - 6.0), pos2(full.right() - 18.0, full.bottom() - 4.0));
    p.rect_filled(track, CornerRadius::same(1), Color32::from_rgba_unmultiplied(255, 255, 255, 24));
    if prog > 0.0 {
        gradient_bar(&p, Rect::from_min_max(track.min, pos2(egui::lerp(track.left()..=track.right(), prog), track.bottom())), &RAINBOW);
    }

    // Countdown.
    if let Some(t) = s.countdown {
        let n = 3 - (t.elapsed().as_secs_f32() as i32).min(2);
        let c = full.center();
        p.circle_filled(c, 44.0, Color32::from_rgba_unmultiplied(0, 0, 0, 170));
        p.circle_stroke(c, 44.0, Stroke::new(2.0, ORANGE));
        p.text(c, Align2::CENTER_CENTER, n.to_string(), display(40.0), Color32::WHITE);
    }

    // Mouse wheel and keys.
    let (wheel, keys) = ctx.input(|i| {
        (
            if i.pointer.hover_pos().is_some() { i.smooth_scroll_delta.y } else { 0.0 },
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::Escape),
            ),
        )
    });
    if wheel != 0.0 {
        s.offset = (s.offset - wheel).clamp(0.0, s.height);
    }
    let mut changed = false;
    if keys.1 {
        app.settings.prompter.speed = (cfg.speed + 5.0).min(300.0);
        changed = true;
    }
    if keys.2 {
        app.settings.prompter.speed = (cfg.speed - 5.0).max(5.0);
        changed = true;
    }
    if keys.3 {
        app.prompter.offset = 0.0;
        app.prompter.playing = false;
    }
    if keys.4 {
        app.prompter.open = false;
    }
    if keys.0 {
        app_play(app);
    }

    // Toolbar: shown while the pointer is over the window or it is paused.
    let hovered = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|h| full.contains(h));
    if hovered || !app.prompter.playing {
        changed |= toolbar(app, ui, Rect::from_min_size(full.min + vec2(10.0, 8.0), vec2(full.width() - 20.0, BAR_H - 8.0)));
    }

    // Resize grip, bottom right.
    let grip = Rect::from_min_size(full.max - vec2(22.0, 22.0), vec2(20.0, 20.0));
    let g = ui.interact(grip, Id::new("prompter_grip"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeNwSe);
    if g.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast));
    }
    for i in 0..3 {
        let o = 5.0 + i as f32 * 4.0;
        p.line_segment([grip.max - vec2(o, 3.0), grip.max - vec2(3.0, o)], Stroke::new(1.2, Color32::from_rgba_unmultiplied(255, 255, 255, 70)));
    }

    if changed {
        app.prompter_save();
    }
}

fn app_play(app: &mut App) {
    app.prompter_play();
}

/// A small dark button with a painted icon or a short label.
fn button(ui: &mut Ui, rect: Rect, id: &str, tip: &str) -> Response {
    let r = ui.interact(rect, Id::new(("prompter", id)), Sense::click()).on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand);
    let fill = if r.hovered() { Color32::from_rgba_unmultiplied(70, 70, 76, 230) } else { Color32::from_rgba_unmultiplied(34, 34, 38, 220) };
    ui.painter().rect_filled(rect, CornerRadius::same((rect.height() / 2.0) as u8), fill);
    r
}

/// Returns true when a setting changed.
fn toolbar(app: &mut App, ui: &mut Ui, bar: Rect) -> bool {
    let p = ui.painter().clone();
    let h = bar.height();
    let cfg = &mut app.settings.prompter;
    let mut changed = false;
    let mut x = bar.left();
    let mut next = |w: f32| {
        let r = Rect::from_min_size(pos2(x, bar.top()), vec2(w, h));
        x += w + 6.0;
        r
    };

    // Play / pause.
    let r = next(h);
    let playing = app.prompter.playing || app.prompter.countdown.is_some();
    let play = button(ui, r, "play", &t!("Play / pause (Space, {})", "Play / pausa (Espacio, {})", app.settings.hotkeys.prompter_play));
    let c = r.center();
    if playing {
        p.rect_filled(Rect::from_center_size(c - vec2(4.0, 0.0), vec2(3.5, 13.0)), CornerRadius::same(1), TEXT);
        p.rect_filled(Rect::from_center_size(c + vec2(4.0, 0.0), vec2(3.5, 13.0)), CornerRadius::same(1), TEXT);
    } else {
        p.add(Shape::convex_polygon(vec![c + vec2(-4.0, -7.0), c + vec2(8.0, 0.0), c + vec2(-4.0, 7.0)], ORANGE, Stroke::NONE));
    }
    // Back to the top.
    let r = next(h);
    let top = button(ui, r, "top", l("Back to the top (Home)", "Volver al inicio (Inicio)"));
    let c = r.center();
    p.line_segment([c + vec2(-6.0, -7.0), c + vec2(6.0, -7.0)], Stroke::new(1.8, TEXT));
    p.add(Shape::convex_polygon(vec![c + vec2(0.0, -4.0), c + vec2(6.0, 6.0), c + vec2(-6.0, 6.0)], TEXT, Stroke::NONE));

    // Speed: − value +.
    let minus = next(h);
    let label = next(62.0);
    let plus = next(h);
    let slower = button(ui, minus, "slower", l("Slower (down arrow)", "Más lento (flecha abajo)"));
    let faster = button(ui, plus, "faster", l("Faster (up arrow)", "Más rápido (flecha arriba)"));
    p.line_segment([minus.center() - vec2(5.0, 0.0), minus.center() + vec2(5.0, 0.0)], Stroke::new(1.8, TEXT));
    p.line_segment([plus.center() - vec2(5.0, 0.0), plus.center() + vec2(5.0, 0.0)], Stroke::new(1.8, TEXT));
    p.line_segment([plus.center() - vec2(0.0, 5.0), plus.center() + vec2(0.0, 5.0)], Stroke::new(1.8, TEXT));
    p.text(label.center() - vec2(0.0, 6.0), Align2::CENTER_CENTER, format!("{:.0}", cfg.speed), semibold(13.0), TEXT);
    p.text(label.center() + vec2(0.0, 8.0), Align2::CENTER_CENTER, l("speed", "velocidad"), body(9.5), MUTED);
    if slower.clicked() {
        cfg.speed = (cfg.speed - 5.0).max(5.0);
        changed = true;
    }
    if faster.clicked() {
        cfg.speed = (cfg.speed + 5.0).min(300.0);
        changed = true;
    }

    // Text size.
    let smaller = next(h);
    let bigger = next(h);
    let a = button(ui, smaller, "smaller", l("Smaller text", "Letra más chica"));
    let b = button(ui, bigger, "bigger", l("Bigger text", "Letra más grande"));
    p.text(smaller.center(), Align2::CENTER_CENTER, "A", semibold(11.0), TEXT);
    p.text(bigger.center(), Align2::CENTER_CENTER, "A", semibold(17.0), TEXT);
    if a.clicked() {
        cfg.font = (cfg.font - 4.0).max(16.0);
        changed = true;
    }
    if b.clicked() {
        cfg.font = (cfg.font + 4.0).min(96.0);
        changed = true;
    }

    // Mirror.
    let r = next(h);
    let m = button(ui, r, "mirror", l("Mirror (for a teleprompter glass)", "Espejo (para vidrio de teleprompter)"));
    let c = r.center();
    let col = if cfg.mirror { ORANGE } else { TEXT };
    p.line_segment([c - vec2(0.0, 8.0), c + vec2(0.0, 8.0)], Stroke::new(1.2, col));
    p.add(Shape::convex_polygon(vec![c + vec2(-2.5, -6.0), c + vec2(-2.5, 6.0), c + vec2(-8.0, 6.0)], col, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![c + vec2(2.5, -6.0), c + vec2(8.0, 6.0), c + vec2(2.5, 6.0)], col, Stroke::NONE));
    if m.clicked() {
        cfg.mirror = !cfg.mirror;
        changed = true;
    }

    // Close, on the right.
    let r = Rect::from_min_size(pos2(bar.right() - h, bar.top()), vec2(h, h));
    let close = button(ui, r, "close", l("Close (Esc)", "Cerrar (Esc)"));
    let c = r.center();
    p.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], Stroke::new(1.8, TEXT));
    p.line_segment([c + vec2(5.0, -5.0), c + vec2(-5.0, 5.0)], Stroke::new(1.8, TEXT));

    if play.clicked() {
        app.prompter_play();
    }
    if top.clicked() {
        app.prompter.offset = 0.0;
        app.prompter.playing = false;
        app.prompter.countdown = None;
    }
    if close.clicked() {
        app.prompter.open = false;
    }
    changed
}

// ---------------------------------------------------------------- "Guion" page

pub fn page(app: &mut App, ui: &mut Ui, area: Rect) {
    let side_w = 300.0;
    let editor = Rect::from_min_max(area.min, pos2(area.right() - side_w - 16.0, area.bottom()));
    let side = Rect::from_min_max(pos2(editor.right() + 16.0, area.top()), area.max);
    let p = ui.painter().clone();

    // The script.
    card(&p, editor);
    let words = app.settings.prompter.script.split_whitespace().count();
    let mins = words as f32 / 150.0;
    p.text(editor.min + vec2(22.0, 20.0), Align2::LEFT_TOP, l("Script", "Guion"), medium(15.0), TEXT);
    p.text(
        pos2(editor.right() - 22.0, editor.top() + 22.0),
        Align2::RIGHT_TOP,
        t!("{} words  ·  ~{} min to read", "{} palabras  ·  ~{} min leyendo", words, (mins.ceil() as u32).max(1)),
        body(12.0),
        MUTED,
    );
    let inner = Rect::from_min_max(editor.min + vec2(18.0, 54.0), editor.max - vec2(18.0, 18.0));
    p.rect_filled(inner, CornerRadius::same(14), Color32::from_rgba_premultiplied(0, 0, 0, 90));
    let mut changed = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner.shrink(12.0)), |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let r = ui.add(
                egui::TextEdit::multiline(&mut app.settings.prompter.script)
                    .font(medium(17.0))
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .desired_rows(20)
                    .hint_text(l("Write or paste your script here…", "Escribe o pega tu guion aquí…")),
            );
            changed |= r.changed();
        });
    });

    // Controls.
    card(&p, side);
    p.text(side.min + vec2(22.0, 20.0), Align2::LEFT_TOP, "Teleprompter", medium(15.0), TEXT);
    let open = app.prompter.open;
    let b = Rect::from_min_size(side.min + vec2(22.0, 58.0), vec2(side.width() - 44.0, 44.0));
    let r = ui.interact(b, Id::new("prompter_open"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(&app.settings.hotkeys.prompter);
    let fill = if open { Color32::from_rgb(60, 60, 66) } else { Color32::WHITE };
    p.rect_filled(b, CornerRadius::same(22), if r.hovered() { fill.gamma_multiply(0.9) } else { fill });
    p.text(b.center(), Align2::CENTER_CENTER, if open { l("Close teleprompter", "Cerrar teleprompter") } else { l("Open teleprompter", "Abrir teleprompter") }, semibold(14.5), if open { TEXT } else { Color32::BLACK });
    if r.clicked() {
        app.toggle_prompter();
    }
    let b2 = Rect::from_min_size(b.left_bottom() + vec2(0.0, 10.0), vec2(b.width(), 38.0));
    let r2 = ui.interact(b2, Id::new("prompter_play"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(&app.settings.hotkeys.prompter_play);
    p.rect_filled(b2, CornerRadius::same(19), if r2.hovered() { Color32::from_rgb(50, 50, 56) } else { Color32::from_rgb(36, 36, 40) });
    let playing = app.prompter.playing || app.prompter.countdown.is_some();
    p.text(b2.center(), Align2::CENTER_CENTER, if playing { l("Pause", "Pausar") } else { "Play" }, semibold(13.5), TEXT);
    if r2.clicked() {
        app.prompter_play();
    }

    let controls = Rect::from_min_max(pos2(side.left() + 22.0, b2.bottom() + 18.0), side.max - vec2(22.0, 18.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(controls), |ui| {
        ui.spacing_mut().slider_width = (controls.width() - 70.0).max(80.0);
        let cfg = &mut app.settings.prompter;
        let mut slider = |ui: &mut Ui, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str| {
            ui.label(egui::RichText::new(label).color(MUTED).size(13.0));
            changed |= ui.add(egui::Slider::new(v, range).suffix(suffix).max_decimals(0)).changed();
            ui.add_space(6.0);
        };
        slider(ui, l("Speed", "Velocidad"), &mut cfg.speed, 5.0..=300.0, "");
        slider(ui, l("Text size", "Tamaño de letra"), &mut cfg.font, 16.0..=96.0, " pt");
        let mut op = cfg.opacity * 100.0;
        slider(ui, l("Background", "Fondo"), &mut op, 0.0..=100.0, " %");
        cfg.opacity = op / 100.0;
        let mut line = cfg.line * 100.0;
        slider(ui, l("Reading line", "Línea de lectura"), &mut line, 10.0..=90.0, " %");
        cfg.line = line / 100.0;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(l("Mirror", "Espejo")).color(MUTED).size(13.0));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                changed |= super::ui::toggle(ui, &mut cfg.mirror).changed();
            });
        });
        ui.add_space(14.0);
        let toggle_key = app.settings.hotkeys.prompter.clone();
        for tip in [
            l("Put it right under the camera.", "Ponlo justo debajo de la cámara.").to_string(),
            l("It never shows in the recording.", "Nunca sale en la grabación.").to_string(),
            l("Drag it by its background.", "Arrástralo desde el fondo.").to_string(),
            l("Double-click or Space: play / pause.", "Doble clic o Espacio: play / pausa.").to_string(),
            l("Wheel: move. Arrows: speed.", "Rueda: mover. Flechas: velocidad.").to_string(),
            if toggle_key.is_empty() { String::new() } else { t!("{} opens or closes it.", "{} lo abre o cierra.", toggle_key) },
        ] {
            ui.label(egui::RichText::new(tip).color(FAINT).size(12.0));
        }
    });
    if changed {
        app.prompter_save();
    }
}

/// The page icon: lines of text with a reading arrow.
pub fn icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.8, color);
    for (i, w) in [14.0f32, 10.0, 14.0].iter().enumerate() {
        let y = c.y - 6.0 + i as f32 * 6.0;
        p.line_segment([pos2(c.x - w / 2.0 + 3.0, y), pos2(c.x + w / 2.0 + 3.0, y)], s);
    }
    p.add(Shape::convex_polygon(vec![c + vec2(-10.0, -4.0), c + vec2(-5.0, 0.0), c + vec2(-10.0, 4.0)], color, Stroke::NONE));
}
