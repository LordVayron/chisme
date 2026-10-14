//! Hardware H.264/HEVC encoding into an MP4 with the Media Foundation sink
//! writer. The sink writer
//! picks the GPU vendor's encoder (NVENC, AMF or Quick Sync) and reads the
//! NV12 textures straight from the shared D3D11 device.

use std::path::Path;

use anyhow::{Context, Result};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::*;
use windows::core::{GUID, HSTRING, Interface};

use super::gpu::Gpu;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Codec {
    H264,
    Hevc,
}

pub struct VideoSpec {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: Codec,
    pub bitrate: u32,
}

pub struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
    audio: Vec<u32>,
    fps: u32,
    _manager: IMFDXGIDeviceManager,
}

/// 48 kHz stereo audio; the AAC encoder takes 16-bit PCM.
fn audio_type(subtype: &GUID, avg_bytes: u32) -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000)?;
        t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, avg_bytes)?;
        if *subtype == MFAudioFormat_PCM {
            t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
        }
        Ok(t)
    }
}

fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

fn video_type(subtype: &GUID, spec: &VideoSpec) -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, pack(spec.width, spec.height))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pack(spec.fps, 1))?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32)?;
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
        Ok(t)
    }
}

impl Encoder {
    /// An MP4 with one video track and `audio_tracks` AAC tracks (192 kb/s).
    pub fn open(gpu: &Gpu, path: &Path, spec: &VideoSpec, audio_tracks: usize) -> Result<Encoder> {
        unsafe {
            let mut token = 0u32;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
            let manager: IMFDXGIDeviceManager = manager.context("no DXGI device manager")?;
            manager.ResetDevice(&gpu.device, token)?;

            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 4)?;
            let attrs: IMFAttributes = attrs.context("no attributes")?;
            attrs.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, &manager)?;
            attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
            attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)?;
            let _ = std::fs::remove_file(path);
            let writer = MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, &attrs)
                .with_context(|| format!("can't create {}", path.display()))?;

            let out = video_type(if spec.codec == Codec::Hevc { &MFVideoFormat_HEVC } else { &MFVideoFormat_H264 }, spec)?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, spec.bitrate)?;
            if spec.codec == Codec::H264 {
                out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
            }
            let stream = writer.AddStream(&out).context("the encoder rejected the output format")?;
            let input = video_type(&MFVideoFormat_NV12, spec)?;
            writer
                .SetInputMediaType(stream, &input, None)
                .context("no encoder for this codec/size (try --codec h264 or a lower --resolution)")?;
            let mut audio = vec![];
            for _ in 0..audio_tracks {
                let s = writer.AddStream(&audio_type(&MFAudioFormat_AAC, 24000)?).context("AAC encoder")?;
                writer.SetInputMediaType(s, &audio_type(&MFAudioFormat_PCM, 192000)?, None).context("AAC input")?;
                audio.push(s);
            }
            writer.BeginWriting()?;
            Ok(Encoder { writer, stream, audio, fps: spec.fps, _manager: manager })
        }
    }

    /// Append stereo frames to audio track `track`, starting `secs` into the file.
    pub fn write_audio(&self, track: usize, secs: f64, frames: &[[f32; 2]]) -> Result<()> {
        if frames.is_empty() {
            return Ok(());
        }
        let pcm: Vec<i16> = frames.iter().flatten().map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect();
        let len = (pcm.len() * 2) as u32;
        unsafe {
            let buf = MFCreateMemoryBuffer(len)?;
            let mut p = std::ptr::null_mut();
            buf.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(pcm.as_ptr() as *const u8, p, len as usize);
            buf.Unlock()?;
            buf.SetCurrentLength(len)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            sample.SetSampleTime((secs * 1e7).round() as i64)?;
            sample.SetSampleDuration((frames.len() as f64 / 48000.0 * 1e7).round() as i64)?;
            self.writer.WriteSample(self.audio[track], &sample)?;
        }
        Ok(())
    }

    /// Append `tex` as output frame number `index`.
    pub fn write(&self, tex: &ID3D11Texture2D, index: i64) -> Result<()> {
        unsafe {
            let buf = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, tex, 0, false)?;
            let len = buf.cast::<IMF2DBuffer>()?.GetContiguousLength()?;
            buf.SetCurrentLength(len)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            let period = 10_000_000 / self.fps as i64;
            sample.SetSampleTime(index * 10_000_000 / self.fps as i64)?;
            sample.SetSampleDuration(period)?;
            self.writer.WriteSample(self.stream, &sample)?;
        }
        Ok(())
    }

    pub fn finish(self) -> Result<()> {
        unsafe { self.writer.Finalize().context("finishing the file") }
    }
}

pub fn startup() -> Result<()> {
    unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).context("Media Foundation is missing (Windows N edition? install the Media Feature Pack)") }
}

pub fn shutdown() {
    unsafe {
        let _ = MFShutdown();
    }
}
