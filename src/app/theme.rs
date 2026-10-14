//! Chisme's look: Ventura's black, wide display type and rainbow gradient
//! (brand), with Channel Analytics' glass cards and icon sidebar (layout).

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Mesh, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(10, 10, 11);
pub const SIDEBAR: Color32 = Color32::from_rgb(20, 20, 22);
pub const TEXT: Color32 = Color32::from_rgb(242, 242, 244);
pub const MUTED: Color32 = Color32::from_rgb(150, 150, 158);
pub const FAINT: Color32 = Color32::from_rgb(92, 92, 100);
pub const CARD: Color32 = Color32::from_rgba_premultiplied(18, 18, 19, 150);
pub const CARD_EDGE: Color32 = Color32::from_rgba_premultiplied(30, 30, 32, 40);
pub const CONTROL: Color32 = Color32::from_rgba_premultiplied(22, 22, 24, 30);
pub const YELLOW: Color32 = Color32::from_rgb(231, 227, 107);
pub const GREEN: Color32 = Color32::from_rgb(76, 214, 140);

/// Ventura's gradient: red, orange, yellow, blue.
pub const RAINBOW: [Color32; 5] = [
    Color32::from_rgb(214, 38, 46),
    Color32::from_rgb(255, 102, 0),
    Color32::from_rgb(255, 196, 0),
    Color32::from_rgb(120, 190, 255),
    Color32::from_rgb(36, 84, 255),
];
pub const RED: Color32 = RAINBOW[0];
pub const ORANGE: Color32 = RAINBOW[1];

pub const R_CARD: u8 = 24;
pub const R_CONTROL: u8 = 12;

pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display".into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

pub fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "inter", include_bytes!("../../assets/fonts/Inter-Regular.ttf"));
    add(&mut fonts, "inter-medium", include_bytes!("../../assets/fonts/Inter-Medium.ttf"));
    add(&mut fonts, "inter-semibold", include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"));
    add(&mut fonts, "krona", include_bytes!("../../assets/fonts/KronaOne-Regular.ttf"));
    fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "inter".into());
    // egui's own fonts stay as fallbacks for symbols.
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    for (name, first) in [("display", "krona"), ("semibold", "inter-semibold"), ("medium", "inter-medium")] {
        let mut list = vec![first.to_string()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), list);
    }
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.visuals = egui::Visuals::dark();
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.button_padding = Vec2::new(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.slider_width = 150.0;
        let v = &mut s.visuals;
        v.panel_fill = BG;
        v.window_fill = Color32::from_rgb(24, 24, 27);
        v.extreme_bg_color = Color32::from_rgb(16, 16, 18);
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ORANGE.gamma_multiply(0.85);
        v.selection.stroke = Stroke::new(1.0, TEXT);
        v.hyperlink_color = ORANGE;
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open, &mut v.widgets.noninteractive] {
            w.corner_radius = CornerRadius::same(R_CONTROL);
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_EDGE);
        v.widgets.inactive.weak_bg_fill = CONTROL;
        v.widgets.inactive.bg_fill = Color32::from_rgba_premultiplied(40, 40, 44, 60);
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.hovered.weak_bg_fill = Color32::from_rgba_premultiplied(40, 40, 44, 60);
        v.widgets.hovered.bg_fill = Color32::from_rgba_premultiplied(60, 60, 66, 90);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgba_premultiplied(60, 60, 64, 70));
        v.widgets.active.weak_bg_fill = Color32::from_rgba_premultiplied(60, 60, 66, 90);
        v.widgets.active.bg_fill = ORANGE;
        v.popup_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
        v.window_corner_radius = CornerRadius::same(16);
        v.menu_corner_radius = CornerRadius::same(14);
    });
}

/// A glass card.
pub fn card(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, CornerRadius::same(R_CARD), CARD);
    painter.rect_stroke(rect, CornerRadius::same(R_CARD), Stroke::new(1.0, CARD_EDGE), StrokeKind::Inside);
}

/// Color at `t` (0..1) along `stops`.
pub fn lerp_stops(stops: &[Color32], t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let i = (t.floor() as usize).min(stops.len() - 2);
    let f = t - i as f32;
    let (a, b) = (stops[i], stops[i + 1]);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round() as u8;
    Color32::from_rgba_premultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()), mix(a.a(), b.a()))
}

/// A horizontal gradient bar (Ventura's stripe).
pub fn gradient_bar(painter: &Painter, rect: Rect, stops: &[Color32]) {
    let mut mesh = Mesh::default();
    let n = 24;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let x = rect.left() + rect.width() * t;
        let c = lerp_stops(stops, t);
        mesh.colored_vertex(Pos2::new(x, rect.top()), c);
        mesh.colored_vertex(Pos2::new(x, rect.bottom()), c);
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    painter.add(mesh);
}

/// A soft round glow (the warm light behind the dashboard).
pub fn glow(painter: &Painter, center: Pos2, radius: Vec2, color: Color32) {
    let mut mesh = Mesh::default();
    let segs = 64;
    let rings = [(0.0, 1.0), (0.45, 0.55), (0.75, 0.2), (1.0, 0.0)];
    for &(r, a) in &rings {
        for s in 0..segs {
            let ang = s as f32 / segs as f32 * std::f32::consts::TAU;
            let p = center + Vec2::new(ang.cos() * radius.x * r, ang.sin() * radius.y * r);
            mesh.colored_vertex(p, color.gamma_multiply(a));
        }
    }
    for ring in 0..rings.len() as u32 - 1 {
        for s in 0..segs as u32 {
            let (a, b) = (ring * segs as u32 + s, ring * segs as u32 + (s + 1) % segs as u32);
            let (c, d) = (a + segs as u32, b + segs as u32);
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
    painter.add(mesh);
}

/// The app icon (window and tray), the same as the .ico in the executables.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    crate::icon::rgba(size)
}
