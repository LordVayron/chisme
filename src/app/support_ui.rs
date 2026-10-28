//! The "Support Chisme" page: how to sponsor it, and the sponsors (from
//! `sponsors.json` in the repo, loaded when the page opens).

use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::App;
use super::media_ui::pill;
use super::theme::{self, *};
use crate::i18n::l;
use crate::sponsors::{self, Sponsor, State, Tier};

/// The sidebar's heart.
pub fn icon(p: &Painter, c: Pos2, color: Color32) {
    heart(p, c, color, 1.0);
}

fn heart(p: &Painter, c: Pos2, color: Color32, scale: f32) {
    let pts: Vec<Pos2> = (0..=40)
        .map(|i| {
            let t = i as f32 / 40.0 * std::f32::consts::TAU;
            let x = 16.0 * t.sin().powi(3);
            let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            c + vec2(x, -y - 1.5) * 0.58 * scale
        })
        .collect();
    p.add(Shape::line(pts, Stroke::new(1.8 * scale.sqrt(), color)));
}

/// Open a web page in the browser.
fn open(url: &str) {
    if url.starts_with("https://") {
        let _ = std::process::Command::new("explorer").arg(url).spawn();
    }
}

pub fn page(app: &mut App, ui: &mut Ui, area: Rect) {
    // Load once; after a failure only "Try again" asks again.
    if *app.sponsors.lock().unwrap() == State::Idle {
        sponsors::spawn_load(&app.sponsors);
    }
    let state = app.sponsors.lock().unwrap().clone();
    if matches!(state, State::Loading) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    let sign_up = match &state {
        State::Ready(list) => list.sign_up().to_string(),
        _ => sponsors::SIGN_UP.to_string(),
    };

    // The pitch.
    let top = Rect::from_min_size(area.min, vec2(area.width(), 132.0));
    let p = ui.painter().clone();
    theme::card(&p, top);
    theme::glow(&p, top.left_center() + vec2(60.0, 0.0), vec2(90.0, 70.0), Color32::from_rgba_premultiplied(90, 30, 20, 70));
    heart(&p, top.left_center() + vec2(52.0, 0.0), ORANGE, 1.7);
    let tx = top.left() + 96.0;
    p.text(pos2(tx, top.top() + 26.0), Align2::LEFT_TOP, l("Chisme is free, with no ads and no account", "Chisme es gratis, sin anuncios y sin cuenta"), medium(17.0), TEXT);
    p.text(
        pos2(tx, top.top() + 52.0),
        Align2::LEFT_TOP,
        l("Sponsors keep it that way. Their names and logos show here, on the website and on GitHub.", "Los sponsors lo hacen posible. Sus nombres y logos salen aquí, en la página web y en GitHub."),
        body(12.5),
        MUTED,
    );
    let b1 = Rect::from_min_size(pos2(tx, top.bottom() - 52.0), vec2(170.0, 36.0));
    if pill(ui, b1, "support-sponsor", l("Become a sponsor", "Ser sponsor"), true) {
        open(&sign_up);
    }
    let b2 = Rect::from_min_size(pos2(b1.right() + 10.0, b1.top()), vec2(140.0, 36.0));
    if pill(ui, b2, "support-tiers", l("Sponsor tiers", "Niveles"), false) {
        open(sponsors::TIERS);
    }

    // The sponsors.
    let rest = Rect::from_min_max(pos2(area.left(), top.bottom() + 16.0), area.max);
    theme::card(&p, rest);
    ui.scope_builder(UiBuilder::new().max_rect(rest.shrink2(vec2(22.0, 18.0))), |ui| {
        egui::ScrollArea::vertical().id_salt("support-page").auto_shrink([false; 2]).show(ui, |ui| match &state {
            State::Idle | State::Loading => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new(l("Loading sponsors…", "Cargando sponsors…")).color(MUTED));
                });
            }
            State::Failed(_) => {
                ui.label(egui::RichText::new(l("Couldn't load the sponsor list.", "No se pudo cargar la lista de sponsors.")).color(MUTED));
                ui.add_space(8.0);
                if ui.button(l("Try again", "Reintentar")).clicked() {
                    sponsors::spawn_load(&app.sponsors);
                }
            }
            State::Ready(list) if list.sponsors.is_empty() => {
                ui.label(egui::RichText::new(l("No sponsors yet", "Todavía no hay sponsors")).font(medium(15.0)).color(TEXT));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(l("Your name or logo could be the first one here.", "Tu nombre o logo puede ser el primero aquí.")).color(MUTED));
            }
            State::Ready(list) => {
                let of = |t: Tier| list.sponsors.iter().filter(move |s| s.tier == t);
                section(ui, l("Main sponsors", "Sponsors principales"), of(Tier::Main).count());
                for s in of(Tier::Main) {
                    let w = ui.available_width();
                    tile(app, ui, s, vec2(w, 130.0), 26.0);
                    ui.add_space(10.0);
                }
                section(ui, l("Partners", "Socios"), of(Tier::Partner).count());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                    for s in of(Tier::Partner) {
                        tile(app, ui, s, vec2(230.0, 96.0), 17.0);
                    }
                });
                section(ui, l("Backers", "Patrocinadores"), of(Tier::Backer).count());
                ui.horizontal_wrapped(|ui| {
                    for s in of(Tier::Backer) {
                        chip(ui, s);
                    }
                });
                section(ui, l("Supporters", "Simpatizantes"), of(Tier::Supporter).count());
                let names: Vec<&str> = of(Tier::Supporter).map(|s| s.name.as_str()).collect();
                if !names.is_empty() {
                    ui.label(egui::RichText::new(names.join("  ·  ")).color(MUTED).size(13.0));
                }
            }
        });
    });
}

