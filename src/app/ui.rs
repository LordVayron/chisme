//! Layout and widgets. Studio: preview card (with record controls and the
//! stats strip), live audio chart, camera controls and recent recordings.

use std::time::{Duration, SystemTime};

use eframe::egui::{
    self, Align2, Color32, ComboBox, CornerRadius, CursorIcon, Frame, Id, Pos2, Rect, Response, ScrollArea, Sense, Shape as EShape, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, pos2, vec2,
};

use super::theme::{self, *};
use super::{App, JobKind, Page};
use crate::i18n::l;
use crate::t;
use crate::compose::{Corner, PipSpec, Place, Scene, Shape, overlay_rect, snap_corner};
use crate::engine::AudioLayout;
use crate::live::Platform;
use crate::subtitles::Model;
use crate::size::fit;
use crate::win::encoder::Codec;

pub fn draw(app: &mut App, ui: &mut Ui) {
    egui::CentralPanel::default().frame(Frame::NONE.fill(BG)).show(ui, |ui| {
        let full = ui.max_rect();
        let p = ui.painter().clone();
        // Warm light behind the dashboard, like Channel Analytics.
        glow(&p, pos2(full.center().x + full.width() * 0.08, full.top() + full.height() * 0.42), vec2(full.width() * 0.5, full.height() * 0.6), Color32::from_rgba_premultiplied(70, 34, 14, 90));
        glow(&p, pos2(full.left() + full.width() * 0.2, full.bottom()), vec2(full.width() * 0.35, full.height() * 0.35), Color32::from_rgba_premultiplied(30, 22, 48, 60));
        let inner = full.shrink(16.0);
        let side = Rect::from_min_size(inner.min, vec2(68.0, inner.height()));
        sidebar(app, ui, side);
        let content = Rect::from_min_max(pos2(side.right() + 20.0, inner.top()), inner.max);
        let body = header(app, ui, content);
        match app.page {
            Page::Studio => studio(app, ui, body),
            Page::Live => live(app, ui, body),
            Page::Templates => super::templates_ui::page(app, ui, body),
            Page::Media => super::media_ui::page(app, ui, body),
            Page::Prompter => super::prompter::page(app, ui, body),
            Page::Library => library(app, ui, body),
            Page::Settings => settings(app, ui, body),
        }
        toast(app, ui, full);
        save_dialog(app, ui.ctx());
    });
}

// ---------------------------------------------------------------- widgets

fn text(p: &egui::Painter, pos: Pos2, align: Align2, s: impl ToString, font: egui::FontId, color: Color32) -> Rect {
    p.text(pos, align, s.to_string(), font, color)
}

/// A rounded button drawn by hand. Returns its response.
fn pill(ui: &mut Ui, rect: Rect, id: &str, label: &str, fill: Color32, color: Color32, font: egui::FontId) -> Response {
    let r = ui.interact(rect, Id::new(id), Sense::click());
    let fill = if r.hovered() { fill.gamma_multiply(if fill.r() > 200 { 0.88 } else { 1.6 }) } else { fill };
    ui.painter().rect_filled(rect, CornerRadius::same((rect.height() / 2.0).min(127.0) as u8), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, color);
    r.on_hover_cursor(CursorIcon::PointingHand)
}

fn round_button(ui: &mut Ui, center: Pos2, radius: f32, id: &str, fill: Color32) -> Response {
    let rect = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
    let r = ui.interact(rect, Id::new(id), Sense::click());
    ui.painter().circle_filled(center, radius, if r.hovered() { fill.gamma_multiply(1.5) } else { fill });
    r.on_hover_cursor(CursorIcon::PointingHand)
}

/// iOS-style switch.
pub(super) fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = vec2(38.0, 22.0);
    let (rect, mut r) = ui.allocate_exact_size(size, Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(r.id, *on);
    let fill = theme::lerp_stops(&[Color32::from_rgb(58, 58, 64), ORANGE], t);
    ui.painter().rect_filled(rect, CornerRadius::same(11), fill);
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    ui.painter().circle_filled(pos2(x, rect.center().y), 8.0, Color32::WHITE);
    r.on_hover_cursor(CursorIcon::PointingHand)
}

/// A row with a label on the left and a widget on the right.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        ui.label(egui::RichText::new(label).color(MUTED).size(13.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add).inner
    })
    .inner
}

fn card_title(p: &egui::Painter, rect: Rect, title: &str, sub: Option<&str>) {
    text(p, rect.min + vec2(22.0, 20.0), Align2::LEFT_TOP, title, medium(15.0), TEXT);
    if let Some(s) = sub {
        text(p, pos2(rect.right() - 22.0, rect.top() + 22.0), Align2::RIGHT_TOP, s, body(12.0), MUTED);
    }
}

fn clock(secs: f64) -> String {
    let s = secs as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

fn ago(t: SystemTime) -> String {
    let s = SystemTime::now().duration_since(t).unwrap_or(Duration::ZERO).as_secs();
    match s {
        0..60 => l("just now", "hace un momento").into(),
        60..3600 => t!("{} min ago", "hace {} min", s / 60),
        3600..86400 => t!("{} h ago", "hace {} h", s / 3600),
        _ => t!("{} d ago", "hace {} d", s / 86400),
    }
}

fn mb(bytes: u64) -> String {
    if bytes >= 1_000_000_000 { format!("{:.2} GB", bytes as f64 / 1e9) } else { format!("{:.1} MB", bytes as f64 / 1e6) }
}

// ---------------------------------------------------------------- sidebar & header

fn icon(p: &egui::Painter, kind: Page, c: Pos2, color: Color32) {
    let s = Stroke::new(1.8, color);
    match kind {
        Page::Studio => {
            p.circle_stroke(c, 9.0, s);
            p.circle_filled(c, 4.0, color);
        }
        Page::Live => {
            // Broadcast: a dot with waves on both sides.
            p.circle_filled(c, 3.0, color);
            for (r, side) in [(7.0f32, 1.0f32), (11.0, 1.0), (7.0, -1.0), (11.0, -1.0)] {
                let pts: Vec<Pos2> = (0..=12)
                    .map(|i| {
                        let a = (-0.75 + 1.5 * i as f32 / 12.0) * 0.9;
                        c + vec2(side * a.cos() * r, a.sin() * r)
                    })
                    .collect();
                p.add(EShape::line(pts, s));
            }
        }
        Page::Templates => {
            // Two overlapping frames.
            p.rect_stroke(Rect::from_center_size(c + vec2(-3.0, -3.0), vec2(16.0, 11.0)), CornerRadius::same(2), s, StrokeKind::Middle);
            p.rect_filled(Rect::from_center_size(c + vec2(3.0, 3.0), vec2(16.0, 11.0)), CornerRadius::same(2), color);
        }
        Page::Media => super::media_ui::icon(p, c, color),
        Page::Prompter => super::prompter::icon(p, c, color),
        Page::Library => {
            for i in 0..3 {
                let y = c.y - 7.0 + i as f32 * 7.0;
                p.rect_stroke(Rect::from_center_size(pos2(c.x, y), vec2(18.0, 4.5)), CornerRadius::same(2), s, StrokeKind::Middle);
            }
        }
        Page::Settings => {
            p.circle_stroke(c, 5.0, s);
            for i in 0..8 {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * 7.5, c + d * 10.0], Stroke::new(2.4, color));
            }
        }
    }
}

fn sidebar(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = ui.painter().clone();
    p.rect_filled(rect, CornerRadius::same(22), SIDEBAR);
    p.rect_stroke(rect, CornerRadius::same(22), Stroke::new(1.0, CARD_EDGE), StrokeKind::Inside);
    // Logo: the rainbow ring.
    let logo = pos2(rect.center().x, rect.top() + 36.0);
    for i in 0..24 {
        let a = i as f32 / 24.0 * std::f32::consts::TAU;
        let col = theme::lerp_stops(&RAINBOW, i as f32 / 23.0);
        p.circle_filled(logo + vec2(a.cos(), a.sin()) * 13.0, 2.6, col);
    }
    let mut item = |page: Page, y: f32, tip: &str| {
        let r = Rect::from_center_size(pos2(rect.center().x, y), Vec2::splat(46.0));
        let resp = ui.interact(r, Id::new(("nav", tip)), Sense::click()).on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand);
        let sel = app.page == page;
        if sel {
            p.rect_filled(r, CornerRadius::same(14), Color32::from_rgba_premultiplied(40, 40, 44, 70));
            p.rect_stroke(r, CornerRadius::same(14), Stroke::new(1.0, Color32::from_rgba_premultiplied(60, 60, 66, 70)), StrokeKind::Inside);
        } else if resp.hovered() {
            p.rect_filled(r, CornerRadius::same(14), Color32::from_rgba_premultiplied(26, 26, 30, 40));
        }
        icon(&p, page, r.center(), if sel { TEXT } else { MUTED });
        if (page == Page::Studio && app.status.recording) || (page == Page::Live && app.status.streams.iter().any(|s| s.live)) {
            p.circle_filled(r.right_top() + vec2(-9.0, 9.0), 4.0, RED);
        }
        if resp.clicked() {
            if page == Page::Settings {
                app.refresh_devices();
            }
            app.page = page;
        }
    };
    item(Page::Studio, rect.top() + 108.0, l("Studio", "Estudio"));
    item(Page::Live, rect.top() + 164.0, l("Live", "En vivo"));
    item(Page::Templates, rect.top() + 220.0, l("Templates", "Plantillas"));
    item(Page::Media, rect.top() + 276.0, "Media Board");
    item(Page::Prompter, rect.top() + 332.0, l("Script & teleprompter", "Guion y teleprompter"));
    item(Page::Library, rect.top() + 388.0, l("Recordings", "Grabaciones"));
    item(Page::Settings, rect.bottom() - 40.0, l("Settings", "Ajustes"));
}

/// The title row; returns the area below it.
fn header(app: &mut App, ui: &mut Ui, content: Rect) -> Rect {
    let p = ui.painter().clone();
    let title = match app.page {
        Page::Studio => "Chisme",
        Page::Live => l("Live", "En vivo"),
        Page::Templates => l("Templates", "Plantillas"),
        Page::Media => "Media Board",
        Page::Prompter => l("Script", "Guion"),
        Page::Library => l("Recordings", "Grabaciones"),
        Page::Settings => l("Settings", "Ajustes"),
    };
    let r = text(&p, content.min + vec2(4.0, 4.0), Align2::LEFT_TOP, title, display(30.0), TEXT);
    gradient_bar(&p, Rect::from_min_size(pos2(r.left() + 2.0, r.bottom() + 6.0), vec2(r.width().max(140.0), 3.0)), &RAINBOW);

    // Status pill.
    let st = &app.status;
    let on_air = st.streams.iter().filter(|s| s.live).map(|s| s.duration).fold(None, |a: Option<f64>, d| Some(a.map_or(d, |a| a.max(d))));
    let (label, fill, fg) = if st.error.is_some() {
        ("ERROR".to_string(), RED, Color32::WHITE)
    } else if let (Some(d), false) = (on_air, st.recording) {
        (t!("●  LIVE  {}", "●  EN VIVO  {}", clock(d)), RED, Color32::WHITE)
    } else if st.recording && st.paused {
        (t!("❚❚  PAUSED  {}", "❚❚  PAUSA  {}", clock(st.rec_secs)), Color32::from_rgb(60, 60, 66), TEXT)
    } else if st.recording {
        (format!("●  REC  {}", clock(st.rec_secs)), RED, Color32::WHITE)
    } else if st.ready {
        (l("●  READY", "●  LISTO").to_string(), Color32::from_rgba_premultiplied(30, 30, 34, 60), GREEN)
    } else {
        (l("STARTING…", "INICIANDO…").to_string(), Color32::from_rgba_premultiplied(30, 30, 34, 60), MUTED)
    };
    let galley_w = 18.0 + label.chars().count() as f32 * 8.2;
    let pill_r = Rect::from_min_size(pos2(content.right() - galley_w - 4.0, content.top() + 6.0), vec2(galley_w, 34.0));
    if (st.recording && !st.paused) || on_air.is_some() {
        // Gradient pill while recording or live (Ventura's stripe).
        let m = Rect::from_min_size(pill_r.min, pill_r.size());
        p.rect_filled(m, CornerRadius::same(17), fill);
        gradient_bar(&p, Rect::from_min_max(pos2(m.left() + 14.0, m.bottom() - 3.0), pos2(m.right() - 14.0, m.bottom() - 1.0)), &RAINBOW[1..]);
    } else {
        p.rect_filled(pill_r, CornerRadius::same(17), fill);
    }
    p.text(pill_r.center(), Align2::CENTER_CENTER, label, semibold(13.0), fg);
    if st.ready && content.width() > 820.0 {
        let source = match &app.settings.window {
            Some((_, w)) => t!("Window · {}", "Ventana · {}", truncate(w, 40)),
            None => t!("Display {}", "Pantalla {}", app.settings.display),
        };
        let label = format!("{source}  ·  {}×{}  ·  {} fps", st.out_size.0, st.out_size.1, st.fps);
        text(&p, pos2(pill_r.left() - 16.0, pill_r.center().y), Align2::RIGHT_CENTER, label, body(12.5), MUTED);
    }
    Rect::from_min_max(pos2(content.left(), content.top() + 62.0), content.max)
}

