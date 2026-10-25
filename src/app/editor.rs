//! The layout editor: select, move and resize layers on the preview (with
//! snapping guides), and the "Capas" card with the layer list and the
//! selected layer's properties.

use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

use super::theme::{self, *};
use super::App;
use crate::i18n::l as tr;
use crate::compose::Shape;
use crate::layout::{self, Layer, Layout, Source, clamp, snap};

/// A move or resize in progress.
#[derive(Clone, Copy)]
pub struct Drag {
    layer: usize,
    /// None: move; Some(corner): 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
    corner: Option<usize>,
    start: [f64; 4],
    from: Pos2,
}

const HANDLE: f32 = 11.0;

fn to_screen(r: [f64; 4], img: Rect) -> Rect {
    Rect::from_min_size(
        pos2(img.left() + r[0] as f32 * img.width(), img.top() + r[1] as f32 * img.height()),
        vec2(r[2] as f32 * img.width(), r[3] as f32 * img.height()),
    )
}

fn corners(r: Rect) -> [Pos2; 4] {
    [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()]
}

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
        }
        self.settings_changed();
        self.page = super::Page::Studio;
    }

    /// Start editing: the current scene as layers.
    pub fn customize_layout(&mut self) {
        // A template becomes the starting point of the custom layout.
        self.settings.template = None;
        if self.settings.layout.is_none() {
            let (w, h) = (self.status.out_size.0.max(16) as f64, self.status.out_size.1.max(9) as f64);
            let cam_aspect = self.status.cam.as_ref().map_or(16.0 / 9.0, |c| c.1.0 as f64 / c.1.1.max(1) as f64);
            self.settings.layout = Some(Layout::from_scene(self.settings.scene, &self.settings.pip(), w, h, cam_aspect, w / h));
        }
        self.sel_layer = None;
        self.settings_changed();
    }
}

