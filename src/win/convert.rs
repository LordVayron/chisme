//! Scaling and color conversion with the D3D11 video processor. Used twice per frame when the
//! webcam is on: capture → BGRA canvas at the output size, then canvas →
//! NV12 (BT.709 limited range) for the encoder.

use std::mem::ManuallyDrop;

use anyhow::{Context, Result};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::Interface;

use super::gpu::Gpu;

pub struct Blitter {
    vdev: ID3D11VideoDevice,
    vctx: ID3D11VideoContext1,
    en: ID3D11VideoProcessorEnumerator,
    vp: ID3D11VideoProcessor,
    /// Views by texture pointer: the same few textures are used every frame.
    inputs: Vec<(usize, ID3D11VideoProcessorInputView)>,
    outputs: Vec<(usize, ID3D11VideoProcessorOutputView)>,
    out_size: (u32, u32),
}

fn rect(x: u32, y: u32, w: u32, h: u32) -> RECT {
    RECT { left: x as i32, top: y as i32, right: (x + w) as i32, bottom: (y + h) as i32 }
}

impl Blitter {
    /// `yuv`: the output is NV12 (BT.709 limited range); else BGRA.
    pub fn new(gpu: &Gpu, in_size: (u32, u32), out_size: (u32, u32), fps: u32, yuv: bool) -> Result<Blitter> {
        let vdev: ID3D11VideoDevice = gpu.device.cast()?;
        let vctx: ID3D11VideoContext1 = gpu.context.cast().context("the GPU driver lacks ID3D11VideoContext1")?;
        let rate = DXGI_RATIONAL { Numerator: fps, Denominator: 1 };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: in_size.0,
            InputHeight: in_size.1,
            OutputFrameRate: rate,
            OutputWidth: out_size.0,
            OutputHeight: out_size.1,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_QUALITY,
        };
        unsafe {
            let en = vdev.CreateVideoProcessorEnumerator(&desc)?;
            let vp = vdev.CreateVideoProcessor(&en, 0)?;
            vctx.VideoProcessorSetStreamFrameFormat(&vp, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            vctx.VideoProcessorSetStreamAutoProcessingMode(&vp, 0, false);
            vctx.VideoProcessorSetStreamColorSpace1(&vp, 0, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
            let black = if yuv {
                vctx.VideoProcessorSetOutputColorSpace1(&vp, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709);
                D3D11_VIDEO_COLOR { Anonymous: D3D11_VIDEO_COLOR_0 { YCbCr: D3D11_VIDEO_COLOR_YCbCrA { Y: 16.0 / 255.0, Cb: 0.5, Cr: 0.5, A: 1.0 } } }
            } else {
                vctx.VideoProcessorSetOutputColorSpace1(&vp, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
                D3D11_VIDEO_COLOR { Anonymous: D3D11_VIDEO_COLOR_0 { RGBA: D3D11_VIDEO_COLOR_RGBA { R: 0.0, G: 0.0, B: 0.0, A: 1.0 } } }
            };
            // Bars (if the picture doesn't fill the frame) are black.
            vctx.VideoProcessorSetOutputBackgroundColor(&vp, yuv, &black);
            vctx.VideoProcessorSetOutputTargetRect(&vp, true, Some(&rect(0, 0, out_size.0, out_size.1)));
            Ok(Blitter { vdev, vctx, en, vp, inputs: vec![], outputs: vec![], out_size })
        }
    }

    fn input(&mut self, tex: &ID3D11Texture2D) -> Result<ID3D11VideoProcessorInputView> {
        let key = tex.as_raw() as usize;
        if let Some((_, v)) = self.inputs.iter().find(|(k, _)| *k == key) {
            return Ok(v.clone());
        }
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
        };
        let mut view = None;
        unsafe { self.vdev.CreateVideoProcessorInputView(tex, &self.en, &desc, Some(&mut view))? };
        let view = view.context("no input view")?;
        self.inputs.push((key, view.clone()));
        Ok(view)
    }

    fn output(&mut self, tex: &ID3D11Texture2D) -> Result<ID3D11VideoProcessorOutputView> {
        let key = tex.as_raw() as usize;
        if let Some((_, v)) = self.outputs.iter().find(|(k, _)| *k == key) {
            return Ok(v.clone());
        }
        let desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
        };
        let mut view = None;
        unsafe { self.vdev.CreateVideoProcessorOutputView(tex, &self.en, &desc, Some(&mut view))? };
        let view = view.context("no output view")?;
        self.outputs.push((key, view.clone()));
        Ok(view)
    }

