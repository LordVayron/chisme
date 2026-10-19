//! Custom layouts: a background and a stack of layers (the screen, the
//! camera…), each placed anywhere on the frame. Positions are fractions of
//! the frame, so a layout works at any resolution. The fixed scenes become
//! starting points (`Layout::from_scene`).

use serde::{Deserialize, Serialize};

use crate::compose::{PipSpec, Place, Scene, Shape, overlay_rect, screen_box, side_by_side};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Screen,
    Camera,
    /// A picture file (`Layer::image`): logo, frame, background.
    Image,
    /// A lower third (`Layer::text`, `subtitle`, `style`).
    Text,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Screen => crate::i18n::l("Screen", "Pantalla"),
            Source::Camera => crate::i18n::l("Camera", "Cámara"),
            Source::Image => crate::i18n::l("Image", "Imagen"),
            Source::Text => crate::i18n::l("Text", "Texto"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    pub source: Source,
    /// `[x, y, w, h]` as fractions of the frame (0..1), from the top-left.
    pub rect: [f64; 4],
    /// Fill the box (cropping the picture) instead of fitting inside it.
    pub fill: bool,
    pub shape: Shape,
    pub roundness: f64,
    /// Border width in pixels at 1080p.
    pub border: f64,
    pub border_color: [u8; 3],
    pub shadow: bool,
    /// Flip horizontally (cameras).
    pub mirror: bool,
    pub visible: bool,
    /// Image layers: the file.
    pub image: Option<String>,
    /// Text layers: name, title and style (`brand::STYLES`).
    pub text: String,
    pub subtitle: String,
    pub style: u8,
}

impl Default for Layer {
    fn default() -> Self {
        Layer {
            source: Source::Screen,
            rect: [0.0, 0.0, 1.0, 1.0],
            fill: true,
            shape: Shape::Rect,
            roundness: 0.2,
            border: 0.0,
            border_color: [255, 255, 255],
            shadow: false,
            mirror: false,
            visible: true,
            image: None,
            text: String::new(),
            subtitle: String::new(),
            style: 1,
        }
    }
}

impl Layer {
    /// An image layer: fitted, in the top-right corner, 18% wide.
    pub fn image(path: &str, aspect: f64, frame_aspect: f64) -> Layer {
        let w = 0.18;
        let h = (w * frame_aspect / aspect.max(0.05)).min(0.9);
        Layer { source: Source::Image, rect: [1.0 - w - 0.02, 0.03, w, h], fill: false, image: Some(path.into()), ..Default::default() }
    }

    /// A lower third in the bottom-left area.
    pub fn lower_third(name: &str, title: &str, style: u8) -> Layer {
        Layer { source: Source::Text, rect: [0.04, 0.76, 0.42, 0.12], text: name.into(), subtitle: title.into(), style, ..Default::default() }
    }