// ---------------------------------------------------------------- studio

/// Studio: preview with its controls, live audio, the camera (or the layer
/// editor while building a custom scene) and the Media Board. Wide windows
/// get two columns; narrow ones stack everything and scroll.
fn studio(app: &mut App, ui: &mut Ui, area: Rect) {
    let narrow = area.width() < 1100.0;
    let media_h = 158.0;
    ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let w = area.width() - 4.0;
            if narrow {
                let pv_h = (w * 9.0 / 16.0 + 96.0).max(330.0);
                let side_h = 380.0;
                let (all, _) = ui.allocate_exact_size(vec2(w, pv_h + side_h + media_h + 32.0), Sense::hover());
                let preview_card = Rect::from_min_size(all.min, vec2(w, pv_h));
                let y2 = preview_card.bottom() + 16.0;
                let half = ((w - 16.0) / 2.0).floor();
                let (audio_card, cam_card) = if w < 640.0 {
                    (Rect::from_min_size(pos2(all.left(), y2), vec2(w, 230.0)), Rect::from_min_size(pos2(all.left(), y2 + 246.0), vec2(w, side_h - 16.0)))
                } else {
                    (Rect::from_min_size(pos2(all.left(), y2), vec2(half, 240.0)), Rect::from_min_size(pos2(all.left() + half + 16.0, y2), vec2(half, side_h)))
                };
                let media = Rect::from_min_size(pos2(all.left(), cam_card.bottom().max(audio_card.bottom()) + 16.0), vec2(w, media_h));
                preview(app, ui, preview_card);
                audio(app, ui, audio_card);
                side_card(app, ui, cam_card);
                super::media_ui::strip(app, ui, media);
            } else {
                let total = area.height().max(660.0);
                let (all, _) = ui.allocate_exact_size(vec2(w, total), Sense::hover());
                let top = Rect::from_min_max(all.min, pos2(all.right(), all.bottom() - media_h - 16.0));
                let left_w = (top.width() * 0.64).floor();
                let preview_card = Rect::from_min_size(top.min, vec2(left_w, top.height()));
                let right = Rect::from_min_max(pos2(preview_card.right() + 16.0, top.top()), top.max);
                let audio_h = (right.height() * 0.36).clamp(200.0, 260.0).floor();
                let audio_card = Rect::from_min_size(right.min, vec2(right.width(), audio_h));
                let cam_card = Rect::from_min_max(pos2(right.left(), audio_card.bottom() + 16.0), right.max);
                let media = Rect::from_min_max(pos2(all.left(), all.bottom() - media_h), all.max);
                preview(app, ui, preview_card);
                audio(app, ui, audio_card);
                side_card(app, ui, cam_card);
                super::media_ui::strip(app, ui, media);
            }
        });
    });
}

/// The layer editor while building a new custom scene; the camera otherwise.
fn side_card(app: &mut App, ui: &mut Ui, card: Rect) {
    if app.settings.layout.is_some() && app.settings.template.is_none() {
        super::editor::layers_card(app, ui, card);
    } else {
        camera(app, ui, card);
    }
}