/// Mouse editing of the layout on the preview image `img`: the 16:9 one,
/// or (`vertical`) the 9:16 one, which starts from the automatic layout.
pub fn layout_edit(app: &mut App, ui: &mut Ui, img: Rect, vertical: bool) {
    let layout = if vertical {
        app.settings.layout_v.clone().unwrap_or_else(|| app.settings.vertical_layout())
    } else {
        let Some(l) = app.settings.layout.clone() else { return };
        l
    };
    let resp = ui.interact(img, Id::new(("layout-edit", vertical)), Sense::click_and_drag());
    // Selection belongs to one picture at a time.
    if (resp.drag_started() || resp.clicked()) && app.edit_v != vertical {
        app.edit_v = vertical;
        app.sel_layer = None;
        app.drag = None;
    }
    let mine = app.edit_v == vertical;
    let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
    let frac = |p: Pos2| (((p.x - img.left()) / img.width()) as f64, ((p.y - img.top()) / img.height()) as f64);
    let shift = ui.input(|i| i.modifiers.shift);

    // Which handle or layer is under the pointer.
    let handle_at = |p: Pos2, sel: Option<usize>| -> Option<usize> {
        let l = layout.layers.get(sel?)?;
        corners(to_screen(l.rect, img)).iter().position(|c| (c.x - p.x).abs() <= HANDLE && (c.y - p.y).abs() <= HANDLE)
    };
    if let Some(p) = resp.hover_pos() {
        let cursor = match handle_at(p, app.sel_layer.filter(|_| mine)) {
            Some(0 | 3) => CursorIcon::ResizeNwSe,
            Some(_) => CursorIcon::ResizeNeSw,
            None if layout.hit(frac(p).0, frac(p).1).is_some() => CursorIcon::Grab,
            None => CursorIcon::Default,
        };
        ui.ctx().set_cursor_icon(cursor);
    }

    if resp.drag_started()
        && let Some(p) = pointer
    {
        let corner = if mine { handle_at(p, app.sel_layer) } else { None };
        let target = if corner.is_some() { app.sel_layer } else { layout.hit(frac(p).0, frac(p).1) };
        app.sel_layer = target;
        app.drag = target.map(|i| Drag { layer: i, corner, start: layout.layers[i].rect, from: p });
    }
    if resp.clicked()
        && let Some(p) = pointer
    {
        app.sel_layer = layout.hit(frac(p).0, frac(p).1);
    }
    app.guides = (None, None);
    if resp.dragged()
        && let (Some(d), Some(p)) = (app.drag, pointer)
    {
        // Resizing shows the diagonal arrow of the corner held; moving, a fist.
        ui.ctx().set_cursor_icon(match d.corner {
            Some(0 | 3) => CursorIcon::ResizeNwSe,
            Some(_) => CursorIcon::ResizeNeSw,
            None => CursorIcon::Grabbing,
        });
        let (dx, dy) = (((p.x - d.from.x) / img.width()) as f64, ((p.y - d.from.y) / img.height()) as f64);
        let [x, y, w, h] = d.start;
        let mut r = match d.corner {
            None => [x + dx, y + dy, w, h],
            Some(c) => {
                let (left, top) = (c == 0 || c == 2, c == 0 || c == 1);
                let mut nw = if left { w - dx } else { w + dx };
                let mut nh = if top { h - dy } else { h + dy };
                if shift {
                    // Keep the shape: follow the larger change.
                    let k = (nw / w).max(nh / h);
                    nw = w * k;
                    nh = h * k;
                }
                let nx = if left { x + w - nw } else { x };
                let ny = if top { y + h - nh } else { y };
                [nx, ny, nw, nh]
            }
        };
        if d.corner.is_none() {
            let others: Vec<[f64; 4]> = layout.layers.iter().enumerate().filter(|(i, l)| *i != d.layer && l.visible).map(|(_, l)| l.rect).collect();
            let s = snap(r, &others, 0.012);
            r[0] += s.dx;
            r[1] += s.dy;
            app.guides = (s.vx, s.hy);
        }
        let target = if vertical {
            let auto = app.settings.vertical_layout();
            Some(app.settings.layout_v.get_or_insert(auto))
        } else {
            app.settings.layout.as_mut()
        };
        if let Some(l) = target.and_then(|l| l.layers.get_mut(d.layer)) {
            l.rect = clamp(r);
        }
        app.layout_live();
    }
    if resp.drag_stopped() {
        app.drag = None;
        app.settings_changed();
    }

    // Selection outline, handles and guides.
    let painter = ui.painter_at(img);
    if let Some(l) = app.sel_layer.filter(|_| app.edit_v == vertical).and_then(|i| layout.layers.get(i)) {
        let r = to_screen(l.rect, img);
        painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.5, ORANGE), StrokeKind::Outside);
        for c in corners(r) {
            let h = Rect::from_center_size(c, Vec2::splat(HANDLE));
            painter.rect_filled(h, CornerRadius::same(3), Color32::WHITE);
            painter.rect_stroke(h, CornerRadius::same(3), Stroke::new(1.5, ORANGE), StrokeKind::Inside);
        }
        painter.text(r.left_top() + vec2(4.0, -16.0), Align2::LEFT_TOP, l.source.label(), semibold(11.0), ORANGE);
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


