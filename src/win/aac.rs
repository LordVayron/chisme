//! Real-time AAC-LC encoding (48 kHz stereo) with Windows' AAC encoder MFT,
//! for streams and the replay buffer.

use anyhow::{Context, Result};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;

/// AudioSpecificConfig for AAC-LC, 48 kHz, stereo.
pub const AAC_ASC: [u8; 2] = [0x11, 0x90];
const RATE: f64 = 48000.0;
const FRAME: usize = 1024;

/// One raw AAC frame (1024 samples).
#[derive(Clone)]
pub struct AacFrame {
    pub pts: f64,
    pub data: Vec<u8>,
}

pub struct AacEncoder {
    mft: IMFTransform,
    out_size: u32,
    /// Host time of the first sample, and samples encoded since.
    t0: Option<f64>,
    frames_out: u64,
    samples_in: i64,
}

fn audio_type(subtype: &windows::core::GUID, avg_bytes: u32) -> Result<IMFMediaType> {
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
        } else {
            t.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?; // raw frames
        }
        Ok(t)
    }
}

impl AacEncoder {
    /// `kbps`: 96, 128, 160 or 192 (what Windows' encoder supports).
    pub fn new(kbps: u32) -> Result<AacEncoder> {
        let avg = match kbps {
            0..=111 => 12000,
            112..=143 => 16000,
            144..=175 => 20000,
            _ => 24000,
        };
        unsafe {
            let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Audio, guidSubtype: MFAudioFormat_PCM };
            let output = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Audio, guidSubtype: MFAudioFormat_AAC };
            let (mut list, mut n) = (std::ptr::null_mut(), 0u32);
            MFTEnumEx(MFT_CATEGORY_AUDIO_ENCODER, MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER, Some(&input), Some(&output), &mut list, &mut n)?;
            let acts: Vec<IMFActivate> = (0..n as usize).filter_map(|i| (*list.add(i)).take()).collect();
            CoTaskMemFree(Some(list as _));
            let mft: IMFTransform = acts.first().context("no AAC encoder (Windows N? install the Media Feature Pack)")?.ActivateObject()?;
            // The AAC encoder wants its input type first.
            mft.SetInputType(0, &audio_type(&MFAudioFormat_PCM, 192000)?, 0)?;
            mft.SetOutputType(0, &audio_type(&MFAudioFormat_AAC, avg)?, 0)?;
            let info = mft.GetOutputStreamInfo(0)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            Ok(AacEncoder { mft, out_size: info.cbSize.max(4096), t0: None, frames_out: 0, samples_in: 0 })
        }
    }

    /// Encode stereo frames starting at host time `t`; returns finished AAC
    /// frames. Timestamps follow the sample count, so they stay continuous.
    pub fn push(&mut self, t: f64, frames: &[[f32; 2]]) -> Result<Vec<AacFrame>> {
        if frames.is_empty() {
            return Ok(vec![]);
        }
        let t0 = *self.t0.get_or_insert(t);
        let pcm: Vec<i16> = frames.iter().flatten().map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect();
        let len = (pcm.len() * 2) as u32;
        let mut out = vec![];
        unsafe {
            let buf = MFCreateMemoryBuffer(len)?;
            let mut p = std::ptr::null_mut();
            buf.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(pcm.as_ptr() as *const u8, p, len as usize);
            buf.Unlock()?;
            buf.SetCurrentLength(len)?;
            let s = MFCreateSample()?;
            s.AddBuffer(&buf)?;
            s.SetSampleTime((self.samples_in as f64 / RATE * 1e7) as i64)?;
            s.SetSampleDuration((frames.len() as f64 / RATE * 1e7) as i64)?;
            self.samples_in += frames.len() as i64;
            self.mft.ProcessInput(0, &s, 0)?;
            loop {
                let ob = MFCreateMemoryBuffer(self.out_size)?;
                let os = MFCreateSample()?;
                os.AddBuffer(&ob)?;
                let mut db = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: std::mem::ManuallyDrop::new(Some(os.clone())), dwStatus: 0, pEvents: std::mem::ManuallyDrop::new(None) }];
                let mut status = 0u32;
                let r = self.mft.ProcessOutput(0, &mut db, &mut status);
                drop(std::mem::ManuallyDrop::take(&mut db[0].pSample));
                drop(std::mem::ManuallyDrop::take(&mut db[0].pEvents));
                match r {
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                    Err(e) => return Err(e.into()),
                    Ok(()) => {}
                }
                let (mut q, mut n) = (std::ptr::null_mut(), 0u32);
                ob.Lock(&mut q, None, Some(&mut n))?;
                let data = std::slice::from_raw_parts(q, n as usize).to_vec();
                ob.Unlock()?;
                if !data.is_empty() {
                    out.push(AacFrame { pts: t0 + (self.frames_out * FRAME as u64) as f64 / RATE, data });
                    self.frames_out += 1;
                }
            }
        }
        Ok(out)
    }
}
