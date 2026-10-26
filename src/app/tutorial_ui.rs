//! The Tutorial mode card on the Studio page: one main switch in the title,
//! and a switch per part with where it goes (recording, stream or both).

use eframe::egui::{self, Color32, CursorIcon, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, pos2, vec2};

use super::App;
use super::theme::{self, *};
use super::ui::{FOLDED_H, combo, fold_title, toggle};
use crate::i18n::l;
use crate::tutorial::{Apply, Part};

/// The card's height when open.
pub(super) const OPEN_H: f32 = 400.0;

/// Its height now.
pub(super) fn height(app: &App) -> f32 {
    if app.settings.fold_tutorial { FOLDED_H } else { OPEN_H }
}

/// A part's row: its switch, name and (while on) where it goes.
fn part_row(ui: &mut Ui, label: &str, hint: &str, part: &mut Part, id: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        changed |= toggle(ui, &mut part.on).on_hover_text(hint).changed();
        ui.add_space(6.0);
        ui.label(egui::RichText::new(label).color(if part.on { TEXT } else { MUTED }).size(13.0)).on_hover_text(hint);
        if part.on {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let opts: Vec<(Apply, String)> = Apply::ALL.iter().map(|a| (*a, a.label().to_string())).collect();
                changed |= combo(ui, id, &mut part.to, &opts, 96.0);
            });
        }
    });
    changed
}

pub(super) fn card(app: &mut App, ui: &mut Ui, card: Rect) {
    let p = ui.painter().clone();
    theme::card(&p, card);
    let folded = app.settings.fold_tutorial;
    if fold_title(ui, card, l("Tutorial mode", "Modo tutorial"), folded, "tutorial") {
        app.settings.fold_tutorial = !folded;
        app.settings_changed();
    }
    // The main switch, in the title so it works folded too.
    let sw = Rect::from_min_size(pos2(card.right() - 60.0, card.top() + 16.0), vec2(40.0, 24.0));
    let mut on = app.settings.tutorial.enabled;
    let main = ui.scope_builder(UiBuilder::new().max_rect(sw), |ui| toggle(ui, &mut on)).inner;
    if main.on_hover_text(l("Click rings, a bigger and smoother cursor, auto zoom and a shortcut banner, live", "Anillos en los clics, cursor más grande y suave, zoom automático y aviso de atajos, en vivo")).changed() {
        app.settings.tutorial.enabled = on;
        app.settings_changed();
    }
    if folded {
        return;
    }
    let inner = Rect::from_min_max(pos2(card.left() + 22.0, card.top() + 54.0), pos2(card.right() - 22.0, card.bottom() - 14.0));
    let mut changed = false;
    let mut look = false;
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ScrollArea::vertical().id_salt("tutorial-card").max_height(inner.height()).auto_shrink([false; 2]).show(ui, |ui| {
            if !app.settings.tutorial.enabled {
                ui.label(egui::RichText::new(l("Off. Turn it on with the switch above.", "Apagado. Actívalo con el interruptor de arriba.")).color(MUTED).size(12.0));
                ui.add_space(4.0);
            }
            let brand = app.settings.brand.primary;
            let zoom_key = app.settings.hotkeys.zoom.clone();
            let t = &mut app.settings.tutorial;
            changed |= part_row(ui, l("Click effects", "Efecto en clics"), l("A ring on each click", "Un anillo en cada clic"), &mut t.clicks, "tut-clicks");
            if t.clicks.on {
                changed |= color_row(ui, &mut t.ring_color, brand);
            }
            changed |= part_row(ui, l("Bigger cursor", "Cursor más grande"), l("Hides while the mouse rests", "Se oculta cuando el mouse no se mueve"), &mut t.big_cursor, "tut-big");
            if t.big_cursor.on {
                changed |= slider_row(ui, l("Size", "Tamaño"), &mut t.cursor_size);
            }
            changed |= part_row(ui, l("Auto zoom", "Zoom automático"), l("Zooms in where you click or type, and back out when you stop", "Acerca donde haces clic o escribes, y se aleja cuando paras"), &mut t.zoom, "tut-zoom");
            if t.zoom.on {
                changed |= slider_row(ui, l("Strength", "Intensidad"), &mut t.zoom_strength);
                ui.horizontal(|ui| {
                    ui.set_min_height(28.0);
                    ui.add_space(44.0);
                    ui.label(egui::RichText::new(l("Pause", "Pausar")).color(MUTED).size(12.0));
                    let hint = if zoom_key.is_empty() {
                        l("Set a shortcut in Settings → Shortcuts", "Pon un atajo en Ajustes → Atajos").to_string()
                    } else {
                        crate::t!("Shortcut: {}", "Atajo: {}", zoom_key)
                    };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        changed |= toggle(ui, &mut t.zoom_paused).on_hover_text(l("Clicks don't zoom while paused; the rest keeps working", "Los clics no acercan mientras está en pausa; lo demás sigue")).changed();
                        ui.label(egui::RichText::new(hint).color(MUTED).size(11.0));
                    });
                });
            }
            changed |= part_row(ui, l("Smooth cursor", "Cursor suave"), l("Glides instead of jumping", "Se desliza en vez de saltar"), &mut t.smooth_cursor, "tut-smooth");
            changed |= part_row(ui, l("Shortcut banner", "Aviso de atajos"), l("Shows Ctrl, Alt and Win shortcuts. Never what you type.", "Muestra atajos con Ctrl, Alt y Win. Nunca lo que escribes."), &mut t.keys, "tut-keys");
            ui.add_space(6.0);
            look = ui
                .add_sized([inner.width().min(240.0), 30.0], egui::Button::new(egui::RichText::new(l("Use the Tutorial look", "Usar el estilo Tutorial")).size(13.0)))
                .on_hover_text(l("Your screen with padding, rounded corners and a shadow on a gradient", "Tu pantalla con margen, esquinas redondeadas y sombra sobre un degradado"))
                .clicked();
        });
    });
    if changed {
        app.settings_changed();
    }
    if look {
        let t = crate::templates::find(crate::templates::tutorial_name()).unwrap_or_else(|| {
            let t = crate::templates::tutorial_scene();
            let _ = crate::templates::save(&t);
            t
        });
        app.apply_template(&t);
    }
}