fn preview(app: &mut App, ui: &mut Ui, card: Rect) {
    let p = ui.painter().clone();
    theme::card(&p, card);
    let inner = card.shrink(12.0);
    let strip_h = 30.0;
    let controls_h = 64.0;
    let view = Rect::from_min_max(inner.min, pos2(inner.right(), inner.bottom() - strip_h - controls_h));
    p.rect_filled(view, CornerRadius::same(18), Color32::from_rgb(6, 6, 7));

    let st = app.status.clone();
    if let Some(err) = &st.error {
        text(&p, view.center() - vec2(0.0, 20.0), Align2::CENTER_CENTER, l("Couldn't start the capture", "No se pudo iniciar la captura"), semibold(16.0), TEXT);
        p.text(view.center() + vec2(0.0, 8.0), Align2::CENTER_CENTER, truncate(err, 90), body(12.5), MUTED);
        let b = Rect::from_center_size(view.center() + vec2(0.0, 52.0), vec2(130.0, 38.0));
        if pill(ui, b, "retry", l("Retry", "Reintentar"), Color32::WHITE, Color32::BLACK, semibold(14.0)).clicked() {
            let cfg = app.settings.config();
            app.restart(cfg);
        }
    } else if let Some(tex) = &app.preview {
        let size = tex.size_vec2();
        let (x, y, w, h) = fit((size.x as u32, size.y as u32), (view.width() as u32, view.height() as u32));
        let img = Rect::from_min_size(view.min + vec2(x as f32, y as f32), vec2(w as f32, h as f32));
        egui::Image::from_texture(tex).corner_radius(CornerRadius::same(14)).paint_at(ui, img);
        if app.settings.layout.is_some() {
            super::editor::layout_edit(app, ui, img);
        } else {
            overlay_drag(app, ui, img);
        }
        // Show Chisme in recordings (top right).
        {
            let label = l("Show Chisme in recording", "Mostrar Chisme en la grabación");
            let w = 64.0 + label.chars().count() as f32 * 6.4;
            let chip = Rect::from_min_size(pos2(img.right() - w - 12.0, img.top() + 12.0), vec2(w, 26.0));
            let r = ui.interact(chip, Id::new("show-self"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(l(
                "Off: Chisme's windows never appear in recordings or streams. On: record a tutorial of Chisme itself.",
                "Apagado: las ventanas de Chisme nunca salen en grabaciones ni transmisiones. Encendido: graba un tutorial del propio Chisme.",
            ));
            p.rect_filled(chip, CornerRadius::same(13), Color32::from_rgba_premultiplied(0, 0, 0, 150));
            text(&p, chip.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, label, semibold(11.0), TEXT);
            let on = app.settings.show_self;
            let sw = Rect::from_min_size(pos2(chip.right() - 38.0, chip.center().y - 8.0), vec2(28.0, 16.0));
            p.rect_filled(sw, CornerRadius::same(8), if on { ORANGE } else { Color32::from_rgb(70, 70, 76) });
            p.circle_filled(pos2(if on { sw.right() - 8.0 } else { sw.left() + 8.0 }, sw.center().y), 6.0, Color32::WHITE);
            if r.clicked() {
                app.settings.show_self = !on;
                app.settings_changed();
            }
        }
        // "Live" chip.
        let chip = Rect::from_min_size(img.min + vec2(12.0, 12.0), vec2(if st.recording { 78.0 } else { 104.0 }, 26.0));
        p.rect_filled(chip, CornerRadius::same(13), Color32::from_rgba_premultiplied(0, 0, 0, 150));
        let (dot, label) = if st.recording { (RED, l("LIVE", "EN VIVO")) } else { (GREEN, l("PREVIEW", "VISTA PREVIA")) };
        p.circle_filled(chip.left_center() + vec2(13.0, 0.0), 4.0, dot);
        text(&p, chip.left_center() + vec2(23.0, 0.0), Align2::LEFT_CENTER, label, semibold(11.0), TEXT);
    } else {
        text(&p, view.center(), Align2::CENTER_CENTER, l("Starting capture…", "Iniciando captura…"), medium(14.0), MUTED);
    }

    // Controls: Record (the white "Start Now" pill), pause, scene, timer.
    let cy = view.bottom() + controls_h / 2.0;
    let rec_r = Rect::from_min_size(pos2(inner.left() + 6.0, cy - 24.0), vec2(168.0, 48.0));
    let label = if st.recording { l("■  Stop", "■  Detener") } else { l("●  Record", "●  Grabar") };
    let can = st.ready && st.error.is_none();
    if pill(ui, rec_r, "record", label, if can { Color32::WHITE } else { Color32::from_gray(90) }, Color32::BLACK, semibold(15.5))
        .on_hover_text(&app.settings.hotkeys.record)
        .clicked()
    {
        app.toggle_record();
    }
    let pc = pos2(rec_r.right() + 34.0, cy);
    let pause = round_button(ui, pc, 24.0, "pause", Color32::from_rgba_premultiplied(36, 36, 40, 80)).on_hover_text(t!("Pause / resume ({})", "Pausar / reanudar ({})", app.settings.hotkeys.pause));
    let pcol = if st.recording { TEXT } else { FAINT };
    if st.paused {
        p.add(EShape::convex_polygon(vec![pc + vec2(-5.0, -8.0), pc + vec2(9.0, 0.0), pc + vec2(-5.0, 8.0)], pcol, Stroke::NONE));
    } else {
        p.rect_filled(Rect::from_center_size(pc - vec2(4.5, 0.0), vec2(4.0, 15.0)), CornerRadius::same(1), pcol);
        p.rect_filled(Rect::from_center_size(pc + vec2(4.5, 0.0), vec2(4.0, 15.0)), CornerRadius::same(1), pcol);
    }
    if pause.clicked() {
        app.toggle_pause();
    }
    // Chapter marker: a flag, with the count so far.
    let fc = pos2(pc.x + 58.0, cy);
    let flag = round_button(ui, fc, 24.0, "marker", Color32::from_rgba_premultiplied(36, 36, 40, 80)).on_hover_text(t!("Mark a chapter ({})", "Marcar capítulo ({})", app.settings.hotkeys.marker));
    let fcol = if st.recording { TEXT } else { FAINT };
    p.line_segment([fc + vec2(-5.0, -9.0), fc + vec2(-5.0, 10.0)], Stroke::new(2.0, fcol));
    p.add(EShape::convex_polygon(vec![fc + vec2(-4.0, -9.0), fc + vec2(9.0, -4.5), fc + vec2(-4.0, 0.0)], if st.recording { ORANGE } else { FAINT }, Stroke::NONE));
    if st.stats.markers > 0 && st.recording {
        let b = Rect::from_center_size(fc + vec2(17.0, -17.0), vec2(18.0, 18.0));
        p.rect_filled(b, CornerRadius::same(9), ORANGE);
        p.text(b.center(), Align2::CENTER_CENTER, st.stats.markers.to_string(), semibold(10.5), Color32::BLACK);
    }
    if flag.clicked() {
        app.marker();
    }
    // Teleprompter on / off.
    let tc = pos2(fc.x + 58.0, cy);
    let on = app.prompter.open;
    let tp = round_button(ui, tc, 24.0, "prompter", if on { Color32::from_rgba_premultiplied(90, 44, 0, 140) } else { Color32::from_rgba_premultiplied(36, 36, 40, 80) })
        .on_hover_text(t!("Teleprompter ({})", "Teleprompter ({})", app.settings.hotkeys.prompter));
    super::prompter::icon(&p, tc + vec2(2.0, 0.0), if on { ORANGE } else { TEXT });
    if tp.clicked() {
        app.toggle_prompter();
    }
    let pc = tc;
    // Timer.
    let timer = if st.recording { clock(st.rec_secs) } else { "00:00".into() };
    text(&p, pos2(inner.right() - 10.0, cy), Align2::RIGHT_CENTER, timer, display(24.0), if st.recording { TEXT } else { FAINT });
    // Quick access: the pinned scenes and templates (Templates page), then
    // Custom. Camera scenes need a camera; missing templates are skipped.
    {
        use crate::settings::Pin;
        let pins: Vec<Pin> = app
            .settings
            .pinned
            .iter()
            .filter(|p| match p {
                Pin::Scene(s) => st.cam.is_some() || !s.uses_cam(),
                Pin::Template(n) => app.templates.iter().any(|t| &t.name == n),
            })
            .cloned()
            .collect();
        let n = pins.len() as f32 + 1.0;
        let w = (50.0 * n).min(inner.right() - 150.0 - pc.x - 40.0);
        let seg = Rect::from_center_size(pos2((pc.x + 40.0 + inner.right() - 130.0) / 2.0, cy), vec2(w, 40.0));
        if seg.width() > 60.0 {
            p.rect_filled(seg, CornerRadius::same(14), Color32::from_rgba_premultiplied(0, 0, 0, 90));
            let tw = seg.width() / n;
            let custom = app.settings.layout.is_some();
            let brand = app.settings.brand.clone();
            for i in 0..=pins.len() {
                let r = Rect::from_min_size(pos2(seg.left() + tw * i as f32, seg.top()), vec2(tw, seg.height())).shrink(3.0);
                let pin = pins.get(i);
                let tip = match pin {
                    Some(Pin::Scene(s)) => s.label().to_string(),
                    Some(Pin::Template(n)) => t!("Template: {}", "Plantilla: {}", n),
                    None => l("Custom: move and resize the layers", "Personalizado: mueve y ajusta las capas").to_string(),
                };
                let resp = ui.interact(r, Id::new(("pin", i)), Sense::click()).on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand);
                let sel = match pin {
                    Some(Pin::Scene(s)) => !custom && app.settings.scene == *s,
                    Some(Pin::Template(n)) => custom && app.settings.template.as_deref() == Some(n.as_str()),
                    None => custom && app.settings.template.as_ref().is_none_or(|t| !pins.contains(&Pin::Template(t.clone()))),
                };
                if sel {
                    p.rect_filled(r, CornerRadius::same(11), Color32::from_rgba_premultiplied(62, 62, 68, 120));
                } else if resp.hovered() {
                    p.rect_filled(r, CornerRadius::same(11), Color32::from_rgba_premultiplied(30, 30, 34, 50));
                }
                let col = if sel { TEXT } else { MUTED };
                match pin {
                    Some(Pin::Scene(s)) => scene_icon(&p, *s, r.center(), col),
                    Some(Pin::Template(n)) => {
                        if let Some(t) = app.templates.iter().find(|t| &t.name == n) {
                            let thumb = Rect::from_center_size(r.center(), vec2(28.0, 16.0));
                            super::templates_ui::thumbnail(&p, thumb, &t.layout, t.brand.as_ref().unwrap_or(&brand));
                        }
                    }
                    None => super::editor::custom_icon(&p, r.center(), col),
                }
                if resp.clicked() {
                    match pin {
                        Some(Pin::Scene(s)) => {
                            app.settings.scene = *s;
                            app.settings.layout = None;
                            app.settings.template = None;
                            app.sel_layer = None;
                            app.settings_changed();
                        }
                        Some(Pin::Template(n)) => {
                            if let Some(t) = app.templates.iter().find(|t| &t.name == n).cloned() {
                                app.apply_template(&t);
                            }
                        }
                        None => app.customize_layout(),
                    }
                }
            }
        }
    }

    // Stats: one quiet line.
    let strip = Rect::from_min_max(pos2(inner.left(), inner.bottom() - strip_h), inner.max);
    let s = &st.stats;
    let items = [
        (if st.recording { clock(st.rec_secs) } else { "—".into() }, l("duration", "duración"), RED),
        (format!("{}", st.fps), "fps", ORANGE),
        (format!("{:.1}", st.bitrate / 1e6), "Mb/s", RAINBOW[2]),
        (format!("{}", s.drops), l("dropped", "perdidos"), if s.drops > 0 { RED } else { RAINBOW[4] }),
    ];
    let mut x = strip.left() + 10.0;
    for (v, label, c) in items {
        p.circle_filled(pos2(x + 3.0, strip.center().y), 3.0, c);
        let r = text(&p, pos2(x + 12.0, strip.center().y), Align2::LEFT_CENTER, v, semibold(12.5), TEXT);
        let r2 = text(&p, pos2(r.right() + 5.0, strip.center().y), Align2::LEFT_CENTER, label, body(11.5), FAINT);
        x = r2.right() + 22.0;
    }
}

/// A tiny drawing of a scene's layout: a 26x16 frame with the screen
/// (outlined) and the camera (a filled head-and-shoulders block).
/// The camera's own settings (what its driver offers: zoom, pan/tilt,
/// exposure, image), like OBSBOT Center's basic panel.
fn cam_controls(app: &mut App, ui: &mut Ui) {
    use crate::win::camera::Ctl;
    let controls = app.status.cam_controls.clone();
    if controls.is_empty() {
        ui.label(egui::RichText::new(l("This camera doesn't offer adjustable controls.", "Esta cámara no ofrece controles ajustables.")).color(MUTED).size(13.0));
        return;
    }
    let send = |app: &App, ctl: Ctl, v: i32, auto: bool| {
        if let Some(e) = &app.engine {
            e.send(crate::engine::Cmd::CameraControl(ctl, v, auto));
        }
    };
    let get = |ctl: Ctl| controls.iter().find(|c| c.ctl == ctl).copied();
    // Pan / tilt pad and zoom buttons.
    let (pan, tilt, zoom) = (get(Ctl::Pan), get(Ctl::Tilt), get(Ctl::Zoom));
    if pan.is_some() || tilt.is_some() || zoom.is_some() {
        ui.horizontal(|ui| {
            if pan.is_some() || tilt.is_some() {
                let (pad, _) = ui.allocate_exact_size(vec2(92.0, 92.0), Sense::hover());
                ui.painter().circle_filled(pad.center(), 44.0, Color32::from_rgba_premultiplied(0, 0, 0, 90));
                let dirs = [(Ctl::Tilt, 1, vec2(0.0, -28.0)), (Ctl::Tilt, -1, vec2(0.0, 28.0)), (Ctl::Pan, -1, vec2(-28.0, 0.0)), (Ctl::Pan, 1, vec2(28.0, 0.0))];
                for (k, (ctl, sign, off)) in dirs.iter().enumerate() {
                    let Some(c) = get(*ctl) else { continue };
                    let b = Rect::from_center_size(pad.center() + *off, vec2(26.0, 26.0));
                    let r = ui.interact(b, Id::new(("ptz", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                    ui.painter().circle_filled(b.center(), 13.0, if r.hovered() { Color32::from_rgb(70, 70, 76) } else { Color32::from_rgb(44, 44, 48) });
                    let d = off.normalized();
                    let ctr = b.center();
                    let perp = vec2(-d.y, d.x);
                    ui.painter().add(EShape::convex_polygon(vec![ctr + d * 5.0, ctr - d * 3.0 + perp * 5.0, ctr - d * 3.0 - perp * 5.0], TEXT, Stroke::NONE));
                    if r.clicked() {
                        let step = ((c.max - c.min) / 20).max(c.step);
                        send(app, *ctl, (c.value + sign * step).clamp(c.min, c.max), false);
                    }
                }
                // Center: back to the middle.
                let b = Rect::from_center_size(pad.center(), vec2(22.0, 22.0));
                let r = ui.interact(b, Id::new("ptz-home"), Sense::click()).on_hover_text(l("Center", "Centrar")).on_hover_cursor(CursorIcon::PointingHand);
                ui.painter().circle_filled(b.center(), 9.0, if r.hovered() { ORANGE } else { Color32::from_rgb(90, 90, 96) });
                if r.clicked() {
                    for c in [pan, tilt].into_iter().flatten() {
                        send(app, c.ctl, c.default, false);
                    }
                }
            }
            if let Some(z) = zoom {
                ui.vertical(|ui| {
                    let step = ((z.max - z.min) / 10).max(z.step);
                    if ui.add(egui::Button::new(egui::RichText::new(l("+  Zoom in", "+  Acercar")).size(13.0)).min_size(vec2(120.0, 28.0))).clicked() {
                        send(app, Ctl::Zoom, (z.value + step).min(z.max), false);
                    }
                    if ui.add(egui::Button::new(egui::RichText::new(l("−  Zoom out", "−  Alejar")).size(13.0)).min_size(vec2(120.0, 28.0))).clicked() {
                        send(app, Ctl::Zoom, (z.value - step).max(z.min), false);
                    }
                    let pct = (z.value - z.min) as f32 / (z.max - z.min).max(1) as f32 * 100.0;
                    ui.label(egui::RichText::new(format!("Zoom {pct:.0}%")).color(MUTED).size(12.0));
                });
            }
        });
        ui.add_space(6.0);
    }
    // Every control as a slider, with Auto where the camera supports it.
    for c in &controls {
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            ui.add_sized([96.0, 20.0], egui::Label::new(egui::RichText::new(c.ctl.label()).color(MUTED).size(12.5)));
            let mut v = c.value;
            ui.spacing_mut().slider_width = (ui.available_width() - 120.0).max(60.0);
            let r = ui.add_enabled(!c.auto, egui::Slider::new(&mut v, c.min..=c.max).step_by(c.step as f64).show_value(false));
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                send(app, c.ctl, v, false);
            }
            if c.can_auto {
                let mut auto = c.auto;
                if ui.checkbox(&mut auto, "Auto").changed() {
                    send(app, c.ctl, c.value, auto);
                }
            }
        });
    }
    ui.add_space(6.0);
    if ui.button(l("Reset all to default", "Restablecer todo")).clicked() {
        for c in &controls {
            send(app, c.ctl, c.default, c.can_auto && matches!(c.ctl, Ctl::Exposure | Ctl::Focus | Ctl::WhiteBalance));
        }
    }
}

pub(super) fn scene_icon_pub(p: &egui::Painter, s: Scene, c: Pos2, color: Color32) {
    scene_icon(p, s, c, color)
}

fn scene_icon(p: &egui::Painter, s: Scene, c: Pos2, color: Color32) {
    let frame = Rect::from_center_size(c, vec2(26.0, 16.0));
    let st = Stroke::new(1.4, color);
    let screen = |r: Rect| {
        p.rect_stroke(r, CornerRadius::same(2), st, StrokeKind::Inside);
    };
    let cam = |r: Rect| {
        p.rect_filled(r, CornerRadius::same(2), color.gamma_multiply(0.35));
        let head = r.center() - vec2(0.0, r.height() * 0.12);
        p.circle_filled(head, (r.height().min(r.width()) * 0.2).max(1.5), color);
    };
    match s {
        Scene::Screen => screen(frame),
        Scene::ScreenCam => {
            screen(frame);
            cam(Rect::from_min_size(frame.right_bottom() - vec2(10.0, 8.0), vec2(8.0, 6.0)));
        }
        Scene::SideBySide => {
            screen(Rect::from_min_size(frame.min, vec2(16.0, 16.0)));
            cam(Rect::from_min_size(frame.min + vec2(18.0, 0.0), vec2(8.0, 16.0)));
        }
        Scene::Cam => cam(frame),
        Scene::CamScreen => {
            cam(frame);
            screen(Rect::from_min_size(frame.right_bottom() - vec2(11.0, 8.0), vec2(9.0, 6.0)));
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// Drag the camera overlay (or, in "camera + screen", the screen box) on
/// the preview; it snaps to corners.
fn overlay_drag(app: &mut App, ui: &mut Ui, img: Rect) {
    let st = &app.status;
    let Some((_, cam_size, _)) = st.cam else { return };
    if !matches!(app.settings.scene, Scene::ScreenCam | Scene::CamScreen) || st.out_size.0 == 0 {
        return;
    }
    let (ow, oh) = (st.out_size.0 as f64, st.out_size.1 as f64);
    let mut pip = app.settings.pip();
    // The box being dragged: the camera, or the screen (its own shape and size).
    let (aspect, shape_of) = if app.settings.scene == Scene::CamScreen {
        (ow / oh, PipSpec { shape: Shape::Rounded, size: pip.size.max(0.3), ..pip.clone() })
    } else {
        (cam_size.0 as f64 / cam_size.1.max(1) as f64, pip.clone())
    };
    let [x, y, w, h] = overlay_rect(&shape_of, ow, oh, aspect);
    let k = img.width() / ow as f32;
    let r = Rect::from_min_size(img.min + vec2(x as f32 * k, y as f32 * k), vec2(w as f32 * k, h as f32 * k));
    let resp = ui.interact(r, Id::new("pip-drag"), Sense::drag()).on_hover_cursor(CursorIcon::Grab);
    if resp.hovered() || resp.dragged() {
        ui.painter().rect_stroke(r.expand(3.0), CornerRadius::same(10), Stroke::new(1.5, Color32::from_white_alpha(170)), StrokeKind::Outside);
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        let c = r.center() + resp.drag_delta();
        pip.place = Place::At([((c.x - img.left()) / img.width()) as f64, ((c.y - img.top()) / img.height()) as f64].map(|v| v.clamp(0.0, 1.0)));
        app.settings.set_pip(&pip);
        if let Some(e) = &app.engine {
            e.set_live(app.settings.live());
        }
    }
    if resp.drag_stopped() {
        if let Some(c) = snap_corner(&PipSpec { place: pip.place, ..shape_of }, ow, oh, aspect, 0.06) {
            pip.place = Place::Corner(c);
            app.settings.set_pip(&pip);
        }
        app.settings_changed();
    }
}

/// A speaker icon, crossed out when muted.
fn speaker(p: &egui::Painter, c: Pos2, color: Color32, muted: bool) {
    p.add(EShape::convex_polygon(vec![c + vec2(-7.0, -3.0), c + vec2(-3.0, -3.0), c + vec2(2.0, -7.0), c + vec2(2.0, 7.0), c + vec2(-3.0, 3.0), c + vec2(-7.0, 3.0)], color, Stroke::NONE));
    if muted {
        p.line_segment([c + vec2(5.0, -4.0), c + vec2(11.0, 4.0)], Stroke::new(1.8, RED));
        p.line_segment([c + vec2(11.0, -4.0), c + vec2(5.0, 4.0)], Stroke::new(1.8, RED));
    } else {
        p.add(EShape::line((0..=8).map(|k| { let a = -0.9 + 1.8 * k as f32 / 8.0; c + vec2(4.0 + a.cos() * 5.0, a.sin() * 5.0) }).collect(), Stroke::new(1.5, color)));
    }
}

fn audio(app: &mut App, ui: &mut Ui, card: Rect) {
    let p = ui.painter().clone();
    theme::card(&p, card);
    let st = app.status.clone();
    card_title(&p, card, l("Live audio", "Audio en vivo"), None);
    // Mute / unmute everything.
    let sources: Vec<usize> = [st.has_system.then_some(0), st.has_mic.then_some(1)].into_iter().flatten().collect();
    if !sources.is_empty() {
        let all_muted = sources.iter().all(|&i| if i == 0 { app.settings.system_muted } else { app.settings.mic_muted });
        let b = Rect::from_min_size(pos2(card.right() - 22.0 - 104.0, card.top() + 14.0), vec2(104.0, 28.0));
        let label = if all_muted { l("Unmute all", "Activar todo") } else { l("Mute all", "Silenciar todo") };
        if pill(ui, b, "mute-all", label, if all_muted { Color32::from_rgba_premultiplied(80, 20, 20, 120) } else { Color32::from_rgba_premultiplied(40, 40, 44, 90) }, if all_muted { RED } else { TEXT }, medium(12.0)).clicked() {
            app.settings.system_muted = !all_muted;
            app.settings.mic_muted = !all_muted;
            app.settings_changed();
        }
    }
    let rows = sources.len().max(1) as f32;
    let controls_h = rows * 34.0 + 8.0;
    let chart = Rect::from_min_max(pos2(card.left() + 50.0, card.top() + 54.0), pos2(card.right() - 24.0, card.bottom() - controls_h - 14.0));
    // Grid: 0, -20, -40, -60 dB.
    for (i, db) in [0, -30, -60].iter().enumerate() {
        let y = chart.top() + chart.height() * i as f32 / 2.0;
        text(&p, pos2(card.left() + 22.0, y), Align2::LEFT_CENTER, db, body(10.5), FAINT);
        p.line_segment([pos2(chart.left(), y), pos2(chart.right(), y)], Stroke::new(1.0, Color32::from_rgba_premultiplied(30, 30, 34, 30)));
    }
    let y_of = |db: f32| chart.bottom() - ((db.clamp(-60.0, 0.0) + 60.0) / 60.0) * chart.height();
    let n = app.levels.len().max(2);
    let line = |sel: fn(&(f32, f32)) -> f32| -> Vec<Pos2> {
        let mut v = 0.0f32;
        app.levels
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let target = sel(l).max(-60.0);
                v = if i == 0 { target } else { v + (target - v) * 0.35 };
                pos2(chart.left() + chart.width() * i as f32 / (n - 1) as f32, y_of(v))
            })
            .collect()
    };
    if st.has_mic && !app.settings.mic_muted {
        p.add(EShape::line(line(|l| l.1), Stroke::new(2.0, TEXT)));
    }
    if st.has_system && !app.settings.system_muted {
        let pts = line(|l| l.0);
        if let Some(last) = pts.last().copied() {
            p.add(EShape::line(pts, Stroke::new(2.0, YELLOW)));
            p.circle_filled(last, 4.0, YELLOW);
        }
    }
    if sources.is_empty() {
        text(&p, chart.center(), Align2::CENTER_CENTER, l("No audio — turn it on in Settings", "Sin audio — actívalo en Ajustes"), body(12.5), MUTED);
        return;
    }
    // One row per source: name, live level, volume, mute.
    let mut y = card.bottom() - controls_h - 4.0;
    for &i in &sources {
        let (name, color, level) = if i == 0 {
            (l("System", "Sistema"), YELLOW, app.levels.back().map_or(-90.0, |l| l.0))
        } else {
            (l("Microphone", "Micrófono"), TEXT, app.levels.back().map_or(-90.0, |l| l.1))
        };
        let muted = if i == 0 { app.settings.system_muted } else { app.settings.mic_muted };
        let row = Rect::from_min_size(pos2(card.left() + 18.0, y), vec2(card.width() - 36.0, 30.0));
        p.rect_filled(row, CornerRadius::same(10), Color32::from_rgba_premultiplied(0, 0, 0, 70));
        p.circle_filled(pos2(row.left() + 12.0, row.center().y), 4.0, if muted { FAINT } else { color });
        text(&p, pos2(row.left() + 22.0, row.center().y), Align2::LEFT_CENTER, name, medium(12.0), if muted { FAINT } else { TEXT });
        let val = if muted { l("muted", "silenciado").to_string() } else if level <= -60.0 { "-∞".into() } else { format!("{level:.0} dB") };
        text(&p, pos2(row.left() + 108.0, row.center().y), Align2::LEFT_CENTER, val, body(11.0), if muted { RED } else { MUTED });
        // Mute button.
        let mb = Rect::from_center_size(pos2(row.right() - 16.0, row.center().y), vec2(26.0, 24.0));
        let mr = ui.interact(mb, Id::new(("mute", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(if muted { l("Unmute", "Activar") } else { l("Mute", "Silenciar") });
        if mr.hovered() {
            p.rect_filled(mb, CornerRadius::same(7), Color32::from_rgba_premultiplied(60, 60, 66, 90));
        }
        speaker(&p, mb.center() - vec2(2.0, 0.0), if muted { FAINT } else { TEXT }, muted);
        if mr.clicked() {
            if i == 0 {
                app.settings.system_muted = !muted;
            } else {
                app.settings.mic_muted = !muted;
            }
            app.settings_changed();
        }
        // Volume (dB, applied live to the mix).
        let sl = Rect::from_min_max(pos2(row.left() + 160.0, row.top()), pos2(mb.left() - 8.0, row.bottom()));
        if sl.width() > 60.0 {
            let gain = if i == 0 { &mut app.settings.system_gain_db } else { &mut app.settings.mic_gain_db };
            let mut changed = false;
            ui.scope_builder(UiBuilder::new().max_rect(sl), |ui| {
                ui.spacing_mut().slider_width = (sl.width() - 58.0).max(40.0);
                ui.horizontal_centered(|ui| {
                    changed = ui.add(egui::Slider::new(gain, -30.0..=12.0).suffix(" dB").max_decimals(0)).on_hover_text(l("Volume in the mix", "Volumen en la mezcla")).changed();
                });
            });
            if changed {
                app.settings_changed();
            }
        }
        y += 34.0;
    }
}

pub(super) fn shape_icon(p: &egui::Painter, s: Shape, c: Pos2, color: Color32) {
    let st = Stroke::new(1.6, color);
    match s {
        Shape::Rect => {
            p.rect_stroke(Rect::from_center_size(c, vec2(20.0, 13.0)), CornerRadius::ZERO, st, StrokeKind::Middle);
        }
        Shape::Circle => {
            p.circle_stroke(c, 8.0, st);
        }
        Shape::Rounded => {
            p.rect_stroke(Rect::from_center_size(c, vec2(20.0, 13.0)), CornerRadius::same(4), st, StrokeKind::Middle);
        }
        Shape::Square => {
            p.rect_stroke(Rect::from_center_size(c, vec2(15.0, 15.0)), CornerRadius::same(2), st, StrokeKind::Middle);
        }
        Shape::Pill => {
            p.rect_stroke(Rect::from_center_size(c, vec2(22.0, 12.0)), CornerRadius::same(6), st, StrokeKind::Middle);
        }
        Shape::Squircle => {
            let pts: Vec<Pos2> = (0..48)
                .map(|i| {
                    let a = i as f32 / 48.0 * std::f32::consts::TAU;
                    let (x, y) = (a.cos(), a.sin());
                    let n = 4.0;
                    pos2(c.x + 8.5 * x.signum() * x.abs().powf(2.0 / n), c.y + 8.5 * y.signum() * y.abs().powf(2.0 / n))
                })
                .collect();
            p.add(EShape::closed_line(pts, st));
        }
    };
}

fn camera(app: &mut App, ui: &mut Ui, card: Rect) {
    let p = ui.painter().clone();
    theme::card(&p, card);
    let cam = app.status.cam.clone();
    text(&p, card.min + vec2(22.0, 20.0), Align2::LEFT_TOP, l("Camera", "Cámara"), medium(15.0), TEXT);
    let inner = Rect::from_min_max(pos2(card.left() + 22.0, card.top() + 54.0), pos2(card.right() - 22.0, card.bottom() - 16.0));
    // Tabs: the look (overlay, background) or the camera's own controls.
    if cam.is_some() {
        let tabs = [l("Look", "Estilo"), l("Controls", "Controles")];
        let mut x = card.right() - 22.0;
        for (k, label) in tabs.iter().enumerate().rev() {
            let w = 22.0 + label.chars().count() as f32 * 7.0;
            let r = Rect::from_min_size(pos2(x - w, card.top() + 14.0), vec2(w, 26.0));
            let sel = app.cam_tab == k as u8;
            if pill(ui, r, &format!("camtab{k}"), label, if sel { Color32::from_rgba_premultiplied(70, 70, 76, 140) } else { Color32::from_rgba_premultiplied(30, 30, 34, 70) }, if sel { TEXT } else { MUTED }, medium(12.0)).clicked() {
                app.cam_tab = k as u8;
            }
            x -= w + 6.0;
        }
        if app.cam_tab == 1 {
            ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| cam_controls(app, ui));
            });
            return;
        }
    }
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        if app.settings.camera.is_none() || cam.is_none() {
            ui.add_space(6.0);
            let msg = if app.settings.camera.is_some() && app.status.ready { l("The camera didn't respond. Pick another:", "La cámara no respondió. Elige otra:") } else { l("Add your camera to the video:", "Agrega tu cámara al video:") };
            ui.label(egui::RichText::new(msg).color(MUTED).size(13.0));
            if app.devices.cameras.is_empty() {
                app.refresh_devices();
            }
            let cams = app.devices.cameras.clone();
            for c in cams {
                if ui.add_sized([inner.width(), 34.0], egui::Button::new(egui::RichText::new(format!("+   {c}")).size(13.0))).clicked() {
                    app.settings.camera = Some(c);
                    app.settings_changed();
                }
            }
            return;
        }
        let mut pip = app.settings.pip();
        let mut changed = false;
        // Shapes.
        ui.horizontal(|ui| {
            for s in [Shape::Circle, Shape::Rounded, Shape::Square, Shape::Pill, Shape::Squircle] {
                let (r, resp) = ui.allocate_exact_size(vec2(44.0, 34.0), Sense::click());
                let sel = pip.shape == s;
                let fill = if sel { Color32::from_rgba_premultiplied(62, 62, 68, 120) } else if resp.hovered() { CONTROL } else { Color32::TRANSPARENT };
                ui.painter().rect_filled(r, CornerRadius::same(10), fill);
                shape_icon(ui.painter(), s, r.center(), if sel { TEXT } else { MUTED });
                if resp.on_hover_text(format!("{s:?}")).clicked() {
                    pip.shape = s;
                    changed = true;
                }
            }
        });
        ui.add_space(2.0);
        changed |= row(ui, l("Size", "Tamaño"), |ui| ui.add(egui::Slider::new(&mut pip.size, 0.1..=0.5).show_value(false))).changed();
        if matches!(pip.shape, Shape::Rounded | Shape::Square | Shape::Squircle) {
            changed |= row(ui, l("Roundness", "Redondez"), |ui| ui.add(egui::Slider::new(&mut pip.roundness, 0.0..=1.0).show_value(false))).changed();
        }
        changed |= row(ui, l("Border", "Borde"), |ui| ui.add(egui::Slider::new(&mut pip.border, 0.0..=24.0).show_value(false))).changed();
        if pip.border > 0.0 {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(l("Color", "Color")).color(MUTED).size(13.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for c in [[36, 84, 255], [255, 196, 0], [255, 102, 0], [214, 38, 46], [255, 255, 255]] {
                        let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                        ui.painter().circle_filled(r.center(), 9.0, Color32::from_rgb(c[0], c[1], c[2]));
                        if pip.border_color == c {
                            ui.painter().circle_stroke(r.center(), 11.0, Stroke::new(1.5, TEXT));
                        }
                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            pip.border_color = c;
                            changed = true;
                        }
                    }
                    changed |= ui.color_edit_button_srgb(&mut pip.border_color).on_hover_text(l("Any color", "Cualquier color")).changed();
                });
            });
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(l("Shadow", "Sombra")).color(MUTED).size(13.0));
            changed |= toggle(ui, &mut pip.shadow).changed();
            ui.add_space(16.0);
            ui.label(egui::RichText::new(l("Mirror", "Espejo")).color(MUTED).size(13.0));
            changed |= toggle(ui, &mut pip.mirror).changed();
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(l("Corner", "Esquina")).color(MUTED).size(13.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (c, l) in [(Corner::BottomRight, "↘"), (Corner::BottomLeft, "↙"), (Corner::TopRight, "↗"), (Corner::TopLeft, "↖")] {
                    let sel = pip.place == Place::Corner(c);
                    if ui.add(egui::Button::new(egui::RichText::new(l).size(14.0)).selected(sel).min_size(vec2(32.0, 28.0))).clicked() {
                        pip.place = Place::Corner(c);
                        changed = true;
                    }
                }
            });
        });
        if changed {
            app.settings.set_pip(&pip);
            app.settings_changed();
        }
        if vbg_row(ui, &mut app.settings.vbg, 190.0) {
            app.settings_changed();
        }
    });
}

