//! Webcam capture with the Media Foundation source reader. Frames are decoded to BGRA by
//! Media Foundation and uploaded into one "latest" texture.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow};
use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::core::PWSTR;

use super::gpu::Gpu;
use super::{SendBox, now};

// MF_SOURCE_READER_FIRST_VIDEO_STREAM
const VIDEO: u32 = 0xFFFF_FFFC;

fn activates() -> Result<Vec<IMFActivate>> {
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.context("no attributes")?;
        attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
        let (mut list, mut count) = (std::ptr::null_mut(), 0u32);
        MFEnumDeviceSources(&attrs, &mut list, &mut count)?;
        let out = (0..count as usize).filter_map(|i| (*list.add(i)).take()).collect();
        CoTaskMemFree(Some(list as _));
        Ok(out)
    }
}

fn name(a: &IMFActivate) -> String {
    unsafe {
        let (mut p, mut len) = (PWSTR::null(), 0u32);
        if a.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut p, &mut len).is_err() {
            return "Camera".into();
        }
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as _));
        s
    }
}

/// Webcam names.
pub fn cameras() -> Result<Vec<String>> {
    Ok(activates()?.iter().map(name).collect())
}

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

#[derive(Default, Clone, Copy)]
pub struct CamInfo {
    pub seq: u64,
    pub first_at: Option<f64>,
}

pub struct Camera {
    pub name: String,
    pub size: (u32, u32),
    pub fps: f64,
    pub latest: ID3D11Texture2D,
    pub info: Arc<Mutex<CamInfo>>,
    /// Virtual background: frames for the segmentation worker (a 256x256
    /// copy, sent when it is free).
    pub seg: Arc<Mutex<Option<std::sync::mpsc::SyncSender<Vec<f32>>>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Camera {
    /// Open a camera ("" or "default" = the first one, else a name
    /// substring) at its best mode up to 1080p, preferring 30 fps or more.
    pub fn start(gpu: &Gpu, query: &str) -> Result<Camera> {
        let stop = Arc::new(AtomicBool::new(false));
        let info = Arc::new(Mutex::new(CamInfo::default()));
        let seg: Arc<Mutex<Option<std::sync::mpsc::SyncSender<Vec<f32>>>>> = Arc::default();
        let seg2 = seg.clone();
        let (ready_tx, ready_rx) = sync_channel::<Result<(String, (u32, u32), f64, SendBox<ID3D11Texture2D>)>>(1);
        let (gpu2, stop2, info2, q) = (SendBox(gpu.clone()), stop.clone(), info.clone(), query.to_lowercase());
        let thread = std::thread::Builder::new().name("camera".into()).spawn(move || {
            let gpu = gpu2.get();
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            let opened = (|| -> Result<_> {
                let list = activates()?;
                let act = if q.is_empty() || q == "default" {
                    list.first()
                } else {
                    list.iter().find(|a| name(a).to_lowercase().contains(&q))
                }
                .ok_or_else(|| if list.is_empty() { anyhow!("no camera found") } else { anyhow!("no camera matches {q:?}; see `chisme devices`") })?;
                let cam_name = name(act);
                unsafe {
                    let source: IMFMediaSource = act.ActivateObject()?;
                    let mut attrs = None;
                    MFCreateAttributes(&mut attrs, 1)?;
                    let attrs = attrs.context("no attributes")?;
                    // Decode MJPEG/NV12/YUY2 to RGB32 inside Media Foundation.
                    attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
                    let reader = MFCreateSourceReaderFromMediaSource(&source, &attrs)?;
                    // Pick the native mode: largest up to 1080p, then highest
                    // frame rate, preferring at least 29 fps.
                    let mut best: Option<(IMFMediaType, (u32, u32), f64)> = None;
                    let score = |s: (u32, u32), fps: f64| ((s.0 * s.1 <= 1920 * 1080) as u8, (fps >= 29.0) as u8, s.0 * s.1, (fps * 100.0) as u32);
                    for i in 0.. {
                        let Ok(t) = reader.GetNativeMediaType(VIDEO, i) else { break };
                        let size = unpack(t.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0));
                        let (n, d) = unpack(t.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0));
                        let fps = n as f64 / d.max(1) as f64;
                        if size.0 == 0 || best.as_ref().is_some_and(|b| score(b.1, b.2) >= score(size, fps)) {
                            continue;
                        }
                        best = Some((t, size, fps));
                    }
                    let (native, size, fps) = best.context("the camera reports no video modes")?;
                    reader.SetCurrentMediaType(VIDEO, None, &native)?;
                    let out = MFCreateMediaType()?;
                    out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                    out.SetUINT64(&MF_MT_FRAME_SIZE, ((size.0 as u64) << 32) | size.1 as u64)?;
                    reader.SetCurrentMediaType(VIDEO, None, &out).context("the camera's format can't be converted")?;
                    // Render-target too: the video processor reads it (virtual background blur).
                    let tex = gpu.texture(size.0, size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET)?;
                    Ok((cam_name, size, fps, tex, reader, source))
                }
            })();
            let (size, tex, reader, source) = match opened {
                Ok((n, size, fps, tex, reader, source)) => {
                    let _ = ready_tx.send(Ok((n, size, fps, SendBox(tex.clone()))));
                    (size, tex, reader, source)
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let row = size.0 as usize * 4;
            let mut pixels = vec![0u8; row * size.1 as usize];
            while !stop2.load(Ordering::Relaxed) {
                let (mut flags, mut sample) = (0u32, None);
                let read = unsafe { reader.ReadSample(VIDEO, 0, None, Some(&mut flags), None, Some(&mut sample)) };
                if read.is_err() || flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 || flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                    break;
                }
                let Some(sample) = sample else { continue };
                // Copy rows top-down whatever the buffer's stride sign.
                let ok = unsafe {
                    (|| -> windows::core::Result<()> {
                        let buf = sample.ConvertToContiguousBuffer()?;
                        super::copy_rows(&buf, &mut pixels, row, size.1 as usize)
                    })()
                };
                if ok.is_err() {
                    continue;
                }
                unsafe { gpu.context.UpdateSubresource(&tex, 0, None, pixels.as_ptr() as _, row as u32, 0) };
                if let Some(tx) = seg2.lock().unwrap().as_ref() {
                    // Dropped when the worker is still busy with the last one.
                    let _ = tx.try_send(crate::segment::downsample(&pixels, size.0 as usize, size.1 as usize));
                }
                let mut i = info2.lock().unwrap();
                i.seq += 1;
                i.first_at.get_or_insert_with(now);
            }
            unsafe {
                let _ = source.Shutdown();
            }
        })?;
        let (name, size, fps, tex) = ready_rx.recv().unwrap_or_else(|_| Err(anyhow!("camera thread ended")))?;
        Ok(Camera { name, size, fps, latest: tex.0, info, seg, stop, thread: Some(thread) })
    }

    pub fn info(&self) -> CamInfo {
        *self.info.lock().unwrap()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.stop();
    }
}
