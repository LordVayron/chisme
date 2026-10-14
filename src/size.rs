//! Output size selection.

use anyhow::{Result, bail};

fn even(x: f64) -> usize {
    ((x / 2.0).round() as usize * 2).max(2)
}

/// Output size for `s` (a preset or WxH, the bounding box) and a source of
/// `native` pixels: the source's aspect ratio fitted inside the box, never
/// larger than the source, even dimensions. A frame with the source's shape
/// fills edge to edge: no letterbox or pillarbox bars, nothing cropped.
pub fn parse_size(s: &str, native: (usize, usize)) -> Result<(usize, usize)> {
    let lower = s.to_lowercase();
    // The capture's own size is the native size rounded down to even.
    let (nw, nh) = ((native.0 & !1).max(2) as f64, (native.1 & !1).max(2) as f64);
    let (bw, bh) = match lower.as_str() {
        "720p" => (1280.0, 720.0),
        "1080p" => (1920.0, 1080.0),
        "1440p" => (2560.0, 1440.0),
        "4k" | "2160p" | "uhd" => (3840.0, 2160.0),
        "native" => (nw, nh),
        _ => {
            let Some((w, h)) = lower.split_once('x') else {
                bail!("bad --resolution {s:?}: use 720p/1080p/1440p/4k/native/WxH")
            };
            (w.trim().parse::<usize>()? as f64, h.trim().parse::<usize>()? as f64)
        }
    };
    // Presets name the frame's height for landscape sources and its width
    // for portrait ones (a 1080p portrait window is 1080 wide).
    let (bw, bh) = if nh > nw && matches!(lower.as_str(), "720p" | "1080p" | "1440p" | "4k" | "2160p" | "uhd") { (bh, bw) } else { (bw, bh) };
    let k = (bw / nw).min(bh / nh).min(1.0);
    Ok((even(nw * k), even(nh * k)))
}

/// The largest rect with `src`'s aspect ratio centred inside `dst`:
/// `(x, y, w, h)`.
pub fn fit(src: (u32, u32), dst: (u32, u32)) -> (u32, u32, u32, u32) {
    let k = (dst.0 as f64 / src.0.max(1) as f64).min(dst.1 as f64 / src.1.max(1) as f64);
    let w = ((src.0 as f64 * k).round() as u32).min(dst.0);
    let h = ((src.1 as f64 * k).round() as u32).min(dst.1);
    ((dst.0 - w) / 2, (dst.1 - h) / 2, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two sizes have the same shape within one pixel of rounding.
    fn same_aspect(w0: f64, h0: f64, w1: f64, h1: f64) -> bool {
        (w0 / h0 - w1 / h1).abs() <= (1.0 / h1).max(1.0 / w1) * (w0 / h0) * 2.0
    }

    #[test]
    fn sizes() {
        // Every size keeps the source's aspect ratio (no bars), even sizes.
        let native = (3456, 2234);
        assert_eq!(parse_size("1080p", native).unwrap(), (1670, 1080));
        assert_eq!(parse_size("4K", native).unwrap(), (3342, 2160));
        assert_eq!(parse_size("native", native).unwrap(), native);
        assert_eq!(parse_size("1280x720", native).unwrap(), (1114, 720));
        assert_eq!(parse_size("1080p", (2880, 1800)).unwrap(), (1728, 1080));
        assert_eq!(parse_size("1080p", (3840, 2160)).unwrap(), (1920, 1080));
        // Never upscaled; odd sources round down to even, like the capture.
        assert_eq!(parse_size("4k", (1201, 801)).unwrap(), (1200, 800));
        // Portrait: the preset names the width.
        assert_eq!(parse_size("1080p", (2160, 3840)).unwrap(), (1080, 1920));
        for (sz, n) in [("1080p", (3456, 2234)), ("720p", (4112, 2658)), ("4k", (5120, 2880)), ("1440p", (3144, 1588))] {
            let (w, h) = parse_size(sz, n).unwrap();
            assert!(w % 2 == 0 && h % 2 == 0);
            assert!(same_aspect(n.0 as f64, n.1 as f64, w as f64, h as f64), "{sz} {n:?} -> {w}x{h}");
        }
        assert!(parse_size("huge", native).is_err());
    }

    #[test]
    fn fit_centres_and_keeps_shape() {
        assert_eq!(fit((1920, 1080), (1920, 1080)), (0, 0, 1920, 1080));
        assert_eq!(fit((1000, 1000), (1920, 1080)), (420, 0, 1080, 1080));
        assert_eq!(fit((3840, 1080), (1920, 1080)), (0, 270, 1920, 540));
    }
}