    /// Scale the top-left `src_size` of `src` into `dst_rect` (x, y, w, h)
    /// of `dst`; the rest of `dst` becomes black.
    pub fn blit(&mut self, src: &ID3D11Texture2D, src_size: (u32, u32), dst: &ID3D11Texture2D, dst_rect: (u32, u32, u32, u32)) -> Result<()> {
        self.blit_rect(src, (0, 0, src_size.0, src_size.1), dst, dst_rect)
    }

    /// Scale the `src_rect` (x, y, w, h) part of `src` into `dst_rect` of `dst`.
    pub fn blit_rect(&mut self, src: &ID3D11Texture2D, src_rect: (u32, u32, u32, u32), dst: &ID3D11Texture2D, dst_rect: (u32, u32, u32, u32)) -> Result<()> {
        let full = (0, 0, self.out_size.0, self.out_size.1);
        self.blit_target(src, src_rect, dst, dst_rect, full)
    }

    /// Scale `src_rect` of `src` to fill all of `dst` (`dst_size` pixels),
    /// whatever size `dst` is.
    pub fn blit_sized(&mut self, src: &ID3D11Texture2D, src_rect: (u32, u32, u32, u32), dst: &ID3D11Texture2D, dst_size: (u32, u32)) -> Result<()> {
        let full = (0, 0, dst_size.0, dst_size.1);
        self.blit_target(src, src_rect, dst, full, full)
    }

    /// Like `blit`, but leaves everything outside `dst_rect` as it is (to
    /// draw a picture on top of another).
    pub fn blit_into(&mut self, src: &ID3D11Texture2D, src_size: (u32, u32), dst: &ID3D11Texture2D, dst_rect: (u32, u32, u32, u32)) -> Result<()> {
        self.blit_target(src, (0, 0, src_size.0, src_size.1), dst, dst_rect, dst_rect)
    }

    /// `blit_rect` that leaves everything outside `dst_rect` as it is.
    pub fn blit_rect_into(&mut self, src: &ID3D11Texture2D, src_rect: (u32, u32, u32, u32), dst: &ID3D11Texture2D, dst_rect: (u32, u32, u32, u32)) -> Result<()> {
        self.blit_target(src, src_rect, dst, dst_rect, dst_rect)
    }

    fn blit_target(&mut self, src: &ID3D11Texture2D, src_rect: (u32, u32, u32, u32), dst: &ID3D11Texture2D, dst_rect: (u32, u32, u32, u32), target: (u32, u32, u32, u32)) -> Result<()> {
        let input = self.input(src)?;
        let output = self.output(dst)?;
        unsafe {
            // The background color fills the target rect outside dst_rect.
            let (tx, ty, tw, th) = target;
            self.vctx.VideoProcessorSetOutputTargetRect(&self.vp, true, Some(&rect(tx, ty, tw, th)));
            let (sx, sy, sw, sh) = src_rect;
            self.vctx.VideoProcessorSetStreamSourceRect(&self.vp, 0, true, Some(&rect(sx, sy, sw, sh)));
            let (x, y, w, h) = dst_rect;
            self.vctx.VideoProcessorSetStreamDestRect(&self.vp, 0, true, Some(&rect(x, y, w, h)));
            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                OutputIndex: 0,
                InputFrameOrField: 0,
                PastFrames: 0,
                FutureFrames: 0,
                ppPastSurfaces: std::ptr::null_mut(),
                pInputSurface: ManuallyDrop::new(Some(input)),
                ppFutureSurfaces: std::ptr::null_mut(),
                ppPastSurfacesRight: std::ptr::null_mut(),
                pInputSurfaceRight: ManuallyDrop::new(None),
                ppFutureSurfacesRight: std::ptr::null_mut(),
            };
            let streams = [stream];
            let r = self.vctx.VideoProcessorBlt(&self.vp, &output, 0, &streams);
            let [mut s] = streams;
            ManuallyDrop::drop(&mut s.pInputSurface);
            r.context("video processor blit")?;
        }
        Ok(())
    }
}

/// NV12 targets for the encoder, used round-robin: the encoder may still be
/// reading the previous ones.
pub struct Ring {
    textures: Vec<ID3D11Texture2D>,
    next: usize,
}

impl Ring {
    pub fn new(gpu: &Gpu, size: (u32, u32), n: usize) -> Result<Ring> {
        let textures = (0..n).map(|_| gpu.texture(size.0, size.1, DXGI_FORMAT_NV12, D3D11_BIND_RENDER_TARGET)).collect::<Result<_>>()?;
        Ok(Ring { textures, next: 0 })
    }

    pub fn next(&mut self) -> ID3D11Texture2D {
        let t = self.textures[self.next].clone();
        self.next = (self.next + 1) % self.textures.len();
        t
    }
}
