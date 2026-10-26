//! Guests: people join from a browser link (the green room page) through a
//! LiveKit room. Chisme joins the same room as the host: it receives each
//! guest's camera and microphone, sends back your microphone and the
//! program (what you're recording or streaming), and tells guests when
//! they're on stage. Invite links carry a token Chisme signs itself with
//! your LiveKit API key, so no server of our own is needed.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Where the green room page lives (GitHub Pages).
pub const JOIN_PAGE: &str = "https://lordvayron.github.io/chisme/join/";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// wss://<project>.livekit.cloud
    pub url: String,
    pub api_key: String,
    /// The room guests join.
    pub room: String,
}

impl Config {
    /// What's missing to make links, if anything.
    pub fn missing(&self, secret: &str) -> Option<&'static str> {
        if !(self.url.starts_with("wss://") || self.url.starts_with("ws://")) {
            Some(crate::i18n::l("the Project URL", "la URL del proyecto"))
        } else if self.api_key.is_empty() {
            Some(crate::i18n::l("the API key", "la API key"))
        } else if secret.is_empty() {
            Some(crate::i18n::l("the API secret", "el API secret"))
        } else {
            None
        }
    }

    /// A project URL as typed (https://…, or just the host) as wss://.
    pub fn normal_url(u: &str) -> String {
        let u = u.trim().trim_end_matches('/');
        if u.is_empty() || u.starts_with("wss://") || u.starts_with("ws://") {
            u.to_string()
        } else if let Some(h) = u.strip_prefix("https://") {
            format!("wss://{h}")
        } else if let Some(h) = u.strip_prefix("http://") {
            format!("ws://{h}")
        } else {
            format!("wss://{u}")
        }
    }

    pub fn ready(&self, secret: &str) -> bool {
        (self.url.starts_with("wss://") || self.url.starts_with("ws://")) && !self.api_key.is_empty() && !secret.is_empty()
    }
}

/// A guest as Chisme sees them.
#[derive(Clone, Debug)]
pub struct Guest {
    pub identity: String,
    pub name: String,
    /// Newest camera frame: size, BGRA pixels, and a counter.
    pub frame: Option<((u32, u32), Arc<Vec<u8>>)>,
    pub frame_seq: u64,
    /// Microphone level (peak, decaying).
    pub level: f32,
    pub on_stage: bool,
    pub muted: bool,
    pub has_video: bool,
    pub has_audio: bool,
}

#[derive(Default)]
pub struct State {
    pub connected: bool,
    pub error: Option<String>,
    pub guests: Vec<Guest>,
    /// Guests on stage, in the order they were brought on.
    pub stage: Vec<String>,
    /// Guests' audio waiting for the engine: (host time, on air, 48 kHz
    /// stereo). You hear everyone; only guests on stage go into the show.
    pub audio: VecDeque<(f64, bool, Vec<[f32; 2]>)>,
}

impl State {
    pub fn guest_mut(&mut self, identity: &str) -> Option<&mut Guest> {
        self.guests.iter_mut().find(|g| g.identity == identity)
    }

    /// The on-stage guests, in stage order.
    pub fn on_stage(&self) -> Vec<&Guest> {
        self.stage.iter().filter_map(|id| self.guests.iter().find(|g| &g.identity == id)).collect()
    }
}

/// The running room, shared with the engine (frames, audio) and the app.
pub fn shared() -> &'static Mutex<Option<Arc<Mutex<State>>>> {
    static S: OnceLock<Mutex<Option<Arc<Mutex<State>>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

/// Host-side audio and program frames going to guests.
pub fn outgoing() -> &'static Mutex<Outgoing> {
    static S: OnceLock<Mutex<Outgoing>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Outgoing::default()))
}

#[derive(Default)]
pub struct Outgoing {
    pub active: bool,
    /// Your microphone, 48 kHz stereo.
    pub mic: VecDeque<Vec<[f32; 2]>>,
}

pub enum Cmd {
    Stage(String, bool),
    Mute(String, bool),
    Remove(String),
}

pub struct Room {
    pub state: Arc<Mutex<State>>,
    cmd: tokio::sync::mpsc::UnboundedSender<Cmd>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl Room {
    pub fn send(&self, c: Cmd) {
        let _ = self.cmd.send(c);
    }
}

impl Drop for Room {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        *shared().lock().unwrap() = None;
        outgoing().lock().unwrap().active = false;
    }
}