    /// What the layer list shows for it.
    pub fn name(&self) -> String {
        match self.source {
            Source::Image => {
                let f = self.image.as_deref().map(|p| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                format!("{} · {}", Source::Image.label(), f.unwrap_or_default())
            }
            Source::Text => format!("{} · {}", Source::Text.label(), self.text),
            s => format!("{}  {}", s.label(), if self.fill { crate::i18n::l("· fill", "· llenar") } else { crate::i18n::l("· fit", "· ajustar") }),
        }
    }

    /// The overlay style (shape, border, shadow…) for drawing this layer.
    pub fn style(&self) -> PipSpec {
        PipSpec {
            shape: self.shape,
            roundness: self.roundness,
            border: self.border,
            border_color: self.border_color,
            shadow: self.shadow,
            mirror: self.mirror && self.source == Source::Camera,
            ..Default::default()
        }
    }

    /// Pixel rect on a `w`x`h` frame.
    pub fn px(&self, w: f64, h: f64) -> [f64; 4] {
        let [x, y, rw, rh] = self.rect;
        [(x * w).round(), (y * h).round(), (rw * w).round().max(2.0), (rh * h).round().max(2.0)]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub background: [u8; 3],
    /// A picture behind every layer (cropped to fill), over the color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_image: Option<String>,
    /// Bottom to top.
    pub layers: Vec<Layer>,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { background: [0, 0, 0], background_image: None, layers: vec![Layer::default()] }
    }
}

fn frac(r: [f64; 4], w: f64, h: f64) -> [f64; 4] {
    [r[0] / w, r[1] / h, r[2] / w, r[3] / h]
}

impl Layout {
    /// The same picture as a fixed scene, as editable layers. `w`x`h`: the
    /// output size; aspects of the camera and screen pictures.
    pub fn from_scene(scene: Scene, pip: &PipSpec, w: f64, h: f64, cam_aspect: f64, screen_aspect: f64) -> Layout {
        let cam = |rect: [f64; 4], full: bool| Layer {
            source: Source::Camera,
            rect,
            fill: true,
            shape: if full { Shape::Rect } else { pip.shape },
            roundness: pip.roundness,
            border: if full { 0.0 } else { pip.border },
            border_color: pip.border_color,
            shadow: !full && pip.shadow,
            mirror: pip.mirror,
            ..Default::default()
        };
        let screen = |rect: [f64; 4]| Layer { source: Source::Screen, rect, fill: false, ..Default::default() };
        let full = [0.0, 0.0, 1.0, 1.0];
        let layers = match scene {
            Scene::Screen => vec![screen(full)],
            Scene::ScreenCam => vec![screen(full), cam(frac(overlay_rect(pip, w, h, cam_aspect), w, h), false)],
            Scene::SideBySide => {
                let (s, c) = side_by_side(w, h);
                let mut c = cam(frac(c, w, h), false);
                if c.shape != Shape::Rect {
                    c.shape = Shape::Rounded;
                }
                vec![screen(frac(s, w, h)), c]
            }
            Scene::Cam => vec![cam(full, true)],
            Scene::CamScreen => {
                let mut s = screen(frac(screen_box(pip, w, h, screen_aspect), w, h));
                s.shape = Shape::Rounded;
                s.roundness = 0.06;
                s.shadow = pip.shadow;
                vec![cam(full, true), s]
            }
        };
        Layout { background: [0, 0, 0], background_image: None, layers }
    }

