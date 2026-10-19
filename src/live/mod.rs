//! Live outputs: streams (YouTube, Twitch, TikTok, X, any RTMP server) and
//! the replay buffer.
//!
//! Each output has its own hardware H.264 + AAC encode of the composited canvas, so
//! TikTok can get a vertical 1080x1920 crop at 30 fps while YouTube gets
//! 1080p60 landscape at the same time.

pub mod replay;
pub mod rtmp;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use crate::i18n::l;
use crate::nal;
use crate::win::aac::AacEncoder;
use crate::win::convert::Blitter;
use crate::win::gpu::Gpu;
use crate::win::h264::{H264Config, H264Encoder};
use replay::Ring;
use rtmp::RtmpOutput;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    Youtube,
    Twitch,
    Tiktok,
    Instagram,
    X,
    Custom,
}

/// Encode settings for a platform (from its published guidance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    /// Bounding box of the encode (the frame keeps its shape inside it).
    pub size: (u32, u32),
    pub max_fps: u32,
    pub kbps: u32,
    pub keyframe_secs: f64,
    pub audio_kbps: u32,
    /// Crop a landscape canvas to 9:16.
    pub vertical: bool,
}

impl Platform {
    pub const ALL: [Platform; 6] = [Platform::Youtube, Platform::Twitch, Platform::Tiktok, Platform::Instagram, Platform::X, Platform::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Platform::Youtube => "YouTube",
            Platform::Twitch => "Twitch",
            Platform::Tiktok => "TikTok",
            Platform::Instagram => "Instagram",
            Platform::X => "X",
            Platform::Custom => crate::i18n::l("Custom RTMP", "RTMP personalizado"),
        }
    }

    /// Credential Manager name for the stream key.
    pub fn id(self) -> &'static str {
        match self {
            Platform::Youtube => "youtube",
            Platform::Twitch => "twitch",
            Platform::Tiktok => "tiktok",
            Platform::Instagram => "instagram",
            Platform::X => "x",
            Platform::Custom => "custom",
        }
    }

    /// Fixed ingest URL; TikTok, X and custom servers give you one.
    pub fn default_url(self) -> Option<&'static str> {
        match self {
            Platform::Youtube => Some("rtmps://a.rtmps.youtube.com:443/live2"),
            Platform::Twitch => Some("rtmps://live.twitch.tv:443/app"),
            _ => None,
        }
    }

    /// Where to get the server URL and stream key.
    pub fn help(self) -> &'static str {
        match self {
            Platform::Youtube => l("Key: YouTube Studio → Go live.", "Clave: YouTube Studio → Emitir en vivo."),
            Platform::Twitch => l("Key: Creator Dashboard → Settings → Stream.", "Clave: Panel de control → Configuración → Transmisión."),
            Platform::Tiktok => l("URL and key: TikTok LIVE Studio (they change every time).", "URL y clave: TikTok LIVE Studio (cambian cada vez)."),
            Platform::Instagram => l("URL and key: instagram.com → Create → Live video → Streaming software.", "URL y clave: instagram.com → Crear → Video en vivo → Software de transmisión."),
            Platform::X => l("URL and key: X Media Studio → Producer → Sources.", "URL y clave: X Media Studio → Producer → Fuentes."),
            Platform::Custom => l("Restream, MediaMTX, nginx-rtmp or another server.", "Restream, MediaMTX, nginx-rtmp u otro servidor."),
        }
    }

    /// - YouTube: 1080p60 at 12 Mb/s (30 fps: 10 Mb/s), keyframes every 2 s.
    /// - Twitch: at most 1080p60 and 6 Mb/s.
    /// - TikTok: vertical 1080x1920, 30 fps, ~4.5 Mb/s CBR, AAC 128 kb/s.
    /// - Instagram (Live Producer): vertical 1080x1920, 30 fps, at most
    ///   6 Mb/s (4.5 used), AAC 128 kb/s, keyframes every 2 s.
    /// - X (Media Studio Producer): H.264, at most 1080p30, 9 Mb/s
    ///   recommended, AAC 128 kb/s max, keyframes every 3 s.
    pub fn preset(self, fps: u32) -> Preset {
        let p = |size, max_fps, kbps, keyframe_secs, audio_kbps, vertical| Preset { size, max_fps, kbps, keyframe_secs, audio_kbps, vertical };
        match self {
            Platform::Youtube => p((1920, 1080), 60, if fps > 30 { 12_000 } else { 10_000 }, 2.0, 160, false),
            Platform::Twitch => p((1920, 1080), 60, 6_000, 2.0, 160, false),
            Platform::Tiktok => p((1080, 1920), 30, 4_500, 2.0, 128, true),
            Platform::Instagram => p((1080, 1920), 30, 4_500, 2.0, 128, true),
            Platform::X => p((1920, 1080), 30, 9_000, 3.0, 128, false),
            Platform::Custom => p((1920, 1080), 60, 6_000, 2.0, 160, false),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StreamSpec {
    pub platform: Platform,
    pub url: String,
    pub key: String,
    pub vertical: bool,
    /// Override the preset's video bitrate.
    pub kbps: Option<u32>,
}

#[derive(Clone)]
pub struct VideoPacket {
    pub pts: f64,
    pub key: bool,
    /// AVCC (length-prefixed NAL units).
    pub avcc: Arc<Vec<u8>>,
}

#[derive(Clone)]
pub struct AudioPacket {
    pub pts: f64,
    pub data: Arc<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamStatus {
    pub id: u64,
    pub platform: Option<Platform>,
    pub name: String,
    pub state: String,
    pub live: bool,
    /// Measured send rate, bits per second.
    pub bitrate: f64,
    pub dropped: u64,
    pub reconnects: u64,
    pub duration: f64,
    pub error: Option<String>,
    pub size: (u32, u32),
    pub fps: u32,
}

/// Redact a secret (stream key) from text meant for logs and the UI.
/// A stream's state ("en vivo", "reintentando en 5 s"...) in the
/// interface language.
pub fn state_label(state: &str) -> String {
    match state {
        "conectando" => l("connecting", "conectando").into(),
        "en vivo" => l("live", "en vivo").into(),
        "reconectando" => l("reconnecting", "reconectando").into(),
        "detenido" => l("stopped", "detenido").into(),
        s => match s.strip_prefix("reintentando en ") {
            Some(rest) => crate::t!("retrying in {}", "reintentando en {}", rest),
            None => s.to_string(),
        },
    }
}

pub fn redact(text: &str, secret: &str) -> String {
    if secret.len() < 4 { text.to_string() } else { text.replace(secret, "••••") }
}

fn even(x: f64) -> u32 {
    ((x / 2.0).floor() as u32 * 2).max(16)
}

/// The part of the canvas to encode (x, y, w, h) and the encode size: a
/// centered 9:16 crop when `vertical` and the canvas is wider than that,
/// fitted inside `bbox` without upscaling.
pub fn geometry(canvas: (u32, u32), bbox: (u32, u32), vertical: bool) -> ((u32, u32, u32, u32), (u32, u32)) {
    let (cw, ch) = (canvas.0 as f64, canvas.1 as f64);
    let src = if vertical && cw / ch > 9.0 / 16.0 + 0.01 {
        let w = even(ch * 9.0 / 16.0);
        (((canvas.0 - w) / 2) & !1, 0, w, canvas.1)
    } else {
        (0, 0, canvas.0, canvas.1)
    };
    // Landscape boxes name the height; a portrait frame turns the box.
    let (bw, bh) = if (src.3 > src.2) != (bbox.1 > bbox.0) { (bbox.1, bbox.0) } else { bbox };
    let k = (bw as f64 / src.2 as f64).min(bh as f64 / src.3 as f64);
    // Never upscale, except vertical video: TikTok expects 1080x1920.
    if vertical {
        // Exactly the platform's size (TikTok and Instagram expect 1080x1920);
        // the crop's rounding stretches it by well under 1%.
        return (src, (bw, bh));
    }
    (src, (even(src.2 as f64 * k.min(1.0)), even(src.3 as f64 * k.min(1.0))))
}

enum Sink {
    Rtmp(RtmpOutput),
    Replay(Ring<VideoPacket, AudioPacket>),
}

/// One live encode and where it goes.
pub struct LiveOut {
    pub id: u64,
    pub platform: Option<Platform>,
    pub name: String,
    enc: H264Encoder,
    aac: AacEncoder,
    blit: Blitter,
    src: (u32, u32, u32, u32),
    /// Encode every Nth engine frame (60 → 30 fps for TikTok and X).
    every: i64,
    params: Option<(Vec<u8>, Vec<u8>)>,
    record: Vec<u8>,
    sink: Sink,
}

impl LiveOut {
    fn open(gpu: &Gpu, id: u64, canvas: (u32, u32), fps: u32, preset: Preset, sink: Sink, name: String, platform: Option<Platform>) -> Result<LiveOut> {
        let (src, size) = geometry(canvas, preset.size, preset.vertical);
        let every = (fps as f64 / preset.max_fps as f64).ceil().max(1.0) as i64;
        let live_fps = (fps as i64 / every).max(1) as u32;
        let enc = H264Encoder::start(gpu, H264Config { width: size.0, height: size.1, fps: live_fps, bitrate: preset.kbps * 1000, keyframe_secs: preset.keyframe_secs })
            .context("starting the hardware H.264 encoder")?;
        Ok(LiveOut {
            id,
            platform,
            name,
            aac: AacEncoder::new(preset.audio_kbps)?,
            blit: Blitter::new(gpu, canvas, size, live_fps, true)?,
            src,
            every,
            params: None,
            record: vec![],
            enc,
            sink,
        })
    }

    pub fn stream(gpu: &Gpu, id: u64, canvas: (u32, u32), fps: u32, spec: &StreamSpec) -> Result<LiveOut> {
        let mut preset = spec.platform.preset(fps);
        preset.vertical = spec.vertical;
        if spec.vertical {
            preset.size = (1080, 1920);
        }
        if let Some(k) = spec.kbps {
            preset.kbps = k;
        }
        let url = spec.platform.default_url().map(str::to_string).unwrap_or_else(|| spec.url.trim().to_string());
        if url.is_empty() {
            bail!(crate::t!("the {} server URL is missing", "falta la URL del servidor de {}", spec.platform.label()));
        }
        if spec.key.trim().is_empty() {
            bail!(crate::t!("the {} stream key is missing", "falta la clave de transmisión de {}", spec.platform.label()));
        }
        let (_, size) = geometry(canvas, preset.size, preset.vertical);
        let rtmp = RtmpOutput::start(&url, spec.key.trim(), size)?;
        Self::open(gpu, id, canvas, fps, preset, Sink::Rtmp(rtmp), spec.platform.label().to_string(), Some(spec.platform))
    }

    pub fn replay(gpu: &Gpu, id: u64, canvas: (u32, u32), fps: u32, secs: f64) -> Result<LiveOut> {
        let preset = Preset { size: (1920, 1080), max_fps: 60, kbps: 12_000, keyframe_secs: 1.0, audio_kbps: 160, vertical: false };
        Self::open(gpu, id, canvas, fps, preset, Sink::Replay(Ring::new(secs)), "Replay".into(), None)
    }

    pub fn is_replay(&self) -> bool {
        matches!(self.sink, Sink::Replay(_))
    }

    /// Draw engine frame `slot` of the canvas into the encoder (call with
    /// the GPU locked).
    pub fn frame(&mut self, canvas: &ID3D11Texture2D, slot: i64, pts: f64) -> Result<()> {
        if slot % self.every != 0 {
            return Ok(());
        }
        let tex = self.enc.target();
        let (w, h) = (self.enc.cfg.width, self.enc.cfg.height);
        self.blit.blit_rect(canvas, self.src, &tex, (0, 0, w, h))?;
        self.enc.encode(tex, pts)
    }

    /// Move encoded video to the stream or replay buffer.
    pub fn pump(&mut self) {
        if let Sink::Rtmp(r) = &mut self.sink
            && r.take_keyframe_request()
        {
            self.enc.force_keyframe();
        }
        for e in self.enc.output() {
            if e.key
                && let Some((sps, pps)) = nal::parameter_sets(&e.data)
                && self.params.as_ref() != Some(&(sps.clone(), pps.clone()))
            {
                self.record = nal::avcc_record(&sps, &pps);
                self.params = Some((sps, pps));
            }
            if self.params.is_none() {
                continue;
            }
            let avcc = nal::to_avcc(&e.data);
            match &mut self.sink {
                Sink::Rtmp(r) => r.video(e.pts, e.key, &avcc, &self.record),
                Sink::Replay(ring) => ring.push_video(VideoPacket { pts: e.pts, key: e.key, avcc: Arc::new(avcc) }),
            }
        }
    }

    pub fn audio(&mut self, t: f64, frames: &[[f32; 2]]) -> Result<()> {
        for f in self.aac.push(t, frames)? {
            match &mut self.sink {
                Sink::Rtmp(r) => r.audio(f.pts, &f.data),
                Sink::Replay(ring) => ring.push_audio(AudioPacket { pts: f.pts, data: Arc::new(f.data) }),
            }
        }
        Ok(())
    }

    pub fn status(&self) -> StreamStatus {
        let base = StreamStatus {
            id: self.id,
            platform: self.platform,
            name: self.name.clone(),
            size: (self.enc.cfg.width, self.enc.cfg.height),
            fps: self.enc.cfg.fps,
            ..Default::default()
        };
        match &self.sink {
            Sink::Rtmp(r) => {
                let mut s = r.status();
                s.dropped += self.enc.dropped;
                StreamStatus { id: base.id, platform: base.platform, name: base.name, size: base.size, fps: base.fps, ..s }
            }
            Sink::Replay(ring) => StreamStatus { state: format!("{:.0} s", ring.held()), live: true, bitrate: ring.bytes() as f64, ..base },
        }
    }

    /// (seconds held, bytes held) of a replay buffer.
    pub fn replay_stats(&self) -> Option<(f64, usize)> {
        match &self.sink {
            Sink::Replay(r) => Some((r.held(), r.bytes())),
            _ => None,
        }
    }

    pub fn set_replay_secs(&mut self, secs: f64) {
        if let Sink::Replay(r) = &mut self.sink {
            r.secs = secs;
        }
    }

    /// Save the replay buffer on a background thread.
    pub fn save_replay(&self, path: PathBuf) -> Result<std::thread::JoinHandle<Result<PathBuf>>> {
        let Sink::Replay(ring) = &self.sink else { bail!("not a replay buffer") };
        let (mut v, a) = ring.snapshot();
        // Audio runs a little behind (the mixer waits for late packets):
        // end the clip where the audio ends so both tracks match.
        if let Some(end) = a.last().map(|x| x.pts + 1024.0 / 48000.0) {
            v.retain(|x| x.pts < end);
        }
        let Some((sps, pps)) = self.params.clone() else { bail!("the replay buffer is still empty") };
        let size = (self.enc.cfg.width, self.enc.cfg.height);
        let fps = self.enc.cfg.fps;
        Ok(std::thread::spawn(move || {
            replay::write_mp4(&path, &v, &a, &sps, &pps, size, fps)?;
            Ok(path)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_follow_platform_guidance() {
        let t = Platform::Tiktok.preset(60);
        assert_eq!((t.size, t.max_fps, t.vertical, t.audio_kbps), ((1080, 1920), 30, true, 128));
        let x = Platform::X.preset(60);
        assert_eq!((x.max_fps, x.kbps, x.keyframe_secs, x.audio_kbps), (30, 9_000, 3.0, 128));
        assert_eq!(Platform::Twitch.preset(60).kbps, 6_000);
        assert_eq!(Platform::Youtube.preset(30).kbps, 10_000);
        assert!(Platform::Tiktok.default_url().is_none() && Platform::X.default_url().is_none());
    }

    #[test]
    fn vertical_crop_and_fit() {
        // Landscape 1080p canvas → centered 9:16 crop, scaled up to TikTok's size.
        let (src, size) = geometry((1920, 1080), (1080, 1920), true);
        assert_eq!(src, (656, 0, 606, 1080));
        assert_eq!(size, (1080, 1920));
        // A 4K canvas crops then scales into the box.
        let (src, size) = geometry((3840, 2160), (1080, 1920), true);
        assert_eq!(src.2, 1214);
        assert_eq!(size, (1080, 1920));
        // A portrait monitor needs no crop.
        let (src, size) = geometry((1080, 1920), (1080, 1920), true);
        assert_eq!((src, size), ((0, 0, 1080, 1920), (1080, 1920)));
        // Landscape stays landscape for YouTube.
        assert_eq!(geometry((2560, 1440), (1920, 1080), false).1, (1920, 1080));
        // A portrait canvas into a landscape preset turns the box.
        assert_eq!(geometry((1080, 1920), (1920, 1080), false).1, (1080, 1920));
    }

    #[test]
    fn keys_are_redacted() {
        assert_eq!(redact("publish refused for live_abc123", "live_abc123"), "publish refused for ••••");
    }
}