fn token(cfg: &Config, secret: &str, identity: &str, name: &str, hours: u64, host: bool) -> Result<String> {
    use livekit_api::access_token::{AccessToken, VideoGrants};
    let grants = VideoGrants {
        room_join: true,
        room: cfg.room.clone(),
        can_publish: Some(true),
        can_subscribe: Some(true),
        can_publish_data: Some(host),
        can_update_own_metadata: Some(true),
        ..Default::default()
    };
    AccessToken::with_api_key(&cfg.api_key, secret)
        .with_identity(identity)
        .with_name(name)
        .with_ttl(std::time::Duration::from_secs(hours * 3600))
        .with_grants(grants)
        .to_jwt()
        .context("signing the token")
}

/// A link for one guest (valid 24 hours).
/// A link for one guest (`name`, optional, fills their name field).
pub fn invite_link(cfg: &Config, secret: &str, name: &str) -> Result<String> {
    let id = format!("guest-{:08x}", rand_u32());
    let shown = if name.trim().is_empty() { "Guest" } else { name.trim() };
    let t = token(cfg, secret, &id, shown, 24, false)?;
    let mut link = format!("{JOIN_PAGE}#url={}&token={}", urlencode(&cfg.url), t);
    if !name.trim().is_empty() {
        link.push_str(&format!("&name={}", urlencode(name.trim())));
    }
    Ok(link)
}

fn rand_u32() -> u32 {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut x = (n as u64) ^ 0x9E37_79B9_7F4A_7C15 ^ (std::process::id() as u64) << 17;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x as u32
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A short random room name.
pub fn new_room_name() -> String {
    format!("chisme-{:06x}", rand_u32() & 0xff_ffff)
}

/// Join the room as the host.
pub fn start(cfg: Config, secret: String) -> Result<Room> {
    let host_token = token(&cfg, &secret, "chisme-host", "Host", 12, true)?;
    let state = Arc::new(Mutex::new(State::default()));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (st, sp) = (state.clone(), stop.clone());
    std::thread::Builder::new().name("guests".into()).spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                st.lock().unwrap().error = Some(format!("{e}"));
                return;
            }
        };
        if let Err(e) = rt.block_on(run(cfg.url.clone(), host_token, st.clone(), rx, sp)) {
            let mut s = st.lock().unwrap();
            s.connected = false;
            s.error = Some(format!("{e:#}"));
        }
    })?;
    *shared().lock().unwrap() = Some(state.clone());
    outgoing().lock().unwrap().active = true;
    Ok(Room { state, cmd: tx, stop })
}

