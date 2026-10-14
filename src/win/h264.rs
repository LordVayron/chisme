//! Real-time hardware H.264 encoding for streams and the replay buffer.
//!
//! The GPU vendor's Media Foundation encoder (NVENC, AMF or Quick Sync) runs
//! as an asynchronous MFT on its own thread: constant bitrate, no B-frames,
//! low-latency mode, a fixed keyframe interval and keyframes on request.
//! Frames arrive as NV12 textures on the shared D3D11 device; encoded access
//! units come back in Annex B form.

use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};
use windows::core::Interface;

use super::SendBox;
use super::convert::Ring;
use super::gpu::Gpu;

#[derive(Clone, Debug, PartialEq)]
pub struct H264Config {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Bits per second (constant bitrate).
    pub bitrate: u32,
    pub keyframe_secs: f64,
}

/// One encoded access unit.
pub struct Encoded {
    /// Host time of the frame (seconds).
    pub pts: f64,
    pub key: bool,
    /// Annex B (start codes).
    pub data: Vec<u8>,
}

struct Frame {
    tex: SendBox<ID3D11Texture2D>,
    pts: f64,
    key: bool,
}

pub struct H264Encoder {
    pub cfg: H264Config,
    /// NV12 targets to draw frames into before `encode`.
    ring: Ring,
    tx: SyncSender<Frame>,
    rx: Receiver<Encoded>,
    want_key: bool,
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
    /// Frames dropped because the encoder fell behind.
    pub dropped: u64,
}

fn variant_u32(v: u32) -> VARIANT {
    let mut x = VARIANT::default();
    unsafe {
        (*x.Anonymous.Anonymous).vt = VT_UI4;
        (*x.Anonymous.Anonymous).Anonymous.ulVal = v;
    }
    x
}

fn variant_bool(v: bool) -> VARIANT {
    let mut x = VARIANT::default();
    unsafe {
        (*x.Anonymous.Anonymous).vt = VT_BOOL;
        (*x.Anonymous.Anonymous).Anonymous.boolVal = windows::Win32::Foundation::VARIANT_BOOL(if v { -1 } else { 0 });
    }
    x
}

fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

/// The first hardware H.264 encoder MFT, activated.
fn hardware_encoder() -> Result<IMFTransform> {
    unsafe {
        let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_NV12 };
        let output = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
        let (mut list, mut n) = (std::ptr::null_mut(), 0u32);
        MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER, Some(&input), Some(&output), &mut list, &mut n)?;
        let acts: Vec<IMFActivate> = (0..n as usize).filter_map(|i| (*list.add(i)).take()).collect();
        CoTaskMemFree(Some(list as _));
        let first = acts.first().context("no hardware H.264 encoder on this PC")?;
        Ok(first.ActivateObject()?)
    }
}

fn configure(mft: &IMFTransform, gpu: &Gpu, cfg: &H264Config) -> Result<(IMFDXGIDeviceManager, ICodecAPI)> {
    unsafe {
        let attrs = mft.GetAttributes()?;
        attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
        let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
        let (mut token, mut manager) = (0u32, None);
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager: IMFDXGIDeviceManager = manager.context("no DXGI device manager")?;
        manager.ResetDevice(&gpu.device, token)?;
        mft.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize).context("the encoder can't use the GPU device")?;

        let api: ICodecAPI = mft.cast()?;
        let gop = ((cfg.fps as f64 * cfg.keyframe_secs).round() as u32).max(1);
        // Rate control before the types (some encoders read it there).
        let _ = api.SetValue(&CODECAPI_AVEncCommonRateControlMode, &variant_u32(eAVEncCommonRateControlMode_CBR.0 as u32));
        let _ = api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(cfg.bitrate));
        let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(gop));
        let _ = api.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &variant_u32(0));
        let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &variant_bool(true));

        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        out.SetUINT32(&MF_MT_AVG_BITRATE, cfg.bitrate)?;
        out.SetUINT64(&MF_MT_FRAME_SIZE, pack(cfg.width, cfg.height))?;
        out.SetUINT64(&MF_MT_FRAME_RATE, pack(cfg.fps, 1))?;
        out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
        mft.SetOutputType(0, &out, 0).context("the encoder rejected the stream size")?;

        let inp = MFCreateMediaType()?;
        inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        inp.SetUINT64(&MF_MT_FRAME_SIZE, pack(cfg.width, cfg.height))?;
        inp.SetUINT64(&MF_MT_FRAME_RATE, pack(cfg.fps, 1))?;
        inp.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        mft.SetInputType(0, &inp, 0).context("the encoder rejected NV12 input")?;

        // Again after the types, for encoders that reset them.
        let _ = api.SetValue(&CODECAPI_AVEncCommonRateControlMode, &variant_u32(eAVEncCommonRateControlMode_CBR.0 as u32));
        let _ = api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(cfg.bitrate));
        let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(gop));

        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
        mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        Ok((manager, api))
    }
}

/// SPS/PPS from the output type, for encoders that don't repeat them in
/// the stream.
fn sequence_header(mft: &IMFTransform) -> Option<Vec<u8>> {
    unsafe {
        let t = mft.GetOutputCurrentType(0).ok()?;
        let n = t.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER).ok()?;
        let mut buf = vec![0u8; n as usize];
        t.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut buf, None).ok()?;
        Some(buf)
    }
}

