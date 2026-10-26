//! The Chat card on the Studio page: Twitch, Kick and YouTube messages in
//! one list. Click one to show it on screen; the gear sets the channels.

use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, Id, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::App;
use super::theme::{self, *};
use crate::chat::{Chat, Platform};
use crate::i18n::l;

/// Credential Manager name for the YouTube Data API key.
pub const YT_KEY: &str = "youtube-api";

impl App {
    /// (Re)connect to the chat channels in settings (stops when none set).
    pub fn restart_chat(&mut self) {
        self.chat = None;
        let cfg = self.settings.chat.clone();
        if cfg.any() {
            self.chat = Some(Chat::start(cfg, crate::win::creds::get(YT_KEY)));
        }
    }
}

pub fn card(app: &mut App, ui: &mut Ui, card: egui::Rect) {
    let p = ui.painter().clone();
    theme::card(&p, card);
    let folded = app.settings.fold_chat;
    if super::ui::fold_title(ui, card, l("Chat", "Chat"), folded, "chat") {
        app.settings.fold_chat = !folded;
        app.settings_changed();
    }
    if folded {
        return;
    }
    // Header: each platform's state, and the gear.
    let states: Vec<(Platform, String)> = app.chat.as_ref().map(|c| c.feed.lock().unwrap().states.clone()).unwrap_or_default();
    let gear = Rect::from_min_size(pos2(card.right() - 22.0 - 30.0, card.top() + 14.0), vec2(30.0, 28.0));
    let g = ui.interact(gear, Id::new("chat-gear"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(l("Chat channels", "Canales del chat"));
    p.rect_filled(gear, CornerRadius::same(10), if app.chat_setup { Color32::from_rgba_premultiplied(70, 34, 0, 120) } else if g.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 90) } else { Color32::from_rgba_premultiplied(40, 40, 44, 80) });
    p.text(gear.center(), Align2::CENTER_CENTER, "⚙", body(15.0), if app.chat_setup { ORANGE } else { TEXT });
    if g.clicked() {
        app.chat_setup = !app.chat_setup;
        app.chat_key = String::new();
    }
    let mut x = gear.left() - 10.0;
    for (pl, state) in states.iter().rev() {
        let ok = !state.contains(' ') || state.starts_with('#');
        let [r, g, b] = pl.color();
        let tag = Rect::from_min_size(pos2(x - 62.0, card.top() + 18.0), vec2(62.0, 20.0));
        let resp = ui.interact(tag, Id::new(("chat-state", *pl as u8)), Sense::hover()).on_hover_text(format!("{}: {state}", pl.label()));
        let _ = resp;
        p.rect_filled(tag, CornerRadius::same(10), Color32::from_rgba_premultiplied(0, 0, 0, 90));
        p.circle_filled(tag.left_center() + vec2(10.0, 0.0), 4.0, if ok { Color32::from_rgb(r, g, b) } else { RED });
        p.text(tag.left_center() + vec2(18.0, 0.0), Align2::LEFT_CENTER, pl.label(), body(10.5), if ok { TEXT } else { RED });
        x = tag.left() - 6.0;
    }
    let inner = Rect::from_min_max(pos2(card.left() + 16.0, card.top() + 50.0), pos2(card.right() - 16.0, card.bottom() - 12.0));
    if app.chat_setup {
        setup(app, ui, inner);
        return;
    }
    let Some(chat) = &app.chat else {
        p.text(inner.center(), Align2::CENTER_CENTER, l("Set your channels with the gear to see your chat here.", "Pon tus canales con el engrane para ver tu chat aquí."), body(12.5), MUTED);
        return;
    };
    let msgs: Vec<crate::chat::Msg> = chat.feed.lock().unwrap().msgs.iter().cloned().collect();
    let pinned = app.settings.chat_pin.clone();
    let mut pick: Option<Option<(String, String)>> = None;
    let mut list = inner;
    if pinned.is_some() {
        let b = Rect::from_min_size(inner.min, vec2(inner.width(), 26.0));
        let r = ui.interact(b, Id::new("chat-unpin"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        p.rect_filled(b, CornerRadius::same(13), if r.hovered() { Color32::from_rgba_premultiplied(90, 40, 0, 160) } else { Color32::from_rgba_premultiplied(70, 34, 0, 130) });
        p.text(b.center(), Align2::CENTER_CENTER, l("On screen  ·  click to remove", "En pantalla  ·  clic para quitar"), semibold(11.5), ORANGE);
        if r.clicked() {
            pick = Some(None);
        }
        list.min.y += 32.0;
    }
    if msgs.is_empty() {
        p.text(list.center(), Align2::CENTER_CENTER, l("No messages yet.", "Aún no hay mensajes."), body(12.5), MUTED);
    }
    ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
        ScrollArea::vertical().max_height(list.height()).auto_shrink([false; 2]).stick_to_bottom(true).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for m in &msgs {
                let on = pinned.as_ref().is_some_and(|(a, t)| *a == m.author && *t == m.text);
                let [pr, pg, pb] = m.platform.color();
                let [ar, ag, ab] = m.color.unwrap_or([pr, pg, pb]);
                let mut job = egui::text::LayoutJob::default();
                let f = |size: f32, c: Color32| egui::TextFormat { font_id: body(size), color: c, ..Default::default() };
                job.append(&format!("{}  ", m.author), 0.0, egui::TextFormat { font_id: semibold(12.5), color: Color32::from_rgb(ar, ag, ab).gamma_multiply(1.2), ..Default::default() });
                job.append(&m.text, 0.0, f(12.5, TEXT));
                job.wrap.max_width = ui.available_width() - 22.0;
                let galley = ui.fonts_mut(|fo| fo.layout_job(job));
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y + 8.0), Sense::click());
                if on {
                    ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_rgba_premultiplied(70, 34, 0, 120));
                    ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, ORANGE), StrokeKind::Inside);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(8), Color32::from_rgba_premultiplied(40, 40, 44, 90));
                }
                ui.painter().circle_filled(pos2(rect.left() + 7.0, rect.top() + 11.0), 3.5, Color32::from_rgb(pr, pg, pb));
                ui.painter().galley(pos2(rect.left() + 16.0, rect.top() + 4.0), galley, TEXT);
                let resp = resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(if on { l("Remove from the screen", "Quitar de la pantalla") } else { l("Show on screen", "Mostrar en pantalla") });
                if resp.clicked() {
                    pick = Some(if on { None } else { Some((m.author.clone(), m.text.clone())) });
                }
            }
        });
    });
    if let Some(v) = pick {
        app.settings.chat_pin = v;
        app.settings_changed();
    }
}

