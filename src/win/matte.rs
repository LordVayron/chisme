//! Virtual background on the GPU: blends the webcam with a background (a
//! blurred copy of itself, or a picture) through the person mask, into a
//! texture the size of the camera that the compositor draws in its place.
//!
//! Each frame, in passes:
//! 1. **Background estimate:** the camera at quarter size, blurred with a
//!    separable Gaussian that leaves the person out (weighted by 1 - mask),
//!    so the blur carries no halo of the person's colors.
//! 2. **Mask refine:** the 256x256 model mask upsampled to full size with a
//!    cross-bilateral filter guided by the camera, so edges follow hair and
//!    shoulders instead of the model's coarse grid.
//! 3. **Composite:** edge pixels have the old background's color taken out
//!    (using the estimate from 1), a little of the new background's light
//!    wraps onto the edge, and the result is blended over the background.

use anyhow::{Context, Result, anyhow};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::{PCSTR, s};

use super::convert::Blitter;
use super::gpu::Gpu;
use super::overlay::Overlay;
use crate::segment::SIZE;

const SHADER: &str = r#"
cbuffer P : register(b0) {
    float2 texel;    // one texel of the quarter-size textures, in uv
    float2 dir;      // blur direction (1,0) or (0,1)
    float radius;    // blur radius in quarter-size texels
    float mode;      // 1 blur, 2 picture
    float2 bgScale;  // picture uv = uv * bgScale + bgOff
    float2 bgOff;
    float edge0;     // mask -> alpha ramp
    float edge1;
    float bgLod;     // picture softness (mip level)
    float wrap;      // light wrap strength
    float2 pad;
};
Texture2D cam : register(t0);
Texture2D mask : register(t1);    // 256x256 model mask
Texture2D bg : register(t2);      // blur source, or the new background
Texture2D orig : register(t3);    // blurred estimate of the real background
Texture2D fine : register(t4);    // refined full-size mask
Texture2D guide : register(t5);   // the camera at quarter size
SamplerState lin : register(s0);
struct V { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
V vs(uint id : SV_VertexID) {
    V o;
    float2 t = float2((id << 1) & 2, id & 2);
    o.uv = t;
    o.pos = float4(t * float2(2, -2) + float2(-1, 1), 0, 1);
    return o;
}

// Person coverage for background sampling: generous, so the fringe the
// model is unsure about stays out of the background too.
float person(float2 uv) { return smoothstep(0.08, 0.45, mask.SampleLevel(lin, uv, 0).r); }

float4 ps_blur(V i) : SV_Target {
    float sigma = max(radius * 0.5, 0.75);
    float stepK = max(radius / 12.0, 1.0);
    float3 acc = 0;
    float ws = 0;
    [unroll] for (int k = -12; k <= 12; k++) {
        float o = k * stepK;
        float2 uv = i.uv + dir * texel * o;
        float w = exp(-o * o / (2.0 * sigma * sigma)) * (1.0 - person(uv) + 0.003);
        acc += bg.SampleLevel(lin, uv, 0).rgb * w;
        ws += w;
    }
    return float4(acc / ws, 1);
}

float4 ps_refine(V i) : SV_Target {
    float2 p = i.uv * 256.0 - 0.5;
    float2 b = floor(p);
    float2 f = p - b;
    float3 c = cam.SampleLevel(lin, i.uv, 0).rgb;
    float acc = 0;
    float ws = 0;
    [unroll] for (int y = -1; y <= 2; y++) {
        [unroll] for (int x = -1; x <= 2; x++) {
            float2 t = (b + float2(x, y) + 0.5) / 256.0;
            float m = mask.SampleLevel(lin, t, 0).r;
            float3 g = guide.SampleLevel(lin, t, 0).rgb;
            float spatial = max(0, 1.6 - abs(x - f.x)) * max(0, 1.6 - abs(y - f.y));
            float3 d = g - c;
            float w = spatial * (exp(-dot(d, d) * 60.0) + 0.002);
            acc += m * w;
            ws += w;
        }
    }
    float plain = mask.SampleLevel(lin, i.uv, 0).r;
    return float4(ws > 1e-5 ? acc / ws : plain, 0, 0, 1);
}

float4 ps(V i) : SV_Target {
    float3 c = cam.Sample(lin, i.uv).rgb;
    float m = smoothstep(edge0, edge1, fine.Sample(lin, i.uv).r);
    float3 o = orig.Sample(lin, i.uv).rgb;
    // Edge pixels mix the person with the old background: take that out.
    float3 f = saturate((c - (1.0 - m) * o) / max(m, 0.2));
    f = lerp(f, c, smoothstep(0.8, 1.0, m));
    float3 b;
    float3 light;
    if (mode < 1.5) {
        b = bg.Sample(lin, i.uv).rgb;
        light = b;
    } else {
        float2 uv = i.uv * bgScale + bgOff;
        b = bg.SampleLevel(lin, uv, bgLod).rgb;
        light = bg.SampleLevel(lin, uv, bgLod + 4.0).rgb;
    }
    // A little of the background's light wraps onto the edge.
    float e = saturate(m * (1.0 - m) * 4.0);
    f = lerp(f, light, e * wrap);
    return float4(lerp(b, f, m), 1);
}
"#;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Params {
    texel: [f32; 2],
    dir: [f32; 2],
    radius: f32,
    mode: f32,
    bg_scale: [f32; 2],
    bg_off: [f32; 2],
    edge0: f32,
    edge1: f32,
    bg_lod: f32,
    wrap: f32,
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

/// A render target and its shader view.
struct Target {
    tex: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    srv: ID3D11ShaderResourceView,
}

impl Target {
    fn new(gpu: &Gpu, size: (u32, u32), format: DXGI_FORMAT) -> Result<Target> {
        let tex = gpu.texture(size.0, size.1, format, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE)?;
        let mut rtv = None;
        unsafe { gpu.device.CreateRenderTargetView(&tex, None, Some(&mut rtv))? };
        Ok(Target { srv: Overlay::srv_of(gpu, &tex)?, rtv: rtv.context("rtv")?, tex })
    }
}

/// The background picture: path, size and view (with mipmaps).
type Picture = (String, (u32, u32), Option<ID3D11ShaderResourceView>);

pub struct Matte {
    gpu: Gpu,
    size: (u32, u32),
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    ps_blur: ID3D11PixelShader,
    ps_refine: ID3D11PixelShader,
    cb: ID3D11Buffer,
    sampler: ID3D11SamplerState,
    /// What the compositor draws as "the camera".
    pub out: ID3D11Texture2D,
    out_rtv: ID3D11RenderTargetView,
    mask: ID3D11Texture2D,
    mask_srv: ID3D11ShaderResourceView,
    /// The camera at quarter size, and two blur buffers.
    small: Target,
    blur_a: Target,
    blur_b: Target,
    small_size: (u32, u32),
    /// The refined full-size mask.
    fine: Target,
    shrink: Blitter,
    picture: Option<Picture>,
    cam: ID3D11Texture2D,
    cam_srv: ID3D11ShaderResourceView,
    pub has_mask: bool,
}

impl Matte {
    pub fn new(gpu: &Gpu, cam: &ID3D11Texture2D, size: (u32, u32), fps: u32) -> Result<Matte> {
        let d = &gpu.device;
        unsafe {
            let (mut vs, mut ps, mut ps_blur, mut ps_refine) = (None, None, None, None);
            d.CreateVertexShader(bytes(&compile(s!("vs"), s!("vs_5_0"))?), None, Some(&mut vs))?;
            d.CreatePixelShader(bytes(&compile(s!("ps"), s!("ps_5_0"))?), None, Some(&mut ps))?;
            d.CreatePixelShader(bytes(&compile(s!("ps_blur"), s!("ps_5_0"))?), None, Some(&mut ps_blur))?;
            d.CreatePixelShader(bytes(&compile(s!("ps_refine"), s!("ps_5_0"))?), None, Some(&mut ps_refine))?;
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
            let out = Target::new(gpu, size, DXGI_FORMAT_B8G8R8A8_UNORM)?;
            let mask = gpu.texture(SIZE as u32, SIZE as u32, DXGI_FORMAT_R8_UNORM, D3D11_BIND_SHADER_RESOURCE)?;
            let small_size = ((size.0 / 4).max(16) & !1, (size.1 / 4).max(16) & !1);
            Ok(Matte {
                gpu: gpu.clone(),
                size,
                vs: vs.context("vs")?,
                ps: ps.context("ps")?,
                ps_blur: ps_blur.context("ps_blur")?,
                ps_refine: ps_refine.context("ps_refine")?,
                cb: cb.context("cb")?,
                sampler: sampler.context("sampler")?,
                out: out.tex,
                out_rtv: out.rtv,
                mask_srv: Overlay::srv_of(gpu, &mask)?,
                mask,
                small: Target::new(gpu, small_size, DXGI_FORMAT_B8G8R8A8_UNORM)?,
                blur_a: Target::new(gpu, small_size, DXGI_FORMAT_B8G8R8A8_UNORM)?,
                blur_b: Target::new(gpu, small_size, DXGI_FORMAT_B8G8R8A8_UNORM)?,
                small_size,
                fine: Target::new(gpu, size, DXGI_FORMAT_R8_UNORM)?,
                shrink: Blitter::new(gpu, size, small_size, fps, false)?,
                picture: None,
                cam: cam.clone(),
                cam_srv: Overlay::srv_of(gpu, cam)?,
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

    /// Passes 1 and 2: the background estimate (blurred by `radius` quarter
    /// texels, into `blur_b`) and the refined mask.
    fn prepare(&mut self, radius: f32) -> Result<Params> {
        let cam = self.cam.clone();
        self.shrink.blit(&cam, self.size, &self.small.tex, (0, 0, self.small_size.0, self.small_size.1))?;
        let base = Params { texel: [1.0 / self.small_size.0 as f32, 1.0 / self.small_size.1 as f32], radius, edge0: 0.3, edge1: 0.7, wrap: 0.35, ..Default::default() };
        let (small, a) = (self.small.srv.clone(), self.blur_a.srv.clone());
        self.pass(&self.ps_blur, &self.blur_a.rtv, self.small_size, &Params { dir: [1.0, 0.0], ..base }, [Some(&small), None, None, None]);
        self.pass(&self.ps_blur, &self.blur_b.rtv, self.small_size, &Params { dir: [0.0, 1.0], ..base }, [Some(&a), None, None, None]);
        self.pass(&self.ps_refine, &self.fine.rtv, self.size, &base, [None, None, None, None]);
        Ok(base)
    }

    /// Camera over a blurred copy of itself (`amount` 0..1).
    pub fn blur(&mut self, _cam: &ID3D11Texture2D, amount: f32) -> Result<()> {
        let radius = 3.0 + amount.clamp(0.0, 1.0) * 17.0;
        let p = self.prepare(radius)?;
        let (b, fine) = (self.blur_b.srv.clone(), self.fine.srv.clone());
        self.pass(&self.ps, &self.out_rtv, self.size, &Params { mode: 1.0, ..p }, [Some(&b), Some(&b), Some(&fine), None]);
        Ok(())
    }

    /// Camera over the picture at `path` (cropped to fill). False if the
    /// picture can't be read.
    pub fn picture(&mut self, path: &str) -> Result<bool> {
        if self.picture.as_ref().is_none_or(|p| p.0 != path) {
            self.picture = Some(match self.load(path) {
                Some((size, srv)) => (path.to_string(), size, Some(srv)),
                None => (path.to_string(), (0, 0), None),
            });
        }
        let Some((_, size, Some(srv))) = self.picture.clone() else { return Ok(false) };
        let p = self.prepare(6.0)?;
        // Cover: scale the picture so it fills the camera's aspect.
        let (ca, pa) = (self.size.0 as f32 / self.size.1 as f32, size.0 as f32 / size.1.max(1) as f32);
        let (sx, sy) = if pa > ca { (ca / pa, 1.0) } else { (1.0, pa / ca) };
        // The mip that matches the camera's size, plus a touch of softness,
        // like a real room behind a webcam subject.
        let lod = ((size.0 as f32 * sx) / self.size.0 as f32).max(1.0).log2() + 0.6;
        let (orig, fine) = (self.blur_b.srv.clone(), self.fine.srv.clone());
        let p = Params { mode: 2.0, bg_scale: [sx, sy], bg_off: [(1.0 - sx) / 2.0, (1.0 - sy) / 2.0], bg_lod: lod, ..p };
        self.pass(&self.ps, &self.out_rtv, self.size, &p, [Some(&srv), Some(&orig), Some(&fine), None]);
        Ok(true)
    }

    /// The picture as a texture with mipmaps (for softness and light wrap).
    fn load(&self, path: &str) -> Option<((u32, u32), ID3D11ShaderResourceView)> {
        let img = image::open(path).ok()?.to_rgba8();
        let longest = img.width().max(img.height());
        let img = if longest > 2560 {
            let k = 2560.0 / longest as f64;
            image::imageops::resize(&img, ((img.width() as f64 * k) as u32).max(1), ((img.height() as f64 * k) as u32).max(1), image::imageops::FilterType::Triangle)
        } else {
            img
        };
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: img.width(),
                Height: img.height(),
                MipLevels: 0,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
            };
            let mut tex = None;
            self.gpu.device.CreateTexture2D(&desc, None, Some(&mut tex)).ok()?;
            let tex = tex?;
            self.gpu.context.UpdateSubresource(&tex, 0, None, img.as_ptr() as _, img.width() * 4, 0);
            let srv = Overlay::srv_of(&self.gpu, &tex).ok()?;
            self.gpu.context.GenerateMips(&srv);
            Some(((img.width(), img.height()), srv))
        }
    }

    /// One full-screen pass. `srvs` are t2..t5 (bg, orig, fine, guide; the
    /// guide defaults to the quarter-size camera); t0 and t1 are always the
    /// camera and the model mask.
    fn pass(&self, ps: &ID3D11PixelShader, rtv: &ID3D11RenderTargetView, size: (u32, u32), p: &Params, srvs: [Option<&ID3D11ShaderResourceView>; 4]) {
        let c = &self.gpu.context;
        unsafe {
            c.UpdateSubresource(&self.cb, 0, None, p as *const _ as _, 0, 0);
            c.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            c.OMSetBlendState(None, None, 0xffff_ffff);
            c.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: size.0 as f32, Height: size.1 as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
            c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            c.IASetInputLayout(None);
            c.VSSetShader(&self.vs, None);
            c.PSSetShader(ps, None);
            c.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            c.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            let [bg, orig, fine, guide] = srvs.map(|s| s.cloned());
            let guide = guide.or_else(|| Some(self.small.srv.clone()));
            c.PSSetShaderResources(0, Some(&[Some(self.cam_srv.clone()), Some(self.mask_srv.clone()), bg, orig, fine, guide]));
            c.Draw(3, 0);
            c.PSSetShaderResources(0, Some(&[None, None, None, None, None, None]));
            c.OMSetRenderTargets(None, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_back(gpu: &Gpu, tex: &ID3D11Texture2D, size: (u32, u32)) -> Vec<u8> {
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: size.0,
                Height: size.1,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..Default::default()
            };
            let mut st = None;
            gpu.device.CreateTexture2D(&desc, None, Some(&mut st)).unwrap();
            let st = st.unwrap();
            gpu.context.CopyResource(&st, tex);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            gpu.context.Map(&st, 0, D3D11_MAP_READ, 0, Some(&mut m)).unwrap();
            let mut out = vec![0u8; (size.0 * size.1 * 4) as usize];
            for y in 0..size.1 as usize {
                let src = std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), size.0 as usize * 4);
                out[y * size.0 as usize * 4..(y + 1) * size.0 as usize * 4].copy_from_slice(src);
            }
            gpu.context.Unmap(&st, 0);
            out
        }
    }

    /// CHISME_VBG_DIR=folder with cam*.png (1920x1080) and bg.png:
    /// writes <cam>_blur_light.png, _blur_strong.png and _picture.png there.
    #[test]
    #[ignore]
    fn render_check() {
        let dir = std::path::PathBuf::from(std::env::var("CHISME_VBG_DIR").unwrap());
        crate::win::encoder::startup().unwrap();
        let gpu = Gpu::new().unwrap();
        let seg = crate::segment::Segmenter::new().unwrap();
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("cam") && name.ends_with("_1080.png")) {
                continue;
            }
            let img = image::open(entry.path()).unwrap().to_rgba8();
            let (w, h) = img.dimensions();
            let mut bgra = img.into_raw();
            for p in bgra.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
            let cam = gpu.texture(w, h, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).unwrap();
            unsafe { gpu.context.UpdateSubresource(&cam, 0, None, bgra.as_ptr() as _, w * 4, 0) };
            let mask: Vec<u8> = seg.run(&bgra, w as usize, h as usize).unwrap().iter().map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8).collect();
            let mut m = Matte::new(&gpu, &cam, (w, h), 30).unwrap();
            m.set_mask(&mask);
            let stem = name.trim_end_matches("_1080.png");
            let mut save = |m: &Matte, tag: &str| {
                let mut px = read_back(&gpu, &m.out, (w, h));
                for p in px.chunks_exact_mut(4) {
                    p.swap(0, 2);
                    p[3] = 255;
                }
                image::save_buffer(dir.join(format!("{stem}_{tag}.png")), &px, w, h, image::ColorType::Rgba8).unwrap();
            };
            m.blur(&cam, 0.35).unwrap();
            save(&m, "blur_light");
            m.blur(&cam, 0.85).unwrap();
            save(&m, "blur_strong");
            assert!(m.picture(&dir.join("bg_1080.png").to_string_lossy()).unwrap());
            save(&m, "picture");
        }
    }
}