/// The virtual background picker: as it is, a light or strong blur, a
/// picture used before, or a new one. True when it changed.
pub(super) fn vbg_row(ui: &mut Ui, v: &mut crate::vbg::VirtualBackground, width: f32) -> bool {
    use crate::vbg::Mode;
    let mut changed = false;
    let name = |p: &str| std::path::Path::new(p).file_stem().map(|s| truncate(&s.to_string_lossy(), 24)).unwrap_or_default();
    let (light, strong) = (l("Light blur", "Desenfoque suave"), l("Strong blur", "Desenfoque fuerte"));
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        ui.label(egui::RichText::new(l("Behind you", "Detrás de ti")).color(MUTED).size(13.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let current = match v.mode {
                Mode::Off => l("As it is", "Tal cual").to_string(),
                Mode::Blur => if v.blur < 0.5 { light } else { strong }.to_string(),
                Mode::Image => v.image.as_deref().map(name).unwrap_or_default(),
            };
            ComboBox::from_id_salt("vbg").width(width).selected_text(current).show_ui(ui, |ui| {
                if ui.selectable_label(v.mode == Mode::Off, l("As it is", "Tal cual")).clicked() {
                    v.mode = Mode::Off;
                    changed = true;
                }
                if ui.selectable_label(v.mode == Mode::Blur && v.blur < 0.5, light).clicked() {
                    v.mode = Mode::Blur;
                    v.blur = 0.35;
                    changed = true;
                }
                if ui.selectable_label(v.mode == Mode::Blur && v.blur >= 0.5, strong).clicked() {
                    v.mode = Mode::Blur;
                    v.blur = 0.85;
                    changed = true;
                }
                if !v.images.is_empty() {
                    ui.separator();
                }
                for p in v.images.clone() {
                    if ui.selectable_label(v.mode == Mode::Image && v.image.as_deref() == Some(p.as_str()), name(&p)).clicked() {
                        v.pick(p);
                        changed = true;
                    }
                }
                ui.separator();
                if ui.button(l("Choose a picture…", "Elegir una imagen…")).clicked()
                    && let Some(p) = super::editor::pick_image()
                {
                    v.pick(p.to_string_lossy().into_owned());
                    changed = true;
                }
            });
        });
    });
    changed
}

