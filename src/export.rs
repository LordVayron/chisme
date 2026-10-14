//! Vertical 9:16 export (TikTok, Reels, Shorts) of any recording or replay:
//! Media Foundation decodes the MP4, the D3D11 video processor crops the
//! center 9:16 and scales it to 1080x1920, and the hardware encoder writes
//! a new MP4 with the audio re-encoded alongside.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PropVariantToUInt64;
use windows::core::HSTRING;

use crate::live::geometry;
use crate::win::convert::{Blitter, Ring};
use crate::win::encoder::{Codec, Encoder, VideoSpec};
use crate::win::gpu::Gpu;

const ANY_STREAM: u32 = 0xFFFF_FFFE;
const FIRST_VIDEO: u32 = 0xFFFF_FFFC;
const FIRST_AUDIO: u32 = 0xFFFF_FFFD;
const MEDIASOURCE: u32 = 0xFFFF_FFFF;

/// `clip.mp4` → `clip vertical.mp4`.
pub fn vertical_path(src: &Path) -> PathBuf {
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    src.with_file_name(format!("{stem} vertical.mp4"))
}

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

/// Export `src` as a vertical 1080x1920 MP4 next to it. `progress`: 0..1.
pub fn vertical(src: &Path, progress: impl Fn(f32)) -> Result<PathBuf> {
    let dst = vertical_path(src);
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs: IMFAttributes = attrs.context("no attributes")?;
        // Lets the reader convert the decoder's NV12 to RGB32.
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(src.as_os_str()), &attrs).with_context(|| format!("can't open {}", src.display()))?;
        // Decoded frames as BGRA in memory (the decoder may use the GPU).
        let vt = MFCreateMediaType()?;
        vt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        vt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        reader.SetCurrentMediaType(FIRST_VIDEO, None, &vt).context("this file's video can't be decoded")?;
        let cur = reader.GetCurrentMediaType(FIRST_VIDEO)?;
        let (w, h) = unpack(cur.GetUINT64(&MF_MT_FRAME_SIZE)?);
        let (num, den) = unpack(cur.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or((60 << 32) | 1));
        let fps = ((num as f64 / den.max(1) as f64).round() as u32).clamp(1, 240);
        let has_audio = {
            let at = MFCreateMediaType()?;
            at.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            at.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
            at.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000)?;
            at.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
            reader.SetCurrentMediaType(FIRST_AUDIO, None, &at).is_ok()
        };
        let duration = reader
            .GetPresentationAttribute(MEDIASOURCE, &MF_PD_DURATION)
            .ok()
            .and_then(|v| PropVariantToUInt64(&v).ok())
            .map_or(0.0, |d| d as f64 * 1e-7);
        // Which actual stream index is the video.
        let video_index = (0..8u32)
            .find(|&i| reader.GetCurrentMediaType(i).ok().and_then(|t| t.GetGUID(&MF_MT_MAJOR_TYPE).ok()) == Some(MFMediaType_Video))
            .context("no video stream")?;

        let gpu = Gpu::new()?;
        let (src_rect, size) = geometry((w, h), (1080, 1920), true);
        let frame = gpu.texture(w, h, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET)?;
        let mut blit = Blitter::new(&gpu, (w, h), size, fps, true)?;
        let mut ring = Ring::new(&gpu, size, 12)?;
        let bitrate = (size.0 * size.1) as f64 * fps as f64 * 0.1;
        let spec = VideoSpec { width: size.0, height: size.1, fps, codec: Codec::H264, bitrate: bitrate as u32 };
        let enc = Encoder::open(&gpu, &dst, &spec, has_audio as usize)?;
        let row = w as usize * 4;
        let mut pixels = vec![0u8; row * h as usize];
        let mut last_index = -1i64;
        loop {
            let (mut index, mut flags, mut ts, mut sample) = (0u32, 0u32, 0i64, None);
            reader.ReadSample(ANY_STREAM, 0, Some(&mut index), Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 && sample.is_none() {
                // One stream ended; stop when the reader has nothing left.
                if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                    bail!("decoding failed");
                }
                let more = reader.ReadSample(ANY_STREAM, 0, Some(&mut index), Some(&mut flags), Some(&mut ts), Some(&mut sample));
                if more.is_err() || sample.is_none() {
                    break;
                }
            }
            let Some(s) = sample else { continue };
            let secs = ts as f64 * 1e-7;
            let buf = s.ConvertToContiguousBuffer()?;
            if index == video_index {
                // Rows top-down whatever the stride sign, into the frame texture.
                crate::win::copy_rows(&buf, &mut pixels, row, h as usize)?;
                let tex = ring.next();
                gpu.locked(|| -> Result<()> {
                    gpu.context.UpdateSubresource(&frame, 0, None, pixels.as_ptr() as _, row as u32, 0);
                    blit.blit_rect(&frame, src_rect, &tex, (0, 0, size.0, size.1))
                })?;
                let i = (secs * fps as f64).round() as i64;
                if i > last_index {
                    enc.write(&tex, i)?;
                    last_index = i;
                }
                if duration > 0.0 {
                    progress((secs / duration).clamp(0.0, 1.0) as f32);
                }
            } else if has_audio {
                let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                buf.Lock(&mut p, None, Some(&mut len))?;
                let v = std::slice::from_raw_parts(p as *const f32, len as usize / 4);
                let frames: Vec<[f32; 2]> = v.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
                buf.Unlock()?;
                enc.write_audio(0, secs.max(0.0), &frames)?;
            }
        }
        if last_index < 0 {
            bail!("no video frames decoded");
        }
        enc.finish()?;
        progress(1.0);
    }
    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(vertical_path(Path::new("C:/v/Chisme replay 1.mp4")), PathBuf::from("C:/v/Chisme replay 1 vertical.mp4"));
    }
}