/// A tier's title (skipped when it has nobody).
fn section(ui: &mut Ui, title: &str, count: usize) {
    if count == 0 {
        return;
    }
    ui.add_space(10.0);
    ui.label(egui::RichText::new(title).font(medium(14.0)).color(TEXT));
    ui.add_space(6.0);
}

/// A logo (or the name in big letters) in a card; a click opens the link.
fn tile(app: &mut App, ui: &mut Ui, s: &Sponsor, size: egui::Vec2, name_size: f32) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = hover_link(resp, s);
    let p = ui.painter();
    let fill = if resp.hovered() { Color32::from_rgba_premultiplied(44, 44, 48, 120) } else { Color32::from_rgba_premultiplied(30, 30, 33, 110) };
    p.rect_filled(rect, CornerRadius::same(16), fill);
    p.rect_stroke(rect, CornerRadius::same(16), Stroke::new(1.0, CARD_EDGE), StrokeKind::Inside);
    match logo(app, ui.ctx(), s) {
        Some((tex, [w, h])) => {
            let room = rect.shrink2(vec2(24.0, 18.0)).size();
            let k = (room.x / w as f32).min(room.y / h as f32).min(1.0);
            let r = Rect::from_center_size(rect.center(), vec2(w as f32, h as f32) * k);
            p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            p.text(rect.center(), Align2::CENTER_CENTER, &s.name, display(name_size), TEXT);
        }
    }
    if resp.clicked()
        && let Some(u) = s.link()
    {
        open(u);
    }
}

/// A name in a pill; a click opens the link.
fn chip(ui: &mut Ui, s: &Sponsor) {
    let galley = ui.painter().layout_no_wrap(s.name.clone(), semibold(13.0), TEXT);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(28.0, 14.0), Sense::click());
    let resp = hover_link(resp, s);
    let fill = if resp.hovered() && s.link().is_some() { Color32::from_rgba_premultiplied(60, 60, 66, 120) } else { Color32::from_rgba_premultiplied(40, 40, 44, 100) };
    ui.painter().rect_filled(rect, CornerRadius::same((rect.height() / 2.0) as u8), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, TEXT);
    if resp.clicked()
        && let Some(u) = s.link()
    {
        open(u);
    }
}

fn hover_link(resp: egui::Response, s: &Sponsor) -> egui::Response {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, &s.name));
    match s.link() {
        Some(u) => resp.on_hover_text(u).on_hover_cursor(CursorIcon::PointingHand),
        None => resp,
    }
}

/// The sponsor's logo as a texture (made once).
fn logo(app: &mut App, ctx: &egui::Context, s: &Sponsor) -> Option<(egui::TextureHandle, [usize; 2])> {
    let img = s.image.as_ref()?;
    let key = format!("{}|{}", s.name, s.logo);
    let tex = app
        .sponsor_logos
        .entry(key.clone())
        .or_insert_with(|| ctx.load_texture(format!("sponsor:{key}"), egui::ColorImage::from_rgba_unmultiplied(img.size, &img.rgba), egui::TextureOptions::LINEAR))
        .clone();
    Some((tex, img.size))
}
