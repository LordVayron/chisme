//! The capture engine: one long-lived session on its own thread that keeps
//! the screen, camera and audio running for the live preview, and attaches
//! an encoder while recording: one capture session serves both preview and
//! recording, so pressing Record is instant.
//!
//! Frames are emitted on an exact `1/fps` grid. Windows.Graphics.Capture,
//! like ScreenCaptureKit, only delivers frames when pixels change, so each
//! slot repeats the newest picture when nothing new arrived. Audio is mixed
//! on the same host clock and cut at the same pause boundaries.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use crate::brand::Brand;
use crate::compose::{PipSpec, Scene, Shape};
use crate::layout::{Layout, Source as LayerSource, crop_to, fit_in};
use crate::live::{LiveOut, StreamSpec, StreamStatus};
use crate::media::{Chunk, Decoder, Kind as MediaKind, Monitor};
use crate::mixer::{AudioKind, Mixer, RATE};
use crate::size::{fit, parse_size};
use crate::timeline::Pauses;
use crate::win::audio::{AudioCapture, Packet, Source};
use crate::win::camera::Camera;
use crate::win::capture::{Capture, Target};
use crate::win::control::{self, Listener};
use crate::win::convert::{Blitter, Ring};
use crate::win::encoder::{self, Codec, Encoder, VideoSpec};
use crate::win::gpu::Gpu;
use crate::win::now;
use crate::win::matte::Matte;
use crate::win::overlay::Overlay;

/// Audio track layout when both system audio and mic are recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioLayout {
    /// Track 1 = system+mic mix (what players use), tracks 2-3 = system and
    /// mic stems for editors.
    Both,
    /// Only the system+mic mix.
    Mix,
    /// Separate system and mic tracks only.
    Stems,
}

/// Settings that need the capture restarted to change.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub target: TargetSpec,
    pub resolution: String,
    pub fps: u32,
    pub codec: Codec,
    /// Mbit/s; None picks one from the size and frame rate.
    pub bitrate: Option<f64>,
    pub cursor: bool,
    pub system_audio: bool,
    /// Limit system audio to one app (name, true) or everything but it
    /// (name, false).
    pub app_audio: Option<(String, bool)>,
    /// RNNoise on the microphone.
    pub mic_denoise: bool,
    /// None = no mic; "" or "default" = the default mic; else a name part.
    pub mic: Option<String>,
    pub audio_tracks: AudioLayout,
    /// None = no camera; "" = the first one; else a name part.
    pub cam: Option<String>,
    /// Replace the picture and system audio with a flash + beep every
    /// second, for A/V sync checks (`scripts/synccheck.py`).
    pub test_pattern: bool,
}

/// Test-pattern audio: a 50 ms 1 kHz beep at frame `fps / 2` of every
/// second after `t0` (mid-second, never at a file's first sample, which AAC
/// fades in), for the samples from host time `from`.
fn beep_samples(t0: f64, fps: u32, from: f64, n: usize) -> Vec<[f32; 2]> {
    let at = (fps / 2) as f64 / fps as f64;
    (0..n)
        .map(|i| {
            let t = from + i as f64 / RATE - t0;
            let v = if t >= 0.0 && (at..at + 0.05).contains(&t.fract()) { 0.5 * (std::f64::consts::TAU * 1000.0 * t).sin() as f32 } else { 0.0 };
            [v, v]
        })
        .collect()
}

/// What to capture, resolved on the engine thread.
#[derive(Clone, Debug, PartialEq)]
pub enum TargetSpec {
    /// 1-based, primary first.
    Display(usize),
    /// Window handle.
    Window(usize, String),
}

/// Settings that apply instantly, even while recording.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveState {
    pub scene: Scene,
    pub pip: PipSpec,
    pub gain_db: [f32; 2],
    /// Muted sources (system, mic): silent in every track and stream.
    pub muted: [bool; 2],
    /// A custom layout replaces the fixed scene.
    pub layout: Option<Layout>,
    /// Colors and font for text layers.
    pub brand: Brand,
    /// Virtual background behind the webcam.
    pub vbg: crate::vbg::VirtualBackground,
}

pub enum Cmd {
    Record {
        path: PathBuf,
        duration: Option<f64>,
    },
    Stop,
    Pause,
    Resume,
    /// Go live on a platform.
    StartStream(StreamSpec),
    StopStream(u64),
    /// Keep the last N seconds in memory (None turns it off).
    Replay(Option<f64>),
    /// Save the replay buffer (None: a timestamped file in the output folder).
    SaveReplay(Option<PathBuf>),
    /// Change a camera control (zoom, exposure…; auto where supported).
    CameraControl(crate::win::camera::Ctl, i32, bool),
    /// Mark a chapter at this moment of the recording (None: "Capítulo N").
    Marker(Option<String>),
    /// Media Board.
    Media(MediaCmd),
    Quit,
}

pub enum MediaCmd {
    /// Play a sound or clip (again: stop it); show or hide an image.
    Play(crate::media::Item),
    /// Stop one item (by path).
    Stop(String),
    StopAll,
}

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub frames_in: u64,
    pub frames_out: u64,
    /// Slots that repeated the previous picture (nothing changed on screen).
    pub dups: u64,
    /// Slots skipped because the pipeline fell more than 0.5 s behind.
    pub drops: u64,
    pub paused_slots: u64,
    pub latency_max: f64,
    pub cam_frames: u64,
    /// One label per audio track, in file order.
    pub tracks: Vec<String>,
    pub audio_secs: f64,
    pub bytes: u64,
    /// Chapter markers added.
    pub markers: usize,
}

#[derive(Default, Debug, Clone)]
pub struct Status {
    /// Capture is running (preview frames flow).
    pub ready: bool,
    /// The engine stopped with this error.
    pub error: Option<String>,
    /// Non-fatal problems (missing mic...).
    pub warnings: Vec<String>,
    pub label: String,
    pub out_size: (u32, u32),
    pub fps: u32,
    pub bitrate: f64,
    pub cam: Option<(String, (u32, u32), f64)>,
    pub audio: Vec<String>,
    pub has_system: bool,
    pub has_mic: bool,
    pub recording: bool,
    pub paused: bool,
    /// Recorded video length so far.
    pub rec_secs: f64,
    pub output: Option<PathBuf>,
    /// Current recording's counters.
    pub stats: Stats,
    /// Peak level per source (system, mic), decaying.
    pub peak: [f32; 2],
    /// The last finished recording, and how many have finished.
    pub last: Option<(PathBuf, Stats)>,
    pub finished: u64,
    /// Live streams.
    pub streams: Vec<StreamStatus>,
    /// Replay buffer: (seconds held, bytes held).
    pub replay: Option<(f64, usize)>,
    /// A message for the user (stream failed to start, replay saved…) and
    /// a counter that changes with each one.
    pub notice: Option<String>,
    pub notices: u64,
    pub last_replay: Option<PathBuf>,
    /// Media Board: sounds and clips playing (path, progress 0-1) and the
    /// image shown.
    pub media: Vec<(String, f32)>,
    pub media_image: Option<String>,
    /// Virtual background: how long the model takes per frame (ms).
    pub vbg_ms: Option<f32>,
    /// The camera's controls and their current values.
    pub cam_controls: Vec<crate::win::camera::Control>,
}