fn recordings(app: &mut App, ui: &mut Ui, card: Rect, max_rows: usize, compact: bool) {
    let p = ui.painter().clone();
    theme::card(&p, card);
    let title = text(&p, card.min + vec2(22.0, 20.0), Align2::LEFT_TOP, l("Your recordings", "Tus grabaciones"), medium(15.0), TEXT);
    // Background work in progress.
    if let Some(j) = app.job.lock().unwrap().as_ref() {
        let name = j.file.file_stem().unwrap_or_default().to_string_lossy().replace("Chisme ", "");
        let what = if j.kind == JobKind::Subtitles { l("Subtitles", "Subtítulos") } else { "Vertical 9:16" };
        let queued = if app.jobs.is_empty() { String::new() } else { t!("  (+{} queued)", "  (+{} en cola)", app.jobs.len()) };
        let msg = format!("{what} · {} {:.0}% · {}{}", j.stage, j.progress * 100.0, name, queued);
        p.circle_filled(pos2(title.right() + 20.0, title.center().y), 4.0, ORANGE);
        text(&p, pos2(title.right() + 30.0, title.center().y), Align2::LEFT_CENTER, msg, body(12.0), ORANGE);
    }
    if compact {
        let link = Rect::from_min_size(pos2(card.right() - 130.0, card.top() + 14.0), vec2(110.0, 30.0));
        if pill(ui, link, "see-all", l("See all  →", "Ver todas  →"), Color32::from_rgba_premultiplied(36, 36, 40, 80), TEXT, medium(12.5)).clicked() {
            app.page = Page::Library;
        }
    } else if !app.recordings.is_empty() {
        // Selection: all / none, and delete what's ticked.
        app.selected.retain(|p| p.is_file());
        let n = app.selected.len();
        let all = n == app.recordings.len();
        let mut x = card.right() - 22.0;
        if n > 0 {
            let w = 150.0;
            let del = Rect::from_min_size(pos2(x - w, card.top() + 14.0), vec2(w, 30.0));
            if pill(ui, del, "delete-sel", &t!("Delete ({})", "Eliminar ({})", n), Color32::from_rgba_premultiplied(110, 20, 20, 170), Color32::WHITE, semibold(12.5)).clicked() {
                app.confirm_delete = true;
            }
            x = del.left() - 8.0;
        }
        let w = 130.0;
        let sel = Rect::from_min_size(pos2(x - w, card.top() + 14.0), vec2(w, 30.0));
        if pill(ui, sel, "select-all", if all { l("Select none", "Ninguna") } else { l("Select all", "Seleccionar todo") }, Color32::from_rgba_premultiplied(36, 36, 40, 80), TEXT, medium(12.5)).clicked() {
            if all {
                app.selected.clear();
            } else {
                app.selected = app.recordings.iter().map(|r| r.path.clone()).collect();
            }
        }
    }
    if app.confirm_delete {
        let n = app.selected.len();
        let (mut yes, mut no) = (false, false);
        egui::Modal::new(Id::new("confirm-delete")).show(ui.ctx(), |ui| {
            ui.set_width(380.0);
            ui.label(egui::RichText::new(t!("Delete {} recording(s)?", "¿Eliminar {} grabación(es)?", n)).font(display(16.0)).color(TEXT));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(l("They go to the Recycle Bin with their subtitles and chapters, so you can still restore them.", "Se van a la Papelera con sus subtítulos y capítulos, así que todavía puedes recuperarlas.")).color(MUTED).size(13.0));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(egui::RichText::new(l("Delete", "Eliminar")).strong().color(Color32::WHITE)).fill(RED).min_size(vec2(110.0, 30.0))).clicked() {
                    yes = true;
                }
                if ui.add(egui::Button::new(l("Cancel", "Cancelar")).min_size(vec2(90.0, 30.0))).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    no = true;
                }
            });
        });
        if yes {
            app.confirm_delete = false;
            let paths: Vec<std::path::PathBuf> = app.selected.iter().cloned().collect();
            app.delete_recordings(&paths);
        } else if no {
            app.confirm_delete = false;
        }
    }
    let table = Rect::from_min_max(pos2(card.left() + 22.0, card.top() + 56.0), pos2(card.right() - 22.0, card.bottom() - 12.0));
    // Date and size columns sit left of the row's buttons (~360 px).
    // (Hidden on narrow windows, where they'd run into the buttons.)
    let wide = table.width() >= 860.0;
    let date_x = table.left() + (table.width() * 0.48).min(table.width() - 360.0 - 250.0).max(240.0);
    let size_x = date_x + 150.0;
    for (x, label) in [(table.left(), "Video"), (date_x, l("Recorded", "Grabado")), (size_x, l("Size", "Tamaño"))] {
        if wide || x == table.left() {
            text(&p, pos2(x, table.top()), Align2::LEFT_TOP, label, body(12.0), MUTED);
        }
    }
    p.line_segment([pos2(table.left(), table.top() + 22.0), pos2(table.right(), table.top() + 22.0)], Stroke::new(1.0, Color32::from_rgba_premultiplied(50, 50, 54, 60)));
    let list = Rect::from_min_max(pos2(table.left(), table.top() + 32.0), table.max);
    if app.recordings.is_empty() {
        text(&p, list.center(), Align2::CENTER_CENTER, t!("No recordings yet. Press Record or {}.", "Aún no hay grabaciones. Presiona Grabar o {}.", app.settings.hotkeys.record), body(13.0), MUTED);
        return;
    }
    let row_h = 64.0;
    let recs: Vec<(std::path::PathBuf, String, u64, SystemTime)> =
        app.recordings.iter().take(if compact { max_rows } else { usize::MAX }).map(|r| (r.path.clone(), r.name.clone(), r.bytes, r.modified)).collect();
    ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for (i, (path, name, bytes, modified)) in recs.iter().enumerate() {
                let (r, _) = ui.allocate_exact_size(vec2(list.width(), row_h), Sense::hover());
                let rr = r.shrink2(vec2(0.0, 4.0));
                let p = ui.painter();
                p.rect_filled(rr, CornerRadius::same(18), Color32::from_rgba_premultiplied(30, 30, 33, 45));
                // A tick box to select it (Recordings page).
                let ox = if compact { 0.0 } else { 34.0 };
                if !compact {
                    let b = Rect::from_center_size(pos2(rr.left() + 20.0, rr.center().y), vec2(20.0, 20.0));
                    let on = app.selected.contains(path);
                    let r = ui.interact(b.expand(4.0), Id::new(("rec-sel", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                    let p = ui.painter();
                    p.rect_filled(b, CornerRadius::same(5), if on { ORANGE } else { Color32::from_rgba_premultiplied(0, 0, 0, 120) });
                    p.rect_stroke(b, CornerRadius::same(5), Stroke::new(1.2, if on { ORANGE } else { Color32::from_white_alpha(70) }), StrokeKind::Inside);
                    if on {
                        p.add(EShape::line(vec![b.center() + vec2(-5.0, 0.0), b.center() + vec2(-1.5, 4.0), b.center() + vec2(5.0, -4.0)], Stroke::new(2.0, Color32::BLACK)));
                    }
                    if r.clicked() {
                        if on {
                            app.selected.remove(path);
                        } else {
                            app.selected.insert(path.clone());
                        }
                    }
                }
                let p = ui.painter();
                // Thumbnail: Ventura's gradient with a play mark.
                let th = Rect::from_min_size(rr.min + vec2(6.0 + ox, 6.0), vec2(72.0, rr.height() - 12.0));
                p.rect_filled(th, CornerRadius::same(12), Color32::from_rgb(16, 16, 18));
                gradient_bar(p, Rect::from_min_max(pos2(th.left() + 8.0, th.bottom() - 9.0), pos2(th.right() - 8.0, th.bottom() - 6.0)), &RAINBOW);
                let c = th.center() - vec2(0.0, 3.0);
                p.add(EShape::convex_polygon(vec![c + vec2(-5.0, -7.0), c + vec2(8.0, 0.0), c + vec2(-5.0, 7.0)], TEXT, Stroke::NONE));
                let title = name.strip_prefix("Chisme ").unwrap_or(name);
                p.text(pos2(th.right() + 16.0, rr.center().y - 8.0), Align2::LEFT_CENTER, title, medium(13.5), TEXT);
                let chapters = crate::chapters::count(path);
                let info = if chapters > 0 { t!("MP4 · H.264 · {} chapters", "MP4 · H.264 · {} capítulos", chapters) } else { "MP4 · H.264".into() };
                p.text(pos2(th.right() + 16.0, rr.center().y + 10.0), Align2::LEFT_CENTER, info, body(11.5), FAINT);
                if wide {
                    p.circle_filled(pos2(date_x + 4.0, rr.center().y), 3.5, YELLOW);
                    p.text(pos2(date_x + 14.0, rr.center().y), Align2::LEFT_CENTER, ago(*modified), body(12.5), TEXT);
                    p.circle_filled(pos2(size_x + 4.0, rr.center().y), 3.5, GREEN);
                    p.text(pos2(size_x + 14.0, rr.center().y), Align2::LEFT_CENTER, mb(*bytes), body(12.5), TEXT);
                }
                // Subtitles and vertical copy: start, show progress, or open the result.
                let job_button = |ui: &mut Ui, app: &mut App, kind: JobKind, rect: Rect, label: &str, done: std::path::PathBuf, tip: &str| {
                    let id = format!("{kind:?}{i}");
                    if let Some((stage, p)) = app.job_for(kind, path) {
                        let pct = if p > 0.0 { format!("{:.0}%", p * 100.0) } else { "…".into() };
                        pill(ui, rect, &id, &pct, Color32::from_rgba_premultiplied(60, 30, 0, 90), ORANGE, semibold(12.5)).on_hover_text(stage);
                    } else if app.jobs.contains(&(kind, path.clone())) {
                        pill(ui, rect, &id, l("Queued", "En cola"), Color32::from_rgba_premultiplied(40, 40, 44, 80), MUTED, medium(12.0));
                    } else if done.is_file() {
                        if pill(ui, rect, &id, &format!("{label} ✓"), Color32::from_rgba_premultiplied(20, 60, 40, 90), GREEN, semibold(12.5)).on_hover_text(l("Open", "Abrir")).clicked() {
                            App::open_path(&done);
                        }
                    } else if pill(ui, rect, &id, label, Color32::from_rgba_premultiplied(40, 40, 44, 80), TEXT, semibold(12.5)).on_hover_text(tip).clicked() {
                        app.queue(kind, path.clone());
                    }
                };
                let is_vertical = name.ends_with(" vertical");
                let cc = Rect::from_min_size(pos2(rr.right() - 352.0, rr.center().y - 16.0), vec2(72.0, 32.0));
                job_button(ui, app, JobKind::Subtitles, cc, "CC", crate::subtitles::srt_path(path), l("Make subtitles with Whisper", "Generar subtítulos con Whisper"));
                if !is_vertical {
                    let v = Rect::from_min_size(pos2(rr.right() - 274.0, rr.center().y - 16.0), vec2(72.0, 32.0));
                    job_button(ui, app, JobKind::Vertical, v, "9:16", crate::export::vertical_path(path), l("Vertical 1080×1920 copy for TikTok, Reels and Shorts", "Copia vertical 1080×1920 para TikTok, Reels y Shorts"));
                }
                let open = Rect::from_min_size(pos2(rr.right() - 196.0, rr.center().y - 16.0), vec2(84.0, 32.0));
                let show = Rect::from_min_size(pos2(rr.right() - 104.0, rr.center().y - 16.0), vec2(94.0, 32.0));
                if pill(ui, open, &format!("open{i}"), l("▶  Open", "▶  Abrir"), Color32::WHITE, Color32::BLACK, semibold(12.5)).clicked() {
                    App::open_path(path);
                }
                if pill(ui, show, &format!("show{i}"), l("Show", "Mostrar"), Color32::from_rgba_premultiplied(40, 40, 44, 80), TEXT, medium(12.5)).clicked() {
                    App::reveal(path);
                }
            }
        });
    });
}

