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
pub fn invite_link(cfg: &Config, secret: &str) -> Result<String> {
    let id = format!("guest-{:08x}", rand_u32());
    let t = token(cfg, secret, &id, "Guest", 24, false)?;
    Ok(format!("{JOIN_PAGE}#url={}&token={}", urlencode(&cfg.url), t))
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

    #[test]
    fn invite_links_carry_a_signed_token() {
        let cfg = Config { url: "wss://demo.livekit.cloud".into(), api_key: "APIdemo".into(), room: "chisme-abc".into() };
        let link = invite_link(&cfg, "secret-secret-secret-secret-secret").unwrap();
        assert!(link.starts_with(JOIN_PAGE));
        assert!(link.contains("url=wss%3A%2F%2Fdemo.livekit.cloud&token=ey"));
        assert!(cfg.ready("x"));
        assert!(!Config::default().ready("x"));
    }
}