#[derive(Default)]
pub struct Preview {
    pub size: (u32, u32),
    /// RGBA8, top-down.
    pub rgba: Vec<u8>,
    pub seq: u64,
}

pub struct Shared {
    pub status: Mutex<Status>,
    pub live: Mutex<LiveState>,
    pub preview: Mutex<Preview>,
    /// Produce preview frames (off while the window is hidden).
    pub preview_on: AtomicBool,
}

pub struct Engine {
    pub shared: Arc<Shared>,
    tx: Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

/// Longest preview side in pixels.
const PREVIEW_MAX: (u32, u32) = (1280, 720);

impl Engine {
    pub fn start(cfg: Config, live: LiveState) -> Engine {
        let shared = Arc::new(Shared { status: Mutex::new(Status::default()), live: Mutex::new(live), preview: Mutex::new(Preview::default()), preview_on: AtomicBool::new(true) });
        let (tx, rx) = channel();
        let s2 = shared.clone();
        let thread = std::thread::Builder::new()
            .name("engine".into())
            .spawn(move || {
                if let Err(e) = run(cfg, &s2, rx) {
                    let mut st = s2.status.lock().unwrap();
                    st.error = Some(format!("{e:#}"));
                    st.ready = false;
                    st.recording = false;
                }
            })
            .expect("engine thread");
        Engine { shared, tx, thread: Some(thread) }
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }

    pub fn sender(&self) -> Sender<Cmd> {
        self.tx.clone()
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().clone()
    }

    pub fn set_live(&self, live: LiveState) {
        *self.shared.live.lock().unwrap() = live;
    }

