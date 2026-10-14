//! Virtual background on the GPU: blends the webcam with a background (a
//! blurred copy of itself, or a picture) through the person mask, into a
//! texture the size of the camera that the compositor draws in its place.
//!
//! - **Mask:** 256x256 from `vbg::Worker`, sampled bilinearly (soft edges)
//!   and tightened with a smoothstep so hair and shoulders, which the model
//!   is less sure about, read as person.
//! - **Blur:** the camera scaled down 8x by the video processor, then a
//!   13-tap Gaussian on that small copy: cheap, and wide.
//! - **Picture:** cropped to fill the camera's shape.

use anyhow::{Context, Result, anyhow};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM};
use windows::core::{PCSTR, s};

use super::convert::Blitter;
use super::gpu::Gpu;
use super::overlay::Overlay;
use crate::segment::SIZE;

const SHADER: &str = r#"
cbuffer P : register(b0) {
    float2 texel;    // one texel of the small blurred copy, in uv
    float mode;      // 1 blur, 2 picture
    float radius;    // blur radius in small texels
    float2 bgScale;  // picture uv = uv * bgScale + bgOff
    float2 bgOff;
    float edge0;
    float edge1;
    float2 pad;
};
Texture2D cam : register(t0);
Texture2D mask : register(t1);
Texture2D bg : register(t2);
SamplerState lin : register(s0);
struct V { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
V vs(uint id : SV_VertexID) {
    V o;
    float2 t = float2((id << 1) & 2, id & 2);
    o.uv = t;
    o.pos = float4(t * float2(2, -2) + float2(-1, 1), 0, 1);
    return o;
}
float4 ps(V i) : SV_Target {
    float3 c = cam.Sample(lin, i.uv).rgb;
    float m = smoothstep(edge0, edge1, mask.Sample(lin, i.uv).r);
    float3 b;
    if (mode < 1.5) {
        // Separable-looking 2D Gaussian: 13 taps on a ring pattern.
        float3 acc = bg.Sample(lin, i.uv).rgb * 0.2;
        float wsum = 0.2;
        [unroll] for (int k = 0; k < 12; k++) {
            float a = k * 0.5235988;
            float r = (k % 2 == 0) ? radius : radius * 0.5;
            float w = (k % 2 == 0) ? 0.05 : 0.083;
            acc += bg.Sample(lin, i.uv + float2(cos(a), sin(a)) * r * texel).rgb * w;
            wsum += w;
        }
        b = acc / wsum;
    } else {
        b = bg.Sample(lin, i.uv * bgScale + bgOff).rgb;
    }
    return float4(lerp(b, c, m), 1);
}
"#;

#[repr(C)]
#[derive(Default)]
struct Params {
    texel: [f32; 2],
    mode: f32,
    radius: f32,
    bg_scale: [f32; 2],
    bg_off: [f32; 2],
    edge0: f32,
    edge1: f32,
    pad: [f32; 2],
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let (mut code, mut err) = (None, None);
    let r = unsafe { D3DCompile(SHADER.as_ptr() as _, SHADER.len(), None, None, None, entry, target, 0, 0, &mut code, Some(&mut err)) };
    if r.is_err() {
        let msg = err
            .map(|e| unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(e.GetBufferPointer() as *const u8, e.GetBufferSize())).into_owned() })
            .unwrap_or_default();
        return Err(anyhow!("matte shader: {msg}"));
    }
    code.context("no shader code")
}

fn bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

pub struct Matte {
    gpu: Gpu,
    size: (u32, u32),
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cb: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    /// What the compositor draws as "the camera".
    pub out: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    mask: ID3D11Texture2D,
    mask_srv: ID3D11ShaderResourceView,
    small: ID3D11Texture2D,
    small_srv: ID3D11ShaderResourceView,
    small_size: (u32, u32),
    shrink: Blitter,
    /// The background picture: (path, size, view).
    picture: Option<(String, (u32, u32), Option<ID3D11ShaderResourceView>)>,
    cam_srv: ID3D11ShaderResourceView,
    pub has_mask: bool,
}

impl Matte {
    pub fn new(gpu: &Gpu, cam: &ID3D11Texture2D, size: (u32, u32), fps: u32) -> Result<Matte> {
        let d = &gpu.device;
        unsafe {
            let (vsb, psb) = (compile(s!("vs"), s!("vs_5_0"))?, compile(s!("ps"), s!("ps_5_0"))?);
            let (mut vs, mut ps) = (None, None);
            d.CreateVertexShader(bytes(&vsb), None, Some(&mut vs))?;
            d.CreatePixelShader(bytes(&psb), None, Some(&mut ps))?;
            let mut cb = None;
            let desc = D3D11_BUFFER_DESC { ByteWidth: size_of::<Params>() as u32, Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, ..Default::default() };
            d.CreateBuffer(&desc, None, Some(&mut cb))?;
            let mut sampler = None;
            let sdesc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            d.CreateSamplerState(&sdesc, Some(&mut sampler))?;
            let out = gpu.texture(size.0, size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE)?;
            let mut rtv = None;
            d.CreateRenderTargetView(&out, None, Some(&mut rtv))?;
            let mask = gpu.texture(SIZE as u32, SIZE as u32, DXGI_FORMAT_R8_UNORM, D3D11_BIND_SHADER_RESOURCE)?;
            let small_size = ((size.0 / 8).max(16) & !1, (size.1 / 8).max(16) & !1);
            let small = gpu.texture(small_size.0, small_size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE)?;
            Ok(Matte {
                gpu: gpu.clone(),
                size,
                vs: vs.context("vs")?,
                ps: ps.context("ps")?,
                cb: cb.context("cb")?,
                sampler: sampler.context("sampler")?,
                rtv: rtv.context("rtv")?,
                mask_srv: Overlay::srv_of(gpu, &mask)?,
                mask,
                small_srv: Overlay::srv_of(gpu, &small)?,
                small,
                small_size,
                shrink: Blitter::new(gpu, size, small_size, fps, false)?,
                picture: None,
                cam_srv: Overlay::srv_of(gpu, cam)?,
                out,
                has_mask: false,
            })
        }
    }

