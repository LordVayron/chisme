//! Screen and window capture with Windows.Graphics.Capture. Every frame is copied on the GPU into one "latest" texture;
//! the pacer reads it on the constant-frame-rate grid.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{IInspectable, Interface};

use super::gpu::Gpu;
use super::{SendBox, now};

pub enum Target {
    Display(HMONITOR),
    Window(HWND),
}

#[derive(Default, Clone, Copy)]
pub struct FrameInfo {
    /// Frames received so far.
    pub seq: u64,
    /// Size of the picture inside the latest texture.
    pub content: (u32, u32),
    /// Capture time of the newest frame (host clock, seconds).
    pub pts: f64,
    pub latency_max: f64,
}

pub struct Capture {
    pub size: (u32, u32),
    pub latest: ID3D11Texture2D,
    pub info: Arc<Mutex<FrameInfo>>,
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    _item: GraphicsCaptureItem,
}

const FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

impl Capture {
    pub fn start(gpu: &Gpu, target: Target, cursor: bool) -> Result<Capture> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe {
            match target {
                Target::Display(h) => interop.CreateForMonitor(h),
                Target::Window(h) => interop.CreateForWindow(h),
            }
        }
        .context("this display or window can't be captured")?;
        let sz = item.Size()?;
        let size = (sz.Width.max(2) as u32, sz.Height.max(2) as u32);
        let latest = gpu.texture(size.0, size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET)?;
        let info = Arc::new(Mutex::new(FrameInfo { content: size, ..Default::default() }));

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&gpu.winrt, FORMAT, 2, sz)?;
        let state = SendBox((gpu.clone(), latest.clone(), Mutex::new(sz)));
        let info2 = info.clone();
        pool.FrameArrived(&TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |pool, _| {
            let (gpu, latest, pool_size) = state.get();
            let Some(pool) = pool.as_ref() else { return Ok(()) };
            let frame = pool.TryGetNextFrame()?;
            let content = frame.ContentSize()?;
            let pts = frame.SystemRelativeTime()?.Duration as f64 * 1e-7;
            let src: ID3D11Texture2D = unsafe { frame.Surface()?.cast::<IDirect3DDxgiInterfaceAccess>()?.GetInterface()? };
            let (w, h) = ((content.Width.max(1) as u32).min(size.0), (content.Height.max(1) as u32).min(size.1));
            let bx = D3D11_BOX { left: 0, top: 0, front: 0, right: w, bottom: h, back: 1 };
            unsafe { gpu.context.CopySubresourceRegion(latest, 0, 0, 0, 0, &src, 0, Some(&bx)) };
            drop(frame);
            // A resized window: recreate the pool at the new size.
            let mut ps = pool_size.lock().unwrap();
            if content != *ps && content.Width > 0 && content.Height > 0 {
                pool.Recreate(&gpu.winrt, FORMAT, 2, content)?;
                *ps = content;
            }
            let mut i = info2.lock().unwrap();
            i.seq += 1;
            i.content = (w, h);
            i.pts = pts;
            i.latency_max = i.latency_max.max(now() - pts);
            Ok(())
        }))?;

        let session = pool.CreateCaptureSession(&item)?;
        session.SetIsCursorCaptureEnabled(cursor)?;
        // Windows 11 draws a yellow border around captured content unless
        // told not to; older versions don't have the setting.
        let _ = session.SetIsBorderRequired(false);
        session.StartCapture()?;
        Ok(Capture { size, latest, info, session, pool, _item: item })
    }

    pub fn info(&self) -> FrameInfo {
        *self.info.lock().unwrap()
    }

    pub fn stop(&self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
