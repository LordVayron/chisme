//! The "Media Board" page: sounds, clips and images as tiles. A click (or
//! Ctrl+Alt+1…9) fires one into the recording and the streams; firing it
//! again stops it (or hides the image). Files can be dropped on the window.

use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::App;
use crate::i18n::l;
use crate::t;
use super::theme::{self, *};
use crate::engine::{Cmd, MediaCmd};
use crate::media::{self, Item, Kind};

impl App {
    /// Fire Media Board item `i` (again: stop it).
    pub fn media_fire(&mut self, i: usize) {
        if let (Some(item), Some(e)) = (self.settings.media.get(i), &self.engine) {
            e.send(Cmd::Media(MediaCmd::Play(item.clone())));
        }
    }

    pub fn media_stop_all(&mut self) {
        if let Some(e) = &self.engine {
            e.send(Cmd::Media(MediaCmd::StopAll));
        }
    }

    /// Add files (skips ones already there and types it can't play).
    pub fn media_add(&mut self, paths: impl IntoIterator<Item = std::path::PathBuf>) {
        let mut added = 0;
        let mut skipped = 0;
        for p in paths {
            match media::item(&p) {
                Some(it) if !self.settings.media.iter().any(|m| m.path == it.path) => {
                    self.settings.media.push(it);
                    added += 1;
                }
                Some(_) => {}
                None => skipped += 1,
            }
        }
        if added > 0 {
            let _ = self.settings.save();
        }
        if skipped > 0 {
            self.toast(t!("{} unsupported file(s): use MP3, WAV, M4A, MP4, MOV, PNG or JPG", "{} archivo(s) no compatibles: usa MP3, WAV, M4A, MP4, MOV, PNG o JPG", skipped));
        }
    }

    fn media_pick(&mut self) {
        let files = rfd::FileDialog::new().set_title(l("Add to the Media Board", "Agregar al Media Board")).add_filter(l("Sounds, clips and images", "Sonidos, clips e imágenes"), &media::EXTENSIONS).pick_files();
        if let Some(files) = files {
            self.media_add(files);
        }
    }

    fn media_remove(&mut self, i: usize) {
        if i < self.settings.media.len() {
            let it = self.settings.media.remove(i);
            self.settings.media_strip.retain(|p| *p != it.path);
            if let Some(e) = &self.engine {
                e.send(Cmd::Media(MediaCmd::Stop(it.path.clone())));
            }
            self.thumbs.remove(&it.path);
            let _ = self.settings.save();
        }
    }

    /// An image item's thumbnail (loaded once).
    fn thumb(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
        self.thumbs
            .entry(path.to_string())
            .or_insert_with(|| {
                let img = image::open(path).ok()?.thumbnail(320, 320).to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                Some(ctx.load_texture(format!("thumb:{path}"), ci, egui::TextureOptions::LINEAR))
            })
            .clone()
    }
}

fn kind_color(k: Kind) -> Color32 {
    match k {
        Kind::Sound => RAINBOW[2],
        Kind::Video => RAINBOW[4],
        Kind::Image => RAINBOW[1],
    }
}