// ---------------------------------------------------------------- live

fn brand(p: Platform) -> Color32 {
    match p {
        Platform::Youtube => Color32::from_rgb(255, 0, 51),
        Platform::Twitch => Color32::from_rgb(145, 70, 255),
        Platform::Tiktok => Color32::from_rgb(37, 244, 238),
        Platform::X => Color32::WHITE,
        Platform::Custom => ORANGE,
    }
}

fn live(app: &mut App, ui: &mut Ui, area: Rect) {
    let cols = if area.width() < 760.0 { 1 } else if area.width() < 1180.0 { 2 } else { 3 };
    let gap = 16.0;
    let rows = 6usize.div_ceil(cols);
    ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let width = area.width() - 4.0;
            let w = (width - gap * (cols - 1) as f32) / cols as f32;
            let h = ((area.height() - gap) / 2.0).max(300.0);
            let (all, _) = ui.allocate_exact_size(vec2(width, rows as f32 * (h + gap) - gap), Sense::hover());
            let cell = |i: usize| Rect::from_min_size(pos2(all.left() + (w + gap) * (i % cols) as f32, all.top() + (h + gap) * (i / cols) as f32), vec2(w, h));
            for (i, p) in Platform::ALL.into_iter().enumerate() {
                platform_card(app, ui, cell(i), p);
            }
            replay_card(app, ui, cell(5));
        });
    });
}

fn platform_card(app: &mut App, ui: &mut Ui, card: Rect, p: Platform) {
    let painter = ui.painter().clone();
    theme::card(&painter, card);
    let status = app.status.streams.iter().find(|s| s.platform == Some(p)).cloned();
    // Title: brand dot (TikTok's is two-tone) and the preset.
    let dot = card.min + vec2(28.0, 30.0);
    if p == Platform::Tiktok {
        painter.circle_filled(dot + vec2(-2.0, 0.0), 6.0, Color32::from_rgb(254, 44, 85));
    }
    painter.circle_filled(dot, 6.0, brand(p));
    text(&painter, dot + vec2(16.0, 0.0), Align2::LEFT_CENTER, p.label(), medium(16.0), TEXT);
    let vertical = app.settings.is_vertical(p);
    let fps = app.status.fps.max(1);
    let preset = p.preset(fps);
    let live_fps = fps / (fps as f64 / preset.max_fps as f64).ceil().max(1.0) as u32;
    let size = if vertical { "1080×1920" } else { "1080p" };
    text(&painter, pos2(card.right() - 22.0, dot.y), Align2::RIGHT_CENTER, format!("{size} · {live_fps} fps · {:.1} Mb/s", preset.kbps as f64 / 1000.0), body(12.0), MUTED);

    let inner = Rect::from_min_max(pos2(card.left() + 22.0, card.top() + 56.0), pos2(card.right() - 22.0, card.bottom() - 16.0));
    let on = status.is_some();
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.add_enabled_ui(!on, |ui| {
            if p.default_url().is_none() {
                let mut url = app.settings.stream_url(p);
                let hint = match p {
                    Platform::X => "rtmps://va.pscp.tv:443/x",
                    Platform::Tiktok => "rtmp://push-rtmp-….tiktokcdn.com/…",
                    _ => l("rtmp:// or rtmps://…", "rtmp:// o rtmps://…"),
                };
                if ui.add(egui::TextEdit::singleline(&mut url).hint_text(hint).desired_width(inner.width())).changed() {
                    app.settings.set_stream_url(p, &url);
                    let _ = app.settings.save();
                }
            }
            let key = app.key_mut(p);
            ui.add(egui::TextEdit::singleline(key).password(true).hint_text(l("Stream key", "Clave de transmisión")).desired_width(inner.width()));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Vertical 9:16").color(MUTED).size(13.0));
                let mut v = vertical;
                if toggle(ui, &mut v).changed() {
                    app.settings.vertical.retain(|q| *q != p);
                    if v {
                        app.settings.vertical.push(p);
                    }
                    let _ = app.settings.save();
                }
            });
        });
        ui.add_space(4.0);
        let (r, _) = ui.allocate_exact_size(vec2(150.0, 40.0), Sense::hover());
        let ready = app.status.ready;
        if on {
            if pill(ui, r, &format!("stop-{}", p.id()), l("■  Stop", "■  Detener"), RED, Color32::WHITE, semibold(14.0)).clicked() {
                app.stop_stream(p);
            }
        } else if pill(ui, r, &format!("go-{}", p.id()), l("●  Go live", "●  Transmitir"), if ready { Color32::WHITE } else { Color32::from_gray(90) }, Color32::BLACK, semibold(14.0)).clicked() && ready {
            app.start_stream(p);
        }
        let painter = ui.painter();
        let sx = r.right() + 14.0;
        match &status {
            Some(s) => {
                let col = if s.live { GREEN } else { ORANGE };
                painter.circle_filled(pos2(sx + 4.0, r.center().y - 8.0), 4.0, col);
                painter.text(pos2(sx + 14.0, r.center().y - 8.0), Align2::LEFT_CENTER, crate::live::state_label(&s.state), semibold(12.5), col);
                let detail = t!("{}  ·  {:.1} Mb/s  ·  {}×{}  ·  dropped {}", "{}  ·  {:.1} Mb/s  ·  {}×{}  ·  perdidos {}", clock(s.duration), s.bitrate / 1e6, s.size.0, s.size.1, s.dropped);
                painter.text(pos2(sx, r.center().y + 10.0), Align2::LEFT_CENTER, detail, body(11.5), MUTED);
            }
            None => {
                painter.text(pos2(sx, r.center().y), Align2::LEFT_CENTER, l("Off air", "Fuera del aire"), body(12.5), FAINT);
            }
        }
        if let Some(err) = status.as_ref().and_then(|s| s.error.clone()) {
            ui.label(egui::RichText::new(truncate(&err, 120)).color(ORANGE).size(11.5));
        }
    });
    text(&painter, pos2(card.left() + 22.0, card.bottom() - 18.0), Align2::LEFT_BOTTOM, p.help(), body(11.0), FAINT);
}

