//! Scene transitions on the GPU. When the scene changes, the last frame of
//! the old one is kept; for the next moments each new frame is blended with
//! it in place on the canvas: a fade, or a slide (the new scene pushes the
//! old one out to the left).

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{PCSTR, s};

use super::gpu::Gpu;
use super::overlay::Overlay;

/// How one scene turns into the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Cut,
    #[default]
    Fade,
    Slide,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Cut, Kind::Fade, Kind::Slide];

    pub fn label(self) -> &'static str {
        use crate::i18n::l;
        match self {
            Kind::Cut => l("Cut", "Corte"),
            Kind::Fade => l("Fade", "Fundido"),
            Kind::Slide => l("Slide", "Deslizar"),
        }
    }
}

/// The transition setting: kind and length.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    pub kind: Kind,
    /// Milliseconds.
    pub ms: u32,
}

impl Default for Transition {
    fn default() -> Self {
        Transition { kind: Kind::Fade, ms: 400 }
    }
}

const SHADER: &str = r#"
cbuffer P : register(b0) {
    float k;       // 0 = all old, 1 = all new
    float mode;    // 1 fade, 2 slide
    float2 pad;
};
Texture2D old : register(t0);
Texture2D cur : register(t1);
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
    if (mode < 1.5) {
        return float4(lerp(old.Sample(lin, i.uv).rgb, cur.Sample(lin, i.uv).rgb, k), 1);
    }
    // The new scene enters from the right and pushes the old one out.
    float edge = 1.0 - k;
    if (i.uv.x < edge) {
        return float4(old.Sample(lin, i.uv + float2(k, 0)).rgb, 1);
    }
    return float4(cur.Sample(lin, i.uv - float2(edge, 0)).rgb, 1);
}
"#;

#[repr(C)]
#[derive(Default)]
struct Params {
    k: f32,
    mode: f32,
    pad: [f32; 2],
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let (mut code, mut err) = (None, None);
    let r = unsafe { D3DCompile(SHADER.as_ptr() as _, SHADER.len(), None, None, None, entry, target, 0, 0, &mut code, Some(&mut err)) };
    if r.is_err() {
        let msg = err
            .map(|e| unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(e.GetBufferPointer() as *const u8, e.GetBufferSize())).into_owned() })
            .unwrap_or_default();
        return Err(anyhow!("transition shader: {msg}"));
    }
    code.context("no shader code")
}

fn bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

/// Blends a canvas with the frame it showed when the scene changed.
pub struct Blend {
    gpu: Gpu,
    size: (u32, u32),
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cb: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    old: ID3D11Texture2D,
    old_srv: ID3D11ShaderResourceView,
    cur: ID3D11Texture2D,
    cur_srv: ID3D11ShaderResourceView,
    canvas_rtv: ID3D11RenderTargetView,
    /// When the running transition started (seconds), and its setting.
    pub running: Option<(f64, Transition)>,
}

impl Blend {
    pub fn new(gpu: &Gpu, canvas: &ID3D11Texture2D, size: (u32, u32)) -> Result<Blend> {
        let d = &gpu.device;
        unsafe {
            let (mut vs, mut ps) = (None, None);
            d.CreateVertexShader(bytes(&compile(s!("vs"), s!("vs_5_0"))?), None, Some(&mut vs))?;
            d.CreatePixelShader(bytes(&compile(s!("ps"), s!("ps_5_0"))?), None, Some(&mut ps))?;
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
            let old = gpu.texture(size.0, size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_SHADER_RESOURCE)?;
            let cur = gpu.texture(size.0, size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_SHADER_RESOURCE)?;
            let mut rtv = None;
            d.CreateRenderTargetView(canvas, None, Some(&mut rtv))?;
            Ok(Blend {
                gpu: gpu.clone(),
                size,
                vs: vs.context("vs")?,
                ps: ps.context("ps")?,
                cb: cb.context("cb")?,
                sampler: sampler.context("sampler")?,
                old_srv: Overlay::srv_of(gpu, &old)?,
                old,
                cur_srv: Overlay::srv_of(gpu, &cur)?,
                cur,
                canvas_rtv: rtv.context("rtv")?,
                running: None,
            })
        }
    }

    /// The scene is about to change: keep what `canvas` shows now.
    pub fn start(&mut self, canvas: &ID3D11Texture2D, now: f64, t: Transition) {
        if t.kind == Kind::Cut || t.ms == 0 {
            self.running = None;
            return;
        }
        unsafe { self.gpu.context.CopyResource(&self.old, canvas) };
        self.running = Some((now, t));
    }

    /// Blend the new frame on `canvas` with the kept one, while running.
    pub fn apply(&mut self, canvas: &ID3D11Texture2D, now: f64) {
        let Some((t0, t)) = self.running else { return };
        let x = ((now - t0) * 1000.0 / t.ms.max(1) as f64).clamp(0.0, 1.0) as f32;
        if x >= 1.0 {
            self.running = None;
            return;
        }
        // Ease in and out.
        let k = x * x * (3.0 - 2.0 * x);
        let p = Params { k, mode: if t.kind == Kind::Slide { 2.0 } else { 1.0 }, ..Default::default() };
        let c = &self.gpu.context;
        unsafe {
            c.CopyResource(&self.cur, canvas);
            c.UpdateSubresource(&self.cb, 0, None, &p as *const _ as _, 0, 0);
            c.OMSetRenderTargets(Some(&[Some(self.canvas_rtv.clone())]), None);
            c.OMSetBlendState(None, None, 0xffff_ffff);
            c.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: self.size.0 as f32, Height: self.size.1 as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
            c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            c.IASetInputLayout(None);
            c.VSSetShader(&self.vs, None);
            c.PSSetShader(&self.ps, None);
            c.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            c.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            c.PSSetShaderResources(0, Some(&[Some(self.old_srv.clone()), Some(self.cur_srv.clone())]));
            c.Draw(3, 0);
            c.PSSetShaderResources(0, Some(&[None, None]));
            c.OMSetRenderTargets(None, None);
        }
    }
}