/// Sound: waveform bars. Clip: a film frame with a play triangle.
fn kind_art(p: &Painter, r: Rect, k: Kind, playing: bool, t: f32) {
    let c = r.center();
    let col = kind_color(k);
    match k {
        Kind::Sound => {
            let n = 13;
            let w = r.width() * 0.5 / n as f32;
            for i in 0..n {
                let x = c.x - r.width() * 0.25 + i as f32 * w * 1.0 + w / 2.0;
                let base = [0.3, 0.55, 0.8, 0.5, 1.0, 0.7, 0.9, 0.6, 1.0, 0.45, 0.75, 0.5, 0.3][i];
                let wob = if playing { 0.65 + 0.35 * ((t * 9.0 + i as f32 * 1.7).sin() * 0.5 + 0.5) } else { 1.0 };
                let h = r.height() * 0.42 * base * wob;
                p.rect_filled(Rect::from_center_size(pos2(x, c.y), vec2(w * 0.55, h)), CornerRadius::same(2), col);
            }
        }
        Kind::Video => {
            let f = Rect::from_center_size(c, vec2(r.width() * 0.4, r.height() * 0.5));
            p.rect_stroke(f, CornerRadius::same(6), Stroke::new(2.0, col), StrokeKind::Middle);
            for i in 0..4 {
                let y = f.top() + 6.0 + i as f32 * (f.height() - 12.0) / 3.0;
                p.rect_filled(Rect::from_center_size(pos2(f.left() + 6.0, y), vec2(4.0, 4.0)), CornerRadius::same(1), col);
                p.rect_filled(Rect::from_center_size(pos2(f.right() - 6.0, y), vec2(4.0, 4.0)), CornerRadius::same(1), col);
            }
            let s = f.height() * 0.22;
            p.add(Shape::convex_polygon(vec![c + vec2(-s * 0.8, -s), c + vec2(s, 0.0), c + vec2(-s * 0.8, s)], col, Stroke::NONE));
        }
        Kind::Image => {
            let f = Rect::from_center_size(c, vec2(r.width() * 0.36, r.height() * 0.44));
            p.rect_stroke(f, CornerRadius::same(6), Stroke::new(2.0, col), StrokeKind::Middle);
            p.circle_filled(f.left_top() + vec2(f.width() * 0.3, f.height() * 0.3), 4.0, col);
            p.add(Shape::line(vec![f.left_bottom() - vec2(-4.0, 6.0), f.center() + vec2(-4.0, 6.0), f.right_bottom() - vec2(6.0, 10.0)], Stroke::new(2.0, col)));
        }
    }
}

/// The sidebar icon: a 2x2 grid of tiles, one filled.
pub fn icon(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.8, color);
    for (dx, dy, fill) in [(-5.0, -5.0, true), (5.0, -5.0, false), (-5.0, 5.0, false), (5.0, 5.0, false)] {
        let r = Rect::from_center_size(c + vec2(dx, dy), vec2(8.0, 8.0));
        if fill {
            p.rect_filled(r, CornerRadius::same(2), color);
        } else {
            p.rect_stroke(r, CornerRadius::same(2), s, StrokeKind::Middle);
        }
    }
}