async fn run(url: String, token: String, state: Arc<Mutex<State>>, mut cmds: tokio::sync::mpsc::UnboundedReceiver<Cmd>, stop: Arc<std::sync::atomic::AtomicBool>) -> Result<()> {
    use livekit::options::TrackPublishOptions;
    use livekit::prelude::*;
    use livekit::webrtc::audio_source::native::NativeAudioSource;
    use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
    use livekit::webrtc::video_source::native::NativeVideoSource;
    use livekit::webrtc::video_source::{RtcVideoSource, VideoResolution};

    let (room, mut events) = Room::connect(&url, &token, RoomOptions::default()).await.context("connecting to LiveKit")?;
    let room = Arc::new(room);
    {
        let mut s = state.lock().unwrap();
        s.connected = true;
        s.error = None;
        // Guests who were already waiting.
        for p in room.remote_participants().values() {
            s.guests.push(Guest { identity: p.identity().to_string(), name: p.name(), frame: None, frame_seq: 0, level: 0.0, on_stage: false, muted: false, has_video: false, has_audio: false });
        }
    }

    // Your microphone to the guests.
    let mic = NativeAudioSource::new(AudioSourceOptions { echo_cancellation: false, noise_suppression: false, auto_gain_control: false }, 48000, 2, 200);
    let mic_track = LocalAudioTrack::create_audio_track("host-mic", RtcAudioSource::Native(mic.clone()));
    room.local_participant()
        .publish_track(LocalTrack::Audio(mic_track), TrackPublishOptions { source: TrackSource::Microphone, ..Default::default() })
        .await
        .context("publishing your microphone")?;
    // The program (Chisme's preview) to the guests.
    let program = NativeVideoSource::new(VideoResolution { width: 1280, height: 720 }, false);
    let program_track = LocalVideoTrack::create_video_track("program", RtcVideoSource::Native(program.clone()));
    room.local_participant()
        .publish_track(LocalTrack::Video(program_track), TrackPublishOptions { source: TrackSource::Camera, ..Default::default() })
        .await
        .context("publishing the program")?;

    // Feeders: microphone (10 ms blocks) and program (15 fps).
    {
        let stop = stop.clone();
        tokio::spawn(async move {
            let mut pending: Vec<i16> = vec![];
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                let blocks: Vec<Vec<[f32; 2]>> = outgoing().lock().unwrap().mic.drain(..).collect();
                for b in blocks {
                    pending.extend(b.iter().flat_map(|f| f.map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)));
                }
                while pending.len() >= 960 {
                    let chunk: Vec<i16> = pending.drain(..960).collect();
                    let frame = livekit::webrtc::audio_frame::AudioFrame { data: chunk.into(), sample_rate: 48000, num_channels: 2, samples_per_channel: 480 };
                    let _ = mic.capture_frame(&frame).await;
                }
            }
        });
    }
    {
        let stop = stop.clone();
        tokio::spawn(async move {
            use livekit::webrtc::video_frame::{I420Buffer, VideoFrame, VideoRotation};
            let mut last = 0u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(66)).await;
                let Some((size, rgba, seq)) = crate::guests::program_frame() else { continue };
                if seq == last || size.0 == 0 {
                    continue;
                }
                last = seq;
                let mut buf = I420Buffer::new(size.0, size.1);
                let (sy, su, sv) = buf.strides();
                let (y, u, v) = buf.data_mut();
                livekit::webrtc::native::yuv_helper::abgr_to_i420(&rgba, size.0 * 4, y, sy, u, su, v, sv, size.0 as i32, size.1 as i32);
                program.capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, buf));
            }
        });
    }

    let announce = |room: &Arc<Room>, stage: Vec<String>| {
        let room = room.clone();
        tokio::spawn(async move {
            let payload = serde_json::json!({ "type": "stage", "on": stage }).to_string().into_bytes();
            let _ = room.local_participant().publish_data(DataPacket { payload, topic: Some("chisme".into()), reliable: true, ..Default::default() }).await;
        });
    };

    loop {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        tokio::select! {
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    RoomEvent::ParticipantConnected(p) => {
                        let mut s = state.lock().unwrap();
                        if s.guest_mut(p.identity().as_str()).is_none() {
                            s.guests.push(Guest { identity: p.identity().to_string(), name: p.name(), frame: None, frame_seq: 0, level: 0.0, on_stage: false, muted: false, has_video: false, has_audio: false });
                        }
                        drop(s);
                        // Tell newcomers who's on stage.
                        let stage = state.lock().unwrap().stage.clone();
                        announce(&room, stage);
                    }
                    RoomEvent::ParticipantNameChanged { participant, name, .. } => {
                        if let Some(g) = state.lock().unwrap().guest_mut(participant.identity().as_str()) {
                            g.name = name;
                        }
                    }
                    RoomEvent::ParticipantDisconnected(p) => {
                        let mut s = state.lock().unwrap();
                        let id = p.identity().to_string();
                        s.guests.retain(|g| g.identity != id);
                        s.stage.retain(|x| *x != id);
                    }
                    RoomEvent::TrackSubscribed { track, participant, .. } => {
                        let id = participant.identity().to_string();
                        match track {
                            RemoteTrack::Video(v) => {
                                if let Some(g) = state.lock().unwrap().guest_mut(&id) {
                                    g.has_video = true;
                                }
                                tokio::spawn(video_reader(v, id, state.clone()));
                            }
                            RemoteTrack::Audio(a) => {
                                if let Some(g) = state.lock().unwrap().guest_mut(&id) {
                                    g.has_audio = true;
                                }
                                tokio::spawn(audio_reader(a, id, state.clone()));
                            }
                        }
                    }
                    RoomEvent::Disconnected { .. } => {
                        state.lock().unwrap().connected = false;
                        break;
                    }
                    _ => {}
                }
            }
            c = cmds.recv() => {
                let Some(c) = c else { break };
                let mut kick = None;
                let stage = {
                    let mut s = state.lock().unwrap();
                    match c {
                        Cmd::Stage(id, on) => {
                            s.stage.retain(|x| *x != id);
                            if on {
                                s.stage.push(id.clone());
                            }
                            if let Some(g) = s.guest_mut(&id) {
                                g.on_stage = on;
                            }
                        }
                        Cmd::Mute(id, m) => {
                            if let Some(g) = s.guest_mut(&id) {
                                g.muted = m;
                            }
                        }
                        Cmd::Remove(id) => {
                            kick = Some(id.clone());
                            s.stage.retain(|x| *x != id);
                            s.guests.retain(|g| g.identity != id);
                        }
                    }
                    s.stage.clone()
                };
                announce(&room, stage);
                if let Some(id) = kick {
                    let room = room.clone();
                    tokio::spawn(async move {
                        let payload = serde_json::json!({ "type": "kick", "id": id }).to_string().into_bytes();
                        let _ = room.local_participant().publish_data(DataPacket { payload, topic: Some("chisme".into()), reliable: true, ..Default::default() }).await;
                    });
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
        }
    }
    let _ = room.close().await;
    Ok(())
}