    /// The topmost visible layer under the point (fractions), if any.
    pub fn hit(&self, x: f64, y: f64) -> Option<usize> {
        self.layers.iter().enumerate().rev().find(|(_, l)| l.visible && x >= l.rect[0] && x <= l.rect[0] + l.rect[2] && y >= l.rect[1] && y <= l.rect[1] + l.rect[3]).map(|(i, _)| i)
    }
}

/// The part of a `src` picture to show in a box of `aspect` (w/h) when
/// filling it: a centered crop, `(x, y, w, h)` in source pixels.
pub fn crop_to(src: (u32, u32), aspect: f64) -> (u32, u32, u32, u32) {
    let (sw, sh) = (src.0 as f64, src.1 as f64);
    if sw / sh > aspect {
        let w = (sh * aspect).round().max(1.0);
        (((sw - w) / 2.0) as u32, 0, w as u32, src.1)
    } else {
        let h = (sw / aspect).round().max(1.0);
        (0, ((sh - h) / 2.0) as u32, src.0, h as u32)
    }
}

/// The largest box with `aspect` centered in `rect` (pixels) — a layer in
/// "fit" mode shrinks to its picture's shape.
pub fn fit_in(rect: [f64; 4], aspect: f64) -> [f64; 4] {
    let [x, y, w, h] = rect;
    if w / h > aspect {
        let nw = (h * aspect).round();
        [x + ((w - nw) / 2.0).round(), y, nw, h]
    } else {
        let nh = (w / aspect).round();
        [x, y + ((h - nh) / 2.0).round(), w, nh]
    }
}

/// Snap guides: positions a moving box's edges and center can snap to.
#[derive(Debug, Default, PartialEq)]
pub struct Snap {
    pub dx: f64,
    pub dy: f64,
    /// Guide lines to draw: vertical at x, horizontal at y (fractions).
    pub vx: Option<f64>,
    pub hy: Option<f64>,
}

/// Snap `rect` (fractions) to the frame's edges and center and to the
/// other layers' edges and centers, within `tol`.
pub fn snap(rect: [f64; 4], others: &[[f64; 4]], tol: f64) -> Snap {
    let [x, y, w, h] = rect;
    let mut xs = vec![0.0, 0.5, 1.0];
    let mut ys = vec![0.0, 0.5, 1.0];
    for o in others {
        xs.extend([o[0], o[0] + o[2] / 2.0, o[0] + o[2]]);
        ys.extend([o[1], o[1] + o[3] / 2.0, o[1] + o[3]]);
    }
    let best = |edges: [f64; 3], targets: &[f64]| -> Option<(f64, f64)> {
        let mut b: Option<(f64, f64)> = None;
        for e in edges {
            for &t in targets {
                let d = t - e;
                if d.abs() <= tol && b.is_none_or(|(bd, _)| d.abs() < bd.abs()) {
                    b = Some((d, t));
                }
            }
        }
        b
    };
    let mut s = Snap::default();
    if let Some((d, t)) = best([x, x + w / 2.0, x + w], &xs) {
        s.dx = d;
        s.vx = Some(t);
    }
    if let Some((d, t)) = best([y, y + h / 2.0, y + h], &ys) {
        s.dy = d;
        s.hy = Some(t);
    }
    s
}

/// Keep a box inside the frame and at least 4% of it in size.
pub fn clamp(rect: [f64; 4]) -> [f64; 4] {
    let w = rect[2].clamp(0.04, 1.0);
    let h = rect[3].clamp(0.04, 1.0);
    [rect[0].clamp(0.0, 1.0 - w), rect[1].clamp(0.0, 1.0 - h), w, h]
}

/// A new camera layer in the bottom-right corner (like the overlay).
pub fn new_camera(pip: &PipSpec, w: f64, h: f64, cam_aspect: f64) -> Layer {
    let p = PipSpec { place: Place::Corner(crate::compose::Corner::BottomRight), ..pip.clone() };
    Layer {
        source: Source::Camera,
        rect: frac(overlay_rect(&p, w, h, cam_aspect), w, h),
        shape: pip.shape,
        roundness: pip.roundness,
        border: pip.border,
        border_color: pip.border_color,
        shadow: pip.shadow,
        mirror: pip.mirror,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenes_become_layers() {
        let pip = PipSpec::default();
        let l = Layout::from_scene(Scene::ScreenCam, &pip, 1920.0, 1080.0, 16.0 / 9.0, 16.0 / 9.0);
        assert_eq!(l.layers.len(), 2);
        assert_eq!(l.layers[0].source, Source::Screen);
        assert_eq!(l.layers[1].px(1920.0, 1080.0), [1460.0, 805.0, 422.0, 237.0], "same place as the overlay");
        let l = Layout::from_scene(Scene::Cam, &pip, 1920.0, 1080.0, 1.0, 1.0);
        assert_eq!((l.layers[0].source, l.layers[0].rect), (Source::Camera, [0.0, 0.0, 1.0, 1.0]));
        for s in Scene::ALL {
            assert!(!Layout::from_scene(s, &pip, 1920.0, 1080.0, 1.5, 1.7).layers.is_empty());
        }
    }

    #[test]
    fn hit_picks_the_top_layer() {
        let pip = PipSpec::default();
        let l = Layout::from_scene(Scene::ScreenCam, &pip, 1920.0, 1080.0, 16.0 / 9.0, 16.0 / 9.0);
        assert_eq!(l.hit(0.9, 0.9), Some(1), "the camera is on top");
        assert_eq!(l.hit(0.1, 0.1), Some(0));
        let mut hidden = l.clone();
        hidden.layers[1].visible = false;
        assert_eq!(hidden.hit(0.9, 0.9), Some(0));
    }

    #[test]
    fn fill_crop_and_fit() {
        assert_eq!(crop_to((1920, 1080), 1.0), (420, 0, 1080, 1080));
        assert_eq!(crop_to((1080, 1920), 16.0 / 9.0), (0, 656, 1080, 608));
        assert_eq!(fit_in([0.0, 0.0, 1000.0, 1000.0], 16.0 / 9.0), [0.0, 219.0, 1000.0, 563.0]);
    }

    #[test]
    fn snapping() {
        // Near the frame's center line: snaps by its center.
        let s = snap([0.395, 0.1, 0.2, 0.2], &[], 0.01);
        assert!((s.dx - 0.005).abs() < 1e-9 && s.vx == Some(0.5));
        // Near another layer's right edge.
        let s = snap([0.705, 0.3, 0.1, 0.1], &[[0.2, 0.2, 0.5, 0.5]], 0.01);
        assert!((s.dx + 0.005).abs() < 1e-9 && s.vx == Some(0.7));
        // Far from everything: no snap.
        assert_eq!(snap([0.13, 0.27, 0.1, 0.1], &[], 0.01), Snap::default());
        assert_eq!(clamp([0.95, -0.2, 0.3, 0.01]), [0.7, 0.0, 0.3, 0.04]);
    }

    #[test]
    fn layouts_round_trip_as_json() {
        let l = Layout::from_scene(Scene::SideBySide, &PipSpec::default(), 1920.0, 1080.0, 1.0, 1.7);
        let j = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<Layout>(&j).unwrap(), l);
    }
}
