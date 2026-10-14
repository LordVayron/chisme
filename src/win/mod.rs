//! Windows platform layer: Windows.Graphics.Capture, WASAPI, Media
//! Foundation and D3D11.

pub mod aac;
pub mod apps;
pub mod audio;
pub mod camera;
pub mod capture;
pub mod control;
pub mod convert;
pub mod creds;
pub mod encoder;
pub mod gpu;
pub mod matte;
pub mod h264;
pub mod overlay;
pub mod sources;

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// Host clock in seconds (QPC). Windows.Graphics.Capture frame times and
/// WASAPI packet times are on the same clock.
pub fn now() -> f64 {
    let (mut c, mut f) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
        let _ = QueryPerformanceFrequency(&mut f);
    }
    c as f64 / f.max(1) as f64
}

/// Copy a decoded frame's rows (`row` bytes, `h` rows) top-down into `dst`,
/// whatever the stride sign. Errors instead of reading past the buffer when
/// it is smaller than the frame it was announced as (a mid-stream format
/// change, a misbehaving driver or a crafted file).
pub fn copy_rows(buf: &windows::Win32::Media::MediaFoundation::IMFMediaBuffer, dst: &mut [u8], row: usize, h: usize) -> windows::core::Result<()> {
    use windows::Win32::Media::MediaFoundation::{IMF2DBuffer2, MF2DBuffer_LockFlags_Read};
    use windows::core::{Error, Interface};
    let bad = || Error::from(windows::Win32::Foundation::E_UNEXPECTED);
    if dst.len() < row * h {
        return Err(bad());
    }
    unsafe {
        let b2: IMF2DBuffer2 = buf.cast()?;
        let (mut scan0, mut pitch, mut start, mut len) = (std::ptr::null_mut(), 0i32, std::ptr::null_mut(), 0u32);
        b2.Lock2DSize(MF2DBuffer_LockFlags_Read, &mut scan0, &mut pitch, &mut start, &mut len)?;
        let (lo, hi) = (start as usize, start as usize + len as usize);
        let row_at = |y: usize| scan0 as isize as i128 + y as i128 * pitch as i128;
        let fits = (pitch.unsigned_abs() as usize) >= row
            && h > 0
            && [row_at(0), row_at(h - 1)].iter().all(|&a| a >= lo as i128 && a + row as i128 <= hi as i128);
        if fits {
            for y in 0..h {
                std::ptr::copy_nonoverlapping(row_at(y) as usize as *const u8, dst.as_mut_ptr().add(y * row), row);
            }
        }
        b2.Unlock2D()?;
        if fits { Ok(()) } else { Err(bad()) }
    }
}

/// Wraps a COM pointer that is used from more than one thread. D3D11 is
/// made thread safe with `ID3D11Multithread::SetMultithreadProtected`.
pub struct SendBox<T>(pub T);
unsafe impl<T> Send for SendBox<T> {}
unsafe impl<T> Sync for SendBox<T> {}

impl<T> SendBox<T> {
    /// Use this inside closures: it captures the whole box (which is Send)
    /// rather than just the inner field.
    pub fn get(&self) -> &T {
        &self.0
    }
}