async fn video_reader(track: livekit::prelude::RemoteVideoTrack, id: String, state: Arc<Mutex<State>>) {
    use futures_util::StreamExt;
    use livekit::webrtc::video_frame::VideoFormatType;
    use livekit::webrtc::video_stream::native::NativeVideoStream;
    let mut stream = NativeVideoStream::new(track.rtc_track());
    let mut buf: Vec<u8> = vec![];
    while let Some(frame) = stream.next().await {
        let (w, h) = (frame.buffer.width(), frame.buffer.height());
        let (dw, dh) = (w, h);
        buf.resize((dw * dh * 4) as usize, 0);
        // libyuv's "ARGB" is B,G,R,A in memory: BGRA bytes.
        frame.buffer.as_ref().to_argb(VideoFormatType::ARGB, &mut buf, dw * 4, dw as i32, dh as i32);
        let mut s = state.lock().unwrap();
        let Some(g) = s.guest_mut(&id) else { break };
        g.frame = Some(((dw, dh), Arc::new(buf.clone())));
        g.frame_seq += 1;
    }
}

async fn audio_reader(track: livekit::prelude::RemoteAudioTrack, id: String, state: Arc<Mutex<State>>) {
    use futures_util::StreamExt;
    use livekit::webrtc::audio_stream::native::NativeAudioStream;
    let mut stream = NativeAudioStream::new(track.rtc_track(), 48000, 2);
    while let Some(frame) = stream.next().await {
        let frames: Vec<[f32; 2]> = frame.data.chunks_exact(2).map(|c| [c[0] as f32 / 32768.0, c[1] as f32 / 32768.0]).collect();
        let peak = frames.iter().fold(0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
        let mut s = state.lock().unwrap();
        let (on, muted) = match s.guest_mut(&id) {
            Some(g) => {
                g.level = g.level.max(peak) * 0.9 + peak * 0.1;
                (g.on_stage, g.muted)
            }
            None => break,
        };
        if !muted {
            s.audio.push_back((crate::win::now(), on, frames));
            while s.audio.len() > 200 {
                s.audio.pop_front();
            }
        }
    }
}

/// The program frame to send to guests (Chisme's preview, RGBA).
pub fn program_frame() -> Option<((u32, u32), Arc<Vec<u8>>, u64)> {
    let g = program().lock().unwrap();
    g.clone()
}

pub fn program() -> &'static Mutex<Option<((u32, u32), Arc<Vec<u8>>, u64)>> {
    static S: OnceLock<Mutex<Option<((u32, u32), Arc<Vec<u8>>, u64)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a real LiveKit project: `CHISME_LK_URL`, `CHISME_LK_KEY`,
    /// and the secret in Credential Manager. A fake guest joins with an
    /// invite link and sends a camera and a tone; the host must get both,
    /// and hear them in the show once on stage.
    #[test]
    #[ignore]
    fn live_room_end_to_end() {
        use livekit::options::TrackPublishOptions;
        use livekit::prelude::*;
        use livekit::webrtc::audio_source::native::NativeAudioSource;
        use livekit::webrtc::audio_source::{AudioSourceOptions, RtcAudioSource};
        use livekit::webrtc::video_frame::{I420Buffer, VideoFrame, VideoRotation};
        use livekit::webrtc::video_source::native::NativeVideoSource;
        use livekit::webrtc::video_source::{RtcVideoSource, VideoResolution};
        let secret = crate::win::creds::get("livekit").expect("secret");
        let cfg = Config { url: std::env::var("CHISME_LK_URL").unwrap(), api_key: std::env::var("CHISME_LK_KEY").unwrap(), room: new_room_name() };
        let host = start(cfg.clone(), secret.clone()).unwrap();
        let link = invite_link(&cfg, &secret, "").unwrap();
        let tok = link.split("token=").nth(1).unwrap().to_string();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guest = rt.block_on(async {
            let (room, mut ev) = Room::connect(&cfg.url, &tok, RoomOptions::default()).await.unwrap();
            room.local_participant().set_name("Test Guest".into()).await.unwrap();
            let vs = NativeVideoSource::new(VideoResolution { width: 640, height: 360 }, false);
            room.local_participant().publish_track(LocalTrack::Video(LocalVideoTrack::create_video_track("cam", RtcVideoSource::Native(vs.clone()))), TrackPublishOptions { source: TrackSource::Camera, ..Default::default() }).await.unwrap();
            let au = NativeAudioSource::new(AudioSourceOptions::default(), 48000, 2, 100);
            room.local_participant().publish_track(LocalTrack::Audio(LocalAudioTrack::create_audio_track("mic", RtcAudioSource::Native(au.clone()))), TrackPublishOptions { source: TrackSource::Microphone, ..Default::default() }).await.unwrap();
            tokio::spawn(async move {
                let mut n = 0u64;
                loop {
                    let mut b = I420Buffer::new(640, 360);
                    let (y, _, _) = b.data_mut();
                    y.fill((n * 3 % 255) as u8);
                    vs.capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, b));
                    for _ in 0..3 {
                        let data: Vec<i16> = (0..480).flat_map(|i| { let s = ((i as f32 / 48.0 * std::f32::consts::TAU).sin() * 8000.0) as i16; [s, s] }).collect();
                        let _ = au.capture_frame(&livekit::webrtc::audio_frame::AudioFrame { data: data.into(), sample_rate: 48000, num_channels: 2, samples_per_channel: 480 }).await;
                    }
                    n += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                }
            });
            // Stage messages from the host.
            let got_stage = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let gs = got_stage.clone();
            tokio::spawn(async move {
                while let Some(e) = ev.recv().await {
                    if let RoomEvent::DataReceived { payload, .. } = e
                        && String::from_utf8_lossy(&payload).contains("\"on\":[\"guest-")
                    {
                        gs.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
            (room, got_stage)
        });
        let t0 = std::time::Instant::now();
        let id = loop {
            assert!(t0.elapsed().as_secs() < 30, "guest never showed up with video and audio: {:?}", host.state.lock().unwrap().error);
            std::thread::sleep(std::time::Duration::from_millis(200));
            let s = host.state.lock().unwrap();
            if let Some(g) = s.guests.iter().find(|g| g.frame.is_some() && g.level > 0.05) {
                println!("guest {:?} {} frame {:?} level {:.2}", g.name, g.identity, g.frame.as_ref().map(|f| f.0), g.level);
                break g.identity.clone();
            }
        };
        assert_eq!(host.state.lock().unwrap().guests[0].name, "Test Guest");
        host.state.lock().unwrap().audio.clear();
        host.send(Cmd::Stage(id.clone(), true));
        std::thread::sleep(std::time::Duration::from_secs(2));
        let s = host.state.lock().unwrap();
        assert!(s.audio.iter().any(|a| a.1), "on-stage audio flagged for the show");
        assert_eq!(s.on_stage()[0].identity, id);
        assert!(_guest.1.load(std::sync::atomic::Ordering::Relaxed), "guest told they're on stage");
        println!("ok: video, audio, stage");
    }

    #[test]
    fn invite_links_carry_a_signed_token() {
        let cfg = Config { url: "wss://demo.livekit.cloud".into(), api_key: "APIdemo".into(), room: "chisme-abc".into() };
        let link = invite_link(&cfg, "secret-secret-secret-secret-secret", "").unwrap();
        assert!(link.starts_with(JOIN_PAGE));
        assert!(link.contains("url=wss%3A%2F%2Fdemo.livekit.cloud&token=ey"));
        assert!(cfg.ready("x"));
        assert!(!Config::default().ready("x"));
        // A name fills the guest's name field.
        let named = invite_link(&cfg, "secret-secret-secret-secret-secret", "Ana Paz").unwrap();
        assert!(named.ends_with("&name=Ana%20Paz"));
    }
}
