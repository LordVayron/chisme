//! The brand kit (logo, colors, font) and lower thirds: name + title boxes
//! drawn on the CPU into an RGBA image (straight alpha), sized to their
//! layer, in five styles.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BrandFont {
    /// Inter: clean and neutral.
    Inter,
    /// Krona One: wide display type (Ventura-like).
    Krona,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Brand {
    /// Path to the logo image (PNG with transparency works best).
    pub logo: Option<String>,
    pub primary: [u8; 3],
    pub secondary: [u8; 3],
    pub font: BrandFont,
}

impl Default for Brand {
    fn default() -> Self {
        // Ventura's orange and a near-black.
        Brand { logo: None, primary: [255, 102, 0], secondary: [18, 18, 20], font: BrandFont::Inter }
    }
}

/// Lower third styles.
pub const STYLES: [&str; 5] = ["Simple", "Bloque", "Pastilla", "Barra", "Display"];

/// A lower-third style's name in the interface language.
pub fn style_name(i: u8) -> &'static str {
    use crate::i18n::l;
    match i {
        1 => l("Block", "Bloque"),
        2 => l("Pill", "Pastilla"),
        3 => l("Bar", "Barra"),
        4 => "Display",
        _ => "Simple",
    }
}

static INTER: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
static INTER_REG: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
static KRONA: &[u8] = include_bytes!("../assets/fonts/KronaOne-Regular.ttf");

/// An RGBA canvas (straight alpha) with simple anti-aliased drawing.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, px: vec![0; w * h * 4] }
    }

    /// Blend `color` with coverage `a` (0..1) over the pixel ("over" in
    /// straight alpha).
    fn blend(&mut self, x: usize, y: usize, color: [u8; 3], a: f32) {
        if x >= self.w || y >= self.h || a <= 0.0 {
            return;
        }
        let p = (y * self.w + x) * 4;
        let da = self.px[p + 3] as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        if oa <= 0.0 {
            return;
        }
        for k in 0..3 {
            let c = (color[k] as f32 * a + self.px[p + k] as f32 * da * (1.0 - a)) / oa;
            self.px[p + k] = c.round().clamp(0.0, 255.0) as u8;
        }
        self.px[p + 3] = (oa * 255.0).round() as u8;
    }

    /// A rounded rectangle (radius `r`), anti-aliased, with `alpha`.
    pub fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: [u8; 3], alpha: f32) {
        let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
        let (x0, y0) = (x.floor().max(0.0) as usize, y.floor().max(0.0) as usize);
        let (x1, y1) = (((x + w).ceil() as usize).min(self.w), ((y + h).ceil() as usize).min(self.h));
        for py in y0..y1 {
            for px in x0..x1 {
                let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
                // Signed distance to the rounded box.
                let qx = (cx - (x + w / 2.0)).abs() - (w / 2.0 - r);
                let qy = (cy - (y + h / 2.0)).abs() - (h / 2.0 - r);
                let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                self.blend(px, py, color, cov * alpha);
            }
        }
    }

    /// Text with its baseline at `y`; returns its width.
    pub fn text(&mut self, font: &FontRef, size: f32, x: f32, y: f32, s: &str, color: [u8; 3], alpha: f32) -> f32 {
        let scale = PxScale::from(size);
        let sf = font.as_scaled(scale);
        let mut pen = x;
        let mut prev = None;
        for ch in s.chars() {
            let id = font.glyph_id(ch);
            if let Some(p) = prev {
                pen += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(scale, point(pen, y));
            if let Some(og) = font.outline_glyph(g) {
                let b = og.px_bounds();
                og.draw(|gx, gy, c| {
                    let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    if px >= 0 && py >= 0 {
                        self.blend(px as usize, py as usize, color, c * alpha);
                    }
                });
            }
            pen += sf.h_advance(id);
            prev = Some(id);
        }
        pen - x
    }
}