/// Channel fields and the YouTube key.
fn setup(app: &mut App, ui: &mut Ui, inner: Rect) {
    let mut connect = false;
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ScrollArea::vertical().max_height(inner.height()).auto_shrink([false; 2]).show(ui, |ui| {
            let w = ui.available_width();
            let field = |ui: &mut Ui, label: &str, v: &mut String, hint: &str| {
                ui.label(egui::RichText::new(label).color(MUTED).size(12.5));
                ui.add(egui::TextEdit::singleline(v).hint_text(hint).desired_width(w));
            };
            field(ui, "Twitch", &mut app.settings.chat.twitch, l("channel name", "nombre del canal"));
            field(ui, "Kick", &mut app.settings.chat.kick, l("channel name", "nombre del canal"));
            field(ui, "YouTube", &mut app.settings.chat.youtube, l("live link, @handle or channel ID", "link del en vivo, @usuario o ID del canal"));
            let has_key = crate::win::creds::get(YT_KEY).is_some();
            ui.label(egui::RichText::new(if has_key { l("YouTube API key (saved)", "Llave de la API de YouTube (guardada)") } else { l("YouTube API key", "Llave de la API de YouTube") }).color(MUTED).size(12.5));
            ui.add(egui::TextEdit::singleline(&mut app.chat_key).password(true).hint_text(if has_key { l("leave empty to keep it", "déjala vacía para conservarla") } else { "AIza…" }).desired_width(w));
            ui.label(
                egui::RichText::new(l(
                    "Get a free key: console.cloud.google.com → new project → enable \"YouTube Data API v3\" → Credentials → Create API key. It's kept in Windows Credential Manager.",
                    "Llave gratis: console.cloud.google.com → proyecto nuevo → habilita \"YouTube Data API v3\" → Credenciales → Crear llave de API. Se guarda en el Administrador de credenciales de Windows.",
                ))
                .color(FAINT)
                .size(11.0),
            );
            ui.add_space(4.0);
            if ui.add(egui::Button::new(egui::RichText::new(l("Connect", "Conectar")).strong().color(Color32::BLACK)).fill(Color32::WHITE).min_size(vec2(120.0, 30.0))).clicked() {
                connect = true;
            }
        });
    });
    if connect {
        let key = std::mem::take(&mut app.chat_key);
        if !key.trim().is_empty() {
            let _ = crate::win::creds::set(YT_KEY, key.trim());
        }
        app.chat_setup = false;
        let _ = app.settings.save();
        app.restart_chat();
    }
}