fn replay_card(app: &mut App, ui: &mut Ui, card: Rect) {
    let painter = ui.painter().clone();
    theme::card(&painter, card);
    text(&painter, card.min + vec2(22.0, 20.0), Align2::LEFT_TOP, "Replay buffer", medium(16.0), TEXT);
    gradient_bar(&painter, Rect::from_min_size(card.min + vec2(22.0, 46.0), vec2(80.0, 2.0)), &RAINBOW);
    let inner = Rect::from_min_max(pos2(card.left() + 22.0, card.top() + 62.0), pos2(card.right() - 22.0, card.bottom() - 16.0));
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.label(egui::RichText::new(l("Save the last seconds at any moment, even when you're not recording.", "Guarda los últimos segundos en cualquier momento, aunque no estés grabando.")).color(MUTED).size(12.5));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(l("On", "Activo")).color(MUTED).size(13.0));
            let mut on = app.settings.replay_enabled;
            if toggle(ui, &mut on).changed() {
                app.set_replay(on);
            }
        });
        let mut secs = app.settings.replay_secs;
        let changed = row(ui, l("Length", "Duración"), |ui| ui.add(egui::Slider::new(&mut secs, 15.0..=300.0).step_by(5.0).suffix(" s"))).drag_stopped();
        app.settings.replay_secs = secs;
        if changed && app.settings.replay_enabled {
            app.set_replay(true);
        }
        if let Some((held, bytes)) = app.status.replay {
            ui.label(egui::RichText::new(t!("In memory: {:.0} s  ·  {}", "En memoria: {:.0} s  ·  {}", held, mb(bytes as u64))).color(TEXT).size(12.5));
        }
        ui.add_space(6.0);
        let (r, _) = ui.allocate_exact_size(vec2(190.0, 40.0), Sense::hover());
        let can = app.status.replay.is_some();
        if pill(ui, r, "save-replay", l("Save replay", "Guardar replay"), if can { Color32::WHITE } else { Color32::from_gray(90) }, Color32::BLACK, semibold(14.0)).on_hover_text(&app.settings.hotkeys.replay).clicked() {
            app.save_replay();
        }
        let (r2, _) = ui.allocate_exact_size(vec2(190.0, 36.0), Sense::hover());
        if pill(ui, r2, "save-replay-v", l("Save vertical 9:16 clip", "Guardar clip vertical 9:16"), Color32::from_rgba_premultiplied(40, 40, 44, 80), if can { TEXT } else { FAINT }, medium(13.0))
            .on_hover_text(l("Saves the replay and a 1080×1920 copy for TikTok, Reels and Shorts", "Guarda el replay y una copia 1080×1920 para TikTok, Reels y Shorts"))
            .clicked()
        {
            app.save_replay_vertical();
        }
        if let Some(p) = app.status.last_replay.clone() {
            ui.add_space(4.0);
            if ui.link(egui::RichText::new(t!("Last: {}", "Último: {}", p.file_name().unwrap_or_default().to_string_lossy())).size(12.0)).clicked() {
                App::reveal(&p);
            }
        }
    });
}

fn library(app: &mut App, ui: &mut Ui, area: Rect) {
    recordings(app, ui, area, usize::MAX, false);
}

// ---------------------------------------------------------------- settings

fn section(ui: &mut Ui, rect: Rect, title: &str, add: impl FnOnce(&mut Ui)) {
    theme::card(ui.painter(), rect);
    text(ui.painter(), rect.min + vec2(22.0, 20.0), Align2::LEFT_TOP, title, medium(15.0), TEXT);
    ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_max(rect.min + vec2(22.0, 54.0), rect.max - vec2(22.0, 14.0))), add);
}

fn combo<T: PartialEq + Clone>(ui: &mut Ui, id: &str, current: &mut T, options: &[(T, String)], width: f32) -> bool {
    let sel = options.iter().find(|(v, _)| v == current).map(|(_, l)| l.clone()).unwrap_or_else(|| "—".into());
    let mut changed = false;
    ComboBox::from_id_salt(id).width(width).selected_text(sel).show_ui(ui, |ui| {
        for (v, l) in options {
            if ui.selectable_label(v == current, l).clicked() && v != current {
                *current = v.clone();
                changed = true;
            }
        }
    });
    changed
}

fn settings(app: &mut App, ui: &mut Ui, area: Rect) {
    if app.devices.displays.is_empty() {
        app.refresh_devices();
    }
    let s0 = app.settings.clone();
    // Restart-only changes wait until the recording ends.
    let mut area = area;
    if app.status.recording && app.settings.config() != app.config {
        let msg = l("Source, video and audio changes apply when the recording ends.", "Los cambios de fuente, video y audio se aplican al terminar la grabación.");
        text(ui.painter(), area.left_top() + vec2(4.0, 0.0), Align2::LEFT_TOP, msg, body(12.5), ORANGE);
        area.min.y += 26.0;
    }
    ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
        ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let width = area.width() - 14.0;
            let (h1, h2, h3, h4) = (196.0, 308.0, 150.0, 340.0);
            let one = width < 900.0;
            let col_w = if one { width } else { (width - 16.0) / 2.0 };
            let total = if one { 2.0 * (h1 + h2 + h3) + h4 + 112.0 } else { h1 + h2 + h3 + h4 + 48.0 };
            let (all, _) = ui.allocate_exact_size(vec2(width, total), Sense::hover());
            // Two columns side by side, or the right one under the left.
            let (lx, rx, dy) = if one { (all.left(), all.left(), h1 + h2 + h3 + 48.0) } else { (all.left(), all.left() + col_w + 16.0, 0.0) };
            let y2 = all.top() + h1 + 16.0;
            let y3 = y2 + h2 + 16.0;
            let y4 = y3 + h3 + 16.0 + if one { dy } else { 0.0 };
            let ry = |y: f32| y + dy;
            let w = col_w - 200.0;
            source_section(app, ui, Rect::from_min_size(pos2(lx, all.top()), vec2(col_w, h1)), w);
            video_section(app, ui, Rect::from_min_size(pos2(rx, ry(all.top())), vec2(col_w, h1)), w);
            audio_section(app, ui, Rect::from_min_size(pos2(lx, y2), vec2(col_w, h2)), w);
            general_section(app, ui, Rect::from_min_size(pos2(rx, ry(y2)), vec2(col_w, h2)), w);
            subtitles_section(app, ui, Rect::from_min_size(pos2(lx, y3), vec2(col_w, h3)));
            brand_section(app, ui, Rect::from_min_size(pos2(rx, ry(y3)), vec2(col_w, h3)));
            shortcuts_section(app, ui, Rect::from_min_size(pos2(lx, y4), vec2(width, h4)));
        });
    });
    if app.settings != s0 {
        app.settings_changed();
    }
}

fn source_section(app: &mut App, ui: &mut Ui, rect: Rect, w: f32) {
    section(ui, rect, l("Source", "Fuente"), |ui| {
        let mut opts: Vec<(Option<(usize, String)>, String)> = vec![];
        for (i, d) in app.devices.displays.iter().enumerate() {
            opts.push((Some((usize::MAX - i, String::new())), t!("Display {}  ·  {}", "Pantalla {}  ·  {}", i + 1, d)));
        }
        for (h, title) in &app.devices.windows {
            opts.push((Some((*h, title.clone())), t!("Window: {}", "Ventana: {}", truncate(title, 48))));
        }
        let mut cur = match &app.settings.window {
            Some(w) => Some(w.clone()),
            None => Some((usize::MAX - (app.settings.display - 1), String::new())),
        };
        row(ui, l("Record", "Grabar"), |ui| {
            if combo(ui, "source", &mut cur, &opts, w) {
                match cur.clone() {
                    Some((h, _)) if h > usize::MAX - 64 => {
                        app.settings.display = usize::MAX - h + 1;
                        app.settings.window = None;
                    }
                    Some(win) => app.settings.window = Some(win),
                    None => {}
                }
            }
            ui.label("")
        });
        row(ui, l("Mouse pointer", "Puntero del mouse"), |ui| toggle(ui, &mut app.settings.cursor));
        row(ui, l("Show Chisme in recordings", "Mostrar Chisme en las grabaciones"), |ui| toggle(ui, &mut app.settings.show_self));
        if let Some(q) = app.settings.camera.clone()
            && let Some(full) = app.devices.cameras.iter().find(|c| c.to_lowercase().contains(&q.to_lowercase()))
        {
            app.settings.camera = Some(full.clone());
        }
        let mut cam = app.settings.camera.clone();
        let mut cams: Vec<(Option<String>, String)> = vec![(None, l("No camera", "Sin cámara").into())];
        cams.extend(app.devices.cameras.iter().map(|c| (Some(c.clone()), c.clone())));
        row(ui, l("Camera", "Cámara"), |ui| {
            if combo(ui, "cam", &mut cam, &cams, w) {
                app.settings.camera = cam.clone();
            }
            ui.label("")
        });
    });
}

fn video_section(app: &mut App, ui: &mut Ui, rect: Rect, w: f32) {
    section(ui, rect, "Video", |ui| {
        let res: Vec<(String, String)> =
            ["720p", "1080p", "1440p", "4k", "native"].iter().map(|r| (r.to_string(), if *r == "native" { l("Native", "Nativa").into() } else { r.to_string() })).collect();
        row(ui, l("Resolution", "Resolución"), |ui| {
            combo(ui, "res", &mut app.settings.resolution, &res, w);
            ui.label("")
        });
        let fps: Vec<(u32, String)> = [24, 30, 60, 120].iter().map(|f| (*f, format!("{f} fps"))).collect();
        row(ui, l("Frame rate", "Cuadros por segundo"), |ui| {
            combo(ui, "fps", &mut app.settings.fps, &fps, w);
            ui.label("")
        });
        let codecs = [(Codec::H264, l("H.264 (compatible)", "H.264 (compatible)").to_string()), (Codec::Hevc, l("HEVC (smaller files)", "HEVC (más ligero)").to_string())];
        row(ui, l("Codec", "Códec"), |ui| {
            combo(ui, "codec", &mut app.settings.codec, &codecs, w);
            ui.label("")
        });
    });
}

fn audio_section(app: &mut App, ui: &mut Ui, rect: Rect, w: f32) {
    section(ui, rect, "Audio", |ui| {
        row(ui, l("System audio", "Audio del sistema"), |ui| toggle(ui, &mut app.settings.system_audio));
        if app.settings.system_audio {
            // Per-app audio: everything, one app, or all but one app.
            let mut apps = app.devices.audio_apps.clone();
            if let Some(a) = &app.settings.system_audio_app
                && !apps.iter().any(|x| x.eq_ignore_ascii_case(a))
            {
                apps.push(a.clone());
            }
            let mut opts: Vec<((Option<String>, bool), String)> = vec![((None, false), l("Everything", "Todo el sistema").into())];
            opts.extend(apps.iter().map(|a| ((Some(a.clone()), false), t!("Only: {}", "Solo: {}", a))));
            opts.extend(apps.iter().map(|a| ((Some(a.clone()), true), t!("All except: {}", "Todo excepto: {}", a))));
            let mut cur = (app.settings.system_audio_app.clone(), app.settings.system_audio_app.is_some() && app.settings.system_audio_exclude);
            row(ui, l("From which apps", "De qué apps"), |ui| {
                if combo(ui, "app-audio", &mut cur, &opts, w) {
                    app.settings.system_audio_app = cur.0.clone();
                    app.settings.system_audio_exclude = cur.1;
                }
                if ui.small_button("↻").on_hover_text(l("Find apps playing sound", "Buscar apps con sonido")).clicked() {
                    app.devices.audio_apps = crate::win::apps::audio_apps().unwrap_or_default();
                }
                ui.label("")
            });
        }
        row(ui, l("Microphone", "Micrófono"), |ui| toggle(ui, &mut app.settings.mic_enabled));
        if app.settings.mic_enabled {
            row(ui, l("Remove background noise (RNNoise)", "Quitar ruido de fondo (RNNoise)"), |ui| toggle(ui, &mut app.settings.mic_denoise));
            let mut mics: Vec<(Option<String>, String)> = vec![(None, l("Windows default", "Predeterminado de Windows").into())];
            mics.extend(app.devices.mics.iter().map(|m| (Some(m.clone()), m.clone())));
            // A saved name part ("maono") selects the device it matches.
            if let Some(q) = app.settings.mic.clone()
                && let Some(full) = app.devices.mics.iter().find(|m| m.to_lowercase().contains(&q.to_lowercase()))
            {
                app.settings.mic = Some(full.clone());
            }
            row(ui, l("Device", "Dispositivo"), |ui| {
                combo(ui, "mic", &mut app.settings.mic, &mics, w);
                ui.label("")
            });
        }
        let layouts = [
            (AudioLayout::Both, l("Mix + separate tracks", "Mezcla + pistas separadas").to_string()),
            (AudioLayout::Mix, l("Mix only", "Solo mezcla").to_string()),
            (AudioLayout::Stems, l("Separate tracks only", "Solo pistas separadas").to_string()),
        ];
        row(ui, l("Tracks", "Pistas"), |ui| {
            combo(ui, "tracks", &mut app.settings.audio_tracks, &layouts, w);
            ui.label("")
        });
        row(ui, l("Mic volume (mix)", "Volumen del micrófono (mezcla)"), |ui| ui.add(egui::Slider::new(&mut app.settings.mic_gain_db, -20.0..=20.0).suffix(" dB")));
    });
}

