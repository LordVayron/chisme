//! The "Guests" page: your LiveKit project, invite links, and the green
//! room: everyone who joined, with their camera and mic, ready to bring on
//! stage. Guest scenes (Interview, Panel…) place the people on stage.

use eframe::egui::{self, Align2, Color32, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::App;
use super::media_ui::{pill, truncate};
use super::theme::{self, *};
use crate::guests::{self, Cmd};
use crate::i18n::l;
use crate::t;

/// The sidebar icon: two heads.
pub fn icon(p: &Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.8, color);
    p.circle_stroke(c + vec2(-4.5, -4.0), 3.6, s);
    p.circle_filled(c + vec2(5.0, -3.0), 3.2, color);
    p.add(egui::Shape::line(vec![c + vec2(-11.0, 9.0), c + vec2(-9.5, 3.0), c + vec2(0.5, 3.0), c + vec2(2.0, 9.0)], s));
    p.rect_filled(Rect::from_min_max(c + vec2(1.0, 3.5), c + vec2(10.5, 9.0)), CornerRadius::same(3), color);
}

impl App {
    pub fn guests_open(&mut self) {
        if self.settings.guests.room.is_empty() {
            self.settings.guests.room = guests::new_room_name();
            let _ = self.settings.save();
        }
        match guests::start(self.settings.guests.clone(), self.guest_secret.clone()) {
            Ok(r) => {
                self.guest_room = Some(r);
                self.toast(l("Green room open", "Sala de espera abierta"));
            }
            Err(e) => self.toast(t!("Couldn't open the room: {:#}", "No pude abrir la sala: {:#}", e)),
        }
    }

    /// One click: a new link for one guest, copied. The green room opens
    /// too, so they can wait there.
    pub fn guests_invite(&mut self, ctx: &egui::Context) {
        if self.settings.guests.room.is_empty() {
            self.settings.guests.room = guests::new_room_name();
            let _ = self.settings.save();
        }
        let name = self.guest_name.trim().to_string();
        match guests::invite_link(&self.settings.guests, &self.guest_secret, &name) {
            Ok(link) => {
                ctx.copy_text(link.clone());
                let shown = if name.is_empty() { t!("Guest {}", "Invitado {}", self.guest_links.len() + 1) } else { name };
                self.guest_links.insert(0, (shown.clone(), link));
                self.guest_links.truncate(12);
                self.guest_name.clear();
                self.toast(t!("Link for {} copied. Paste it to them.", "Link de {} copiado. Pégaselo.", shown));
                if self.guest_room.is_none() {
                    self.guests_open();
                }
            }
            Err(e) => self.toast(format!("{e:#}")),
        }
    }
}

