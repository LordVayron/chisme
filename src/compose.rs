//! Webcam overlay geometry and shape masks; the drawing itself is in
//! `win/overlay.rs` (D3D11).

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    /// Rounded rectangle, camera aspect ratio (corner radius from roundness).
    Rounded,
    /// Circle (center square crop).
    Circle,
    /// Square, rounded by roundness (center square crop).
    Square,
    /// Capsule: camera aspect ratio, fully round ends.
    Pill,
    /// Superellipse square (continuous corners); roundness sets the curve.
    Squircle,
    /// Plain rectangle (same as rounded with roundness 0).
    Rect,
}

impl Shape {
    /// Crops the camera to a centered square.
    pub fn is_square(self) -> bool {
        matches!(self, Shape::Circle | Shape::Square | Shape::Squircle)
    }
}

/// What the output frame shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scene {
    /// The screen only.
    Screen,
    /// The screen with the webcam picture-in-picture.
    #[default]
    ScreenCam,
    /// The webcam full frame.
    Cam,
    /// The screen on the left, the webcam in a column on the right.
    SideBySide,
    /// The webcam full frame with the screen in a box (placed and sized
    /// like the webcam overlay).
    CamScreen,
}

impl Scene {
    pub const ALL: [Scene; 5] = [Scene::Screen, Scene::ScreenCam, Scene::SideBySide, Scene::Cam, Scene::CamScreen];

    pub fn label(self) -> &'static str {
        match self {
            Scene::Screen => crate::i18n::l("Screen", "Pantalla"),
            Scene::ScreenCam => crate::i18n::l("Screen + camera", "Pantalla + cámara"),
            Scene::SideBySide => crate::i18n::l("Side by side", "Lado a lado"),
            Scene::Cam => crate::i18n::l("Camera", "Cámara"),
            Scene::CamScreen => crate::i18n::l("Camera + screen", "Cámara + pantalla"),
        }
    }
}

/// Side by side on a `w`x`h` frame: (screen area, camera column), each
/// `[x, y, w, h]`. The camera takes ~30% of the width, with a margin
/// around both.
pub fn side_by_side(w: f64, h: f64) -> ([f64; 4], [f64; 4]) {
    let m = margin(w);
    let cam_w = (w * 0.3).round();
    let screen = [m, m, (w - cam_w - 3.0 * m).max(16.0), (h - 2.0 * m).max(16.0)];
    let cam = [w - cam_w - m, m, cam_w, (h - 2.0 * m).max(16.0)];
    (screen, cam)
}

/// Where the screen goes in the "camera + screen" scene: the overlay's
/// place and size, with the screen's shape.
pub fn screen_box(pip: &PipSpec, w: f64, h: f64, screen_aspect: f64) -> [f64; 4] {
    let as_rect = PipSpec { shape: Shape::Rounded, size: pip.size.max(0.3), ..pip.clone() };
    overlay_rect(&as_rect, w, h, screen_aspect)
}

impl Scene {
    pub fn uses_cam(self) -> bool {
        self != Scene::Screen
    }
}

/// Uniform scale and offset that fits (`cover` false) or fills (`cover`
/// true) a `sw`x`sh` source into a `dw`x`dh` canvas, centred.
pub fn fit(sw: f64, sh: f64, dw: f64, dh: f64, cover: bool) -> (f64, f64, f64) {
    let (a, b) = (dw / sw, dh / sh);
    let s = if cover { a.max(b) } else { a.min(b) };
    (s, (dw - sw * s) / 2.0, (dh - sh * s) / 2.0)
}

/// Where the overlay sits: snapped to a corner (stays there when the size
/// changes) or centered at a point given as fractions of the frame from its
/// top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Place {
    Corner(Corner),
    At([f64; 2]),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PipSpec {
    /// Overlay width as a fraction of the output width.
    pub size: f64,
    pub place: Place,
    pub shape: Shape,
    /// Corner rounding, 0 (sharp) to 1 (as round as the shape allows).
    pub roundness: f64,
    /// Border width in pixels at 1080p (scaled with the output height).
    pub border: f64,
    pub border_color: [u8; 3],
    pub shadow: bool,
    pub mirror: bool,
}

impl Default for PipSpec {
    fn default() -> Self {
        PipSpec {
            size: 0.22,
            place: Place::Corner(Corner::BottomRight),
            shape: Shape::Rounded,
            roundness: 0.2,
            border: 0.0,
            border_color: [255, 255, 255],
            shadow: false,
            mirror: true,
        }
    }
}