/// The "Capas" card (in place of the camera card while a layout is custom).
pub fn layers_card(app: &mut App, ui: &mut Ui, card: Rect) {
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
    let inner = Rect::from_min_max(pos2(card.left() + 18.0, card.top() + 50.0), pos2(card.right() - 18.0, card.bottom() - 12.0));
    let cam_aspect = app.status.cam.as_ref().map_or(16.0 / 9.0, |c| c.1.0 as f64 / c.1.1.max(1) as f64);
    let (ow, oh) = (app.status.out_size.0.max(16) as f64, app.status.out_size.1.max(9) as f64);
    let has_cam = app.status.cam.is_some();
    let pip = app.settings.pip();
    let mut changed = false;
    let mut save = false;
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let Some(layout) = app.settings.layout.as_mut() else { return };
            // Add layers, and save the scene.
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr("+ Screen", "+ Pantalla")).clicked() {
                    layout.layers.push(Layer { source: Source::Screen, rect: [0.1, 0.1, 0.5, 0.5], fill: false, ..Default::default() });
                    app.sel_layer = Some(layout.layers.len() - 1);
                    changed = true;
                }
                if ui.add_enabled(has_cam, egui::Button::new(tr("+ Camera", "+ Cámara"))).on_disabled_hover_text(tr("Pick a camera in Settings", "Elige una cámara en Ajustes")).clicked() {
                    layout.layers.push(layout::new_camera(&pip, ow, oh, cam_aspect));
                    app.sel_layer = Some(layout.layers.len() - 1);
                    changed = true;
                }
                if ui.button(tr("+ Image", "+ Imagen")).on_hover_text(tr("PNG or JPG: logo, frame, background…", "PNG o JPG: logo, marco, fondo…")).clicked()
                    && let Some(path) = pick_image()
                {
                    let aspect = image::image_dimensions(&path).map_or(1.0, |(w, h)| w as f64 / h.max(1) as f64);
                    layout.layers.push(Layer::image(&path.to_string_lossy(), aspect, ow / oh));
                    app.sel_layer = Some(layout.layers.len() - 1);
                    changed = true;
                }
                if ui.button(tr("+ Guest", "+ Invitado")).on_hover_text(tr("A guest from the Guests page, once on stage", "Un invitado de la página Invitados, cuando está en vivo")).clicked() {
                    let seat = layout.layers.iter().filter(|l| l.source == Source::Guest).count().min(7) as u8;
                    layout.layers.push(Layer { source: Source::Guest, guest: seat, rect: [0.55, 0.3, 0.4, 0.4], fill: true, roundness: 0.06, ..Default::default() });
                    app.sel_layer = Some(layout.layers.len() - 1);
                    changed = true;
                }
                if ui.button(tr("+ Text", "+ Texto")).on_hover_text(tr("Lower third: name and title", "Lower third: nombre y título")).clicked() {
                    layout.layers.push(Layer::lower_third(tr("Your name", "Tu nombre"), tr("Your title", "Tu título"), 1));
                    app.sel_layer = Some(layout.layers.len() - 1);
                    changed = true;
                }
                if ui.add(egui::Button::new(egui::RichText::new(tr("Save scene…", "Guardar escena…")).strong().color(Color32::BLACK)).fill(Color32::WHITE)).on_hover_text(tr("Save this scene as a template", "Guardar esta escena como plantilla")).clicked() {
                    save = true;
                }
            });
            ui.add_space(2.0);
            if has_cam && super::ui::vbg_row(ui, &mut app.settings.vbg, 180.0) {
                changed = true;
            }
            // Background: a color (the brand's first) or your own picture.
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(tr("Background", "Fondo")).color(MUTED).size(13.0));
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
            ui.add_space(4.0);
            // The list, top layer first.
            let n = layout.layers.len();
            let mut action: Option<(usize, i32)> = None; // (layer, -1 down / +1 up / 0 delete)
            for i in (0..n).rev() {
                let sel = app.sel_layer == Some(i);
                let l = &mut layout.layers[i];
                let frame = egui::Frame::new()
                    .fill(if sel { Color32::from_rgba_premultiplied(70, 34, 0, 90) } else { Color32::from_rgba_premultiplied(30, 30, 33, 45) })
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
                        if name.chars().count() > 26 {
                            name = format!("{}…", name.chars().take(25).collect::<String>());
                        }
                        let label = ui.add(egui::Label::new(egui::RichText::new(name).size(13.0).color(if l.visible { TEXT } else { FAINT })).sense(Sense::click()));
                        if label.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            app.sel_layer = Some(i);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button(ui, Icon::Remove, tr("Delete layer", "Eliminar capa")) {
                                action = Some((i, 0));
                            }
                            if icon_button(ui, Icon::Down, tr("Send backward", "Enviar atrás")) {
                                action = Some((i, -1));
                            }
                            if icon_button(ui, Icon::Up, tr("Bring forward", "Traer al frente")) {
                                action = Some((i, 1));
                            }
                        });
                    });
                });
            }
            if let Some((i, a)) = action {
                match a {
                    0 => {
                        layout.layers.remove(i);
                        app.sel_layer = None;
                    }
                    1 if i + 1 < n => {
                        layout.layers.swap(i, i + 1);
                        app.sel_layer = Some(i + 1);
                    }
                    -1 if i > 0 => {
                        layout.layers.swap(i, i - 1);
                        app.sel_layer = Some(i - 1);
                    }
                    _ => {}
                }
                changed = true;
            }
            // The selected layer's properties.
            if let Some(l) = app.sel_layer.and_then(|i| layout.layers.get_mut(i)) {
                ui.add_space(6.0);
                ui.separator();
                match l.source {
                    Source::Text => {
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
                            for i in 0..crate::brand::STYLES.len() {
                                if ui.selectable_label(l.style == i as u8, crate::brand::style_name(i as u8)).clicked() {
                                    l.style = i as u8;
                                    changed = true;
                                }
                            }
                        });
                        ui.label(egui::RichText::new(tr("Colors and font: Settings → Brand.", "Colores y tipografía: Ajustes → Marca.")).color(FAINT).size(11.5));
                        return;
                    }
                    Source::Guest => {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(tr("Seat", "Lugar")).color(MUTED).size(13.0));
                            for s in 0..4u8 {
                                changed |= ui.selectable_value(&mut l.guest, s, format!("{}", s + 1)).on_hover_text(tr("Who was brought on stage 1st, 2nd…", "Quién subió 1.º, 2.º…")).changed();
                            }
                        });
                    }
                    Source::Image => {
                        ui.horizontal(|ui| {
                            if ui.button(tr("Change image…", "Cambiar imagen…")).clicked()
                                && let Some(path) = pick_image()
                            {
                                l.image = Some(path.to_string_lossy().into_owned());
                                changed = true;
                            }
                        });
                    }
                    _ => {}
                }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr("Picture", "Imagen")).color(MUTED).size(13.0));
                    changed |= ui.selectable_value(&mut l.fill, true, tr("Fill", "Llenar")).changed();
                    changed |= ui.selectable_value(&mut l.fill, false, tr("Fit", "Ajustar")).changed();
                });
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tr("Shape", "Forma")).color(MUTED).size(13.0));
                    for s in [Shape::Rect, Shape::Rounded, Shape::Circle, Shape::Pill, Shape::Squircle] {
                        let (r, resp) = ui.allocate_exact_size(vec2(36.0, 28.0), Sense::click());
                        let sel = l.shape == s;
                        let fill = if sel { Color32::from_rgba_premultiplied(62, 62, 68, 120) } else if resp.hovered() { CONTROL } else { Color32::TRANSPARENT };
                        ui.painter().rect_filled(r, CornerRadius::same(8), fill);
                        super::ui::shape_icon(ui.painter(), s, r.center(), if sel { TEXT } else { MUTED });
                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            l.shape = s;
                            changed = true;
                        }
                    }
                });
                if matches!(l.shape, Shape::Rounded | Shape::Squircle | Shape::Square) {
                    changed |= ui.add(egui::Slider::new(&mut l.roundness, 0.0..=1.0).text(tr("Roundness", "Redondez"))).changed();
                }
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
            } else {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tr("Click a layer on the preview to move and resize it. Shift keeps its proportions.", "Haz clic en una capa del preview para moverla y cambiar su tamaño. Shift mantiene la proporción.")).color(FAINT).size(12.0));
            }
        });
    });
    if changed {
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