    /// Stop (finishing any recording) and wait for the thread.
    pub fn quit(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A bitrate that looks clean for screen content: ~0.1 bits per pixel per
/// frame for H.264, 30% less for HEVC.
fn default_bitrate(w: u32, h: u32, fps: u32, codec: Codec) -> f64 {
    let bpp = if codec == Codec::Hevc { 0.07 } else { 0.1 };
    (w * h) as f64 * fps as f64 * bpp
}

/// One audio track: which sources it mixes, at what gain.
struct Track {
    label: String,
    system: bool,
    mic: bool,
    mix: bool,
    /// Gets Media Board audio: the mix, the system stem, or a lone track.
    media: bool,
}

fn plan_tracks(system: bool, mic: bool, layout: AudioLayout) -> Vec<Track> {
    let t = |label: &str, system, mic| Track { label: label.into(), system, mic, mix: system && mic, media: system };
    let mut tracks = match (system, mic) {
        (false, false) => vec![],
        (true, false) => vec![t("system audio", true, false)],
        (false, true) => vec![t("microphone", false, true)],
        (true, true) => match layout {
            AudioLayout::Mix => vec![t("mix", true, true)],
            AudioLayout::Stems => {
                vec![t("system audio", true, false), t("microphone", false, true)]
            }
            AudioLayout::Both => vec![t("mix", true, true), t("system audio", true, false), t("microphone", false, true)],
        },
    };
    if let [only] = tracks.as_mut_slice() {
        only.media = true;
    }
    tracks
}

/// How far ahead of playback media is decoded and mixed.
const MEDIA_AHEAD: f64 = 0.25;

/// A Media Board sound or clip playing.
struct Player {
    key: String,
    volume: f32,
    /// Keeps the decoders running.
    _dec: Decoder,
    /// Per stream (audio, picture): its channel, a chunk not yet due, and
    /// whether it finished.
    streams: Vec<(std::sync::mpsc::Receiver<Chunk>, Option<Chunk>, bool)>,
    /// Host time of the file's 0 s: set once every stream has its first
    /// data, so the decoders' start-up neither clips the beginning nor puts
    /// the sound ahead of the picture.
    start: Option<f64>,
    created: f64,
    frames: VecDeque<(f64, (u32, u32), Vec<u8>)>,
    tex: Option<((u32, u32), ID3D11Texture2D, ID3D11ShaderResourceView)>,
    /// The last content decoded ends at `end` (file seconds).
    end: f64,
    duration: f64,
}

impl Player {
    fn new(item: &crate::media::Item, out: (u32, u32)) -> Player {
        let video = (item.kind == MediaKind::Video).then_some(out);
        let mut dec = Decoder::open(&item.path, video);
        let streams = std::mem::take(&mut dec.streams).into_iter().map(|rx| (rx, None, false)).collect();
        Player { key: item.path.clone(), volume: item.volume, _dec: dec, streams, start: None, created: crate::win::now(), frames: VecDeque::new(), tex: None, end: 0.0, duration: 0.0 }
    }

    fn progress(&self, t: f64) -> f32 {
        match self.start {
            Some(s) if self.duration > 0.0 => ((t - s) / self.duration).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        }
    }

    fn done(&self, t: f64) -> bool {
        self.streams.iter().all(|s| s.2 && s.1.is_none()) && self.frames.is_empty() && self.start.is_none_or(|s| t - s > self.end)
    }

    /// Take what is due: audio (with its host time) to `audio`, the newest
    /// due picture onto the texture. Returns a decoding error.
    fn pump(&mut self, gpu: &Gpu, t: f64, mut audio: impl FnMut(f64, Vec<[f32; 2]>)) -> Result<Option<String>> {
        use std::sync::mpsc::TryRecvError;
        if self.start.is_none() {
            for si in 0..self.streams.len() {
                while self.streams[si].1.is_none() && !self.streams[si].2 {
                    match self.streams[si].0.try_recv() {
                        Ok(Chunk::Duration(d)) => self.duration = self.duration.max(d),
                        Ok(Chunk::Failed(e)) => {
                            self.streams[si].2 = true;
                            return Ok(Some(e));
                        }
                        Ok(c) => self.streams[si].1 = Some(c),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => self.streams[si].2 = true,
                    }
                }
            }
            // A stream stuck for a second doesn't hold the others back.
            if !self.streams.iter().all(|s| s.1.is_some() || s.2) && t - self.created < 1.0 {
                return Ok(None);
            }
            self.start = Some(t + 0.03);
        }
        for si in 0..self.streams.len() {
            loop {
                let stream = &mut self.streams[si];
                let c = match stream.1.take() {
                    Some(c) => c,
                    None => match stream.0.try_recv() {
                        Ok(c) => c,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            stream.2 = true;
                            break;
                        }
                    },
                };
                let at = match &c {
                    Chunk::Audio(at, _) | Chunk::Video(at, _, _) => *at,
                    Chunk::Duration(d) => {
                        self.duration = self.duration.max(*d);
                        continue;
                    }
                    Chunk::Failed(e) => {
                        self.streams[si].2 = true;
                        return Ok(Some(e.clone()));
                    }
                };
                let start = self.start.unwrap_or(t);
                if at > t - start + MEDIA_AHEAD {
                    self.streams[si].1 = Some(c);
                    break;
                }
                match c {
                    Chunk::Audio(at, frames) => {
                        self.end = self.end.max(at + frames.len() as f64 / RATE);
                        let v = self.volume;
                        audio(start + at, frames.into_iter().map(|f| f.map(|s| s * v)).collect());
                    }
                    Chunk::Video(at, size, px) => {
                        self.end = self.end.max(at + 0.04);
                        self.frames.push_back((at, size, px));
                    }
                    _ => {}
                }
            }
        }
        let Some(start) = self.start else {
            return Ok(None);
        };
        let mut newest = None;
        while self.frames.front().is_some_and(|f| f.0 <= t - start) {
            newest = self.frames.pop_front();
        }
        if let Some((_, size, px)) = newest {
            if self.tex.as_ref().is_none_or(|x| x.0 != size) {
                let tex = gpu.texture(size.0, size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_SHADER_RESOURCE)?;
                let srv = Overlay::srv_of(gpu, &tex)?;
                self.tex = Some((size, tex, srv));
            }
            if let Some((_, tex, _)) = &self.tex {
                gpu.locked(|| unsafe { gpu.context.UpdateSubresource(tex, 0, None, px.as_ptr() as _, size.0 * 4, 0) });
            }
        }
        Ok(None)
    }
}

/// Media Board pictures over the scene: playing clips (letterboxed on
/// black), then the image shown.
fn draw_media(gpu: &Gpu, pipe: &mut Pipeline, players: &[Player], image: Option<&str>, out: (u32, u32)) -> Result<()> {
    let full = [0.0, 0.0, out.0 as f64, out.1 as f64];
    let plain = PipSpec { shape: Shape::Rect, roundness: 0.0, border: 0.0, shadow: false, mirror: false, ..Default::default() };
    for p in players {
        if let Some((size, _, srv)) = &p.tex {
            pipe.overlay.clear_to([0.0; 3]);
            let r = fit_in(full, size.0 as f64 / size.1.max(1) as f64);
            pipe.overlay.draw_texture(srv, *size, [1.0, 1.0], Some(r), &plain)?;
        }
    }
    if let Some(path) = image {
        let key = format!("img:{path}");
        if let Some((size, srv)) = picture(gpu, pipe, &key, || load_picture(path)) {
            let r = fit_in(full, size.0 as f64 / size.1.max(1) as f64);
            pipe.overlay.draw_texture_alpha(&srv, size, [1.0, 1.0], Some(r), &plain, true)?;
        }
    }
    Ok(())
}

/// An image file as RGBA, at most 4096 px on its longest side.
fn load_picture(path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::open(path).ok()?.to_rgba8();
    // Keep textures reasonable (a 6000 px photo as a background).
    let longest = img.width().max(img.height());
    let img = if longest > 4096 {
        let k = 4096.0 / longest as f64;
        let (w, h) = (((img.width() as f64 * k) as u32).max(1), ((img.height() as f64 * k) as u32).max(1));
        image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Some((img.width(), img.height(), img.into_raw()))
}

/// A recording in progress.
struct Rec {
    enc: Encoder,
    path: PathBuf,
    /// Engine grid slot where it started, and its host time.
    start_slot: i64,
    t0: f64,
    pauses: Pauses,
    mixers: Vec<Mixer>,
    duration: Option<f64>,
    stats: Stats,
    audio_secs: f64,
    markers: Vec<crate::chapters::Marker>,
}

impl Rec {
    /// Write everything mixed so far. `floor`: emit at least up to this host
    /// time (silence where nothing arrived). `limit`: cut at this slot.
    fn write_audio(&mut self, fps: f64, floor: f64, limit: Option<f64>) -> Result<()> {
        for (track, m) in self.mixers.iter_mut().enumerate() {
            m.set_floor(floor);
            while let Some((t, frames)) = m.pull_frames(false) {
                let a = (t - self.t0) * fps;
                let b = (a + frames.len() as f64 / RATE * fps).min(limit.unwrap_or(f64::INFINITY));
                let Some((from, to, off)) = self.pauses.keep(a, b) else {
                    continue;
                };
                let i0 = (((from - a) / fps * RATE).round() as usize).min(frames.len());
                let i1 = (((to - a) / fps * RATE).round() as usize).clamp(i0, frames.len());
                let at = (from - off as f64) / fps;
                self.enc.write_audio(track, at, &frames[i0..i1])?;
                if track == 0 {
                    self.audio_secs = self.audio_secs.max(at + (i1 - i0) as f64 / RATE);
                }
            }
        }
        Ok(())
    }
}

/// GPU resources for one frame: screen → canvas (+ camera) → NV12 / preview.
struct Pipeline {
    canvas: ID3D11Texture2D,
    scale: Blitter,
    nv12: Blitter,
    ring: Ring,
    overlay: Overlay,
    /// Screen layers scaled to their box (by size), then decorated.
    layer_tex: Vec<((u32, u32), ID3D11Texture2D, ID3D11ShaderResourceView)>,
    /// Image files and rendered text, by key (most recent last).
    pictures: Vec<(String, Option<((u32, u32), ID3D11ShaderResourceView)>)>,
    /// Guests' cameras: identity, frame counter, size, texture.
    guest_tex: Vec<(String, u64, (u32, u32), ID3D11Texture2D, ID3D11ShaderResourceView)>,
    preview_blit: Blitter,
    preview_tex: ID3D11Texture2D,
    staging: ID3D11Texture2D,
    preview_size: (u32, u32),
    /// For drawing the test pattern.
    canvas_rtv: ID3D11RenderTargetView,
    /// The webcam with its virtual background (what the compositor draws
    /// as the camera).
    matte: Option<Matte>,
}

fn run(cfg: Config, shared: &Shared, rx: Receiver<Cmd>) -> Result<()> {
    unsafe {
        let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
    }
    encoder::startup()?;
    let r = engine(cfg, shared, &rx);
    encoder::shutdown();
    r
}

fn resolve(t: &TargetSpec) -> Result<(Target, String)> {
    Ok(match t {
        TargetSpec::Window(h, title) => (Target::Window(windows::Win32::Foundation::HWND(*h as _)), format!("window \"{title}\"")),
        TargetSpec::Display(n) => {
            let displays = crate::win::sources::displays();
            let d = n.checked_sub(1).and_then(|i| displays.get(i)).ok_or_else(|| anyhow!("no display {n}; there are {}", displays.len()))?;
            (Target::Display(d.hmon), format!("display {n} ({})", d.name))
        }
    })
}

fn engine(cfg: Config, shared: &Shared, rx: &Receiver<Cmd>) -> Result<()> {
    let (target, label) = resolve(&cfg.target)?;
    let gpu = Gpu::new()?;
    let cap = Capture::start(&gpu, target, cfg.cursor)?;
    let (w, h) = parse_size(&cfg.resolution, (cap.size.0 as usize, cap.size.1 as usize))?;
    let out = (w as u32, h as u32);
    let bitrate = cfg.bitrate.map(|m| m * 1e6).unwrap_or_else(|| default_bitrate(out.0, out.1, cfg.fps, cfg.codec));
    let spec = VideoSpec { width: out.0, height: out.1, fps: cfg.fps, codec: cfg.codec, bitrate: bitrate as u32 };
    let mut warnings = vec![];

    let mut cam = match &cfg.cam {
        Some(q) => match Camera::start(&gpu, q) {
            Ok(c) => Some(c),
            Err(e) => {
                warnings.push(format!("camera: {e:#}"));
                None
            }
        },
        None => None,
    };

    let (_, _, pw, ph) = fit(out, PREVIEW_MAX);
    let preview_size = (pw.min(out.0).max(2) & !1, ph.min(out.1).max(2) & !1);
    let canvas = gpu.texture(out.0, out.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE)?;
    let preview_tex = gpu.texture(preview_size.0, preview_size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_RENDER_TARGET)?;
    let staging = {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: preview_size.0,
            Height: preview_size.1,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut t = None;
        unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut t))? };
        t.context("staging texture")?
    };
    let matte = match &cam {
        Some(c) => match Matte::new(&gpu, &c.latest, c.size, cfg.fps) {
            Ok(m) => Some(m),
            Err(e) => {
                warnings.push(format!("virtual background: {e:#}"));
                None
            }
        },
        None => None,
    };
    let mut pipe = Pipeline {
        scale: Blitter::new(&gpu, cap.size, out, cfg.fps, false)?,
        nv12: Blitter::new(&gpu, out, out, cfg.fps, true)?,
        ring: Ring::new(&gpu, out, 12)?,
        overlay: Overlay::new(&gpu, &canvas, out, matte.as_ref().map(|m| &m.out).or(cam.as_ref().map(|c| &c.latest)))?,
        matte,
        layer_tex: vec![],
        pictures: vec![],
        guest_tex: vec![],
        preview_blit: Blitter::new(&gpu, out, preview_size, 30, false)?,
        canvas_rtv: {
            let mut v = None;
            unsafe { gpu.device.CreateRenderTargetView(&canvas, None, Some(&mut v))? };
            v.context("canvas view")?
        },
        canvas,
        preview_tex,
        staging,
        preview_size,
    };