fn general_section(app: &mut App, ui: &mut Ui, rect: Rect, w: f32) {
    section(ui, rect, l("General", "General"), |ui| {
        let langs: Vec<(crate::i18n::Lang, String)> = crate::i18n::Lang::ALL.iter().map(|l| (*l, l.label().to_string())).collect();
        row(ui, l("Language", "Idioma"), |ui| {
            if combo(ui, "language", &mut app.settings.language, &langs, w) {
                crate::i18n::set(app.settings.language);
            }
            ui.label("")
        });
        ui.add_space(8.0);
        ui.label(egui::RichText::new(l("Recordings folder", "Carpeta de grabaciones")).color(MUTED).size(13.0));
        let mut dir = app.settings.output_dir.display().to_string();
        ui.horizontal(|ui| {
            if ui.add(egui::TextEdit::singleline(&mut dir).desired_width(rect.width() - 150.0)).changed() {
                app.settings.output_dir = dir.clone().into();
            }
            if ui.button(l("Open", "Abrir")).clicked() {
                let _ = std::fs::create_dir_all(&app.settings.output_dir);
                App::open_path(&app.settings.output_dir);
            }
        });
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(l(
                "Closing the window while recording keeps recording in the tray. Shortcuts work even when the window is hidden.",
                "Cerrar la ventana mientras grabas la manda a la bandeja. Los atajos funcionan aunque la ventana esté oculta.",
            ))
            .color(FAINT)
            .size(12.0),
        );
    });
}

fn subtitles_section(app: &mut App, ui: &mut Ui, rect: Rect) {
    section(ui, rect, l("Subtitles (Whisper, on your PC)", "Subtítulos (Whisper, en tu PC)"), |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(l("After every recording", "Al terminar cada grabación")).color(MUTED).size(13.0));
            toggle(ui, &mut app.settings.subtitles_auto);
            ui.add_space(12.0);
            let langs: Vec<(String, String)> = [("auto", l("Detect language", "Detectar idioma")), ("es", "Español"), ("en", "English"), ("pt", "Português"), ("fr", "Français"), ("de", "Deutsch"), ("it", "Italiano")]
                .iter()
                .map(|(c, l)| (c.to_string(), l.to_string()))
                .collect();
            combo(ui, "sub-lang", &mut app.settings.subtitle_lang, &langs, 140.0);
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(l("Model", "Modelo")).color(MUTED).size(13.0));
            let models: Vec<(Model, String)> = Model::ALL.iter().map(|m| (*m, format!("{}{}", m.label(), if m.downloaded() { "  ✓" } else { "" }))).collect();
            combo(ui, "sub-model", &mut app.settings.subtitle_model, &models, 260.0);
        });
        ui.label(egui::RichText::new(l("Writes an .srt next to the video. The audio never leaves your PC.", "Crea un .srt junto al video. El audio nunca sale de tu PC.")).color(FAINT).size(12.0));
    });
}

fn brand_section(app: &mut App, ui: &mut Ui, rect: Rect) {
    section(ui, rect, l("Brand", "Marca"), |ui| {
        let b = &mut app.settings.brand;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Logo").color(MUTED).size(13.0));
            let name = b.logo.as_deref().and_then(|p| std::path::Path::new(p).file_name()).map_or(l("None", "Ninguno").to_string(), |n| truncate(&n.to_string_lossy(), 22));
            ui.label(egui::RichText::new(name).size(13.0));
            if ui.button(l("Choose…", "Elegir…")).clicked()
                && let Some(p) = super::editor::pick_image()
            {
                b.logo = Some(p.to_string_lossy().into_owned());
            }
            if b.logo.is_some() && ui.small_button(l("Remove", "Quitar")).clicked() {
                b.logo = None;
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(l("Colors", "Colores")).color(MUTED).size(13.0));
            ui.color_edit_button_srgb(&mut b.primary).on_hover_text(l("Primary", "Principal"));
            ui.color_edit_button_srgb(&mut b.secondary).on_hover_text(l("Secondary", "Secundario"));
            ui.add_space(12.0);
            ui.label(egui::RichText::new(l("Font", "Letra")).color(MUTED).size(13.0));
            ui.selectable_value(&mut b.font, crate::brand::BrandFont::Inter, "Inter");
            ui.selectable_value(&mut b.font, crate::brand::BrandFont::Krona, egui::RichText::new("KRONA").font(display(12.0)));
        });
        let has_logo = b.logo.is_some();
        if ui.add_enabled(has_logo, egui::Button::new(l("Add the logo to the layout", "Agregar el logo al layout"))).on_disabled_hover_text(l("Choose a logo first", "Elige un logo primero")).clicked() {
            app.add_logo();
        }
    });
}

/// Shortcuts: click a combo, press the new keys (Esc cancels, Backspace
/// turns it off). The numbered ones pick a modifier set.
fn shortcuts_section(app: &mut App, ui: &mut Ui, rect: Rect) {
    use super::hotkeys::{Action, GROUP_MODS, from_egui, used_by};
    // A combo being typed.
    if let Some(a) = app.capturing {
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, mods)) = pressed {
            match key {
                egui::Key::Escape => app.capturing = None,
                egui::Key::Backspace | egui::Key::Delete if mods.is_none() => {
                    a.slot(&mut app.settings.hotkeys).clear();
                    app.capturing = None;
                }
                _ => {
                    if let Some(combo) = from_egui(key, mods) {
                        match used_by(&app.settings.hotkeys, &combo, a) {
                            Some(other) => app.toast(t!("{} is already used by “{}”", "{} ya lo usa “{}”", combo, other.label())),
                            None => {
                                *a.slot(&mut app.settings.hotkeys) = combo;
                                app.capturing = None;
                            }
                        }
                    }
                }
            }
        }
    }
    section(ui, rect, l("Shortcuts", "Atajos"), |ui| {
        ui.label(
            egui::RichText::new(l(
                "They work in any app. Click one and press the new keys: Esc cancels, Backspace turns it off.",
                "Funcionan en cualquier app. Haz clic en uno y presiona las teclas nuevas: Esc cancela, Retroceso lo apaga.",
            ))
            .color(FAINT)
            .size(12.0),
        );
        ui.add_space(8.0);
        let half = (rect.width() - 44.0 - 24.0) / 2.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(half);
                for a in Action::SINGLE {
                    shortcut_row(app, ui, a, half);
                }
            });
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.set_width(half);
                for (label, which, slot) in [
                    (l("Templates 1–9", "Plantillas 1–9"), Action::Template(0), 0),
                    (l("Media Board tiles 1–9", "Media Board, tarjetas 1–9"), Action::Media(0), 1),
                ] {
                    ui.horizontal(|ui| {
                        ui.set_min_height(34.0);
                        ui.label(egui::RichText::new(label).color(MUTED).size(13.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let cur = which.slot(&mut app.settings.hotkeys);
                            let mut v = cur.clone();
                            let opts: Vec<(String, String)> =
                                GROUP_MODS.iter().map(|m| (m.to_string(), if m.is_empty() { l("Off", "Apagado").to_string() } else { format!("{m} + 1…9") })).collect();
                            if combo(ui, &format!("group{slot}"), &mut v, &opts, 190.0) {
                                *which.slot(&mut app.settings.hotkeys) = v;
                            }
                        });
                    });
                    let taken: Vec<String> = app.hotkeys.taken.iter().filter(|t| matches!((t, slot), (Action::Template(_), 0) | (Action::Media(_), 1))).map(|t| t.combo(&app.settings.hotkeys)).collect();
                    if !taken.is_empty() {
                        ui.label(egui::RichText::new(t!("Taken by another app: {}", "Ocupados por otra app: {}", taken.join(", "))).color(ORANGE).size(11.5));
                    }
                }
                ui.add_space(14.0);
                if ui.button(l("Restore the defaults", "Restaurar los predeterminados")).clicked() {
                    app.settings.hotkeys = Default::default();
                    app.capturing = None;
                }
            });
        });
    });
}

fn shortcut_row(app: &mut App, ui: &mut Ui, a: super::hotkeys::Action, width: f32) {
    ui.horizontal(|ui| {
        ui.set_width(width);
        ui.set_min_height(34.0);
        ui.label(egui::RichText::new(a.label()).color(MUTED).size(13.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let capturing = app.capturing == Some(a);
            let combo = a.combo(&app.settings.hotkeys);
            let label = if capturing {
                l("Press the keys…", "Presiona las teclas…").to_string()
            } else if combo.is_empty() {
                l("Off", "Apagado").to_string()
            } else {
                combo
            };
            let b = egui::Button::new(egui::RichText::new(label).font(semibold(13.0)).color(if capturing { ORANGE } else { TEXT })).min_size(vec2(190.0, 28.0));
            let b = if capturing { b.fill(Color32::from_rgb(40, 24, 10)).stroke(Stroke::new(1.5, ORANGE)) } else { b };
            if ui.add(b).clicked() {
                app.capturing = if capturing { None } else { Some(a) };
            }
            if app.hotkeys.taken.contains(&a) {
                ui.label(egui::RichText::new(l("taken by another app", "ocupado por otra app")).color(ORANGE).size(11.5));
            }
        });
    });
}

/// "Save scene": a name, and optionally a group (one of the templates'
/// groups, or a new one).
fn save_dialog(app: &mut App, ctx: &egui::Context) {
    let Some(d) = &mut app.save_dialog else { return };
    let mut groups: Vec<String> = app.templates.iter().map(|t| t.group.clone()).filter(|g| !g.is_empty()).collect();
    groups.sort();
    groups.dedup();
    let (mut save, mut cancel) = (false, false);
    egui::Modal::new(Id::new("save-scene")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(egui::RichText::new(l("Save scene", "Guardar escena")).font(display(18.0)).color(TEXT));
        ui.add_space(10.0);
        ui.label(egui::RichText::new(l("Name", "Nombre")).color(MUTED).size(13.0));
        let r = ui.add(egui::TextEdit::singleline(&mut d.name).hint_text(l("My scene", "Mi escena")).desired_width(f32::INFINITY));
        r.request_focus();
        ui.add_space(8.0);
        ui.label(egui::RichText::new(l("Group (optional)", "Grupo (opcional)")).color(MUTED).size(13.0));
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(d.group.is_empty(), l("None", "Ninguno")).clicked() {
                d.group.clear();
            }
            for g in &groups {
                if ui.selectable_label(&d.group == g, g).clicked() {
                    d.group = g.clone();
                }
            }
        });
        ui.add(egui::TextEdit::singleline(&mut d.group).hint_text(l("…or type a new group", "…o escribe un grupo nuevo")).desired_width(f32::INFINITY));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ok = !d.name.trim().is_empty();
            if ui.add_enabled(ok, egui::Button::new(egui::RichText::new(l("Save", "Guardar")).strong().color(Color32::BLACK)).fill(Color32::WHITE).min_size(vec2(110.0, 30.0))).clicked()
                || (ok && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                save = true;
            }
            if ui.add(egui::Button::new(l("Cancel", "Cancelar")).min_size(vec2(90.0, 30.0))).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
            }
        });
    });
    if save && let Some(d) = app.save_dialog.take() {
        app.save_template(&d.name, &d.group);
    } else if cancel {
        app.save_dialog = None;
    }
}

fn toast(app: &mut App, ui: &mut Ui, full: Rect) {
    let Some((msg, at)) = &app.toast else { return };
    if at.elapsed() > Duration::from_secs(4) {
        app.toast = None;
        return;
    }
    let p = ui.painter();
    let w = 40.0 + msg.chars().count() as f32 * 7.2;
    let r = Rect::from_center_size(pos2(full.center().x, full.bottom() - 44.0), vec2(w, 40.0));
    p.rect_filled(r, CornerRadius::same(20), Color32::from_rgb(28, 28, 31));
    p.rect_stroke(r, CornerRadius::same(20), Stroke::new(1.0, Color32::from_rgba_premultiplied(70, 70, 76, 90)), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, msg, medium(13.0), TEXT);
}