    /// The newest mask (256x256 bytes).
    pub fn set_mask(&mut self, data: &[u8]) {
        if data.len() == SIZE * SIZE {
            unsafe { self.gpu.context.UpdateSubresource(&self.mask, 0, None, data.as_ptr() as _, SIZE as u32, 0) };
            self.has_mask = true;
        }
    }

    /// The camera unchanged (virtual background off, or no mask yet).
    pub fn pass_through(&self, cam: &ID3D11Texture2D) {
        unsafe { self.gpu.context.CopyResource(&self.out, cam) };
    }

    /// Camera over a blurred copy of itself (`amount` 0..1).
    pub fn blur(&mut self, cam: &ID3D11Texture2D, amount: f32) -> Result<()> {
        self.shrink.blit(cam, self.size, &self.small, (0, 0, self.small_size.0, self.small_size.1))?;
        let p = Params {
            texel: [1.0 / self.small_size.0 as f32, 1.0 / self.small_size.1 as f32],
            mode: 1.0,
            radius: 1.0 + amount.clamp(0.0, 1.0) * 5.0,
            edge0: 0.25,
            edge1: 0.6,
            ..Default::default()
        };
        let bg = self.small_srv.clone();
        self.draw(&p, &bg);
        Ok(())
    }

    /// Camera over the picture at `path` (cropped to fill). False if the
    /// picture can't be read.
    pub fn picture(&mut self, path: &str) -> Result<bool> {
        if self.picture.as_ref().is_none_or(|p| p.0 != path) {
            let loaded = (|| {
                let img = image::open(path).ok()?.to_rgba8();
                let longest = img.width().max(img.height());
                let img = if longest > 2048 {
                    let k = 2048.0 / longest as f64;
                    image::imageops::resize(&img, ((img.width() as f64 * k) as u32).max(1), ((img.height() as f64 * k) as u32).max(1), image::imageops::FilterType::Triangle)
                } else {
                    img
                };
                let tex = self.gpu.texture(img.width(), img.height(), DXGI_FORMAT_R8G8B8A8_UNORM, D3D11_BIND_SHADER_RESOURCE).ok()?;
                unsafe { self.gpu.context.UpdateSubresource(&tex, 0, None, img.as_ptr() as _, img.width() * 4, 0) };
                Some(((img.width(), img.height()), Overlay::srv_of(&self.gpu, &tex).ok()?))
            })();
            self.picture = Some(match loaded {
                Some((size, srv)) => (path.to_string(), size, Some(srv)),
                None => (path.to_string(), (0, 0), None),
            });
        }
        let Some((_, size, Some(srv))) = self.picture.clone() else { return Ok(false) };
        // Cover: scale the picture so it fills the camera's aspect.
        let (ca, pa) = (self.size.0 as f32 / self.size.1 as f32, size.0 as f32 / size.1.max(1) as f32);
        let (sx, sy) = if pa > ca { (ca / pa, 1.0) } else { (1.0, pa / ca) };
        let p = Params { mode: 2.0, bg_scale: [sx, sy], bg_off: [(1.0 - sx) / 2.0, (1.0 - sy) / 2.0], edge0: 0.25, edge1: 0.6, ..Default::default() };
        self.draw(&p, &srv);
        Ok(true)
    }

    fn draw(&self, p: &Params, bg: &ID3D11ShaderResourceView) {
        let c = &self.gpu.context;
        unsafe {
            c.UpdateSubresource(&self.cb, 0, None, p as *const _ as _, 0, 0);
            c.OMSetRenderTargets(Some(&[Some(self.rtv.clone())]), None);
            c.OMSetBlendState(None, None, 0xffff_ffff);
            c.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: self.size.0 as f32, Height: self.size.1 as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
            c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            c.IASetInputLayout(None);
            c.VSSetShader(&self.vs, None);
            c.PSSetShader(&self.ps, None);
            c.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            c.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            c.PSSetShaderResources(0, Some(&[Some(self.cam_srv.clone()), Some(self.mask_srv.clone()), Some(bg.clone())]));
            c.Draw(3, 0);
            c.PSSetShaderResources(0, Some(&[None, None, None]));
            c.OMSetRenderTargets(None, None);
        }
    }
}