/// Gap between a corner-snapped overlay and the frame edges.
pub fn margin(w: f64) -> f64 {
    (w * 0.02).round()
}

/// Overlay size in output pixels for a `w`x`h` output.
pub fn overlay_size(pip: &PipSpec, w: f64, h: f64, cam_aspect: f64) -> (f64, f64) {
    let aspect = if pip.shape.is_square() { 1.0 } else { cam_aspect.max(0.1) };
    let ow = (w * pip.size).round().clamp(16.0, w);
    let oh = (ow / aspect).round().clamp(16.0, h);
    (ow, oh)
}

/// Overlay rectangle `[x, y, width, height]` in output pixels, measured
/// from the top-left corner, always inside the frame.
pub fn overlay_rect(pip: &PipSpec, w: f64, h: f64, cam_aspect: f64) -> [f64; 4] {
    let (ow, oh) = overlay_size(pip, w, h, cam_aspect);
    let (x, y) = match pip.place {
        Place::Corner(c) => {
            let m = margin(w);
            let x = if matches!(c, Corner::TopLeft | Corner::BottomLeft) { m } else { w - m - ow };
            let y = if matches!(c, Corner::TopLeft | Corner::TopRight) { m } else { h - m - oh };
            (x, y)
        }
        Place::At([cx, cy]) => ((cx * w - ow / 2.0).round(), (cy * h - oh / 2.0).round()),
    };
    [x.clamp(0.0, w - ow), y.clamp(0.0, h - oh), ow, oh]
}