pub fn page(app: &mut App, ui: &mut Ui, area: Rect) {
    let ctx = ui.ctx().clone();
    let ready = app.settings.guests.ready(&app.guest_secret);
    let state = app.guest_room.as_ref().map(|r| r.state.clone());
    let (connected, error, guest_list) = match &state {
        Some(s) => {
            let s = s.lock().unwrap();
            (s.connected, s.error.clone(), s.guests.clone())
        }
        None => (false, None, vec![]),
    };
    if app.guest_room.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(66));
    }

    // Top bar.
    let bar = Rect::from_min_size(area.min, vec2(area.width(), 64.0));
    theme::card(ui.painter(), bar);
    let p = ui.painter().clone();
    p.text(bar.left_center() + vec2(22.0, -8.0), Align2::LEFT_CENTER, l("Bring people into your show from a link", "Trae gente a tu show con un link"), medium(14.0), TEXT);
    let sub = match (&app.guest_room, connected, &error) {
        (None, ..) => l("Guests open the link in their browser, set up camera and mic, and wait here.", "Tus invitados abren el link, ponen cámara y micro, y esperan aquí.").to_string(),
        (Some(_), _, Some(e)) => t!("Problem: {}", "Problema: {}", e),
        (Some(_), false, None) => l("Connecting…", "Conectando…").to_string(),
        (Some(_), true, None) => t!("Room {} is open · {} waiting", "Sala {} abierta · {} esperando", app.settings.guests.room, guest_list.iter().filter(|g| !g.on_stage).count()),
    };
    p.text(bar.left_center() + vec2(22.0, 11.0), Align2::LEFT_CENTER, sub, body(12.0), if error.is_some() { RED } else { FAINT });
    // Right: server settings (gear), close room, the name and Invite.
    let gear = Rect::from_min_size(pos2(bar.right() - 20.0 - 38.0, bar.center().y - 19.0), vec2(38.0, 38.0));
    {
        let tip = l("Server settings (LiveKit)", "Ajustes del servidor (LiveKit)");
        let resp = ui.interact(gear, egui::Id::new("g-gear"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
        p.rect_filled(gear, CornerRadius::same(19), if app.guest_setup || resp.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 120) } else { Color32::from_rgba_premultiplied(40, 40, 44, 100) });
        p.text(gear.center(), Align2::CENTER_CENTER, "⚙", semibold(16.0), if app.guest_setup { ORANGE } else { TEXT });
        if resp.clicked() {
            app.guest_setup = !app.guest_setup;
        }
    }
    let mut right = gear.left() - 10.0;
    if app.guest_room.is_some() {
        let b = Rect::from_min_size(pos2(right - 130.0, gear.top()), vec2(130.0, 38.0));
        if pill(ui, b, "g-close", l("Close room", "Cerrar sala"), false) {
            app.guest_room = None;
            app.guest_thumbs.clear();
        }
        right = b.left() - 10.0;
    }
    if ready {
        let b = Rect::from_min_size(pos2(right - 170.0, gear.top()), vec2(170.0, 38.0));
        if pill(ui, b, "g-invite", l("Invite a guest", "Invitar a alguien"), true) {
            app.guests_invite(&ctx);
        }
        let nr = Rect::from_min_size(pos2(b.left() - 10.0 - 190.0, gear.top() + 4.0), vec2(190.0, 30.0));
        if nr.left() > bar.left() + 360.0 {
            ui.scope_builder(UiBuilder::new().max_rect(nr), |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut app.guest_name).hint_text(l("Their name (optional)", "Su nombre (opcional)")).desired_width(nr.width()));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    app.guests_invite(&ctx);
                }
            });
        }
    }

    let mut below = Rect::from_min_max(pos2(area.left(), bar.bottom() + 16.0), area.max);
    if !ready || app.guest_setup {
        setup(app, ui, below, ready);
        if !ready {
            return;
        }
        below.min.y += 380.0 + 16.0;
    }
    // The links made: one per person, each with Copy.
    if !app.guest_links.is_empty() {
        let rows = app.guest_links.len().min(4);
        let lr = Rect::from_min_size(below.min, vec2(below.width(), 48.0 + rows as f32 * 34.0));
        theme::card(ui.painter(), lr);
        p.text(lr.min + vec2(18.0, 22.0), Align2::LEFT_CENTER, l("Invite links (one per person, 24 hours)", "Links de invitación (uno por persona, 24 horas)"), semibold(13.0), MUTED);
        let mut copy = None;
        let links = app.guest_links.clone();
        for (k, (name, link)) in links.iter().take(rows).enumerate() {
            let y = lr.top() + 48.0 + k as f32 * 34.0 + 12.0;
            p.text(pos2(lr.left() + 18.0, y), Align2::LEFT_CENTER, truncate(name, 22), medium(13.0), TEXT);
            p.text(pos2(lr.left() + 200.0, y), Align2::LEFT_CENTER, truncate(link, ((lr.width() - 320.0) / 7.0).max(10.0) as usize), body(12.0), FAINT);
            let b = Rect::from_min_size(pos2(lr.right() - 18.0 - 80.0, y - 14.0), vec2(80.0, 28.0));
            if pill(ui, b, &format!("g-copy-{k}"), l("Copy", "Copiar"), false) {
                copy = Some(link.clone());
            }
        }
        if let Some(link) = copy {
            ctx.copy_text(link);
            app.toast(l("Link copied", "Link copiado"));
        }
        below.min.y = lr.bottom() + 16.0;
    }
    if app.guest_room.is_none() {
        let r = Rect::from_min_size(below.min, vec2(below.width(), 200.0));
        p.rect_stroke(r, CornerRadius::same(R_CARD), Stroke::new(1.5, Color32::from_white_alpha(30)), StrokeKind::Inside);
        p.text(r.center() - vec2(0.0, 14.0), Align2::CENTER_CENTER, l("Click \"Invite a guest\" and send them the link", "Haz clic en \"Invitar a alguien\" y mándale el link"), medium(16.0), TEXT);
        p.text(r.center() + vec2(0.0, 14.0), Align2::CENTER_CENTER, l("Each person gets their own link. They show up here, and you bring them on stage.", "Cada persona tiene su propio link. Aparecen aquí y tú los subes al escenario."), body(12.5), MUTED);
        return;
    }

    // Scenes row.
    let scenes_r = Rect::from_min_size(below.min, vec2(below.width(), 52.0));
    theme::card(ui.painter(), scenes_r);
    p.text(scenes_r.left_center() + vec2(18.0, 0.0), Align2::LEFT_CENTER, l("Scene", "Escena"), semibold(13.0), MUTED);
    let mut x = scenes_r.left() + 84.0;
    for s in crate::templates::guest_scenes() {
        let w = 26.0 + s.name.len() as f32 * 7.2;
        let r = Rect::from_min_size(pos2(x, scenes_r.center().y - 16.0), vec2(w, 32.0));
        let on = app.settings.template.as_deref() == Some(s.name.as_str());
        if r.right() < scenes_r.right() - 8.0 && pill(ui, r, &format!("g-scene-{}", s.name), &s.name, on) {
            let t = crate::templates::find(&s.name).unwrap_or(s);
            app.apply_template(&t);
        }
        x += w + 8.0;
    }

    // Guest cards.
    let grid = Rect::from_min_max(pos2(below.left(), scenes_r.bottom() + 16.0), below.max);
    if guest_list.is_empty() {
        let r = Rect::from_min_size(grid.min, vec2(grid.width(), 200.0));
        p.rect_stroke(r, CornerRadius::same(R_CARD), Stroke::new(1.5, Color32::from_white_alpha(30)), StrokeKind::Inside);
        p.text(r.center() - vec2(0.0, 14.0), Align2::CENTER_CENTER, l("Nobody here yet", "Todavía no hay nadie"), medium(16.0), TEXT);
        p.text(r.center() + vec2(0.0, 14.0), Align2::CENTER_CENTER, l("Click \"Invite a guest\" and send the link. Each person gets their own.", "Haz clic en \"Invitar a alguien\" y manda el link. Cada persona tiene el suyo."), body(12.5), MUTED);
        return;
    }
    let cols = ((grid.width() + 16.0) / 300.0).floor().clamp(1.0, 4.0) as usize;
    let gap = 16.0;
    let cw = (grid.width() - gap * (cols - 1) as f32) / cols as f32;
    let art_h = (cw - 24.0) * 9.0 / 16.0;
    let ch = art_h + 104.0;
    let mut send: Vec<Cmd> = vec![];
    ui.scope_builder(UiBuilder::new().max_rect(grid), |ui| {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let rows = guest_list.len().div_ceil(cols);
            let (all, _) = ui.allocate_exact_size(vec2(grid.width(), rows as f32 * (ch + gap)), egui::Sense::hover());
            for (i, g) in guest_list.iter().enumerate() {
                let card = Rect::from_min_size(all.min + vec2((i % cols) as f32 * (cw + gap), (i / cols) as f32 * (ch + gap)), vec2(cw, ch));
                let p = ui.painter().clone();
                theme::card(&p, card);
                if g.on_stage {
                    p.rect_stroke(card, CornerRadius::same(R_CARD), Stroke::new(1.5, RED), StrokeKind::Inside);
                }
                let art = Rect::from_min_size(card.min + vec2(12.0, 12.0), vec2(cw - 24.0, art_h));
                p.rect_filled(art, CornerRadius::same(12), Color32::from_rgb(12, 12, 14));
                match thumb(app, &ctx, g) {
                    Some(t) => {
                        let s = t.size_vec2();
                        let k = (art.width() / s.x).min(art.height() / s.y);
                        egui::Image::from_texture(&t).corner_radius(CornerRadius::same(10)).paint_at(ui, Rect::from_center_size(art.center(), s * k));
                    }
                    None => {
                        p.text(art.center(), Align2::CENTER_CENTER, if g.has_video { l("Starting camera…", "Iniciando cámara…") } else { l("No camera", "Sin cámara") }, body(12.5), MUTED);
                    }
                }
                if g.on_stage {
                    let seat = state.as_ref().map(|s| s.lock().unwrap().stage.iter().position(|x| *x == g.identity).unwrap_or(0)).unwrap_or(0);
                    let chip = Rect::from_min_size(art.min + vec2(10.0, 10.0), vec2(98.0, 22.0));
                    p.rect_filled(chip, CornerRadius::same(11), Color32::from_rgba_premultiplied(0, 0, 0, 170));
                    p.circle_filled(chip.left_center() + vec2(11.0, 0.0), 3.5, RED);
                    p.text(chip.left_center() + vec2(20.0, 0.0), Align2::LEFT_CENTER, t!("ON STAGE {}", "EN VIVO {}", seat + 1), semibold(10.5), TEXT);
                }
                // Name and mic level.
                let ty = art.bottom() + 18.0;
                p.text(pos2(card.left() + 16.0, ty), Align2::LEFT_CENTER, truncate(&g.name, (cw / 8.5) as usize), medium(14.0), TEXT);
                let meter = Rect::from_min_size(pos2(card.right() - 16.0 - 70.0, ty - 3.0), vec2(70.0, 6.0));
                p.rect_filled(meter, CornerRadius::same(3), Color32::from_white_alpha(25));
                let lv = if g.muted { 0.0 } else { (g.level * 1.6).min(1.0) };
                p.rect_filled(Rect::from_min_size(meter.min, vec2(meter.width() * lv, meter.height())), CornerRadius::same(3), if g.muted { FAINT } else { GREEN });
                // Buttons.
                let by = ty + 22.0;
                let bw = (cw - 24.0 - 16.0) / 3.0;
                let b = |k: f32| Rect::from_min_size(pos2(card.left() + 12.0 + k * (bw + 8.0), by), vec2(bw, 34.0));
                if pill(ui, b(0.0), &format!("g-stage-{i}"), if g.on_stage { l("Off stage", "Bajar") } else { l("On stage", "Subir") }, !g.on_stage) {
                    send.push(Cmd::Stage(g.identity.clone(), !g.on_stage));
                }
                if pill(ui, b(1.0), &format!("g-mute-{i}"), if g.muted { l("Unmute", "Activar") } else { l("Mute", "Silenciar") }, false) {
                    send.push(Cmd::Mute(g.identity.clone(), !g.muted));
                }
                if pill(ui, b(2.0), &format!("g-kick-{i}"), l("Remove", "Sacar"), false) {
                    send.push(Cmd::Remove(g.identity.clone()));
                }
            }
        });
    });
    let first_on_stage = send.iter().any(|c| matches!(c, Cmd::Stage(_, true)));
    if let Some(r) = &app.guest_room {
        for c in send {
            r.send(c);
        }
    }
    // Bringing someone on with no guest scene showing: switch to Interview.
    let guest_scene = app.settings.layout.as_ref().is_some_and(|l| l.layers.iter().any(|x| x.source == crate::layout::Source::Guest));
    if first_on_stage && !guest_scene {
        if let Some(t) = crate::templates::guest_scenes().into_iter().next() {
            let t = crate::templates::find(&t.name).unwrap_or(t);
            app.apply_template(&t);
        }
    }
}

