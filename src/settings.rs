//! App settings, saved as JSON in %APPDATA%\Chisme\settings.json.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, FOLDERID_Videos, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::compose::{PipSpec, Place, Scene, Shape};
use crate::engine::{AudioLayout, Config, LiveState, TargetSpec};
use crate::brand::Brand;
use crate::layout::Layout;
use crate::live::Platform;
use crate::subtitles::Model;
use crate::win::encoder::Codec;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub output_dir: PathBuf,
    /// 1-based display number.
    pub display: usize,
    pub resolution: String,
    pub fps: u32,
    pub codec: Codec,
    pub bitrate_mbps: Option<f64>,
    pub cursor: bool,
    pub system_audio: bool,
    /// Record only this app's audio (or everything but it, with
    /// `system_audio_exclude`) instead of all system audio.
    pub system_audio_app: Option<String>,
    pub system_audio_exclude: bool,
    pub mic_enabled: bool,
    /// Remove background noise from the mic (RNNoise).
    pub mic_denoise: bool,
    /// Part of the mic's name; None = the default mic.
    pub mic: Option<String>,
    pub audio_tracks: AudioLayout,
    pub system_gain_db: f32,
    pub mic_gain_db: f32,
    /// Part of the camera's name; None = no camera.
    pub camera: Option<String>,
    pub scene: Scene,
    pub pip_size: f64,
    pub pip_place: Place,
    pub pip_shape: Shape,
    pub pip_roundness: f64,
    pub pip_border: f64,
    pub pip_border_color: [u8; 3],
    pub pip_shadow: bool,
    pub mirror: bool,
    /// A custom layout (replaces the scene while set).
    pub layout: Option<Layout>,
    /// Logo, colors and font.
    pub brand: Brand,
    /// The template last applied (highlighted in the app).
    pub template: Option<String>,
    /// Server URLs for platforms that give you one (TikTok, X, custom).
    pub stream_urls: Vec<(Platform, String)>,
    /// Platforms streamed vertically (9:16).
    pub vertical: Vec<Platform>,
    pub replay_enabled: bool,
    pub replay_secs: f64,
    /// Write subtitles with Whisper after every recording.
    pub subtitles_auto: bool,
    pub subtitle_model: Model,
    /// "auto" or a language code (es, en…).
    pub subtitle_lang: String,
    /// The teleprompter's script and look.
    pub prompter: Prompter,
    /// Media Board tiles, in order (the first nine have hotkeys).
    pub media: Vec<crate::media::Item>,
    /// Interface language.
    pub language: crate::i18n::Lang,
    /// Global shortcuts.
    pub hotkeys: Hotkeys,
    /// Virtual background behind the webcam.
    pub vbg: crate::vbg::VirtualBackground,
    /// Window to record instead of a display (not saved: handles change).
    #[serde(skip)]
    pub window: Option<(usize, String)>,
}

/// Global shortcuts as combos ("Ctrl+Shift+R"; empty: off). Templates and
/// Media Board tiles use a modifier set plus 1…9.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub record: String,
    pub pause: String,
    pub marker: String,
    pub replay: String,
    pub prompter: String,
    pub prompter_play: String,
    pub templates: String,
    pub media: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Hotkeys {
            record: "Ctrl+Shift+R".into(),
            pause: "Ctrl+Shift+P".into(),
            marker: "Ctrl+Shift+M".into(),
            replay: "Ctrl+Shift+S".into(),
            prompter: "Ctrl+Shift+T".into(),
            prompter_play: "Ctrl+Shift+Space".into(),
            templates: "Ctrl+Shift".into(),
            media: "Ctrl+Alt".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prompter {
    pub script: String,
    /// Scroll speed in points per second.
    pub speed: f32,
    /// Font size in points.
    pub font: f32,
    /// Flip horizontally (for a beam-splitter glass).
    pub mirror: bool,
    /// Background opacity, 0–1.
    pub opacity: f32,
    /// Where the reading line sits, as a fraction of the window height.
    pub line: f32,
}

impl Default for Prompter {
    fn default() -> Self {
        Prompter {
            script: "Hi, welcome.\n\nWrite your script here. The teleprompter scrolls it for you while you talk.\n\nSpace: play / pause. Mouse wheel: move. Arrows: speed.".into(),
            speed: 40.0,
            font: 34.0,
            mirror: false,
            opacity: 0.72,
            line: 0.33,
        }
    }
}

fn known(id:&windows::core::GUID) -> PathBuf {
    unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok().and_then(|p| p.to_string().ok()).map(PathBuf::from).unwrap_or_default() }
}

pub fn videos_dir() -> PathBuf {
    known(&FOLDERID_Videos).join("Chisme")
}