    // Audio sources. A missing device is a warning, not a failure.
    let (tx, audio_rx) = channel::<Packet>();
    let mut sources: Vec<AudioCapture> = vec![];
    let mut audio_names = vec![];
    // The test pattern generates its own "system audio".
    let mut has_system = cfg.test_pattern;
    if cfg.test_pattern {
        audio_names.push("test pattern beeps".into());
    } else if cfg.system_audio {
        let source = match &cfg.app_audio {
            Some((name, include)) => Source::App { name: name.clone(), include: *include },
            None => Source::System,
        };
        match AudioCapture::start(source, tx.clone()) {
            Ok(a) => {
                audio_names.push(format!("system audio ({})", a.name));
                sources.push(a);
                has_system = true;
            }
            Err(e) => warnings.push(format!("system audio: {e:#}")),
        }
    }
    let mut has_mic = false;
    if let Some(m) = cfg.mic.as_ref().filter(|_| !cfg.test_pattern) {
        match AudioCapture::start(Source::Mic(Some(m.clone())), tx.clone()) {
            Ok(a) => {
                audio_names.push(format!("mic ({})", a.name));
                sources.push(a);
                has_mic = true;
            }
            Err(e) => warnings.push(format!("microphone: {e:#}")),
        }
    }
    drop(tx);
    let tracks = plan_tracks(has_system, has_mic, cfg.audio_tracks);
    let control = Listener::new().ok();