/// A guest's camera as a texture (refreshed ~15 times a second).
fn thumb(app: &mut App, ctx: &egui::Context, g: &guests::Guest) -> Option<egui::TextureHandle> {
    let ((w, h), px) = g.frame.clone()?;
    let entry = app.guest_thumbs.get(&g.identity);
    if let Some((seq, tex)) = entry
        && (*seq == g.frame_seq || g.frame_seq < seq + 2)
    {
        return Some(tex.clone());
    }
    // BGRA -> RGBA, downscaled to at most 480 wide.
    let step = (w / 480).max(1) as usize;
    let (tw, th) = (w as usize / step, h as usize / step);
    let mut rgba = Vec::with_capacity(tw * th * 4);
    for y in 0..th {
        let row = &px[y * step * w as usize * 4..];
        for x in 0..tw {
            let s = &row[x * step * 4..x * step * 4 + 4];
            rgba.extend_from_slice(&[s[2], s[1], s[0], 255]);
        }
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([tw, th], &rgba);
    let tex = match app.guest_thumbs.get_mut(&g.identity) {
        Some((seq, tex)) => {
            tex.set(img, egui::TextureOptions::LINEAR);
            *seq = g.frame_seq;
            tex.clone()
        }
        None => {
            let tex = ctx.load_texture(format!("guest:{}", g.identity), img, egui::TextureOptions::LINEAR);
            app.guest_thumbs.insert(g.identity.clone(), (g.frame_seq, tex.clone()));
            tex
        }
    };
    Some(tex)
}

/// LiveKit settings (before the room is open).
fn setup(app: &mut App, ui: &mut Ui, area: Rect, ready: bool) {
    let card = Rect::from_min_size(area.min, vec2(area.width().min(720.0), 380.0));
    theme::card(ui.painter(), card);
    ui.scope_builder(UiBuilder::new().max_rect(card.shrink(22.0)), |ui| {
        ui.label(egui::RichText::new(l("Your LiveKit project", "Tu proyecto de LiveKit")).font(medium(16.0)).color(TEXT));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(l(
            "Guests connect through LiveKit Cloud (free: about 80 hours of guest time a month). Create a project at cloud.livekit.io, open Settings → API keys, create a key, and paste the three values here. The secret stays in Windows Credential Manager.",
            "Los invitados se conectan por LiveKit Cloud (gratis: unas 80 horas de invitados al mes). Crea un proyecto en cloud.livekit.io, abre Settings → API keys, crea una clave y pega aquí los tres valores. El secreto se guarda en el Administrador de credenciales de Windows.",
        )).font(body(12.5)).color(MUTED));
        ui.hyperlink_to(l("Open LiveKit Cloud", "Abrir LiveKit Cloud"), "https://cloud.livekit.io/");
        ui.add_space(10.0);
        let w = ui.available_width();
        let mut changed = false;
        ui.label(egui::RichText::new(l("Project URL", "URL del proyecto")).color(MUTED).size(12.5));
        changed |= ui.add(egui::TextEdit::singleline(&mut app.settings.guests.url).hint_text("wss://your-project.livekit.cloud").desired_width(w)).changed();
        ui.label(egui::RichText::new("API key").color(MUTED).size(12.5));
        changed |= ui.add(egui::TextEdit::singleline(&mut app.settings.guests.api_key).hint_text("API…").desired_width(w)).changed();
        ui.label(egui::RichText::new("API secret").color(MUTED).size(12.5));
        if ui.add(egui::TextEdit::singleline(&mut app.guest_secret).password(true).desired_width(w)).changed() {
            let s = app.guest_secret.trim().to_string();
            let _ = if s.is_empty() { crate::win::creds::delete("livekit") } else { crate::win::creds::set("livekit", &s) };
        }
        if changed {
            app.settings.guests.url = app.settings.guests.url.trim().to_string();
            app.settings.guests.api_key = app.settings.guests.api_key.trim().to_string();
            let _ = app.settings.save();
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new(if ready {
            l("Ready. You only do this once: click \"Invite a guest\" up top.", "Listo. Esto se hace una sola vez: haz clic en \"Invitar a alguien\" arriba.")
        } else {
            l("Fill in all three to open the green room.", "Llena los tres para abrir la sala.")
        }).font(body(12.5)).color(if ready { GREEN } else { FAINT }));
    });
}