/// Width of `s` at `size` without drawing.
pub fn measure(font: &FontRef, size: f32, s: &str) -> f32 {
    let sf = font.as_scaled(PxScale::from(size));
    let mut w = 0.0;
    let mut prev = None;
    for ch in s.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

/// The largest font size (up to `max`) at which `s` fits in `width`.
fn fit_size(font: &FontRef, s: &str, max: f32, width: f32) -> f32 {
    let w = measure(font, max, s);
    if w <= width || w <= 0.0 { max } else { (max * width / w).max(6.0) }
}

fn lum(c: [u8; 3]) -> f32 {
    0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
}

/// Black or white, whichever reads better on `bg`.
fn on(bg: [u8; 3]) -> [u8; 3] {
    if lum(bg) > 150.0 { [12, 12, 14] } else { [255, 255, 255] }
}

/// Draw a lower third (`name`, optional `title`) in `style` (0..5) into a
/// `w`x`h` RGBA image, left-aligned.
pub fn lower_third(name: &str, title: &str, style: u8, brand: &Brand, w: usize, h: usize) -> Canvas {
    let mut c = Canvas::new(w.max(8), h.max(8));
    let (wf, hf) = (c.w as f32, c.h as f32);
    let bold = FontRef::try_from_slice(if brand.font == BrandFont::Krona { KRONA } else { INTER }).expect("bundled font");
    let regular = FontRef::try_from_slice(INTER_REG).expect("bundled font");
    let has_title = !title.trim().is_empty();
    let pad = hf * 0.12;
    let (p, s) = (brand.primary, brand.secondary);
    match style % 5 {
        // Simple: text with a soft shadow, no box.
        0 => {
            let ns = fit_size(&bold, name, hf * if has_title { 0.42 } else { 0.6 }, wf - 2.0 * pad);
            let base = if has_title { hf * 0.5 } else { hf * 0.68 };
            for (dx, dy) in [(2.0, 2.0), (1.0, 1.0)] {
                c.text(&bold, ns, pad + dx, base + dy, name, [0, 0, 0], 0.45);
            }
            c.text(&bold, ns, pad, base, name, [255, 255, 255], 1.0);
            if has_title {
                let ts = fit_size(&regular, title, hf * 0.24, wf - 2.0 * pad);
                c.text(&regular, ts, pad + 1.0, hf * 0.82 + 1.0, title, [0, 0, 0], 0.45);
                c.text(&regular, ts, pad, hf * 0.82, title, [235, 235, 238], 1.0);
            }
        }
        // Bloque: the name on a primary box, the title on a secondary one below.
        1 => {
            let nh = if has_title { hf * 0.58 } else { hf };
            let ns = fit_size(&bold, name, nh * 0.6, wf - 2.0 * pad);
            let nw = (measure(&bold, ns, name) + 2.0 * pad).min(wf);
            c.rrect(0.0, 0.0, nw, nh, hf * 0.06, p, 1.0);
            c.text(&bold, ns, pad, nh * 0.72, name, on(p), 1.0);
            if has_title {
                let th = hf - nh;
                let ts = fit_size(&regular, title, th * 0.6, wf - 2.0 * pad);
                let tw = (measure(&regular, ts, title) + 2.0 * pad).min(wf);
                c.rrect(0.0, nh, tw, th, hf * 0.04, s, 1.0);
                c.text(&regular, ts, pad, nh + th * 0.72, title, on(s), 1.0);
            }
        }
        // Pastilla: one rounded pill, "Name · title".
        2 => {
            let ph = hf * 0.7;
            let y = (hf - ph) / 2.0;
            let label = if has_title { format!("{name}  ·  {title}") } else { name.to_string() };
            let ns = fit_size(&bold, &label, ph * 0.5, wf - 2.0 * ph * 0.5);
            let pw = (measure(&bold, ns, &label) + ph).min(wf);
            c.rrect(0.0, y, pw, ph, ph / 2.0, [255, 255, 255], 1.0);
            c.text(&bold, ns, ph * 0.5, y + ph * 0.68, &label, [12, 12, 14], 1.0);
        }
        // Barra: a dark box with a rainbow accent bar, Ventura-style.
        3 => {
            let bar = (hf * 0.06).max(3.0);
            let ns = fit_size(&bold, name, hf * if has_title { 0.38 } else { 0.5 }, wf - 3.0 * pad - bar);
            let ts = fit_size(&regular, title, hf * 0.22, wf - 3.0 * pad - bar);
            let tw = measure(&bold, ns, name).max(if has_title { measure(&regular, ts, title) } else { 0.0 });
            let bw = (tw + 2.0 * pad + bar + pad * 0.5).min(wf);
            c.rrect(0.0, 0.0, bw, hf, hf * 0.08, s, 0.88);
            let stops = crate::icon::RAINBOW;
            for yy in 0..c.h {
                let col = crate::icon::lerp(&stops, yy as f32 / hf);
                for xx in 0..bar as usize {
                    c.blend(xx + (pad * 0.5) as usize, yy, col, if yy as f32 > hf * 0.1 && (yy as f32) < hf * 0.9 { 1.0 } else { 0.0 });
                }
            }
            let x = pad + bar + pad * 0.3;
            c.text(&bold, ns, x, if has_title { hf * 0.5 } else { hf * 0.66 }, name, [255, 255, 255], 1.0);
            if has_title {
                c.text(&regular, ts, x, hf * 0.8, title, p, 1.0);
            }
        }
        // Display: the wide display font, uppercase, with a primary underline.
        _ => {
            let krona = FontRef::try_from_slice(KRONA).expect("bundled font");
            let upper = name.to_uppercase();
            let ns = fit_size(&krona, &upper, hf * if has_title { 0.36 } else { 0.5 }, wf - 2.0 * pad);
            let base = if has_title { hf * 0.46 } else { hf * 0.62 };
            let nw = c.text(&krona, ns, pad, base, &upper, [255, 255, 255], 1.0);
            c.rrect(pad, base + hf * 0.07, nw.min(wf - 2.0 * pad), (hf * 0.04).max(2.0), 1.0, p, 1.0);
            if has_title {
                let ts = fit_size(&regular, title, hf * 0.22, wf - 2.0 * pad);
                c.text(&regular, ts, pad, hf * 0.86, title, [225, 225, 230], 1.0);
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage(c: &Canvas) -> f32 {
        c.px.chunks(4).filter(|p| p[3] > 0).count() as f32 / (c.w * c.h) as f32
    }

    #[test]
    fn every_style_draws_inside_its_box() {
        let b = Brand::default();
        for style in 0..5 {
            let c = lower_third("Ana García", "Fundador · Chisme", style, &b, 600, 120);
            assert_eq!(c.px.len(), 600 * 120 * 4);
            let cov = coverage(&c);
            assert!(cov > 0.02 && cov < 0.98, "style {style}: {cov}");
        }
        // No title still draws.
        assert!(coverage(&lower_third("Roni", "", 1, &b, 300, 80)) > 0.05);
    }

    /// CHISME_LT_OUT=file.png cargo test --release --lib lower_third_sheet -- --ignored
    #[test]
    #[ignore]
    fn lower_third_sheet() {
        let b = Brand::default();
        let (w, h) = (640usize, 110usize);
        let mut sheet = vec![70u8; w * h * 5 * 4];
        for s in 0..5u8 {
            let c = lower_third("Ana García", "Fundador de Chisme", s, &b, w, h);
            for (i, p) in c.px.chunks(4).enumerate() {
                let o = (s as usize * w * h + i) * 4;
                let a = p[3] as f32 / 255.0;
                for k in 0..3 {
                    sheet[o + k] = (p[k] as f32 * a + 70.0 * (1.0 - a)) as u8;
                }
                sheet[o + 3] = 255;
            }
        }
        image::save_buffer(std::env::var("CHISME_LT_OUT").unwrap(), &sheet, w as u32, (h * 5) as u32, image::ColorType::Rgba8).unwrap();
    }

    #[test]
    fn long_names_shrink_to_fit() {
        let f = FontRef::try_from_slice(INTER).unwrap();
        let s = fit_size(&f, "Un nombre larguísimo para un lower third", 60.0, 200.0);
        assert!(measure(&f, s, "Un nombre larguísimo para un lower third") <= 200.5);
        assert_eq!(fit_size(&f, "Ok", 40.0, 500.0), 40.0);
    }

    #[test]
    fn brand_colors_are_used() {
        let b = Brand { primary: [0, 200, 0], ..Default::default() };
        let c = lower_third("Ana", "", 1, &b, 300, 80);
        // The block's corner area is the primary color.
        let p = (20 * 300 + 4) * 4;
        assert_eq!(&c.px[p..p + 4], &[0, 200, 0, 255]);
        assert_eq!(on([255, 255, 255]), [12, 12, 14]);
        assert_eq!(on([20, 20, 20]), [255, 255, 255]);
    }
}
