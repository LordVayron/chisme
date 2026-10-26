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
        let mut mask: Vec<f32> = out[0].to_plain_array_view::<f32>()?.iter().copied().collect();
        keep_main(&mut mask);
        Ok(mask)
    }
}

/// MODNet (portrait matting, Apache-2.0; the ONNX export from Xenova on
/// Hugging Face). Slower than the selfie model, so it runs a few times a
/// second beside it and only tells it where the person can't be.
pub const MODNET_URL: &str = "https://huggingface.co/Xenova/modnet/resolve/main/onnx/model.onnx";
pub const MODNET_SHA256: &str = "07c308cf0fc7e6e8b2065a12ed7fc07e1de8febb7dc7839d7b7f15dd66584df9";
/// Its input: 256x160 (a multiple of 32, close to 16:9).
const MW: usize = 256;
const MH: usize = 160;

pub fn modnet_path() -> std::path::PathBuf {
    crate::subtitles::models_dir().join("modnet.onnx")
}

/// The MODNet file, downloaded and checked the first time.
pub fn ensure_modnet() -> Result<std::path::PathBuf> {
    use std::io::{Read, Write};
    let path = modnet_path();
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(crate::subtitles::models_dir())?;
    let part = path.with_extension("part");
    let resp = ureq::get(MODNET_URL).call()?;
    let mut reader = resp.into_body().into_reader();
    let mut file = std::io::BufWriter::new(std::fs::File::create(&part)?);
    let mut hash = <sha2::Sha256 as sha2::Digest>::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hash, &buf[..n]);
        file.write_all(&buf[..n])?;
    }
    file.flush()?;
    drop(file);
    // Only the exact published file is ever loaded.
    let got: String = sha2::Digest::finalize(hash).iter().map(|b| format!("{b:02x}")).collect();
    if got != MODNET_SHA256 {
        let _ = std::fs::remove_file(&part);
        anyhow::bail!("the downloaded MODNet doesn't match the published one");
    }
    std::fs::rename(&part, &path)?;
    Ok(path)
}

pub struct ModNet {
    plan: Arc<TypedRunnableModel>,
}

impl ModNet {
    pub fn new(path: &std::path::Path) -> Result<ModNet> {
        let plan = tract_onnx::onnx().model_for_path(path)?.with_input_fact(0, f32::fact([1, 3, MH, MW]).into())?.into_optimized()?.into_runnable()?;
        Ok(ModNet { plan })
    }

    /// `input`: a `downsample`d frame (planar 256x256 RGB, 0..1). Returns
    /// the person's alpha at 256x256.
    pub fn run(&self, input: &[f32]) -> Result<Vec<f32>> {
        let plane = SIZE * SIZE;
        let mut x = vec![0f32; 3 * MW * MH];
        for c in 0..3 {
            for y in 0..MH {
                let sy = (((y as f32 + 0.5) * SIZE as f32 / MH as f32) as usize).min(SIZE - 1);
                for xx in 0..MW {
                    x[c * MW * MH + y * MW + xx] = input[c * plane + sy * SIZE + xx] * 2.0 - 1.0;
                }
            }
        }
        let t: Tensor = tract_ndarray::Array4::from_shape_vec((1, 3, MH, MW), x)?.into();
        let out = self.plan.run(tvec!(t.into()))?;
        let a: Vec<f32> = out[0].to_plain_array_view::<f32>()?.iter().copied().collect();
        // Back to 256 rows (linear).
        let mut m = vec![0f32; plane];
        for y in 0..SIZE {
            let fy = ((y as f32 + 0.5) * MH as f32 / SIZE as f32 - 0.5).clamp(0.0, (MH - 1) as f32);
            let (y0, k) = (fy.floor() as usize, fy.fract());
            let y1 = (y0 + 1).min(MH - 1);
            for xx in 0..SIZE {
                m[y * SIZE + xx] = a[y0 * MW + xx] * (1.0 - k) + a[y1 * MW + xx] * k;
            }
        }
        Ok(m)
    }
}

