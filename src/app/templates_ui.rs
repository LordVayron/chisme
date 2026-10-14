//! The "Plantillas" page: saved layouts as cards with a drawn thumbnail,
//! apply / export / delete, save the current one, import a .chisme.

use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, Id, Painter, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::App;
use crate::i18n::l;
use crate::t;
use super::theme::{self, *};
use crate::brand::Brand;
use crate::compose::Shape;
use crate::layout::{Layout, Source};

/// A schematic of a layout: background, then each layer as a shape colored
/// by kind (screen outline, camera with a head, image, text bars).
pub fn thumbnail(p: &Painter, frame: Rect, layout: &Layout, brand: &Brand) {
    let [r, g, b] = layout.background;
    p.rect_filled(frame, CornerRadius::same(8), Color32::from_rgb(r, g, b));
    for l in layout.layers.iter().filter(|l| l.visible) {
        let [x, y, w, h] = l.rect;
        let lr = Rect::from_min_size(
            pos2(frame.left() + x as f32 * frame.width(), frame.top() + y as f32 * frame.height()),
            vec2(w as f32 * frame.width(), h as f32 * frame.height()),
        );
        let round = match l.shape {
            Shape::Circle | Shape::Pill => CornerRadius::same((lr.height().min(lr.width()) / 2.0).min(120.0) as u8),
            Shape::Rect => CornerRadius::ZERO,
            _ => CornerRadius::same((l.roundness as f32 * lr.height().min(lr.width()) / 2.0).min(120.0) as u8),
        };
        match l.source {
            Source::Screen => {
                p.rect_filled(lr, round, Color32::from_rgb(52, 54, 60));
                p.rect_filled(Rect::from_min_size(lr.min, vec2(lr.width(), (lr.height() * 0.08).max(2.0))), CornerRadius::ZERO, Color32::from_rgb(80, 82, 90));
            }
            Source::Camera => {
                p.rect_filled(lr, round, Color32::from_rgb(120, 70, 40));
                let c = lr.center();
                let s = lr.height().min(lr.width());
                p.circle_filled(c - vec2(0.0, s * 0.1), s * 0.18, Color32::from_rgb(230, 190, 160));
                p.rect_filled(Rect::from_center_size(c + vec2(0.0, s * 0.28), vec2(s * 0.55, s * 0.3)), CornerRadius::same((s * 0.12) as u8), Color32::from_rgb(230, 190, 160));
            }
            Source::Image => {
                p.rect_filled(lr, round, Color32::from_rgb(36, 84, 160));
                p.line_segment([lr.left_bottom(), lr.center() + vec2(0.0, lr.height() * 0.1)], Stroke::new(1.5, Color32::from_rgb(140, 190, 255)));
                p.line_segment([lr.center() + vec2(0.0, lr.height() * 0.1), lr.right_bottom()], Stroke::new(1.5, Color32::from_rgb(140, 190, 255)));
            }
            Source::Text => {
                let [pr, pg, pb] = brand.primary;
                let bar = Rect::from_min_size(lr.min, vec2(lr.width() * 0.75, lr.height() * 0.5));
                p.rect_filled(bar, CornerRadius::same(2), Color32::from_rgb(pr, pg, pb));
                p.rect_filled(Rect::from_min_size(lr.min + vec2(0.0, lr.height() * 0.6), vec2(lr.width() * 0.5, lr.height() * 0.3)), CornerRadius::same(2), Color32::from_gray(200));
            }
        }
        if l.border > 0.0 {
            let [br, bg, bb] = l.border_color;
            p.rect_stroke(lr, round, Stroke::new(1.2, Color32::from_rgb(br, bg, bb)), StrokeKind::Inside);
        }
    }
    p.rect_stroke(frame, CornerRadius::same(8), Stroke::new(1.0, Color32::from_white_alpha(25)), StrokeKind::Inside);
}