fn take_output(mft: &IMFTransform, base: f64) -> Result<Option<Encoded>> {
    unsafe {
        let mut out = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(None), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
        let mut status = 0u32;
        let r = mft.ProcessOutput(0, &mut out, &mut status);
        let sample = ManuallyDrop::take(&mut out[0].pSample);
        drop(ManuallyDrop::take(&mut out[0].pEvents));
        match r {
            Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                // Output type renegotiation: take the encoder's proposal.
                let t = mft.GetOutputAvailableType(0, 0)?;
                mft.SetOutputType(0, &t, 0)?;
                return Ok(None);
            }
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(()) => {}
        }
        let Some(sample) = sample else { return Ok(None) };
        let buf = sample.ConvertToContiguousBuffer()?;
        let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
        buf.Lock(&mut p, None, Some(&mut len))?;
        let mut data = std::slice::from_raw_parts(p, len as usize).to_vec();
        buf.Unlock()?;
        let key = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) == 1;
        if key && !crate::nal::has_parameter_sets(&data)
            && let Some(mut h) = sequence_header(mft)
        {
            h.extend_from_slice(&data);
            data = h;
        }
        let t = sample.GetSampleTime().unwrap_or(0);
        Ok(Some(Encoded { pts: base + t as f64 * 1e-7, key: key || crate::nal::has_idr(&data), data }))
    }
}

impl H264Encoder {
    pub fn start(gpu: &Gpu, cfg: H264Config) -> Result<H264Encoder> {
        let ring = Ring::new(gpu, (cfg.width, cfg.height), 8)?;
        let (tx, frames) = sync_channel::<Frame>(3);
        let (out_tx, rx) = sync_channel::<Encoded>(240);
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = sync_channel::<Result<()>>(1);
        let (g, c, st, err) = (SendBox(gpu.clone()), cfg.clone(), stop.clone(), error.clone());
        let thread = std::thread::Builder::new().name("h264".into()).spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let setup = (|| -> Result<_> {
                let mft = hardware_encoder()?;
                let (manager, api) = configure(&mft, g.get(), &c)?;
                let events: IMFMediaEventGenerator = mft.cast()?;
                Ok((mft, manager, api, events))
            })();
            let (mft, _manager, api, events) = match setup {
                Ok(x) => {
                    let _ = ready_tx.send(Ok(()));
                    x
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let mut base: Option<f64> = None;
            let period = (1e7 / c.fps as f64) as i64;
            let r = (|| -> Result<()> {
                while !st.load(Ordering::Relaxed) {
                    let ev = match unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                        Ok(ev) => ev,
                        Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => {
                            std::thread::sleep(Duration::from_millis(1));
                            continue;
                        }
                        Err(e) => return Err(e.into()),
                    };
                    let typ = unsafe { ev.GetType()? } as i32;
                    if typ == METransformNeedInput.0 {
                        // Wait for the next frame (checking for stop).
                        let f = loop {
                            match frames.recv_timeout(Duration::from_millis(50)) {
                                Ok(f) => break Some(f),
                                Err(RecvTimeoutError::Timeout) if !st.load(Ordering::Relaxed) => continue,
                                Err(_) => break None,
                            }
                        };
                        let Some(f) = f else { return Ok(()) };
                        let b = *base.get_or_insert(f.pts);
                        unsafe {
                            if f.key {
                                let _ = api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &variant_u32(1));
                            }
                            let buf = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &f.tex.0, 0, false)?;
                            let len = buf.cast::<IMF2DBuffer>()?.GetContiguousLength()?;
                            buf.SetCurrentLength(len)?;
                            let s = MFCreateSample()?;
                            s.AddBuffer(&buf)?;
                            s.SetSampleTime(((f.pts - b) * 1e7).round() as i64)?;
                            s.SetSampleDuration(period)?;
                            mft.ProcessInput(0, &s, 0)?;
                        }
                    } else if typ == METransformHaveOutput.0 {
                        if let Some(e) = take_output(&mft, base.unwrap_or(0.0))? {
                            // A full queue means nobody reads: drop.
                            let _ = out_tx.try_send(e);
                        }
                    }
                }
                Ok(())
            })();
            if let Err(e) = r {
                *err.lock().unwrap() = Some(format!("{e:#}"));
            }
            unsafe {
                let _ = mft.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            }
        })?;
        ready_rx.recv().map_err(|_| anyhow!("encoder thread ended"))??;
        Ok(H264Encoder { cfg, ring, tx, rx, want_key: true, stop, error, thread: Some(thread), dropped: 0 })
    }

    /// The next NV12 texture to draw a frame into.
    pub fn target(&mut self) -> ID3D11Texture2D {
        self.ring.next()
    }

    pub fn force_keyframe(&mut self) {
        self.want_key = true;
    }

    /// Queue `tex` (from `target`) captured at host time `pts`.
    pub fn encode(&mut self, tex: ID3D11Texture2D, pts: f64) -> Result<()> {
        if let Some(e) = self.error.lock().unwrap().clone() {
            bail!("encoder: {e}");
        }
        match self.tx.try_send(Frame { tex: SendBox(tex), pts, key: self.want_key }) {
            Ok(()) => self.want_key = false,
            Err(TrySendError::Full(_)) => self.dropped += 1,
            Err(TrySendError::Disconnected(_)) => bail!("encoder stopped"),
        }
        Ok(())
    }

    /// Encoded frames ready so far.
    pub fn output(&self) -> Vec<Encoded> {
        self.rx.try_iter().collect()
    }
}

impl Drop for H264Encoder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
