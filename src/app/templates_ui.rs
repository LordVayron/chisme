//! The Templates page: studios (named sets of scenes), and saved layouts as
//! cards with a drawn thumbnail: apply, add to the studio, delete.

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
            Source::Camera | Source::Guest => {
                p.rect_filled(lr, round, if l.source == Source::Guest { Color32::from_rgb(40, 90, 110) } else { Color32::from_rgb(120, 70, 40) });
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
    // Top bar: save the current layout, create a studio.
    let bar = Rect::from_min_size(area.min, vec2(area.width(), 64.0));
    theme::card(ui.painter(), bar);
    ui.scope_builder(UiBuilder::new().max_rect(bar.shrink2(vec2(20.0, 14.0))), |ui| {
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new(l("Save the current view as", "Guardar la vista actual como")).color(MUTED).size(13.0));
            ui.add(egui::TextEdit::singleline(&mut app.template_name).hint_text(l("Template name", "Nombre de la plantilla")).desired_width(240.0));
            let ok = !app.template_name.trim().is_empty();
            if ui.add_enabled(ok, egui::Button::new(egui::RichText::new(l("Save template", "Guardar plantilla")).strong())).clicked() {
                let name = std::mem::take(&mut app.template_name);
                app.save_dialog = Some(super::SaveDialog { name, group: String::new() });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(egui::RichText::new(l("+  Create Studio", "+  Crear Studio")).strong().color(Color32::BLACK)).fill(Color32::WHITE)).clicked() {
                    let n = app.settings.studios.len() + 1;
                    app.settings.add_studio(t!("Studio {}", "Studio {}", n));
                    let _ = app.settings.save();
                }
                if !app.settings.hotkeys.templates.is_empty() {
                    ui.label(egui::RichText::new(t!("{}+1…9 switches template, even while recording", "{}+1…9 cambia de plantilla, incluso grabando", app.settings.hotkeys.templates)).color(FAINT).size(12.0));
                }
            });
        });
    });

    let quick = Rect::from_min_size(pos2(area.left(), bar.bottom() + 16.0), vec2(area.width(), 214.0));
    studios_card(app, ui, quick);

    // The studio's templates, or all of them.
    let studio = app.settings.studios.get(app.settings.studio).map(|s| s.name.clone()).unwrap_or_default();
    let bar2 = Rect::from_min_size(pos2(area.left(), quick.bottom() + 12.0), vec2(area.width(), 34.0));
    {
        let p = ui.painter().clone();
        let title = if app.templates_all { l("All templates", "Todas las plantillas").to_string() } else { t!("Templates in {}", "Plantillas en {}", studio) };
        p.text(bar2.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, title, medium(15.0), TEXT);
        let label = if app.templates_all { t!("Only {}", "Solo {}", studio) } else { l("See all", "Ver todo").to_string() };
        let w = 30.0 + ui.painter().layout_no_wrap(label.clone(), semibold(12.5), TEXT).size().x;
        let b = Rect::from_min_size(pos2(bar2.right() - w, bar2.top() + 2.0), vec2(w, 30.0));
        if button(ui, b, ("see-all", 0), &label, !app.templates_all) {
            app.templates_all = !app.templates_all;
        }
        if app.templates_all {
            p.text(pos2(b.left() - 14.0, bar2.center().y), Align2::RIGHT_CENTER, t!("Add templates to {} with + Add on each card.", "Agrega plantillas a {} con + Agregar en cada tarjeta.", studio), body(12.0), FAINT);
        }
    }
    // Cards: 4 per row.
    let grid = Rect::from_min_max(pos2(area.left(), bar2.bottom() + 10.0), area.max);
    let cols = ((grid.width() + 16.0) / 300.0).floor().clamp(1.0, 6.0) as usize;
    let gap = 16.0;
    let cw = (grid.width() - gap * (cols - 1) as f32) / cols as f32;
    let ch = cw * 9.0 / 16.0 + 92.0;
    let show_all = app.templates_all;
    let templates: Vec<(usize, crate::templates::Template)> =
        app.templates.iter().cloned().enumerate().filter(|(_, t)| show_all || app.settings.pinned.contains(&crate::settings::Pin::Template(t.name.clone()))).collect();
    let current = app.settings.template.clone();
    let custom = app.settings.layout.is_some();
    let brand = app.settings.brand.clone();
    let hk = app.settings.hotkeys.clone();
    let mut regroup = false;
    ui.scope_builder(UiBuilder::new().max_rect(grid), |ui| {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let groups: Vec<&str> = templates.iter().map(|(_, t)| t.group.as_str()).collect();
            let all_groups = super::groups::order(&groups);
            let layout = super::groups::grid(&groups, ui.cursor().min, cols, vec2(cw, ch), gap);
            let (all, _) = ui.allocate_exact_size(vec2(grid.width(), layout.height.max(ch)), Sense::hover());
            super::groups::draw_headers(ui.painter(), &layout);
            if templates.is_empty() {
                let msg = if show_all || app.templates.is_empty() {
                    l("No templates yet. Build your layout in the Studio and save it here.", "Aún no hay plantillas. Arma tu layout en el Estudio y guárdalo aquí.").to_string()
                } else {
                    t!("{} has no templates yet. Click See all and use + Add on the ones you want.", "{} aún no tiene plantillas. Dale a Ver todo y usa + Agregar en las que quieras.", studio)
                };
                ui.painter().text(all.center_top() + vec2(0.0, 60.0), Align2::CENTER_CENTER, msg, body(13.0), MUTED);
            }
            for (k, (i, t)) in templates.iter().enumerate() {
                let i = *i;
                let card = layout.cells[k];
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
                resp.context_menu(|ui| {
                    if let Some(g) = super::groups::menu(ui, &t.group, &all_groups, &mut app.group_input) {
                        let moved = crate::templates::Template { group: g, ..t.clone() };
                        let _ = crate::templates::save(&moved);
                        regroup = true;
                    }
                });
                let p = ui.painter();
                let ty = thumb.bottom() + 20.0;
                p.text(pos2(card.left() + 16.0, ty), Align2::LEFT_CENTER, &t.name, medium(14.0), TEXT);
                if i < 9 {
                    p.text(pos2(card.right() - 16.0, ty), Align2::RIGHT_CENTER, super::hotkeys::Action::Template(i).combo(&hk), body(11.5), FAINT);
                }
                let by = ty + 30.0;
                let apply = Rect::from_min_size(pos2(card.left() + 14.0, by - 15.0), vec2(90.0, 30.0));
                let delete = Rect::from_min_size(pos2(card.right() - 14.0 - 30.0, by - 15.0), vec2(30.0, 30.0));
                // In the studio or not: a clear button.
                let pin = crate::settings::Pin::Template(t.name.clone());
                let pinned = app.settings.pinned.contains(&pin);
                let label = if pinned { format!("✓ {}", l("In studio", "En el Studio")) } else { format!("+ {}", l("Add", "Agregar")) };
                let pw = 22.0 + ui.painter().layout_no_wrap(label.clone(), semibold(12.0), TEXT).size().x;
                let pin_r = Rect::from_min_size(pos2(delete.left() - 8.0 - pw, by - 15.0), vec2(pw, 30.0));
                let pr = ui.interact(pin_r, Id::new(("tpl-pin", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(if pinned {
                    t!("In {}. Click to remove it from the studio.", "En {}. Clic para quitarla del Studio.", studio)
                } else {
                    t!("Add to the studio {}", "Agregar al Studio {}", studio)
                });
                ui.painter().rect_filled(pin_r, CornerRadius::same(15), if pinned { Color32::from_rgba_premultiplied(70, 34, 0, 130) } else if pr.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 110) } else { Color32::from_rgba_premultiplied(40, 40, 44, 90) });
                ui.painter().text(pin_r.center(), Align2::CENTER_CENTER, label, semibold(12.0), if pinned { ORANGE } else { TEXT });
                if pr.clicked() {
                    toggle_pin(app, pin);
                }
                if button(ui, apply, ("apply", i), if active { l("Active", "Activa") } else { l("Apply", "Aplicar") }, !active) {
                    app.apply_template(t);
                }
                if button(ui, delete, ("delete", i), "🗑", false) {
                    app.delete_template(&t.name);
                }
            }
        });
    });
    if regroup {
        app.templates = crate::templates::list();
    }
}

