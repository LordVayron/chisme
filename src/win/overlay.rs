//! Draws pictures (the webcam, the screen) onto the BGRA canvas with a D3D11
//! pixel shader, each in a box with
//! a shape mask, border and shadow from `compose::Decor`, built on the CPU
//! once per size and style and cached.

use anyhow::{Context, Result, anyhow};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;
use windows::core::{PCSTR, s};

use super::gpu::Gpu;
use crate::compose::{Decor, PipSpec, Scene, Shape, fit, overlay_rect, side_by_side};

const SHADER: &str = r#"
cbuffer P : register(b0) {
    float2 region;   // drawn area in pixels
    float2 origin;   // the box's top-left inside it
    float2 camSize;  // picture size inside the box (cover crop)
    float2 camOff;   // its offset inside the box
    float mirror;
    float useDecor;
    float clip;      // hide the picture outside its own area
    float useAlpha;  // images and text: use the texture's alpha
    float2 uvScale;  // the used part of the texture (window captures)
    float2 pad1;
    float4 border;
};
Texture2D cam : register(t0);
Texture2D decor : register(t1);
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
    float2 p = i.uv * region;
    float2 cu = (p - origin - camOff) / camSize;
    float inside = (clip > 0.5 && (any(cu < 0) || any(cu > 1))) ? 0 : 1;
    if (mirror > 0.5) cu.x = 1 - cu.x;
    float4 t = cam.Sample(lin, saturate(cu) * uvScale);
    float3 c = t.rgb;
    float sh = 0, ob = 0, ib = 1;
    if (useDecor > 0.5) {
        float4 d = decor.Load(int3(p, 0));
        sh = d.r; ob = d.g; ib = d.b;
    }
    ib *= inside;
    if (useAlpha > 0.5) { ib *= t.a; sh *= t.a; }
    // Premultiplied: shadow (black), then border, then the picture.
    float3 col = 0;
    float a = sh;
    col = border.rgb * ob + col * (1 - ob); a = ob + a * (1 - ob);
    col = c * ib + col * (1 - ib);          a = ib + a * (1 - ib);
    return float4(col, a);
}
"#;

#[repr(C)]
#[derive(Default)]
struct Params {
    region: [f32; 2],
    origin: [f32; 2],
    cam_size: [f32; 2],
    cam_off: [f32; 2],
    mirror: f32,
    use_decor: f32,
    clip: f32,
    use_alpha: f32,
    uv_scale: [f32; 2],
    pad1: [f32; 2],
    border: [f32; 4],
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<ID3DBlob> {
    let (mut code, mut err) = (None, None);
    let r = unsafe { D3DCompile(SHADER.as_ptr() as _, SHADER.len(), None, None, None, entry, target, 0, 0, &mut code, Some(&mut err)) };
    if r.is_err() {
        let msg = err
            .map(|e| unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(e.GetBufferPointer() as *const u8, e.GetBufferSize())).into_owned() })
            .unwrap_or_default();
        return Err(anyhow!("shader: {msg}"));
    }
    code.context("no shader code")
}