    // Windows.Graphics.Capture sends a first frame promptly; webcams take a
    // moment to start.
    let wait = now();
    while cap.info().seq == 0 || cam.as_ref().is_some_and(|c| c.info().seq == 0 && now() - wait < 5.0) {
        if now() - wait > 3.0 && cap.info().seq == 0 {
            cap.stop();
            bail!("no frames from {label} after 3 s");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    {
        let mut st = shared.status.lock().unwrap();
        *st = Status {
            ready: true,
            warnings,
            label: label.clone(),
            out_size: out,
            fps: cfg.fps,
            bitrate,
            cam: cam.as_ref().map(|c| (c.name.clone(), c.size, c.fps)),
            audio: audio_names,
            has_system,
            has_mic,
            ..Default::default()
        };
    }

    let fps = cfg.fps as f64;
    let period = 1.0 / fps;
    let preview_every = (cfg.fps as f64 / 30.0).round().max(1.0) as i64;
    let t0 = now();
    // Test-pattern audio generated so far (host time).
    let mut synth_t = t0;
    let mut deferred: Option<Cmd> = None;
    let mut slot: i64 = 0;
    let mut last_seq = 0;
    let mut rec: Option<Rec> = None;
    let mut peak = [0f32; 2];
    let mut quit = false;
    // Live outputs (streams, replay buffer) and their shared audio mix.
    let mut lives: Vec<LiveOut> = vec![];
    let mut live_mix: Option<Mixer> = None;
    let mut next_live = 0u64;
    let mut saving: Vec<std::thread::JoinHandle<Result<PathBuf>>> = vec![];
    // Media Board. When system audio is captured from the speakers, it
    // already hears the monitor; otherwise media goes into the mix directly.
    let mut players: Vec<Player> = vec![];
    let mut media_image: Option<String> = None;
    let mut monitor: Option<Monitor> = None;
    // Virtual background: the segmentation worker and the last mask used.
    let mut vbg_worker: Option<crate::vbg::Worker> = None;
    let mut vbg_failed = false;
    let mut vbg_seq = 0u64;
    let loopback_hears_us = !cfg.test_pattern && has_system && cfg.app_audio.as_ref().is_none_or(|(_, include)| !include);
    let inject = !loopback_hears_us;
    let notify = |msg: String| {
        let mut st = shared.status.lock().unwrap();
        st.notice = Some(msg);
        st.notices += 1;
    };

    let finish = |rec: Rec, end_slot: i64, shared: &Shared| -> Result<()> {
        let mut rec = rec;
        let end = rec.t0 + (end_slot - rec.start_slot) as f64 * period;
        let audio = rec.write_audio(fps, end, Some((end_slot - rec.start_slot) as f64));
        rec.stats.audio_secs = rec.audio_secs;
        let Rec { enc, path, mut stats, markers, .. } = rec;
        let done = enc.finish();
        let _ = crate::chapters::save(&path, &markers);
        stats.bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut st = shared.status.lock().unwrap();
        st.recording = false;
        st.paused = false;
        st.last = Some((path, stats));
        st.finished += 1;
        drop(st);
        audio?;
        done
    };

    let result: Result<()> = (|| {
        while !quit {
            // Commands from the app/CLI and from `chisme stop|pause|resume`.
            let mut cmds: Vec<Cmd> = deferred.take().into_iter().chain(rx.try_iter()).collect();
            if let Some(l) = &control {
                cmds.extend(l.poll().into_iter().map(|c| match c {
                    control::Command::Stop => Cmd::Stop,
                    control::Command::Pause => Cmd::Pause,
                    control::Command::Resume => Cmd::Resume,
                    control::Command::SaveReplay => Cmd::SaveReplay(None),
                    control::Command::Marker => Cmd::Marker(None),
                }));
            }
            for c in cmds {
                match c {
                    // The test pattern starts recording on a whole second,
                    // so the file begins with a full flash and beep.
                    Cmd::Record { path, duration } if rec.is_none() && cfg.test_pattern && slot % (cfg.fps as i64) != 0 => {
                        deferred = Some(Cmd::Record { path, duration });
                    }
                    Cmd::Record { path, duration } if rec.is_none() => {
                        let enc = Encoder::open(&gpu, &path, &spec, tracks.len()).with_context(|| format!("can't record to {}", path.display()))?;
                        let gains = shared.live.lock().unwrap().gain_db;
                        let rt0 = t0 + slot as f64 * period;
                        rec = Some(Rec {
                            enc,
                            path: path.clone(),
                            start_slot: slot,
                            t0: rt0,
                            pauses: Pauses::default(),
                            mixers: tracks.iter().map(|t| Mixer::new(rt0, t.system, t.mic, if t.mix { gains } else { [0.0; 2] }).with_mic_denoise(cfg.mic_denoise).with_media(t.media)).collect(),
                            duration,
                            stats: Stats { tracks: tracks.iter().map(|t| t.label.clone()).collect(), ..Default::default() },
                            audio_secs: 0.0,
                            markers: vec![],
                        });
                        let mut st = shared.status.lock().unwrap();
                        st.recording = true;
                        st.paused = false;
                        st.rec_secs = 0.0;
                        st.output = Some(path);
                        st.stats = Stats::default();
                    }
                    Cmd::Record { .. } => {}
                    Cmd::Stop => {
                        if let Some(r) = rec.take() {
                            finish(r, slot, shared)?;
                        }
                    }
                    Cmd::Pause => {
                        if let Some(r) = &mut rec {
                            r.pauses.pause(slot - r.start_slot);
                        }
                    }
                    Cmd::Resume => {
                        if let Some(r) = &mut rec {
                            r.pauses.resume(slot - r.start_slot);
                        }
                    }
                    Cmd::Marker(label) => match &mut rec {
                        Some(r) => {
                            let at = r.stats.frames_out as f64 * period;
                            let label = label.unwrap_or_else(|| crate::t!("Chapter {}", "Capítulo {}", r.markers.len() + 1));
                            notify(crate::t!("Marker \"{}\" at {}", "Marcador \"{}\" en {}", label, crate::chapters::stamp(at)));
                            r.markers.push(crate::chapters::Marker { at, label });
                            r.stats.markers = r.markers.len();
                        }
                        None => notify(crate::i18n::l("Markers are added while recording", "Los marcadores se agregan mientras grabas").into()),
                    },
                    Cmd::StartStream(spec) => {
                        next_live += 1;
                        match LiveOut::stream(&gpu, next_live, out, cfg.fps, &spec) {
                            Ok(o) => lives.push(o),
                            Err(e) => notify(crate::live::redact(&format!("{}: {e:#}", spec.platform.label()), &spec.key)),
                        }
                    }
                    Cmd::StopStream(id) => lives.retain(|o| o.id != id),
                    Cmd::Replay(Some(secs)) => match lives.iter_mut().find(|o| o.is_replay()) {
                        Some(r) => r.set_replay_secs(secs),
                        None => {
                            next_live += 1;
                            match LiveOut::replay(&gpu, next_live, out, cfg.fps, secs) {
                                Ok(o) => lives.push(o),
                                Err(e) => notify(format!("Replay: {e:#}")),
                            }
                        }
                    },
                    Cmd::Replay(None) => lives.retain(|o| !o.is_replay()),
                    Cmd::SaveReplay(path) => {
                        let path = match path {
                            Some(p) => Ok(p),
                            None => {
                                let s = crate::settings::Settings::load();
                                std::fs::create_dir_all(&s.output_dir).map(|_| s.output_dir.join(crate::settings::timestamped().replace("Chisme ", "Chisme replay ")))
                            }
                        };
                        let r = match (lives.iter().find(|o| o.is_replay()), path) {
                            (Some(o), Ok(p)) => o.save_replay(p),
                            (None, _) => Err(anyhow!(crate::i18n::l("the replay buffer is off", "el replay buffer está apagado"))),
                            (_, Err(e)) => Err(e.into()),
                        };
                        match r {
                            Ok(h) => saving.push(h),
                            Err(e) => notify(format!("Replay: {e:#}")),
                        }
                    }
                    Cmd::Media(MediaCmd::Play(item)) => match item.kind {
                        MediaKind::Image => {
                            media_image = if media_image.as_deref() == Some(item.path.as_str()) { None } else { Some(item.path) };
                        }
                        _ => {
                            if let Some(i) = players.iter().position(|p| p.key == item.path) {
                                players.remove(i);
                            } else {
                                if monitor.is_none() {
                                    match Monitor::start() {
                                        Ok(m) => monitor = Some(m),
                                        Err(e) => notify(crate::t!("Speakers: {:#}", "Bocinas: {:#}", e)),
                                    }
                                }
                                players.push(Player::new(&item, out));
                            }
                        }
                    },
                    Cmd::Media(MediaCmd::Stop(key)) => {
                        players.retain(|p| p.key != key);
                        if media_image.as_deref() == Some(key.as_str()) {
                            media_image = None;
                        }
                    }
                    Cmd::Media(MediaCmd::StopAll) => {
                        players.clear();
                        media_image = None;
                    }
                    Cmd::CameraControl(ctl, value, auto) => {
                        if let Some(c) = &cam {
                            c.set(ctl, value, auto);
                        }
                    }
                    Cmd::Quit => quit = true,
                }
            }
            if quit {
                break;
            }

            let due = t0 + slot as f64 * period;
            let t = now();
            if t < due {
                std::thread::sleep(Duration::from_secs_f64((due - t).min(period)));
                continue;
            }
            if t - due > 0.5 {
                let skip = ((t - due) / period) as i64;
                if let Some(r) = &mut rec {
                    r.stats.drops += skip as u64;
                }
                slot += skip;
                continue;
            }

            // Audio: meters always, into the file while recording and into
            // the live mix while streaming.
            let live = shared.live.lock().unwrap().clone();
            // Volume and mute apply live, even mid-recording. Gains shape the
            // mix; the separate stems keep their level but follow mute.
            let silence = |g: [f32; 2]| [0, 1].map(|i| if live.muted[i] { -200.0 } else { g[i] });
            if let Some(r) = &mut rec {
                for (t, m) in tracks.iter().zip(r.mixers.iter_mut()) {
                    m.set_gain(silence(if t.mix { live.gain_db } else { [0.0; 2] }));
                }
            }
            if let Some(m) = &mut live_mix {
                m.set_gain(silence(live.gain_db));
            }
            if lives.is_empty() {
                live_mix = None;
            } else if live_mix.is_none() {
                live_mix = Some(Mixer::new(t, has_system, has_mic, live.gain_db).with_mic_denoise(cfg.mic_denoise));
            }
            let mut packets: Vec<Packet> = audio_rx.try_iter().collect();
            if cfg.test_pattern {
                let n = ((due - synth_t) * RATE).round().max(0.0) as usize;
                if n > 0 {
                    packets.push(Packet { kind: AudioKind::System, pts: synth_t, rate: RATE, frames: beep_samples(t0, cfg.fps, synth_t, n) });
                    synth_t += n as f64 / RATE;
                }
            }
            let to_guests = crate::guests::outgoing().lock().unwrap().active;
            for p in packets {
                let i = (p.kind == AudioKind::Mic) as usize;
                if to_guests && p.kind == AudioKind::Mic {
                    let frames = resample_48k(&p.frames, p.rate);
                    let mut o = crate::guests::outgoing().lock().unwrap();
                    o.mic.push_back(frames);
                    while o.mic.len() > 100 {
                        o.mic.pop_front();
                    }
                }
                peak[i] = p.frames.iter().fold(peak[i], |a, f| a.max(f[0].abs()).max(f[1].abs()));
                if let Some(r) = &mut rec {
                    for m in r.mixers.iter_mut().filter(|m| m.takes(p.kind)) {
                        m.push(p.kind, p.pts, p.rate, p.frames.clone());
                    }
                }
                if let Some(m) = &mut live_mix {
                    m.push(p.kind, p.pts, p.rate, p.frames.clone());
                }
            }

            // Media Board: due audio into the mix and the speakers, due
            // pictures onto their textures.
            for p in players.iter_mut() {
                let failed = p.pump(&gpu, t, |pts, frames| {
                    if inject {
                        if let Some(r) = &mut rec {
                            for m in r.mixers.iter_mut().filter(|m| m.takes(AudioKind::Media)) {
                                m.push(AudioKind::Media, pts, RATE, frames.clone());
                            }
                        }
                        if let Some(m) = &mut live_mix {
                            m.push(AudioKind::Media, pts, RATE, frames.clone());
                        }
                    }
                    if let Some(m) = &monitor {
                        m.play(pts, frames);
                    }
                })?;
                if let Some(e) = failed {
                    notify(format!("Media Board: {e}"));
                }
            }
            players.retain(|p| !p.done(t));

            // Guests on stage: into the mix and the speakers, like media.
            let room = crate::guests::shared().lock().unwrap().clone();
            if let Some(room) = room {
                let blocks: Vec<(f64, bool, Vec<[f32; 2]>)> = room.lock().unwrap().audio.drain(..).collect();
                if !blocks.is_empty() && monitor.is_none() {
                    match Monitor::start() {
                        Ok(m) => monitor = Some(m),
                        Err(e) => notify(crate::t!("Speakers: {:#}", "Bocinas: {:#}", e)),
                    }
                }
                for (pts, on_air, frames) in blocks {
                    if inject && on_air {
                        if let Some(r) = &mut rec {
                            for m in r.mixers.iter_mut().filter(|m| m.takes(AudioKind::Media)) {
                                m.push(AudioKind::Media, pts, RATE, frames.clone());
                            }
                        }
                        if let Some(m) = &mut live_mix {
                            m.push(AudioKind::Media, pts, RATE, frames.clone());
                        }
                    }
                    if let Some(m) = &monitor {
                        m.play(pts, frames);
                    }
                }
            }

            let rec_slot = rec.as_ref().and_then(|r| r.pauses.offset_at((slot - r.start_slot) as f64).map(|off| slot - r.start_slot - off));
            if rec.is_some()
                && rec_slot.is_none()
                && let Some(r) = &mut rec
            {
                r.stats.paused_slots += 1;
            }
            let want_preview = (shared.preview_on.load(Ordering::Relaxed) || to_guests) && slot % preview_every == 0;
            let mut live_failed: Vec<(u64, String)> = vec![];
            if rec_slot.is_some() || want_preview || !lives.is_empty() {
                let info = cap.info();
                let fresh = info.seq != last_seq;
                last_seq = info.seq;
                let cam_ready = cam.as_ref().is_some_and(|c| c.info().seq > 0);
                // The worker runs only while a virtual background is on.
                let want_vbg = cam_ready && pipe.matte.is_some() && live.vbg.active();
                if want_vbg && vbg_worker.is_none() && !vbg_failed {
                    match crate::vbg::Worker::start() {
                        Ok(w) => {
                            if let Some(c) = &cam {
                                *c.seg.lock().unwrap() = Some(w.input.clone());
                            }
                            vbg_worker = Some(w);
                        }
                        Err(e) => {
                            vbg_failed = true;
                            notify(crate::t!("Virtual background: {:#}", "Fondo virtual: {:#}", e));
                        }
                    }
                } else if !want_vbg && vbg_worker.is_some() {
                    if let Some(c) = &cam {
                        *c.seg.lock().unwrap() = None;
                    }
                    vbg_worker = None;
                    if let Some(m) = &mut pipe.matte {
                        m.has_mask = false;
                    }
                }
                let new_mask = vbg_worker.as_ref().and_then(|w| {
                    let m = w.mask.lock().unwrap();
                    (m.seq != vbg_seq).then(|| (m.seq, m.data.clone()))
                });
                let scene = if cam.is_some() { live.scene } else { Scene::Screen };
                let tex = gpu.locked(|| -> Result<Option<ID3D11Texture2D>> {
                    // The camera with its virtual background, or as it is.
                    if let (Some(m), Some(c), true) = (&mut pipe.matte, &cam, cam_ready) {
                        if let Some((seq, data)) = &new_mask {
                            m.set_mask(data);
                            vbg_seq = *seq;
                        }
                        let done = match (want_vbg && m.has_mask, live.vbg.mode, &live.vbg.image) {
                            (true, crate::vbg::Mode::Blur, _) => {
                                m.blur(&c.latest, live.vbg.blur)?;
                                true
                            }
                            (true, crate::vbg::Mode::Image, Some(path)) => m.picture(path)?,
                            _ => false,
                        };
                        if !done {
                            m.pass_through(&c.latest);
                        }
                    }
                    if cfg.test_pattern {
                        // White for 3 frames from frame fps/2 of each second
                        // (when the beep starts), else dark.
                        let mid = cfg.fps as i64 / 2;
                        let v = if (mid..mid + 3).contains(&(slot % cfg.fps as i64)) { 1.0 } else { 0.08 };
                        unsafe { gpu.context.ClearRenderTargetView(&pipe.canvas_rtv, &[v, v, v, 1.0]) };
                    } else if let Some(layout) = &live.layout {
                        render_layout(&gpu, &mut pipe, &cap, info.content, cam.as_ref().filter(|_| cam_ready), layout, &live.brand, out)?;
                    } else {
                        // The screen fitted inside `area` (x, y, w, h).
                        let place = |area: [f64; 4]| {
                            let (x, y, w, h) = fit(info.content, (area[2] as u32, area[3] as u32));
                            (area[0] as u32 + x, area[1] as u32 + y, w, h)
                        };
                        match scene {
                            Scene::Cam | Scene::CamScreen => unsafe { gpu.context.ClearRenderTargetView(&pipe.canvas_rtv, &[0.0, 0.0, 0.0, 1.0]) },
                            Scene::SideBySide => {
                                unsafe { gpu.context.ClearRenderTargetView(&pipe.canvas_rtv, &[0.0, 0.0, 0.0, 1.0]) };
                                let area = crate::compose::side_by_side(out.0 as f64, out.1 as f64).0;
                                pipe.scale.blit_into(&cap.latest, info.content, &pipe.canvas, place(area))?;
                            }
                            _ => pipe.scale.blit(&cap.latest, info.content, &pipe.canvas, fit(info.content, out))?,
                        }
                    }
                    let fixed = live.layout.is_none() && !cfg.test_pattern;
                    if fixed
                        && cam_ready
                        && scene.uses_cam()
                        && let Some(c) = &cam
                    {
                        pipe.overlay.draw(scene, &live.pip, c.size)?;
                    }
                    if fixed && scene == Scene::CamScreen {
                        let aspect = info.content.0 as f64 / info.content.1.max(1) as f64;
                        let area = crate::compose::screen_box(&live.pip, out.0 as f64, out.1 as f64, aspect);
                        pipe.scale.blit_into(&cap.latest, info.content, &pipe.canvas, place_exact(area))?;
                    }
                    if !cfg.test_pattern {
                        draw_media(&gpu, &mut pipe, &players, media_image.as_deref(), out)?;
                    }
                    let tex = match rec_slot {
                        Some(_) => {
                            let t = pipe.ring.next();
                            pipe.nv12.blit(&pipe.canvas, out, &t, (0, 0, out.0, out.1))?;
                            Some(t)
                        }
                        None => None,
                    };
                    if want_preview {
                        let ps = pipe.preview_size;
                        pipe.preview_blit.blit(&pipe.canvas, out, &pipe.preview_tex, (0, 0, ps.0, ps.1))?;
                        read_preview(&gpu, &pipe, shared)?;
                    }
                    for o in lives.iter_mut() {
                        if let Err(e) = o.frame(&pipe.canvas, slot, due) {
                            live_failed.push((o.id, format!("{}: {e:#}", o.name)));
                        }
                    }
                    Ok(tex)
                })?;
                if let (Some(r), Some(idx), Some(tex)) = (&mut rec, rec_slot, tex) {
                    r.enc.write(&tex, idx)?;
                    r.stats.frames_out += 1;
                    if !fresh {
                        r.stats.dups += 1;
                    }
                }
            }
            slot += 1;

            // Live outputs: encoded video out, mixed audio in.
            for (id, msg) in live_failed {
                lives.retain(|o| o.id != id);
                notify(msg);
            }
            if let Some(m) = &mut live_mix {
                m.set_floor(t - 0.6);
                while let Some((at, frames)) = m.pull_frames(false) {
                    for o in lives.iter_mut() {
                        if let Err(e) = o.audio(at, &frames) {
                            notify(format!("{} (audio): {e:#}", o.name));
                        }
                    }
                }
            }
            for o in lives.iter_mut() {
                o.pump();
            }
            for h in std::mem::take(&mut saving) {
                if !h.is_finished() {
                    saving.push(h);
                    continue;
                }
                match h.join() {
                    Ok(Ok(p)) => {
                        shared.status.lock().unwrap().last_replay = Some(p.clone());
                        notify(crate::t!("Replay saved: {}", "Replay guardado: {}", p.file_name().unwrap_or_default().to_string_lossy()));
                    }
                    Ok(Err(e)) => notify(format!("Replay: {e:#}")),
                    Err(_) => notify(crate::i18n::l("Replay: saving failed", "Replay: falló al guardar").into()),
                }
            }

            if let Some(r) = &mut rec {
                // Audio waits up to 0.6 s for late packets, then fills silence.
                r.write_audio(fps, t - 0.6, None)?;
            }
            let done = rec.as_ref().is_some_and(|r| r.duration.is_some_and(|d| r.stats.frames_out as f64 >= d * fps));
            {
                let mut st = shared.status.lock().unwrap();
                st.peak = peak;
                st.streams = lives.iter().filter(|o| !o.is_replay()).map(|o| o.status()).collect();
                st.replay = lives.iter().find_map(|o| o.replay_stats());
                st.media = players.iter().map(|p| (p.key.clone(), p.progress(t))).collect();
                st.media_image = media_image.clone();
                st.vbg_ms = vbg_worker.as_ref().map(|w| w.mask.lock().unwrap().ms);
                st.cam_controls = cam.as_ref().map(|c| c.controls.lock().unwrap().list.clone()).unwrap_or_default();
                if let Some(r) = &rec {
                    st.rec_secs = r.stats.frames_out as f64 * period;
                    st.paused = r.pauses.is_paused();
                    st.stats = r.stats.clone();
                    st.stats.frames_in = cap.info().seq;
                    st.stats.latency_max = cap.info().latency_max;
                    st.stats.cam_frames = cam.as_ref().map_or(0, |c| c.info().seq);
                }
            }
            // Meters fall ~20 dB per second.
            peak = peak.map(|p| p * 0.1f32.powf(period as f32));
            if done && let Some(r) = rec.take() {
                finish(r, slot, shared)?;
            }
        }
        Ok(())
    })();
    // Always finish a recording, even after an error, so what was recorded plays.
    std::thread::sleep(Duration::from_millis(60));
    for s in &mut sources {
        s.stop();
    }
    for p in audio_rx.try_iter() {
        if let Some(r) = &mut rec {
            for m in r.mixers.iter_mut().filter(|m| m.takes(p.kind)) {
                m.push(p.kind, p.pts, p.rate, p.frames.clone());
            }
        }
    }
    let finished = match rec.take() {
        Some(r) => finish(r, slot, shared),
        None => Ok(()),
    };
    cap.stop();
    if let Some(c) = &mut cam {
        c.stop();
    }
    result.and(finished)
}

/// Draw a custom layout: the background, then each visible layer bottom to
/// top. A screen layer is scaled into a box-sized texture by the video
/// processor (sharp text), then drawn with its shape and border like the
/// camera.
#[allow(clippy::too_many_arguments)]
fn render_layout(gpu: &Gpu, pipe: &mut Pipeline, cap: &Capture, content: (u32, u32), cam: Option<&Camera>, layout: &Layout, brand: &Brand, out: (u32, u32)) -> Result<()> {
    let bg = layout.background.map(|c| c as f32 / 255.0);
    pipe.overlay.clear_to(bg);
    if let Some(path) = &layout.background_image {
        let key = format!("img:{path}");
        if let Some((size, srv)) = picture(gpu, pipe, &key, || load_picture(path)) {
            let plain = PipSpec { shape: Shape::Rect, roundness: 0.0, border: 0.0, shadow: false, mirror: false, ..Default::default() };
            pipe.overlay.draw_texture(&srv, size, [1.0, 1.0], None, &plain)?;
        }
    }
    let (ow, oh) = (out.0 as f64, out.1 as f64);
    for layer in layout.layers.iter().filter(|l| l.visible) {
        let rect = layer.px(ow, oh);
        let style = layer.style();
        match layer.source {
            LayerSource::Screen => {
                let aspect = content.0 as f64 / content.1.max(1) as f64;
                let r = if layer.fill { rect } else { fit_in(rect, aspect) };
                let size = ((r[2] as u32).max(2) & !1, (r[3] as u32).max(2) & !1);
                let crop = if layer.fill { crop_to(content, size.0 as f64 / size.1 as f64) } else { (0, 0, content.0, content.1) };
                let i = match pipe.layer_tex.iter().position(|t| t.0 == size) {
                    Some(i) => i,
                    None => {
                        let tex = gpu.texture(size.0, size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE)?;
                        let srv = Overlay::srv_of(gpu, &tex)?;
                        if pipe.layer_tex.len() >= 6 {
                            pipe.layer_tex.remove(0);
                        }
                        pipe.layer_tex.push((size, tex, srv));
                        pipe.layer_tex.len() - 1
                    }
                };
                let (tex, srv) = (pipe.layer_tex[i].1.clone(), pipe.layer_tex[i].2.clone());
                pipe.scale.blit_sized(&cap.latest, crop, &tex, size)?;
                pipe.overlay.draw_texture(&srv, size, [1.0, 1.0], Some([r[0], r[1], size.0 as f64, size.1 as f64]), &style)?;
            }
            LayerSource::Camera => {
                let (Some(c), Some(srv)) = (cam, pipe.overlay.cam()) else {
                    continue;
                };
                let r = if layer.fill { rect } else { fit_in(rect, c.size.0 as f64 / c.size.1.max(1) as f64) };
                pipe.overlay.draw_texture(&srv, c.size, [1.0, 1.0], Some(r), &style)?;
            }
            LayerSource::Image => {
                let Some(path) = layer.image.clone() else {
                    continue;
                };
                let key = format!("img:{path}");
                let Some((size, srv)) = picture(gpu, pipe, &key, || load_picture(&path)) else {
                    continue;
                };
                let r = if layer.fill { rect } else { fit_in(rect, size.0 as f64 / size.1.max(1) as f64) };
                pipe.overlay.draw_texture_alpha(&srv, size, [1.0, 1.0], Some(r), &style, true)?;
            }
            LayerSource::Text => {
                let (w, h) = (rect[2] as usize, rect[3] as usize);
                let key = format!("txt:{}|{}|{}|{w}x{h}|{:?}", layer.text, layer.subtitle, layer.style, brand);
                let Some((size, srv)) = picture(gpu, pipe, &key, || {
                    let c = crate::brand::lower_third(&layer.text, &layer.subtitle, layer.style, brand, w, h);
                    Some((c.w as u32, c.h as u32, c.px))
                }) else {
                    continue;
                };
                pipe.overlay.draw_texture_alpha(&srv, size, [1.0, 1.0], Some(rect), &style, true)?;
            }
            LayerSource::Guest => {
                let Some((size, srv)) = guest_texture(gpu, pipe, layer.guest as usize) else {
                    continue;
                };
                let r = if layer.fill { rect } else { fit_in(rect, size.0 as f64 / size.1.max(1) as f64) };
                pipe.overlay.draw_texture(&srv, size, [1.0, 1.0], Some(r), &style)?;
            }
        }
    }
    Ok(())
}

/// The camera of the guest in stage seat `seat`, uploaded when it changes.
fn guest_texture(gpu: &Gpu, pipe: &mut Pipeline, seat: usize) -> Option<((u32, u32), ID3D11ShaderResourceView)> {
    let room = crate::guests::shared().lock().unwrap().clone()?;
    let (id, seq, (size, px)) = {
        let s = room.lock().unwrap();
        let g = *s.on_stage().get(seat)?;
        (g.identity.clone(), g.frame_seq, g.frame.clone()?)
    };
    let i = match pipe.guest_tex.iter().position(|t| t.0 == id && t.2 == size) {
        Some(i) => i,
        None => {
            pipe.guest_tex.retain(|t| t.0 != id);
            let tex = gpu.texture(size.0, size.1, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_BIND_SHADER_RESOURCE).ok()?;
            let srv = Overlay::srv_of(gpu, &tex).ok()?;
            pipe.guest_tex.push((id, 0, size, tex, srv));
            pipe.guest_tex.len() - 1
        }
    };
    let t = &mut pipe.guest_tex[i];
    if t.1 != seq {
        t.1 = seq;
        gpu.locked(|| unsafe { gpu.context.UpdateSubresource(&t.3, 0, None, px.as_ptr() as _, size.0 * 4, 0) });
    }
    Some((size, t.4.clone()))
}

/// An RGBA picture as a texture, made once per `key` by `make` (width,
/// height, pixels). Failures are remembered too, so a missing file isn't
/// retried every frame.
fn picture(gpu: &Gpu, pipe: &mut Pipeline, key: &str, make: impl FnOnce() -> Option<(u32, u32, Vec<u8>)>) -> Option<((u32, u32), ID3D11ShaderResourceView)> {
    if let Some(i) = pipe.pictures.iter().position(|p| p.0 == key) {
        let hit = pipe.pictures.remove(i);
        let out = hit.1.clone();
        pipe.pictures.push(hit);
        return out;
    }
    let made = make().and_then(|(w, h, px)| {
        let tex = gpu.texture(w, h, windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM, D3D11_BIND_SHADER_RESOURCE).ok()?;
        gpu.locked(|| unsafe { gpu.context.UpdateSubresource(&tex, 0, None, px.as_ptr() as _, w * 4, 0) });
        Some(((w, h), Overlay::srv_of(gpu, &tex).ok()?))
    });
    if pipe.pictures.len() >= 24 {
        pipe.pictures.remove(0);
    }
    pipe.pictures.push((key.to_string(), made.clone()));
    made
}

/// Audio at 48 kHz for guests (linear; it's a voice chat).
fn resample_48k(frames: &[[f32; 2]], rate: f64) -> Vec<[f32; 2]> {
    if (rate - 48000.0).abs() < 1.0 || frames.is_empty() {
        return frames.to_vec();
    }
    let n = (frames.len() as f64 * 48000.0 / rate).round() as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * rate / 48000.0;
            let (a, k) = (x.floor() as usize, (x.fract()) as f32);
            let (p, q) = (frames[a.min(frames.len() - 1)], frames[(a + 1).min(frames.len() - 1)]);
            [p[0] + (q[0] - p[0]) * k, p[1] + (q[1] - p[1]) * k]
        })
        .collect()
}