/// A push pin, filled when pinned.
pub fn pin_icon(p: &Painter, c: egui::Pos2, color: Color32, filled: bool) {
    let head = egui::Rect::from_center_size(c - vec2(0.0, 3.0), vec2(9.0, 8.0));
    if filled {
        p.rect_filled(head, CornerRadius::same(2), color);
    } else {
        p.rect_stroke(head, CornerRadius::same(2), Stroke::new(1.5, color), StrokeKind::Middle);
    }
    p.line_segment([c + vec2(-6.0, 1.5), c + vec2(6.0, 1.5)], Stroke::new(1.8, color));
    p.line_segment([c + vec2(0.0, 1.5), c + vec2(0.0, 8.0)], Stroke::new(1.6, color));
}

fn toggle_pin(app: &mut App, pin: crate::settings::Pin) {
    if let Some(i) = app.settings.pinned.iter().position(|p| *p == pin) {
        app.settings.pinned.remove(i);
    } else {
        app.settings.pinned.push(pin);
    }
    let _ = app.settings.save();
}

/// Studios: pick one (it becomes the active one), rename or delete it, and
/// edit its scenes in order (templates join from their cards).
fn studios_card(app: &mut App, ui: &mut Ui, rect: Rect) {
    use crate::compose::Scene;
    use crate::settings::Pin;
    let p = ui.painter().clone();
    theme::card(&p, rect);
    p.text(rect.min + vec2(22.0, 18.0), Align2::LEFT_TOP, l("Studios", "Studios"), medium(14.0), TEXT);
    p.text(
        pos2(rect.right() - 22.0, rect.top() + 20.0),
        Align2::RIGHT_TOP,
        l("A studio is a set of scenes for one kind of show. Its scenes appear under the preview.", "Un Studio es un grupo de escenas para un tipo de show. Sus escenas aparecen debajo del preview."),
        body(12.0),
        FAINT,
    );
    // The studios, as pills: click to make one active.
    let mut x = rect.left() + 22.0;
    let names: Vec<String> = app.settings.studios.iter().map(|s| s.name.clone()).collect();
    let mut pick = None;
    for (i, name) in names.iter().enumerate() {
        let w = 32.0 + ui.painter().layout_no_wrap(name.clone(), medium(12.5), TEXT).size().x;
        if x + w > rect.right() - 22.0 {
            break;
        }
        let b = Rect::from_min_size(pos2(x, rect.top() + 48.0), vec2(w, 30.0));
        let sel = i == app.settings.studio;
        let r = ui.interact(b, Id::new(("studio-pill", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        p.rect_filled(b, CornerRadius::same(15), if sel { Color32::WHITE } else if r.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 110) } else { Color32::from_rgba_premultiplied(40, 40, 44, 100) });
        p.text(b.center(), Align2::CENTER_CENTER, name, medium(12.5), if sel { Color32::BLACK } else { TEXT });
        if r.clicked() {
            pick = Some(i);
        }
        x += w + 8.0;
    }
    if let Some(i) = pick {
        app.settings.switch_studio(i);
        let _ = app.settings.save();
    }
    // Rename and delete the active one.
    let row = Rect::from_min_size(pos2(rect.left() + 22.0, rect.top() + 88.0), vec2(rect.width() - 44.0, 30.0));
    let mut delete = false;
    ui.scope_builder(UiBuilder::new().max_rect(row), |ui| {
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new(l("Name", "Nombre")).color(MUTED).size(12.5));
            let cur = app.settings.studio;
            if let Some(s) = app.settings.studios.get_mut(cur)
                && ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(220.0)).lost_focus()
            {
                let _ = app.settings.save();
            }
            if app.settings.studios.len() > 1 && ui.button(l("Delete studio", "Borrar Studio")).clicked() {
                delete = true;
            }
        });
    });
    if delete {
        app.settings.remove_studio(app.settings.studio);
        let _ = app.settings.save();
    }
    let brand = app.settings.brand.clone();
    let pins = app.settings.pinned.clone();
    let mut action: Option<(usize, i32)> = None; // (index, -1 left / +1 right / 0 unpin)
    let mut x = rect.left() + 22.0;
    let y = rect.top() + 130.0;
    if pins.is_empty() {
        p.text(pos2(x, y + 17.0), Align2::LEFT_CENTER, l("No scenes yet: add some below, or from a template's card.", "Aún no tiene escenas: agrega abajo, o desde la tarjeta de una plantilla."), body(12.5), MUTED);
    }
    for (i, pin) in pins.iter().enumerate() {
        let (label, tw) = match pin {
            Pin::Scene(s) => (s.label().to_string(), 22.0),
            Pin::Template(n) => (n.clone(), 34.0),
        };
        let f = medium(12.5);
        let text_w = ui.painter().layout_no_wrap(label.clone(), f.clone(), TEXT).size().x;
        let w = 30.0 + tw + text_w + 70.0;
        if x + w > rect.right() - 22.0 {
            break;
        }
        let chip = Rect::from_min_size(pos2(x, y), vec2(w, 34.0));
        p.rect_filled(chip, CornerRadius::same(17), Color32::from_rgba_premultiplied(40, 40, 44, 100));
        let mut cx = chip.left() + 16.0;
        match pin {
            Pin::Scene(s) => {
                super::ui::scene_icon_pub(&p, *s, pos2(cx + 6.0, chip.center().y), TEXT);
                cx += 22.0;
            }
            Pin::Template(n) => {
                if let Some(t) = app.templates.iter().find(|t| &t.name == n) {
                    super::templates_ui::thumbnail(&p, Rect::from_min_size(pos2(cx - 4.0, chip.center().y - 9.0), vec2(30.0, 18.0)), &t.layout, t.brand.as_ref().unwrap_or(&brand));
                }
                cx += 34.0;
            }
        }
        p.text(pos2(cx, chip.center().y), Align2::LEFT_CENTER, &label, f, TEXT);
        for (k, (glyph, act)) in [("<", -1), (">", 1), ("x", 0)].iter().enumerate() {
            let b = Rect::from_center_size(pos2(chip.right() - 58.0 + k as f32 * 20.0, chip.center().y), vec2(18.0, 22.0));
            let r = ui.interact(b, Id::new(("qa", i, k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(match act {
                -1 => l("Move left", "Mover a la izquierda"),
                1 => l("Move right", "Mover a la derecha"),
                _ => l("Unpin", "Quitar"),
            });
            if r.hovered() {
                p.rect_filled(b, CornerRadius::same(6), Color32::from_rgba_premultiplied(70, 70, 76, 120));
            }
            p.text(b.center(), Align2::CENTER_CENTER, glyph, semibold(12.0), if *act == 0 { RED } else { MUTED });
            if r.clicked() {
                action = Some((i, *act));
            }
        }
        x += w + 8.0;
    }
    // The built-in scenes not pinned yet.
    let mut x = rect.left() + 22.0;
    let y2 = rect.top() + 176.0;
    p.text(pos2(x, y2 + 11.0), Align2::LEFT_CENTER, l("Add a scene:", "Agregar escena:"), body(12.0), MUTED);
    x += 100.0;
    for s in Scene::ALL {
        if pins.contains(&Pin::Scene(s)) {
            continue;
        }
        let w = 40.0 + ui.painter().layout_no_wrap(s.label().to_string(), body(12.0), TEXT).size().x;
        let b = Rect::from_min_size(pos2(x, y2), vec2(w, 24.0));
        let r = ui.interact(b, Id::new(("qa-add", s.label())), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        p.rect_filled(b, CornerRadius::same(12), if r.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 110) } else { Color32::from_rgba_premultiplied(30, 30, 34, 80) });
        p.text(pos2(b.left() + 12.0, b.center().y), Align2::LEFT_CENTER, "+", semibold(13.0), ORANGE);
        p.text(pos2(b.left() + 26.0, b.center().y), Align2::LEFT_CENTER, s.label(), body(12.0), TEXT);
        if r.clicked() {
            toggle_pin(app, Pin::Scene(s));
        }
        x += w + 8.0;
    }
    if let Some((i, act)) = action {
        let v = &mut app.settings.pinned;
        match act {
            0 => {
                v.remove(i);
            }
            -1 if i > 0 => v.swap(i, i - 1),
            1 if i + 1 < v.len() => v.swap(i, i + 1),
            _ => {}
        }
        let _ = app.settings.save();
    }
}

fn button(ui: &mut Ui, r: Rect, id: (&str, usize), label: &str, primary: bool) -> bool {
    let resp = ui.interact(r, Id::new(id), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let fill = if primary { Color32::WHITE } else if resp.hovered() { Color32::from_rgba_premultiplied(60, 60, 66, 90) } else { Color32::from_rgba_premultiplied(40, 40, 44, 80) };
    ui.painter().rect_filled(r, CornerRadius::same(15), fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, semibold(12.5), if primary { Color32::BLACK } else { TEXT });
    resp.clicked()
}