/// The corner whose snapped position is within `radius` (a fraction of
/// the output width) of the overlay's current center, if any. Used by the
/// app's drag-to-place (phase 3).
#[allow(dead_code)]
pub fn snap_corner(pip: &PipSpec, w: f64, h: f64, cam_aspect: f64, radius: f64) -> Option<Corner> {
    let center = |p: &PipSpec| {
        let r = overlay_rect(p, w, h, cam_aspect);
        (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
    };
    let c = center(pip);
    [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight]
        .into_iter()
        .map(|k| {
            let q = center(&PipSpec { place: Place::Corner(k), ..pip.clone() });
            (k, ((q.0 - c.0).powi(2) + (q.1 - c.1).powi(2)).sqrt())
        })
        .filter(|(_, d)| *d <= radius * w)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// Superellipse exponent for a squircle: 2 (a circle) at roundness 1, 4
/// (the classic squircle) at 0.5, 10 (nearly square) at 0.
pub fn squircle_exponent(roundness: f64) -> f64 {
    2.0 + 8.0 * (1.0 - roundness.clamp(0.0, 1.0)).powi(2)
}

/// Anti-aliased coverage (0..1) of the pixel centered at (`px`, `py`) by
/// `shape` filling a `w`x`h` box.
pub fn coverage(shape: Shape, w: f64, h: f64, roundness: f64, px: f64, py: f64) -> f32 {
    let (hw, hh) = (w / 2.0, h / 2.0);
    let (x, y) = ((px - hw).abs(), (py - hh).abs());
    // Approximate signed distance to the edge in pixels (negative inside).
    let d = match shape {
        Shape::Squircle => {
            let n = squircle_exponent(roundness);
            (((x / hw).powf(n) + (y / hh).powf(n)).powf(1.0 / n) - 1.0) * hw.min(hh)
        }
        Shape::Circle => (((x / hw).powi(2) + (y / hh).powi(2)).sqrt() - 1.0) * hw.min(hh),
        _ => {
            let r = match shape {
                Shape::Pill => hw.min(hh),
                Shape::Rect => 0.0,
                _ => roundness.clamp(0.0, 1.0) * hw.min(hh),
            };
            let (qx, qy) = (x - (hw - r), y - (hh - r));
            (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
        }
    };
    (0.5 - d).clamp(0.0, 1.0) as f32
}

/// Alpha mask (row-major, `w`x`h`) of the shape inset by `inset` pixels.
pub fn mask(shape: Shape, w: usize, h: usize, roundness: f64, inset: f64) -> Vec<u8> {
    let (iw, ih) = (w as f64 - 2.0 * inset, h as f64 - 2.0 * inset);
    let mut out = vec![0u8; w * h];
    if iw <= 0.0 || ih <= 0.0 {
        return out;
    }
    for y in 0..h {
        for x in 0..w {
            let c = coverage(shape, iw, ih, roundness, x as f64 + 0.5 - inset, y as f64 + 0.5 - inset);
            out[y * w + x] = (c * 255.0).round() as u8;
        }
    }
    out
}

/// Soft drop shadow: `alpha` padded by `pad` pixels on every side, blurred
/// with three box passes (close to a Gaussian) and scaled by `opacity`.
pub fn shadow(alpha: &[u8], w: usize, h: usize, pad: usize, opacity: f32) -> Vec<u8> {
    let (sw, sh) = (w + 2 * pad, h + 2 * pad);
    let mut a = vec![0f32; sw * sh];
    for y in 0..h {
        for x in 0..w {
            a[(y + pad) * sw + x + pad] = alpha[y * w + x] as f32 / 255.0;
        }
    }
    let r = (pad / 3).max(1);
    let mut tmp = vec![0f32; sw * sh];
    for _ in 0..3 {
        box_blur(&a, &mut tmp, sw, sh, r, true);
        box_blur(&tmp, &mut a, sw, sh, r, false);
    }
    a.iter().map(|v| (v * opacity * 255.0).round().clamp(0.0, 255.0) as u8).collect()
}

fn box_blur(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, horizontal: bool) {
    let (n, lines) = if horizontal { (w, h) } else { (h, w) };
    let at = |line: usize, i: usize| if horizontal { line * w + i } else { i * w + line };
    let k = (2 * r + 1) as f32;
    for line in 0..lines {
        let mut sum: f32 = (0..=r.min(n - 1)).map(|i| src[at(line, i)]).sum();
        for i in 0..n {
            dst[at(line, i)] = sum / k;
            if i + r + 1 < n {
                sum += src[at(line, i + r + 1)];
            }
            if i >= r {
                sum -= src[at(line, i - r)];
            }
        }
    }
}

/// Border width in output pixels.
pub fn border_px(pip: &PipSpec, out_h: f64) -> f64 {
    (pip.border.max(0.0) * out_h / 1080.0).round()
}

/// The overlay's static layers in one RGBA8 image covering the overlay plus
/// the shadow's reach: R = shadow alpha, G = outer shape (border), B = inner
/// shape (where the camera shows). Built once on the CPU per size and style.
pub struct Decor {
    pub key: (usize, usize, Shape, u64, u64, bool),
    /// Image size and where the overlay's top-left sits inside it.
    pub size: (usize, usize),
    pub origin: (usize, usize),
    pub rgba: Vec<u8>,
}

impl Decor {
    pub fn key(pip: &PipSpec, w: usize, h: usize, out_h: f64) -> (usize, usize, Shape, u64, u64, bool) {
        (w, h, pip.shape, pip.roundness.to_bits(), border_px(pip, out_h).to_bits(), pip.shadow)
    }

    pub fn build(pip: &PipSpec, w: usize, h: usize, out_h: f64) -> Decor {
        let bw = border_px(pip, out_h);
        let outer = mask(pip.shape, w, h, pip.roundness, 0.0);
        let inner = if bw > 0.0 { mask(pip.shape, w, h, pip.roundness, bw) } else { outer.clone() };
        // The shadow drops slightly below the overlay.
        let (pad, drop) = if pip.shadow { ((out_h * 0.03).round().max(3.0) as usize, (out_h * 0.006).round() as usize) } else { (0, 0) };
        let size = (w + 2 * pad, h + 2 * pad + drop);
        let mut rgba = vec![0u8; size.0 * size.1 * 4];
        if pip.shadow {
            let s = shadow(&outer, w, h, pad, 0.45);
            for y in 0..h + 2 * pad {
                for x in 0..w + 2 * pad {
                    rgba[((y + drop) * size.0 + x) * 4] = s[y * (w + 2 * pad) + x];
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                let p = ((y + pad) * size.0 + x + pad) * 4;
                rgba[p + 1] = if bw > 0.0 { outer[y * w + x] } else { 0 };
                rgba[p + 2] = inner[y * w + x];
                rgba[p + 3] = 255;
            }
        }
        Decor { key: Self::key(pip, w, h, out_h), size, origin: (pad, pad), rgba }
    }
}

/// `X,Y` fractions of the frame, both 0..1.
pub fn parse_pos(s: &str) -> anyhow::Result<[f64; 2]> {
    let v: Vec<f64> = s.split(',').map(|x| x.trim().parse::<f64>()).collect::<Result<_, _>>()?;
    match v[..] {
        [x, y] if (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) => Ok([x, y]),
        _ => anyhow::bail!("--pip-pos must be X,Y with both between 0 and 1"),
    }
}

/// `RRGGBB` (optionally with a leading #).
pub fn parse_hex(s: &str) -> anyhow::Result<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        anyhow::bail!("color must be RRGGBB hex");
    }
    let c = |i: usize| u8::from_str_radix(&s[i..i + 2], 16);
    Ok([c(0)?, c(2)?, c(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_letterboxes_and_cover_crops() {
        // 4:3 camera into a 16:9 canvas.
        let (s, tx, ty) = fit(1280.0, 960.0, 1920.0, 1080.0, false);
        assert_eq!((s, tx, ty), (1.125, 240.0, 0.0));
        let (s, tx, ty) = fit(1280.0, 960.0, 1920.0, 1080.0, true);
        assert_eq!((s, tx, ty), (1.5, 0.0, -180.0));
    }

    fn spec(shape: Shape, place: Place) -> PipSpec {
        PipSpec { shape, place, ..Default::default() }
    }

    #[test]
    fn corners_and_free_positions() {
        let a = 16.0 / 9.0;
        let p = spec(Shape::Rounded, Place::Corner(Corner::BottomRight));
        // 22% of 1920 = 422 wide, 237 tall, 38 px margin.
        assert_eq!(overlay_rect(&p, 1920.0, 1080.0, a), [1460.0, 805.0, 422.0, 237.0]);
        let p = spec(Shape::Circle, Place::Corner(Corner::TopLeft));
        assert_eq!(overlay_rect(&p, 1920.0, 1080.0, a), [38.0, 38.0, 422.0, 422.0]);
        let p = spec(Shape::Rounded, Place::At([0.5, 0.5]));
        assert_eq!(overlay_rect(&p, 1920.0, 1080.0, a), [749.0, 422.0, 422.0, 237.0]);
        // Dragged past the edge: kept inside the frame.
        let p = spec(Shape::Square, Place::At([1.2, -0.3]));
        assert_eq!(overlay_rect(&p, 1920.0, 1080.0, a), [1498.0, 0.0, 422.0, 422.0]);
    }

    #[test]
    fn corner_placement_survives_resize() {
        let mut p = spec(Shape::Rounded, Place::Corner(Corner::BottomRight));
        p.size = 0.4;
        let r = overlay_rect(&p, 1920.0, 1080.0, 16.0 / 9.0);
        assert_eq!((r[0] + r[2], r[1] + r[3]), (1920.0 - 38.0, 1080.0 - 38.0));
    }

    #[test]
    fn snapping() {
        let a = 16.0 / 9.0;
        let near = spec(Shape::Rounded, Place::At([0.85, 0.82]));
        assert_eq!(snap_corner(&near, 1920.0, 1080.0, a, 0.06), Some(Corner::BottomRight));
        let middle = spec(Shape::Rounded, Place::At([0.5, 0.5]));
        assert_eq!(snap_corner(&middle, 1920.0, 1080.0, a, 0.06), None);
        let top_left = spec(Shape::Circle, Place::At([0.1, 0.18]));
        assert_eq!(snap_corner(&top_left, 1920.0, 1080.0, a, 0.06), Some(Corner::TopLeft));
    }

    #[test]
    fn shape_coverage() {
        let cov = |s, r, x, y| coverage(s, 100.0, 100.0, r, x, y);
        for s in [Shape::Circle, Shape::Rounded, Shape::Square, Shape::Pill, Shape::Squircle] {
            assert_eq!(cov(s, 0.5, 50.0, 50.0), 1.0, "{s:?} center");
            assert_eq!(cov(s, 0.5, 50.0, 0.5), 1.0, "{s:?} top edge middle");
        }
        // The corner pixel: covered only by sharp shapes.
        assert_eq!(cov(Shape::Rect, 0.0, 0.5, 0.5), 1.0);
        assert_eq!(cov(Shape::Rounded, 0.0, 0.5, 0.5), 1.0, "roundness 0 is a rectangle");
        assert_eq!(cov(Shape::Rounded, 0.5, 0.5, 0.5), 0.0);
        assert_eq!(cov(Shape::Circle, 0.0, 0.5, 0.5), 0.0);
        assert_eq!(cov(Shape::Squircle, 0.5, 0.5, 0.5), 0.0);
        // The squircle is fuller than a circle at the diagonal (0.8, 0.8).
        assert_eq!(cov(Shape::Circle, 0.5, 90.0, 90.0), 0.0);
        assert_eq!(cov(Shape::Squircle, 0.5, 90.0, 90.0), 1.0);
        // Roundness 1 on a square equals a circle.
        assert_eq!(cov(Shape::Square, 1.0, 90.0, 90.0), 0.0);
        // Anti-aliased edge.
        let edge = cov(Shape::Circle, 0.5, 100.0, 50.0);
        assert!(edge > 0.0 && edge < 1.0, "edge {edge}");
    }

    #[test]
    fn pill_has_round_ends() {
        let cov = |x, y| coverage(Shape::Pill, 200.0, 100.0, 0.0, x, y);
        assert_eq!(cov(5.0, 50.0), 1.0);
        assert_eq!(cov(5.0, 5.0), 0.0);
        assert_eq!(cov(100.0, 1.0), 1.0);
        assert_eq!(cov(195.0, 95.0), 0.0);
    }

    #[test]
    fn border_inset_mask() {
        let outer = mask(Shape::Square, 40, 40, 0.0, 0.0);
        let inner = mask(Shape::Square, 40, 40, 0.0, 4.0);
        assert_eq!((outer[2 * 40 + 20], inner[2 * 40 + 20]), (255, 0), "border band");
        assert_eq!((outer[20 * 40 + 20], inner[20 * 40 + 20]), (255, 255));
    }

    #[test]
    fn shadow_spreads_beyond_the_shape() {
        let a = mask(Shape::Square, 20, 20, 0.0, 0.0);
        let s = shadow(&a, 20, 20, 9, 0.5);
        let w = 38;
        assert!(s[19 * w + 19] > 100, "under the shape");
        assert!(s[19 * w + 4] > 0, "outside the shape");
        assert_eq!(s[0], 0, "far corner");
        assert!(s.iter().all(|&v| v <= 128));
    }

    #[test]
    fn decor_layers() {
        let pip = PipSpec { shape: Shape::Square, roundness: 0.0, border: 12.0, shadow: true, ..Default::default() };
        let d = Decor::build(&pip, 100, 100, 1080.0);
        let (pad, drop) = (32, 6);
        assert_eq!(d.origin, (pad, pad));
        assert_eq!(d.size, (100 + 2 * pad, 100 + 2 * pad + drop));
        let px = |x: usize, y: usize| &d.rgba[(y * d.size.0 + x) * 4..][..4];
        // Center: camera shows, border layer set, shadow under it.
        let c = px(pad + 50, pad + 50);
        assert_eq!((c[1], c[2]), (255, 255));
        assert!(c[0] > 100);
        // In the 12 px border band: border only.
        let b = px(pad + 50, pad + 3);
        assert_eq!((b[1], b[2]), (255, 0));
        // Below the overlay: shadow only (it drops downward).
        let s = px(pad + 50, pad + 100 + 4);
        assert_eq!((s[1], s[2]), (0, 0));
        assert!(s[0] > 0);
    }

    #[test]
    fn new_scene_layouts() {
        let (screen, cam) = side_by_side(1920.0, 1080.0);
        // 38 px margins; the camera column is 30% wide.
        assert_eq!(cam, [1306.0, 38.0, 576.0, 1004.0]);
        assert_eq!(screen, [38.0, 38.0, 1230.0, 1004.0]);
        assert!(screen[0] + screen[2] + 38.0 == cam[0], "one margin between them");
        // The screen box keeps the screen's 16:9 shape and the overlay's corner.
        let b = screen_box(&PipSpec::default(), 1920.0, 1080.0, 16.0 / 9.0);
        assert_eq!(b, [1306.0, 718.0, 576.0, 324.0]);
        assert!(Scene::ALL.iter().all(|s| !s.label().is_empty()));
        assert!(Scene::SideBySide.uses_cam() && Scene::CamScreen.uses_cam());
    }

    #[test]
    fn option_parsing() {
        assert_eq!(parse_pos("0.25,0.75").unwrap(), [0.25, 0.75]);
        assert!(parse_pos("1.5,0").is_err());
        assert_eq!(parse_hex("#ff8800").unwrap(), [255, 136, 0]);
        assert!(parse_hex("fff").is_err());
    }
}