pub fn page(app: &mut App, ui: &mut Ui, area: Rect) {
    let ctx = ui.ctx().clone();
    // Files dropped on the window.
    let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
    if !dropped.is_empty() {
        app.media_add(dropped);
    }
    let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());

    // Top bar.
    let bar = Rect::from_min_size(area.min, vec2(area.width(), 64.0));
    theme::card(ui.painter(), bar);
    let p = ui.painter().clone();
    p.text(bar.left_center() + vec2(22.0, -8.0), Align2::LEFT_CENTER, l("Fire sounds, clips and images while you record or stream", "Lanza sonidos, clips e imágenes mientras grabas o transmites"), medium(14.0), TEXT);
    p.text(bar.left_center() + vec2(22.0, 11.0), Align2::LEFT_CENTER, if app.settings.hotkeys.media.is_empty() {
            l("Click to fire; again to stop. You hear it, and it lands in the recording.", "Clic para lanzar; otra vez lo detiene. Se oye en tus bocinas y queda en la grabación.").to_string()
        } else {
            t!("Click or {}+1…9; again to stop. You hear it, and it lands in the recording.", "Clic o {}+1…9; otra vez lo detiene. Se oye en tus bocinas y queda en la grabación.", app.settings.hotkeys.media)
        }, body(12.0), FAINT);
    let add = Rect::from_min_size(pos2(bar.right() - 20.0 - 170.0, bar.center().y - 19.0), vec2(170.0, 38.0));
    if pill(ui, add, "media-add", l("Add files", "Agregar archivos"), true) {
        app.media_pick();
    }
    let any = !app.status.media.is_empty() || app.status.media_image.is_some();
    let stop = Rect::from_min_size(pos2(add.left() - 10.0 - 130.0, add.top()), vec2(130.0, 38.0));
    if any && pill(ui, stop, "media-stop", l("Stop all", "Detener todo"), false) {
        app.media_stop_all();
    }

    // Tiles.
    let grid = Rect::from_min_max(pos2(area.left(), bar.bottom() + 16.0), area.max);
    let cols = ((grid.width() + 16.0) / 236.0).floor().max(2.0) as usize;
    let gap = 16.0;
    let cw = (grid.width() - gap * (cols - 1) as f32) / cols as f32;
    let art_h = (cw * 0.56).round();
    let ch = art_h + 96.0;
    let items: Vec<Item> = app.settings.media.clone();
    let now = ctx.input(|i| i.time) as f32;
    let mut fire = None;
    let mut remove = None;
    let mut move_to: Option<(usize, String)> = None;
    let mut save = false;
    ui.scope_builder(UiBuilder::new().max_rect(grid), |ui| {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let groups: Vec<&str> = items.iter().map(|it| it.group.as_str()).collect();
            let all_groups = super::groups::order(&groups);
            let origin = ui.cursor().min;
            let layout = super::groups::grid(&groups, origin, cols, vec2(cw, ch), gap);
            let (all, _) = ui.allocate_exact_size(vec2(grid.width(), layout.height.max(ch)), Sense::hover());
            super::groups::draw_headers(ui.painter(), &layout);
            if items.is_empty() {
                let p = ui.painter();
                let r = Rect::from_min_size(all.min, vec2(all.width(), 220.0));
                p.rect_stroke(r, CornerRadius::same(R_CARD), Stroke::new(1.5, Color32::from_white_alpha(if hovering_files { 120 } else { 30 })), StrokeKind::Inside);
                p.text(r.center() - vec2(0.0, 14.0), Align2::CENTER_CENTER, l("Drop sounds, clips or images here", "Arrastra aquí sonidos, clips o imágenes"), medium(16.0), TEXT);
                p.text(r.center() + vec2(0.0, 14.0), Align2::CENTER_CENTER, "MP3, WAV, M4A · MP4, MOV · PNG, JPG", body(12.5), MUTED);
            }
            for (i, it) in items.iter().enumerate() {
                let card = layout.cells[i];
                let playing = app.status.media.iter().find(|m| m.0 == it.path).map(|m| m.1);
                let shown = it.kind == Kind::Image && app.status.media_image.as_deref() == Some(it.path.as_str());
                let active = playing.is_some() || shown;
                let p = ui.painter().clone();
                theme::card(&p, card);
                if active {
                    p.rect_stroke(card, CornerRadius::same(R_CARD), Stroke::new(1.5, ORANGE), StrokeKind::Inside);
                }
                // The art: click to fire.
                let art = Rect::from_min_size(card.min + vec2(12.0, 12.0), vec2(cw - 24.0, art_h));
                p.rect_filled(art, CornerRadius::same(14), Color32::from_rgb(12, 12, 14));
                let thumb = if it.kind == Kind::Image { app.thumb(&ctx, &it.path) } else { None };
                match thumb {
                    Some(t) => {
                        let s = t.size_vec2();
                        let k = (art.width() / s.x).min(art.height() / s.y);
                        let r = Rect::from_center_size(art.center(), s * k);
                        egui::Image::from_texture(&t).corner_radius(CornerRadius::same(10)).paint_at(ui, r);
                    }
                    None => kind_art(&p, art, it.kind, playing.is_some(), now),
                }
                let tip = if active {
                    l("Stop", "Detener")
                } else if it.kind == Kind::Image {
                    l("Show on screen", "Mostrar en pantalla")
                } else {
                    l("Play", "Reproducir")
                };
                let resp = ui.interact(art, Id::new(("media-art", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip);
                if resp.hovered() {
                    p.rect_stroke(art, CornerRadius::same(14), Stroke::new(1.5, Color32::from_white_alpha(140)), StrokeKind::Outside);
                }
                if resp.clicked() {
                    fire = Some(i);
                }
                {
                    let pr = Rect::from_min_size(pos2(art.right() - 40.0, art.top() + 8.0), vec2(32.0, 32.0));
                    let pinned = app.settings.media_strip.contains(&it.path);
                    let presp = ui.interact(pr, Id::new(("media-pin", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(if pinned {
                        l("On the Studio page. Click to take it off.", "En la página Estudio. Clic para quitarlo.")
                    } else {
                        l("Pin to the Studio page", "Fijar en la página Estudio")
                    });
                    p.circle_filled(pr.center(), 16.0, if pinned { Color32::from_rgba_premultiplied(90, 44, 0, 200) } else if presp.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 200) } else { Color32::from_rgba_premultiplied(0, 0, 0, 170) });
                    super::templates_ui::pin_icon(&p, pr.center(), if pinned { ORANGE } else { TEXT }, pinned);
                    if presp.clicked() {
                        if pinned {
                            app.settings.media_strip.retain(|x| *x != it.path);
                        } else {
                            app.settings.media_strip.push(it.path.clone());
                        }
                        save = true;
                    }
                }
                resp.context_menu(|ui| {
                    if let Some(g) = super::groups::menu(ui, &it.group, &all_groups, &mut app.group_input) {
                        move_to = Some((i, g));
                    }
                });
                // State: progress for sounds and clips, a chip for images.
                if let Some(prog) = playing {
                    let track = Rect::from_min_max(pos2(art.left() + 12.0, art.bottom() - 10.0), pos2(art.right() - 12.0, art.bottom() - 7.0));
                    p.rect_filled(track, CornerRadius::same(2), Color32::from_white_alpha(30));
                    if prog > 0.0 {
                        gradient_bar(&p, Rect::from_min_max(track.min, pos2(egui::lerp(track.left()..=track.right(), prog), track.bottom())), &RAINBOW);
                    }
                }
                if active {
                    let chip = Rect::from_min_size(art.min + vec2(10.0, 10.0), vec2(if shown { 100.0 } else { 80.0 }, 22.0));
                    p.rect_filled(chip, CornerRadius::same(11), Color32::from_rgba_premultiplied(0, 0, 0, 170));
                    p.circle_filled(chip.left_center() + vec2(11.0, 0.0), 3.5, RED);
                    p.text(chip.left_center() + vec2(20.0, 0.0), Align2::LEFT_CENTER, if shown { l("ON SCREEN", "EN PANTALLA") } else { l("PLAYING", "SONANDO") }, semibold(10.5), TEXT);
                }
                // Name, kind and hotkey.
                let ty = art.bottom() + 18.0;
                p.text(pos2(card.left() + 16.0, ty), Align2::LEFT_CENTER, truncate(&it.name, (cw / 8.5) as usize), medium(14.0), TEXT);
                let key = if i < 9 { super::hotkeys::Action::Media(i).combo(&app.settings.hotkeys) } else { String::new() };
                p.text(pos2(card.right() - 16.0, ty), Align2::RIGHT_CENTER, key, body(11.5), FAINT);
                p.circle_filled(pos2(card.left() + 20.0, ty + 22.0), 4.0, kind_color(it.kind));
                p.text(pos2(card.left() + 30.0, ty + 22.0), Align2::LEFT_CENTER, it.kind.label(), body(11.5), MUTED);
                // Volume (sounds and clips) and delete.
                let row = Rect::from_min_max(pos2(card.left() + 14.0, ty + 36.0), pos2(card.right() - 14.0, card.bottom() - 10.0));
                let del = Rect::from_min_size(pos2(row.right() - 30.0, row.center().y - 15.0), vec2(30.0, 30.0));
                if small(ui, del, ("media-del", i), "🗑") {
                    remove = Some(i);
                }
                if it.kind != Kind::Image {
                    // Loop on / off.
                    let lr = Rect::from_min_size(pos2(del.left() - 38.0, del.top()), vec2(30.0, 30.0));
                    let on = it.looped;
                    let resp = ui.interact(lr, Id::new(("media-loop", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(if on {
                        l("Loop: on (plays again until you stop it)", "Repetir: encendido (se repite hasta que lo detengas)")
                    } else {
                        l("Loop: off", "Repetir: apagado")
                    });
                    let fill = if on { Color32::from_rgba_premultiplied(90, 44, 0, 150) } else if resp.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 90) } else { Color32::from_rgba_premultiplied(40, 40, 44, 80) };
                    ui.painter().rect_filled(lr, CornerRadius::same(15), fill);
                    ui.painter().text(lr.center(), Align2::CENTER_CENTER, "↻", semibold(15.0), if on { ORANGE } else { TEXT });
                    if resp.clicked() {
                        app.settings.media[i].looped = !on;
                        save = true;
                    }
                    let slot = Rect::from_min_max(row.min, pos2(lr.left() - 8.0, row.max.y));
                    ui.scope_builder(UiBuilder::new().max_rect(slot), |ui| {
                        ui.spacing_mut().slider_width = (slot.width() - 74.0).max(40.0);
                        ui.horizontal_centered(|ui| {
                            let mut v = app.settings.media[i].volume * 100.0;
                            let r = ui.add(egui::Slider::new(&mut v, 0.0..=200.0).suffix(" %").max_decimals(0).show_value(true)).on_hover_text(l("Volume", "Volumen"));
                            if r.changed() {
                                app.settings.media[i].volume = v / 100.0;
                            }
                            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                                save = true;
                            }
                        });
                    });
                }
            }
        });
    });
    if let Some(i) = fire {
        app.media_fire(i);
    }
    if let Some((i, g)) = move_to {
        app.settings.media[i].group = g;
        let _ = app.settings.save();
    }
    if let Some(i) = remove {
        app.media_remove(i);
    }
    if save {
        // Saves, and tells the engine (loop applies to what is playing).
        app.settings_changed();
    }
    if !app.status.media.is_empty() {
        ctx.request_repaint();
    }
}