fn bytes(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

type DecorKey = (usize, usize, Shape, u64, u64, bool);

pub struct Overlay {
    gpu: Gpu,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cb: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    blend: ID3D11BlendState,
    rtv: ID3D11RenderTargetView,
    cam_srv: Option<ID3D11ShaderResourceView>,
    /// Decorations by size and style, most recent last.
    decor: Vec<(DecorKey, ID3D11ShaderResourceView, (usize, usize), (usize, usize))>,
    out: (u32, u32),
}

/// Keep at most this many decorations (layers × sizes while resizing).
const DECOR_CACHE: usize = 12;

impl Overlay {
    pub fn new(gpu: &Gpu, canvas: &ID3D11Texture2D, out: (u32, u32), cam: Option<&ID3D11Texture2D>) -> Result<Overlay> {
        let d = &gpu.device;
        unsafe {
            let (vsb, psb) = (compile(s!("vs"), s!("vs_5_0"))?, compile(s!("ps"), s!("ps_5_0"))?);
            let (mut vs, mut ps) = (None, None);
            d.CreateVertexShader(bytes(&vsb), None, Some(&mut vs))?;
            d.CreatePixelShader(bytes(&psb), None, Some(&mut ps))?;
            let mut cb = None;
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: size_of::<Params>() as u32,
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                ..Default::default()
            };
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
            let mut bdesc = D3D11_BLEND_DESC::default();
            bdesc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE,
                DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let mut blend = None;
            d.CreateBlendState(&bdesc, Some(&mut blend))?;
            let mut rtv = None;
            d.CreateRenderTargetView(canvas, None, Some(&mut rtv))?;
            let cam_srv = match cam {
                Some(t) => Some(Self::srv_of(gpu, t)?),
                None => None,
            };
            Ok(Overlay {
                gpu: gpu.clone(),
                vs: vs.context("vs")?,
                ps: ps.context("ps")?,
                cb: cb.context("cb")?,
                sampler: sampler.context("sampler")?,
                blend: blend.context("blend")?,
                rtv: rtv.context("rtv")?,
                cam_srv,
                decor: vec![],
                out,
            })
        }
    }

    /// A shader view of a texture, for `draw_texture`.
    pub fn srv_of(gpu: &Gpu, tex: &ID3D11Texture2D) -> Result<ID3D11ShaderResourceView> {
        let mut srv = None;
        unsafe { gpu.device.CreateShaderResourceView(tex, None, Some(&mut srv))? };
        srv.context("no shader view")
    }

    /// Fill the canvas with a color (0..1 RGB).
    pub fn clear_to(&self, rgb: [f32; 3]) {
        unsafe { self.gpu.context.ClearRenderTargetView(&self.rtv, &[rgb[0], rgb[1], rgb[2], 1.0]) };
    }

    fn decor_srv(&mut self, pip: &PipSpec, w: usize, h: usize) -> Result<(ID3D11ShaderResourceView, (usize, usize), (usize, usize))> {
        let key = Decor::key(pip, w, h, self.out.1 as f64);
        if let Some(i) = self.decor.iter().position(|d| d.0 == key) {
            let hit = self.decor.remove(i);
            let out = (hit.1.clone(), hit.2, hit.3);
            self.decor.push(hit);
            return Ok(out);
        }
        let decor = Decor::build(pip, w, h, self.out.1 as f64);
        let tex = self.gpu.texture(decor.size.0 as u32, decor.size.1 as u32, DXGI_FORMAT_R8G8B8A8_UNORM, D3D11_BIND_SHADER_RESOURCE)?;
        unsafe { self.gpu.context.UpdateSubresource(&tex, 0, None, decor.rgba.as_ptr() as _, decor.size.0 as u32 * 4, 0) };
        let srv = Self::srv_of(&self.gpu, &tex)?;
        if self.decor.len() >= DECOR_CACHE {
            self.decor.remove(0);
        }
        self.decor.push((key, srv.clone(), decor.size, decor.origin));
        Ok((srv, decor.size, decor.origin))
    }

    /// Draw the camera (`cam_size` pixels) for a fixed `scene`.
    pub fn draw(&mut self, scene: Scene, pip: &PipSpec, cam_size: (u32, u32)) -> Result<()> {
        let Some(cam) = self.cam_srv.clone() else { return Ok(()) };
        let (ow, oh) = (self.out.0 as f64, self.out.1 as f64);
        let aspect = cam_size.0 as f64 / cam_size.1.max(1) as f64;
        match scene {
            Scene::Cam | Scene::CamScreen => self.draw_texture(&cam, cam_size, [1.0, 1.0], None, pip),
            // The camera column keeps the overlay's border, shadow and
            // roundness, as a rounded rectangle.
            Scene::SideBySide => {
                let style = PipSpec { shape: if pip.shape == Shape::Rect { Shape::Rect } else { Shape::Rounded }, ..pip.clone() };
                self.draw_texture(&cam, cam_size, [1.0, 1.0], Some(side_by_side(ow, oh).1), &style)
            }
            _ => self.draw_texture(&cam, cam_size, [1.0, 1.0], Some(overlay_rect(pip, ow, oh, aspect)), pip),
        }
    }

    /// The camera's shader view, if there is a camera.
    pub fn cam(&self) -> Option<ID3D11ShaderResourceView> {
        self.cam_srv.clone()
    }

    /// Draw a picture (`src_size` pixels; `uv_scale` = the used part of its
    /// texture) filling `rect` (x, y, w, h; cropped to fill) with `style`'s
    /// shape, border, shadow and mirroring, or the whole frame with None.
    pub fn draw_texture(&mut self, srv: &ID3D11ShaderResourceView, src_size: (u32, u32), uv_scale: [f32; 2], rect: Option<[f64; 4]>, style: &PipSpec) -> Result<()> {
        self.draw_texture_alpha(srv, src_size, uv_scale, rect, style, false)
    }

    /// `draw_texture` for pictures with transparency (images, text).
    pub fn draw_texture_alpha(&mut self, srv: &ID3D11ShaderResourceView, src_size: (u32, u32), uv_scale: [f32; 2], rect: Option<[f64; 4]>, style: &PipSpec, alpha: bool) -> Result<()> {
        let (ow, oh) = (self.out.0 as f64, self.out.1 as f64);
        let (cw, ch) = (src_size.0.max(1) as f64, src_size.1.max(1) as f64);
        let mut p = Params { mirror: style.mirror as u32 as f32, clip: 1.0, uv_scale, use_alpha: alpha as u32 as f32, ..Default::default() };
        let (region_xy, decor) = if let Some([x, y, w, h]) = rect {
            let (dsrv, size, origin) = self.decor_srv(style, w as usize, h as usize)?;
            let (s, tx, ty) = fit(cw, ch, w, h, true);
            let [r, g, b] = style.border_color.map(|c| c as f32 / 255.0);
            p.region = [size.0 as f32, size.1 as f32];
            p.origin = [origin.0 as f32, origin.1 as f32];
            p.cam_size = [(cw * s) as f32, (ch * s) as f32];
            p.cam_off = [tx as f32, ty as f32];
            p.use_decor = 1.0;
            p.border = [r, g, b, 1.0];
            ((x - origin.0 as f64, y - origin.1 as f64), Some(dsrv))
        } else {
            // Full frame, cropped to fill.
            let (s, tx, ty) = fit(cw, ch, ow, oh, true);
            p.region = [ow as f32, oh as f32];
            p.cam_size = [(cw * s) as f32, (ch * s) as f32];
            p.cam_off = [tx as f32, ty as f32];
            ((0.0, 0.0), None)
        };
        let c = &self.gpu.context;
        unsafe {
            c.UpdateSubresource(&self.cb, 0, None, &p as *const _ as _, 0, 0);
            c.OMSetRenderTargets(Some(&[Some(self.rtv.clone())]), None);
            c.OMSetBlendState(&self.blend, Some(&[0.0; 4]), 0xffff_ffff);
            c.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: region_xy.0 as f32,
                TopLeftY: region_xy.1 as f32,
                Width: p.region[0],
                Height: p.region[1],
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            c.IASetInputLayout(None);
            c.VSSetShader(&self.vs, None);
            c.PSSetShader(&self.ps, None);
            c.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            c.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            c.PSSetShaderResources(0, Some(&[Some(srv.clone()), decor]));
            c.Draw(3, 0);
            // Unbind so the canvas can be read by the video processor.
            c.PSSetShaderResources(0, Some(&[None, None]));
            c.OMSetRenderTargets(None, None);
        }
        Ok(())
    }
}