impl Default for Settings {
    fn default() -> Self {
        let pip = PipSpec::default();
        Settings {
            output_dir: videos_dir(),
            display: 1,
            resolution: "1080p".into(),
            fps: 60,
            codec: Codec::H264,
            bitrate_mbps: None,
            cursor: true,
            system_audio: true,
            system_audio_app: None,
            system_audio_exclude: false,
            mic_enabled: true,
            mic_denoise: true,
            mic: None,
            audio_tracks: AudioLayout::Both,
            system_gain_db: 0.0,
            mic_gain_db: 0.0,
            camera: None,
            scene: Scene::ScreenCam,
            pip_size: pip.size,
            pip_place: pip.place,
            pip_shape: pip.shape,
            pip_roundness: pip.roundness,
            pip_border: pip.border,
            pip_border_color: pip.border_color,
            pip_shadow: pip.shadow,
            mirror: pip.mirror,
            layout: None,
            brand: Brand::default(),
            template: None,
            stream_urls: vec![],
            vertical: vec![Platform::Tiktok],
            replay_enabled: false,
            replay_secs: 60.0,
            subtitles_auto: false,
            subtitle_model: Model::Small,
            subtitle_lang: "auto".into(),
            prompter: Prompter::default(),
            media: vec![],
            language: crate::i18n::Lang::En,
            hotkeys: Hotkeys::default(),
            vbg: Default::default(),
            window: None,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        known(&FOLDERID_RoamingAppData).join("Chisme").join("settings.json")
    }

    pub fn load() -> Settings {
        // Editors like Notepad may add a byte-order mark.
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let p = Self::path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn pip(&self) -> PipSpec {
        PipSpec {
            size: self.pip_size,
            place: self.pip_place,
            shape: self.pip_shape,
            roundness: self.pip_roundness,
            border: self.pip_border,
            border_color: self.pip_border_color,
            shadow: self.pip_shadow,
            mirror: self.mirror,
        }
    }

    pub fn set_pip(&mut self, p: &PipSpec) {
        self.pip_size = p.size;
        self.pip_place = p.place;
        self.pip_shape = p.shape;
        self.pip_roundness = p.roundness;
        self.pip_border = p.border;
        self.pip_border_color = p.border_color;
        self.pip_shadow = p.shadow;
        self.mirror = p.mirror;
    }

    /// The settings that need the capture restarted.
    pub fn config(&self) -> Config {
        Config {
            target: match &self.window {
                Some((h, t)) => TargetSpec::Window(*h, t.clone()),
                None => TargetSpec::Display(self.display),
            },
            resolution: self.resolution.clone(),
            fps: self.fps,
            codec: self.codec,
            bitrate: self.bitrate_mbps,
            cursor: self.cursor,
            system_audio: self.system_audio,
            app_audio: self.system_audio_app.clone().map(|a| (a, !self.system_audio_exclude)),
            mic_denoise: self.mic_denoise,
            mic: self.mic_enabled.then(|| self.mic.clone().unwrap_or_default()),
            audio_tracks: self.audio_tracks,
            cam: self.camera.clone(),
            test_pattern: false,
        }
    }

    pub fn live(&self) -> LiveState {
        LiveState { scene: self.scene, pip: self.pip(), gain_db: [self.system_gain_db, self.mic_gain_db], layout: self.layout.clone(), brand: self.brand.clone(), vbg: self.vbg.clone() }
    }

    pub fn stream_url(&self, p: Platform) -> String {
        match p.default_url() {
            Some(u) => u.to_string(),
            None => self.stream_urls.iter().find(|(q, _)| *q == p).map(|(_, u)| u.clone()).unwrap_or_default(),
        }
    }

    pub fn set_stream_url(&mut self, p: Platform, url: &str) {
        self.stream_urls.retain(|(q, _)| *q != p);
        if !url.trim().is_empty() {
            self.stream_urls.push((p, url.trim().to_string()));
        }
    }

    pub fn is_vertical(&self, p: Platform) -> bool {
        self.vertical.contains(&p)
    }

    /// A fresh timestamped file in the output folder.
    pub fn new_output_file(&self) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.output_dir)?;
        Ok(self.output_dir.join(timestamped()))
    }
}

/// `Chisme 2026-10-13 14.05.09.mp4`
pub fn timestamped() -> String {
    let t = unsafe { GetLocalTime() };
    format!("Chisme {}-{:02}-{:02} {:02}.{:02}.{:02}.mp4", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::Corner;

    #[test]
    fn round_trips_and_fills_missing_fields() {
        let mut s = Settings { fps: 30, camera: Some("obsbot".into()), pip_place: Place::Corner(Corner::TopLeft), ..Default::default() };
        s.pip_shape = Shape::Squircle;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), s);
        // An older file without newer fields still loads.
        let old: Settings = serde_json::from_str(r#"{"fps": 24}"#).unwrap();
        assert_eq!(old.fps, 24);
        assert_eq!(old.resolution, "1080p");
    }

    #[test]
    fn config_maps_mic_and_camera() {
        let s = Settings { mic_enabled: false, ..Default::default() };
        assert_eq!(s.config().mic, None);
        let s = Settings { mic: Some("maono".into()), ..Default::default() };
        assert_eq!(s.config().mic.as_deref(), Some("maono"));
        assert_eq!(Settings::default().config().mic.as_deref(), Some(""));
        assert_eq!(Settings::default().config().cam, None);
    }

    #[test]
    fn output_names() {
        let n = timestamped();
        assert!(n.starts_with("Chisme ") && n.ends_with(".mp4"), "{n}");
    }
}