/// Where the person can be: MODNet's alpha grown by `r` cells (to cover
/// movement since it last ran) and softened. Multiplied into the selfie
/// mask, it removes what MODNet sees as background.
pub fn allow_mask(alpha: &[f32], r: usize) -> Vec<f32> {
    let n = SIZE;
    let on: Vec<f32> = alpha.iter().map(|&a| if a > 0.25 { 1.0 } else { 0.0 }).collect();
    let pass = |src: &[f32], horizontal: bool, reach: isize, max: bool| -> Vec<f32> {
        let mut out = vec![0f32; n * n];
        for y in 0..n {
            for x in 0..n {
                let (mut m, mut s, mut c) = (0f32, 0f32, 0f32);
                for o in -reach..=reach {
                    let (xx, yy) = if horizontal { (x as isize + o, y as isize) } else { (x as isize, y as isize + o) };
                    if xx >= 0 && yy >= 0 && (xx as usize) < n && (yy as usize) < n {
                        let v = src[yy as usize * n + xx as usize];
                        m = m.max(v);
                        s += v;
                        c += 1.0;
                    }
                }
                out[y * n + x] = if max { m } else { s / c };
            }
        }
        out
    };
    let r = r as isize;
    let grown = pass(&pass(&on, true, r, true), false, r, true);
    // A short ramp so the cut isn't hard.
    pass(&pass(&grown, true, 3, false), false, 3, false)
}

/// Drop islands the model mistakes for a person (a lamp's glow, a patch of
/// wall): keep the regions at least a fifth the size of the largest one.
pub fn keep_main(mask: &mut [f32]) {
    if mask.len() != SIZE * SIZE {
        return;
    }
    let mut label = vec![0u32; SIZE * SIZE];
    let mut sizes = vec![0usize];
    let mut stack = Vec::new();
    for start in 0..SIZE * SIZE {
        if label[start] != 0 || mask[start] < 0.5 {
            continue;
        }
        let id = sizes.len() as u32;
        let mut n = 0;
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            n += 1;
            let (x, y) = (i % SIZE, i / SIZE);
            let mut visit = |j: usize| {
                if label[j] == 0 && mask[j] >= 0.5 {
                    label[j] = id;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < SIZE {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - SIZE);
            }
            if y + 1 < SIZE {
                visit(i + SIZE);
            }
        }
        sizes.push(n);
    }
    let largest = sizes.iter().copied().max().unwrap_or(0);
    if largest == 0 {
        return;
    }
    let keep: Vec<bool> = sizes.iter().map(|&n| n * 5 >= largest).collect();
    // Weak pixels (< 0.5) next to a dropped island fade with it.
    let mut drop = vec![false; SIZE * SIZE];
    for i in 0..SIZE * SIZE {
        if label[i] != 0 && !keep[label[i] as usize] {
            drop[i] = true;
        }
    }
    for (i, m) in mask.iter_mut().enumerate() {
        if drop[i] {
            *m = 0.0;
        } else if *m < 0.5 {
            let (x, y) = (i % SIZE, i / SIZE);
            let near = |dx: i32, dy: i32| {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                nx >= 0 && ny >= 0 && (nx as usize) < SIZE && (ny as usize) < SIZE && drop[ny as usize * SIZE + nx as usize]
            };
            if near(-2, 0) || near(2, 0) || near(0, -2) || near(0, 2) {
                *m = 0.0;
            }
        }
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

    #[test]
    fn allow_mask_grows_the_person() {
        let mut a = vec![0f32; SIZE * SIZE];
        for y in 100..150 {
            for x in 100..150 {
                a[y * SIZE + x] = 1.0;
            }
        }
        let m = allow_mask(&a, 10);
        assert_eq!(m[125 * SIZE + 125], 1.0, "inside");
        assert!(m[125 * SIZE + 155] > 0.9, "a little outside is still allowed");
        assert_eq!(m[20 * SIZE + 20], 0.0, "far away is not");
    }

    #[test]
    fn small_islands_are_dropped() {
        let mut m = vec![0f32; SIZE * SIZE];
        for y in 100..200 {
            for x in 80..180 {
                m[y * SIZE + x] = 1.0; // the person
            }
        }
        for y in 10..20 {
            for x in 10..20 {
                m[y * SIZE + x] = 0.9; // a lamp
            }
        }
        keep_main(&mut m);
        assert_eq!(m[15 * SIZE + 15], 0.0);
        assert_eq!(m[150 * SIZE + 130], 1.0);
    }

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