/// A layout rect as whole pixels (x, y, w, h).
fn place_exact(r: [f64; 4]) -> (u32, u32, u32, u32) {
    (r[0].max(0.0) as u32, r[1].max(0.0) as u32, r[2].max(2.0) as u32, r[3].max(2.0) as u32)
}

/// Copy the preview texture to the CPU as RGBA.
fn read_preview(gpu: &Gpu, pipe: &Pipeline, shared: &Shared) -> Result<()> {
    let (w, h) = pipe.preview_size;
    unsafe {
        gpu.context.CopyResource(&pipe.staging, &pipe.preview_tex);
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        gpu.context.Map(&pipe.staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
        let mut p = shared.preview.lock().unwrap();
        p.size = (w, h);
        p.rgba.resize((w * h * 4) as usize, 0);
        for y in 0..h as usize {
            let src = std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), w as usize * 4);
            let dst = &mut p.rgba[y * w as usize * 4..][..w as usize * 4];
            for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], 255]);
            }
        }
        p.seq += 1;
        // Guests watch the program.
        if crate::guests::outgoing().lock().unwrap().active {
            *crate::guests::program().lock().unwrap() = Some(((w, h), Arc::new(p.rgba.clone()), p.seq));
        }
        gpu.context.Unmap(&pipe.staging, 0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_layouts() {
        let labels = |s, m, l| plan_tracks(s, m, l).into_iter().map(|t| t.label).collect::<Vec<_>>();
        assert_eq!(labels(true, true, AudioLayout::Both), ["mix", "system audio", "microphone"]);
        assert_eq!(labels(true, true, AudioLayout::Mix), ["mix"]);
        assert_eq!(labels(true, true, AudioLayout::Stems), ["system audio", "microphone"]);
        assert_eq!(labels(false, true, AudioLayout::Both), ["microphone"]);
        assert!(labels(false, false, AudioLayout::Both).is_empty());
        // Gains apply to the mix only.
        let t = plan_tracks(true, true, AudioLayout::Both);
        assert_eq!(t.iter().map(|t| t.mix).collect::<Vec<_>>(), [true, false, false]);
        // Media Board audio: the mix and the system stem, not the mic stem;
        // a lone track always.
        assert_eq!(t.iter().map(|t| t.media).collect::<Vec<_>>(), [true, true, false]);
        assert!(plan_tracks(false, true, AudioLayout::Both)[0].media);
    }
}