pub fn page(app: &mut App, ui: &mut Ui, area: Rect) {
    // Top bar: save the current layout, import.
    let bar = Rect::from_min_size(area.min, vec2(area.width(), 64.0));
    theme::card(ui.painter(), bar);
    ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(vec2(20.0, 14.0))), |ui| {
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new(l("Save the current view as", "Guardar la vista actual como")).color(MUTED).size(13.0));
            ui.add(egui::TextEdit::singleline(&mut app.template_name).hint_text(l("Template name", "Nombre de la plantilla")).desired_width(240.0));
            let ok = !app.template_name.trim().is_empty();
            if ui.add_enabled(ok, egui::Button::new(egui::RichText::new(l("Save template", "Guardar plantilla")).strong())).clicked() {
                let name = std::mem::take(&mut app.template_name);
                app.save_template(&name);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(l("Import .chisme…", "Importar .chisme…")).clicked() {
                    app.import_template();
                }
                if !app.settings.hotkeys.templates.is_empty() {
                    ui.label(egui::RichText::new(t!("{}+1…9 switches template, even while recording", "{}+1…9 cambia de plantilla, incluso grabando", app.settings.hotkeys.templates)).color(FAINT).size(12.0));
                }
            });
        });
    });

    // Cards: 4 per row.
    let grid = Rect::from_min_max(pos2(area.left(), bar.bottom() + 16.0), area.max);
    let cols = 4;
    let gap = 16.0;
    let cw = (grid.width() - gap * (cols - 1) as f32) / cols as f32;
    let ch = cw * 9.0 / 16.0 + 92.0;
    let templates = app.templates.clone();
    let current = app.settings.template.clone();
    let custom = app.settings.layout.is_some();
    let brand = app.settings.brand.clone();
    let hk = app.settings.hotkeys.clone();
    ui.scope_builder(UiBuilder::new().max_rect(grid), |ui| {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let rows = templates.len().div_ceil(cols).max(1);
            let (all, _) = ui.allocate_exact_size(vec2(grid.width(), rows as f32 * (ch + gap)), Sense::hover());
            if templates.is_empty() {
                ui.painter().text(all.center_top() + vec2(0.0, 60.0), Align2::CENTER_CENTER, l("No templates yet. Build your layout in the Studio and save it here.", "Aún no hay plantillas. Arma tu layout en el Estudio y guárdalo aquí."), body(13.0), MUTED);
            }
            for (i, t) in templates.iter().enumerate() {
                let card = Rect::from_min_size(pos2(all.left() + (cw + gap) * (i % cols) as f32, all.top() + (ch + gap) * (i / cols) as f32), vec2(cw, ch));
                let p = ui.painter();
                theme::card(p, card);
                let active = custom && current.as_deref() == Some(t.name.as_str());
                if active {
                    p.rect_stroke(card, CornerRadius::same(R_CARD), Stroke::new(1.5, ORANGE), StrokeKind::Inside);
                }
                let thumb = Rect::from_min_size(card.min + vec2(12.0, 12.0), vec2(cw - 24.0, (cw - 24.0) * 9.0 / 16.0));
                thumbnail(p, thumb, &t.layout, t.brand.as_ref().unwrap_or(&brand));
                // Clicking the picture applies it.
                let resp = ui.interact(thumb, Id::new(("tpl-thumb", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(l("Apply", "Aplicar"));
                if resp.hovered() {
                    ui.painter().rect_stroke(thumb, CornerRadius::same(8), Stroke::new(1.5, Color32::from_white_alpha(140)), StrokeKind::Outside);
                }
                if resp.clicked() {
                    app.apply_template(t);
                }
                let p = ui.painter();
                let ty = thumb.bottom() + 20.0;
                p.text(pos2(card.left() + 16.0, ty), Align2::LEFT_CENTER, &t.name, medium(14.0), TEXT);
                if i < 9 {
                    p.text(pos2(card.right() - 16.0, ty), Align2::RIGHT_CENTER, super::hotkeys::Action::Template(i).combo(&hk), body(11.5), FAINT);
                }
                let by = ty + 30.0;
                let apply = Rect::from_min_size(pos2(card.left() + 14.0, by - 15.0), vec2(90.0, 30.0));
                let export = Rect::from_min_size(pos2(apply.right() + 8.0, by - 15.0), vec2(86.0, 30.0));
                let delete = Rect::from_min_size(pos2(card.right() - 14.0 - 30.0, by - 15.0), vec2(30.0, 30.0));
                if button(ui, apply, ("apply", i), if active { l("Active", "Activa") } else { l("Apply", "Aplicar") }, !active) {
                    app.apply_template(t);
                }
                if button(ui, export, ("export", i), l("Export", "Exportar"), false) {
                    app.export_template(t);
                }
                if button(ui, delete, ("delete", i), "🗑", false) {
                    app.delete_template(&t.name);
                }
            }
        });
    });
}

fn button(ui: &mut Ui, r: Rect, id: (&str, usize), label: &str, primary: bool) -> bool {
    let resp = ui.interact(r, Id::new(id), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    let fill = if primary { Color32::WHITE } else if resp.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 90) } else { Color32::from_rgba_premultiplied(40, 40, 44, 80) };
    ui.painter().rect_filled(r, CornerRadius::same(15), fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, semibold(12.5), if primary { Color32::BLACK } else { TEXT });
    resp.clicked()
}