/// Sound pad colors (the Studio's sampler).
pub const PAD_COLORS: [Color32; 8] = [
    Color32::from_rgb(255, 122, 26),
    Color32::from_rgb(239, 68, 68),
    Color32::from_rgb(236, 72, 153),
    Color32::from_rgb(168, 85, 247),
    Color32::from_rgb(59, 130, 246),
    Color32::from_rgb(6, 182, 212),
    Color32::from_rgb(34, 197, 94),
    Color32::from_rgb(234, 179, 8),
];

fn pad_color(it: &Item, place: usize) -> Color32 {
    PAD_COLORS[it.color.map(|c| c as usize).unwrap_or(place) % PAD_COLORS.len()]
}

/// The Studio's Media Board: the pinned items in your order. Sounds are
/// colored pads; clips and images show their art. "Edit" reorders them
/// (drag) and picks pad colors.
pub fn strip(app: &mut App, ui: &mut Ui, card: Rect) {
    let p = ui.painter().clone();
    theme::card(&p, card);
    p.text(card.min + vec2(22.0, 16.0), Align2::LEFT_TOP, "Media Board", medium(14.0), TEXT);
    let see = Rect::from_min_size(pos2(card.right() - 22.0 - 140.0, card.top() + 11.0), vec2(140.0, 28.0));
    if pill(ui, see, "media-strip-all", l("Open Media Board", "Abrir Media Board"), false) {
        app.page = super::Page::Media;
        app.strip_edit = false;
    }
    let items = app.settings.media.clone();
    // Pinned, in order (items deleted meanwhile are skipped).
    let pins: Vec<usize> = app.settings.media_strip.iter().filter_map(|path| items.iter().position(|it| it.path == *path)).collect();
    if !pins.is_empty() {
        let edit = Rect::from_min_size(pos2(see.left() - 10.0 - 76.0, see.top()), vec2(76.0, 28.0));
        if pill(ui, edit, "media-strip-edit", if app.strip_edit { l("Done", "Listo") } else { l("Edit", "Editar") }, app.strip_edit) {
            app.strip_edit = !app.strip_edit;
            app.strip_drag = None;
        }
        if app.strip_edit {
            p.text(pos2(edit.left() - 12.0, edit.center().y), Align2::RIGHT_CENTER, l("Drag to reorder · pick a color for each sound", "Arrastra para ordenar · elige un color para cada sonido"), body(11.5), FAINT);
        }
    }
    let row = Rect::from_min_max(pos2(card.left() + 16.0, card.top() + 48.0), pos2(card.right() - 16.0, card.bottom() - 12.0));
    if pins.is_empty() {
        let msg = if items.is_empty() {
            l("Add sounds, clips and images on the Media Board page", "Agrega sonidos, clips e imágenes en la página Media Board")
        } else {
            l("Pin sounds and clips on the Media Board page (the pin on each tile) to have them here", "Fija sonidos y clips en la página Media Board (el pin de cada tarjeta) para tenerlos aquí")
        };
        p.text(row.center(), Align2::CENTER_CENTER, msg, body(12.5), MUTED);
        return;
    }
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time) as f32;
    let th = row.height();
    let tw_clip = (th * 1.25).min(150.0);
    let gap = 10.0;
    // Each tile's width: sounds are square pads.
    let widths: Vec<f32> = pins.iter().map(|&i| if items[i].kind == Kind::Sound { th } else { tw_clip }).collect();
    let total: f32 = widths.iter().sum::<f32>() + gap * widths.len() as f32;
    let editing = app.strip_edit;
    let mut fire = None;
    let mut set_color = None;
    let mut unpin = None;
    let mut moved: Option<(usize, usize)> = None;
    ui.scope_builder(UiBuilder::new().max_rect(row), |ui| {
        egui::ScrollArea::horizontal().auto_shrink([false; 2]).show(ui, |ui| {
            let (all, _) = ui.allocate_exact_size(vec2(total, th), Sense::hover());
            let mut x = all.left();
            let mut lefts = vec![];
            for (k, &i) in pins.iter().enumerate() {
                let it = &items[i];
                let r = Rect::from_min_size(pos2(x, all.top()), vec2(widths[k], th));
                lefts.push(x);
                x += widths[k] + gap;
                let prog = app.status.media.iter().find(|m| m.0 == it.path).map(|m| m.1);
                let playing = prog.is_some();
                let shown = it.kind == Kind::Image && app.status.media_image.as_deref() == Some(it.path.as_str());
                let dragging = editing && app.strip_drag == Some(k);
                let p = ui.painter().clone();
                let key = if i < 9 { super::hotkeys::Action::Media(i).combo(&app.settings.hotkeys) } else { String::new() };
                if it.kind == Kind::Sound {
                    // A sampler pad: dim at rest, lit while it plays.
                    let c = pad_color(it, k);
                    let fill = if playing { c } else { c.gamma_multiply(0.55) };
                    p.rect_filled(r, CornerRadius::same(14), fill);
                    p.rect_filled(Rect::from_min_max(r.min, pos2(r.right(), r.top() + r.height() * 0.45)), CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 }, Color32::from_white_alpha(if playing { 34 } else { 14 }));
                    if playing {
                        p.rect_stroke(r.expand(2.0), CornerRadius::same(16), Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
                    }
                    let name_y = if editing { r.center().y - 10.0 } else { r.center().y };
                    p.text(pos2(r.center().x, name_y), Align2::CENTER_CENTER, truncate(&it.name, (r.width() / 7.5) as usize), semibold(12.5), Color32::WHITE);
                    if !key.is_empty() && !editing {
                        p.text(r.left_top() + vec2(8.0, 7.0), Align2::LEFT_TOP, key.replace("Ctrl+Alt+", ""), body(10.0), Color32::from_white_alpha(170));
                    }
                    if let Some(f) = prog {
                        let track = Rect::from_min_max(pos2(r.left() + 10.0, r.bottom() - 9.0), pos2(r.right() - 10.0, r.bottom() - 6.0));
                        p.rect_filled(track, CornerRadius::same(2), Color32::from_black_alpha(90));
                        p.rect_filled(Rect::from_min_max(track.min, pos2(egui::lerp(track.left()..=track.right(), f), track.bottom())), CornerRadius::same(2), Color32::WHITE);
                    }
                    if it.looped {
                        p.text(r.right_top() + vec2(-8.0, 6.0), Align2::RIGHT_TOP, "↻", semibold(12.0), Color32::from_white_alpha(200));
                    }
                } else {
                    p.rect_filled(r, CornerRadius::same(12), Color32::from_rgb(14, 14, 16));
                    if playing || shown {
                        p.rect_stroke(r, CornerRadius::same(12), Stroke::new(1.5, ORANGE), StrokeKind::Inside);
                    }
                    let art = Rect::from_min_max(r.min + vec2(6.0, 6.0), pos2(r.right() - 6.0, r.bottom() - 22.0));
                    match if it.kind == Kind::Image { app.thumb(&ctx, &it.path) } else { None } {
                        Some(t) => {
                            let s = t.size_vec2();
                            let k = (art.width() / s.x).min(art.height() / s.y);
                            egui::Image::from_texture(&t).corner_radius(CornerRadius::same(8)).paint_at(ui, Rect::from_center_size(art.center(), s * k));
                        }
                        None => kind_art(&p, art, it.kind, playing, now),
                    }
                    p.text(pos2(r.left() + 8.0, r.bottom() - 11.0), Align2::LEFT_CENTER, truncate(&it.name, (r.width() / 7.5) as usize), medium(11.5), TEXT);
                }
                let sense = if editing { Sense::click_and_drag() } else { Sense::click() };
                let resp = ui.interact(r, Id::new(("strip", i)), sense).on_hover_cursor(if editing { CursorIcon::Grab } else { CursorIcon::PointingHand });
                let resp = if editing { resp } else { resp.on_hover_text(if key.is_empty() { it.name.clone() } else { format!("{}  ({key})", it.name) }) };
                if resp.hovered() && !dragging {
                    p.rect_stroke(r, CornerRadius::same(12), Stroke::new(1.0, Color32::from_white_alpha(120)), StrokeKind::Inside);
                }
                if editing {
                    if dragging {
                        p.rect_filled(r, CornerRadius::same(12), Color32::from_black_alpha(120));
                    }
                    // Unpin.
                    let xr = Rect::from_center_size(r.right_top() + vec2(-12.0, 12.0), vec2(20.0, 20.0));
                    let xresp = ui.interact(xr, Id::new(("strip-unpin", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(l("Take off the Studio", "Quitar del Estudio"));
                    p.circle_filled(xr.center(), 10.0, if xresp.hovered() { RED } else { Color32::from_black_alpha(190) });
                    p.text(xr.center(), Align2::CENTER_CENTER, "×", semibold(13.0), Color32::WHITE);
                    if xresp.clicked() {
                        unpin = Some(it.path.clone());
                    }
                    // Pad colors.
                    if it.kind == Kind::Sound {
                        let n = PAD_COLORS.len();
                        let d = ((r.width() - 16.0) / n as f32).min(14.0);
                        let y = r.center().y + 14.0;
                        let x0 = r.center().x - d * n as f32 / 2.0;
                        // Two rows of four when the pad is small.
                        let (cols, dd) = if d < 9.0 { (4, (r.width() - 20.0) / 4.0) } else { (n, d) };
                        for (ci, c) in PAD_COLORS.iter().enumerate() {
                            let (cx, cy) = if cols == n { (x0 + dd * (ci as f32 + 0.5), y) } else { (r.center().x - dd * 2.0 + dd * ((ci % 4) as f32 + 0.5), y - 6.0 + 14.0 * (ci / 4) as f32) };
                            let cr = Rect::from_center_size(pos2(cx, cy), vec2(dd, dd));
                            let cresp = ui.interact(cr, Id::new(("pad-color", i, ci)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                            let sel = pad_color(it, k) == *c;
                            p.circle_filled(cr.center(), dd * 0.36, *c);
                            p.circle_stroke(cr.center(), dd * 0.36, Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { Color32::WHITE } else { Color32::from_black_alpha(140) }));
                            if cresp.clicked() {
                                set_color = Some((i, ci as u8));
                            }
                        }
                    }
                    if resp.drag_started() {
                        app.strip_drag = Some(k);
                    }
                } else if resp.clicked() {
                    fire = Some(i);
                }
            }
            // Dragging: a line where it lands; it moves on release.
            if let (true, Some(from)) = (editing, app.strip_drag) {
                if let Some(ptr) = ui.ctx().pointer_interact_pos() {
                    let mut to = lefts.iter().zip(&widths).filter(|(l, w)| ptr.x > **l + **w / 2.0).count();
                    to = to.min(pins.len());
                    let lx = if to < lefts.len() { lefts[to] - gap / 2.0 } else { x - gap / 2.0 };
                    ui.painter().line_segment([pos2(lx, all.top()), pos2(lx, all.bottom())], Stroke::new(3.0, ORANGE));
                    if ui.input(|i| i.pointer.any_released()) {
                        moved = Some((from, to));
                    }
                }
                if !ui.input(|i| i.pointer.any_down()) && moved.is_none() {
                    app.strip_drag = None;
                }
            }
        });
    });
    let mut save = false;
    if let Some((from, to)) = moved {
        app.strip_drag = None;
        // Positions are in the pinned list, which may skip deleted paths.
        let mut list: Vec<String> = pins.iter().map(|&i| items[i].path.clone()).collect();
        let x = list.remove(from);
        list.insert(if to > from { to - 1 } else { to }, x);
        app.settings.media_strip = list;
        save = true;
    }
    if let Some((i, c)) = set_color {
        app.settings.media[i].color = Some(c);
        save = true;
    }
    if let Some(path) = unpin {
        app.settings.media_strip.retain(|p| *p != path);
        save = true;
    }
    if save {
        app.settings_changed();
    }
    if let Some(i) = fire {
        app.media_fire(i);
    }
    if !app.status.media.is_empty() || app.strip_drag.is_some() {
        ctx.request_repaint();
    }
}

pub(super) fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>()) }
}

pub(super) fn pill(ui: &mut Ui, r: Rect, id: &str, label: &str, primary: bool) -> bool {
    let resp = ui.interact(r, Id::new(id), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let fill = if primary {
        if resp.hovered() { Color32::from_gray(225) } else { Color32::WHITE }
    } else if resp.hovered() {
        Color32::from_rgba_premultiplied(60, 60, 66, 120)
    } else {
        Color32::from_rgba_premultiplied(40, 40, 44, 100)
    };
    ui.painter().rect_filled(r, CornerRadius::same((r.height() / 2.0) as u8), fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, semibold(13.5), if primary { Color32::BLACK } else { TEXT });
    resp.clicked()
}

fn small(ui: &mut Ui, r: Rect, id: (&str, usize), label: &str) -> bool {
    let resp = ui.interact(r, Id::new(id), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(l("Remove", "Quitar"));
    let fill = if resp.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 90) } else { Color32::from_rgba_premultiplied(40, 40, 44, 80) };
    ui.painter().rect_filled(r, CornerRadius::same(15), fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, semibold(12.5), TEXT);
    resp.clicked()
}