/// The ring's color: the brand's, a few presets, or any color.
fn color_row(ui: &mut Ui, color: &mut Option<[u8; 3]>, brand: [u8; 3]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.add_space(44.0);
        ui.label(egui::RichText::new(l("Color", "Color")).color(MUTED).size(12.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut pick = color.unwrap_or(brand);
            if ui.color_edit_button_srgb(&mut pick).on_hover_text(l("Any color", "Cualquier color")).changed() {
                *color = Some(pick);
                changed = true;
            }
            let presets: [(Option<[u8; 3]>, &str); 6] = [
                (Some([255, 255, 255]), ""),
                (Some([46, 204, 113]), ""),
                (Some([214, 38, 46]), ""),
                (Some([36, 84, 255]), ""),
                (Some([255, 196, 0]), ""),
                (None, l("Brand color", "Color de marca")),
            ];
            for (c, tip) in presets {
                let rgb = c.unwrap_or(brand);
                let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                ui.painter().circle_filled(r.center(), 8.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                if c.is_none() {
                    ui.painter().circle_stroke(r.center(), 8.0, Stroke::new(1.0, Color32::from_gray(20)));
                }
                if *color == c {
                    ui.painter().circle_stroke(r.center(), 11.0, Stroke::new(1.5, TEXT));
                }
                let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
                let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
                if resp.clicked() && *color != c {
                    *color = c;
                    changed = true;
                }
            }
        });
    });
    changed
}

fn slider_row(ui: &mut Ui, label: &str, v: &mut f64) -> bool {
    ui.horizontal(|ui| {
        ui.set_min_height(28.0);
        ui.add_space(44.0);
        ui.label(egui::RichText::new(label).color(MUTED).size(12.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(egui::Slider::new(v, 1.2..=3.0).max_decimals(1).suffix("x")).changed()).inner
    })
    .inner
}
