//! Person segmentation for the virtual background: Google's MediaPipe
//! Selfie Segmentation (Apache-2.0; the ONNX export from onnx-community),
//! run on the CPU with tract. It takes a 256x256 RGB picture and returns
//! how much each pixel is the person (0..1). Small (460 KB) and fast
//! (a few ms), so it keeps up with a 30 fps webcam on any PC.

use std::sync::Arc;

use anyhow::Result;
use tract_onnx::prelude::*;

pub const SIZE: usize = 256;

static MODEL: &[u8] = include_bytes!("../assets/models/selfie_segmentation.onnx");

pub struct Segmenter {
    plan: Arc<TypedRunnableModel>,
}

impl Segmenter {
    pub fn new() -> Result<Segmenter> {
        let model = tract_onnx::onnx().model_for_read(&mut std::io::Cursor::new(MODEL))?.into_typed()?;
        // The file leaves the batch open ("batch_size"): one frame at a time.
        let batch = model.symbols.sym("batch_size");
        let model = model.set_symbols(&std::collections::HashMap::from([(batch, TDim::Val(1))]))?;
        let plan = model.into_optimized()?.into_runnable()?;
        Ok(Segmenter { plan })
    }

    /// `bgra`: a `w`x`h` BGRA picture (any size; sampled down to 256x256).
    /// Returns the person mask, 256x256, row by row.
    pub fn run(&self, bgra: &[u8], w: usize, h: usize) -> Result<Vec<f32>> {
        self.run_input(downsample(bgra, w, h))
    }

    /// Run on a `downsample`d picture.
    pub fn run_input(&self, input: Vec<f32>) -> Result<Vec<f32>> {
        let t: Tensor = tract_ndarray::Array4::from_shape_vec((1, 3, SIZE, SIZE), input)?.into();
        let out = self.plan.run(tvec!(t.into()))?;
        Ok(out[0].to_plain_array_view::<f32>()?.iter().copied().collect())
    }
}

/// Box-filtered 256x256 planar RGB in 0..1 (the model's input), stretching
/// the whole frame like the model was trained on.
pub fn downsample(bgra: &[u8], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0f32; 3 * SIZE * SIZE];
    let plane = SIZE * SIZE;
    for oy in 0..SIZE {
        let (y0, y1) = (oy * h / SIZE, ((oy + 1) * h / SIZE).max(oy * h / SIZE + 1).min(h));
        for ox in 0..SIZE {
            let (x0, x1) = (ox * w / SIZE, ((ox + 1) * w / SIZE).max(ox * w / SIZE + 1).min(w));
            // Up to 4x4 samples per cell is plenty for a mask.
            let (sy, sx) = (((y1 - y0) / 4).max(1), ((x1 - x0) / 4).max(1));
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    let p = (y * w + x) * 4;
                    b += bgra[p] as u32;
                    g += bgra[p + 1] as u32;
                    r += bgra[p + 2] as u32;
                    n += 1;
                    x += sx;
                }
                y += sy;
            }
            let k = 1.0 / (255.0 * n.max(1) as f32);
            let i = oy * SIZE + ox;
            out[i] = r as f32 * k;
            out[plane + i] = g as f32 * k;
            out[2 * plane + i] = b as f32 * k;
        }
    }
    expose(&mut out);
    out
}

/// Auto-exposure for the model only (the video is untouched): webcams in a
/// dim room give pictures the model barely sees a person in. Stretch the
/// brightest 1% to white (up to 8x), and lift the shadows if it's still dark.
fn expose(rgb: &mut [f32]) {
    let plane = SIZE * SIZE;
    let mut hist = [0u32; 256];
    for i in 0..plane {
        let v = rgb[i].max(rgb[plane + i]).max(rgb[2 * plane + i]);
        hist[(v * 255.0) as usize] += 1;
    }
    let mut seen = 0;
    let p99 = (0..256).rev().find(|&b| {
        seen += hist[b];
        seen as usize >= plane / 100
    });
    let gain = (0.95 / (p99.unwrap_or(255).max(1) as f32 / 255.0)).clamp(1.0, 8.0);
    let mean = rgb.iter().sum::<f32>() / rgb.len() as f32 * gain;
    let gamma = if mean < 0.3 { 0.6 } else { 1.0 };
    if gain > 1.01 || gamma < 1.0 {
        for v in rgb.iter_mut() {
            *v = (*v * gain).min(1.0).powf(gamma);
        }
    }
}

/// Follows the newest mask smoothly: fast where the person moves, slow
/// where it flickers (edges), so the cut-out doesn't shimmer.
pub fn smooth(prev: &mut [f32], new: &[f32]) {
    for (p, n) in prev.iter_mut().zip(new) {
        let d = (n - *p).abs();
        let k = (0.35 + d * 1.2).min(1.0);
        *p += (n - *p) * k;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame with a bright figure on a dark background: the model runs and
    /// gives a 256x256 mask in 0..1.
    #[test]
    fn runs() {
        let seg = Segmenter::new().unwrap();
        let (w, h) = (640, 360);
        let mut img = vec![30u8; w * h * 4];
        for y in 60..h {
            for x in 240..400 {
                let p = (y * w + x) * 4;
                img[p..p + 3].copy_from_slice(&[150, 170, 210]);
            }
        }
        let t = std::time::Instant::now();
        let m = seg.run(&img, w, h).unwrap();
        eprintln!("segmentation: {:?}", t.elapsed());
        assert_eq!(m.len(), SIZE * SIZE);
        assert!(m.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn smoothing_settles() {
        let mut p = vec![0.0f32; 4];
        for _ in 0..20 {
            smooth(&mut p, &[1.0, 0.0, 0.5, 1.0]);
        }
        assert!((p[0] - 1.0).abs() < 1e-3 && p[1] == 0.0 && (p[2] - 0.5).abs() < 1e-3);
    }

    /// Prints the mask of a real frame next to the reference (onnxruntime)
    /// one: `cargo test --release segment -- --ignored --nocapture` with
    /// CHISME_SEG_FRAME=frame.raw (BGRA 1280x720) and CHISME_SEG_OUT=mask.raw.
    #[test]
    #[ignore]
    fn real_frame() {
        let raw = std::fs::read(std::env::var("CHISME_SEG_FRAME").unwrap()).unwrap();
        let seg = Segmenter::new().unwrap();
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let m = seg.run(&raw, 1280, 720).unwrap();
            eprintln!("segmentation: {:?}", t.elapsed());
            let bytes: Vec<u8> = m.iter().map(|v| (v * 255.0) as u8).collect();
            std::fs::write(std::env::var("CHISME_SEG_OUT").unwrap(), bytes).unwrap();
        }
    }
}
