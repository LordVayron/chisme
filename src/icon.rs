//! The Chisme icon: Ventura's rainbow as a ring around a black disc. Plain
//! Rust with no dependencies, so `build.rs` can also use it to make the
//! .ico embedded in the executables.

/// Ventura's gradient: red, orange, yellow, light blue, blue.
pub const RAINBOW: [[u8; 3]; 5] = [[214, 38, 46], [255, 102, 0], [255, 196, 0], [120, 190, 255], [36, 84, 255]];

/// Color at `t` (0..1) along `stops`.
pub fn lerp(stops: &[[u8; 3]], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let i = (t.floor() as usize).min(stops.len() - 2);
    let f = t - i as f32;
    let (a, b) = (stops[i], stops[i + 1]);
    [0, 1, 2].map(|c| (a[c] as f32 + (b[c] as f32 - a[c] as f32) * f).round() as u8)
}

/// RGBA pixels (top-down) of the icon at `size`x`size`.
pub fn rgba(size: u32) -> Vec<u8> {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let c = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let outer = (c - d).clamp(0.0, 1.0);
            if outer <= 0.0 {
                continue;
            }
            let t = ((dy.atan2(dx) / std::f32::consts::TAU) + 0.5).fract();
            let ring = lerp(&RAINBOW, t);
            // Black center, anti-aliased.
            let inner = ((c * 0.58) - d).clamp(0.0, 1.0);
            let p = ((y * size + x) * 4) as usize;
            for k in 0..3 {
                px[p + k] = (ring[k] as f32 * (1.0 - inner) + 10.0 * inner) as u8;
            }
            px[p + 3] = (outer * 255.0) as u8;
        }
    }
    px
}

/// A Windows .ico with 32-bit images at the given sizes.
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&s| {
            let px = rgba(s);
            let mask_row = (s.div_ceil(32) * 4) as usize;
            let mut b = Vec::new();
            // BITMAPINFOHEADER; height counts the color and mask halves.
            b.extend(40u32.to_le_bytes());
            b.extend((s as i32).to_le_bytes());
            b.extend((s as i32 * 2).to_le_bytes());
            b.extend(1u16.to_le_bytes());
            b.extend(32u16.to_le_bytes());
            b.extend([0u8; 24]);
            // BGRA rows, bottom-up.
            for y in (0..s).rev() {
                for x in 0..s {
                    let p = ((y * s + x) * 4) as usize;
                    b.extend([px[p + 2], px[p + 1], px[p], px[p + 3]]);
                }
            }
            // AND mask (unused with alpha): all zeros.
            b.extend(vec![0u8; mask_row * s as usize]);
            b
        })
        .collect();
    let mut out = Vec::new();
    out.extend([0u16, 1, sizes.len() as u16].iter().flat_map(|v| v.to_le_bytes()));
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend([dim, dim, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend((img.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend(img);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_shape() {
        let px = rgba(64);
        let at = |x: usize, y: usize| &px[(y * 64 + x) * 4..][..4];
        assert_eq!(at(0, 0)[3], 0, "transparent corner");
        assert_eq!(at(32, 32)[..3], [10, 10, 10], "black center");
        assert_eq!(at(32, 2)[3], 255, "opaque ring");
    }

    #[test]
    fn ico_header() {
        let ico = ico(&[16, 256]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(ico[6], 16);
        assert_eq!(ico[22], 0, "256 is written as 0");
        let first = u32::from_le_bytes(ico[18..22].try_into().unwrap());
        assert_eq!(first, 6 + 32);
    }
}
