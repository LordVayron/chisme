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
    /// Muted sources: system audio, microphone.
    pub system_muted: bool,
    pub mic_muted: bool,
    /// Media Board volume in the mix (dB) and mute.
    pub media_gain_db: f32,
    pub media_muted: bool,
    /// Show Chisme's own windows in recordings (normally hidden).
    pub show_self: bool,
    /// Studio page: Live audio and Camera cards folded to their title.
    pub fold_audio: bool,
    pub fold_camera: bool,
    pub fold_chat: bool,
    /// Custom layouts can't be moved on the previews while locked.
    pub layout_locked: bool,
    /// The same for the 9:16 layout.
    pub layout_locked_v: bool,
    /// Quick-access scenes and templates under the preview, in order.
    pub pinned: Vec<Pin>,
    /// Studios: named sets of scenes. The active one's scenes are `pinned`
    /// (kept in step when switching).
    pub studios: Vec<Studio>,
    /// The active studio (index into `studios`).
    pub studio: usize,
    /// Which built-in additions the studios already got (see `ensure_studios`).
    pub studios_rev: u8,
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
    /// The current scene's 9:16 layout (None: made from the 16:9 one).
    pub layout_v: Option<Layout>,
    /// Every scene's own 9:16 layout, by scene key (see `scene_key`).
    pub verticals: Vec<(String, Layout)>,
    /// Show the 9:16 preview next to the 16:9 one.
    pub vertical_preview: bool,
    /// Show the 16:9 preview (hidden only while the 9:16 one shows).
    pub horizontal_preview: bool,
    /// What Record writes: horizontal, vertical or both.
    pub output: crate::engine::Output,
    /// How one scene turns into the next.
    pub transition: crate::win::transition::Transition,
    /// Bumped on every scene change (not saved).
    #[serde(skip)]
    pub switch: u64,
    /// Chat channels (the YouTube API key lives in Credential Manager).
    pub chat: crate::chat::Config,
    /// The chat comment on screen (not saved).
    #[serde(skip)]
    pub chat_pin: Option<(String, String)>,
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
    /// The starter sound pad was added once (deleting it keeps it gone).
    pub media_seeded: bool,
    /// Which starter pack was added (2: the ElevenLabs one).
    pub media_pack: u32,
    /// Media Board items pinned to the Studio, in their order (by path).
    pub media_strip: Vec<String>,
    /// Platforms the Studio's Go live button starts (None: every one with a
    /// key; set once you pick).
    pub go_live: Option<Vec<crate::live::Platform>>,
    /// Guests: your LiveKit project (the secret is in Credential Manager).
    pub guests: crate::guests::Config,
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

/// A quick-access button under the preview: a built-in scene or a saved
/// template (by name).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pin {
    Scene(Scene),
    Template(String),
}

/// A studio: a name and the scenes shown under the preview when it's on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Studio {
    pub name: String,
    pub scenes: Vec<Pin>,
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
    pub studios: String,
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
            studios: "Alt+Shift".into(),
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
            system_muted: false,
            mic_muted: false,
            media_gain_db: 0.0,
            media_muted: false,
            show_self: false,
            fold_audio: false,
            fold_camera: false,
            fold_chat: false,
            media_strip: vec![],
            go_live: None,
            layout_locked: false,
            layout_locked_v: false,
            pinned: Scene::ALL.iter().map(|s| Pin::Scene(*s)).collect(),
            studios: vec![],
            studio: 0,
            studios_rev: 0,
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
            layout_v: None,
            verticals: vec![],
            vertical_preview: false,
            horizontal_preview: true,
            output: crate::engine::Output::Horizontal,
            transition: Default::default(),
            switch: 0,
            chat: Default::default(),
            chat_pin: None,
            brand: Brand::default(),
            template: None,
            stream_urls: vec![],
            vertical: vec![Platform::Tiktok, Platform::Instagram],
            replay_enabled: false,
            replay_secs: 60.0,
            subtitles_auto: false,
            subtitle_model: Model::Small,
            subtitle_lang: "auto".into(),
            prompter: Prompter::default(),
            media: vec![],
            media_seeded: false,
            media_pack: 0,
            guests: Default::default(),
            language: crate::i18n::Lang::En,
            hotkeys: Hotkeys::default(),
            vbg: Default::default(),
            window: None,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        // CHISME_SETTINGS=file: another settings file (tests and screenshots
        // never touch the real one).
        if let Some(p) = std::env::var_os("CHISME_SETTINGS") {
            return PathBuf::from(p);
        }
        known(&FOLDERID_RoamingAppData).join("Chisme").join("settings.json")
    }

    pub fn load() -> Settings {
        // Editors like Notepad may add a byte-order mark.
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
    }

    /// Create the studios the first time: your current scenes, then
    /// Podcast, Content / Reels, Tutorial and Gaming / Stream. `templates`
    /// are the names on disk (some studios use the guest templates).
    pub fn ensure_studios(&mut self, templates: &[String]) {
        if !self.studios.is_empty() {
            self.studio = self.studio.min(self.studios.len() - 1);
            // The active studio's scenes live in `pinned`.
            self.studios[self.studio].scenes = self.pinned.clone();
            self.add_stream_screens(templates);
            return;
        }
        use crate::i18n::l;
        let tpl = |name: &str| templates.iter().any(|t| t == name).then(|| Pin::Template(name.to_string()));
        let guests: Vec<Pin> = crate::templates::guest_scenes().iter().filter_map(|t| tpl(&t.name)).collect();
        let presenter = tpl(l("Presenter", "Presentador"));
        let mut content = vec![Pin::Scene(Scene::Cam), Pin::Scene(Scene::ScreenCam), Pin::Scene(Scene::SideBySide)];
        content.extend(presenter);
        self.studios = vec![
            Studio { name: l("My scenes", "Mis escenas").into(), scenes: self.pinned.clone() },
            Studio { name: "Podcast".into(), scenes: guests },
            Studio { name: l("Content / Reels", "Contenido / Reels").into(), scenes: content },
            Studio { name: "Tutorial".into(), scenes: vec![Pin::Scene(Scene::Screen), Pin::Scene(Scene::ScreenCam), Pin::Scene(Scene::CamScreen)] },
            Studio { name: "Gaming / Stream".into(), scenes: vec![Pin::Scene(Scene::ScreenCam), Pin::Scene(Scene::Screen), Pin::Scene(Scene::Cam)] },
        ];
        self.studio = 0;
        self.add_stream_screens(templates);
    }

    /// Once: put "Be right back" and "Starting soon" in Gaming / Stream.
    fn add_stream_screens(&mut self, templates: &[String]) {
        if self.studios_rev >= 1 {
            return;
        }
        self.studios_rev = 1;
        let screens: Vec<Pin> = crate::templates::stream_scenes().iter().filter(|t| templates.iter().any(|n| *n == t.name)).map(|t| Pin::Template(t.name.clone())).collect();
        let cur = self.studio;
        if let Some((i, s)) = self.studios.iter_mut().enumerate().find(|(_, s)| s.name.starts_with("Gaming")) {
            for p in &screens {
                if !s.scenes.contains(p) {
                    s.scenes.push(p.clone());
                }
            }
            if i == cur {
                self.pinned = s.scenes.clone();
            }
        }
    }

    /// Make studio `i` the active one.
    pub fn switch_studio(&mut self, i: usize) {
        if i >= self.studios.len() {
            return;
        }
        if let Some(cur) = self.studios.get_mut(self.studio) {
            cur.scenes = self.pinned.clone();
        }
        self.studio = i;
        self.pinned = self.studios[i].scenes.clone();
    }

    /// Add an empty studio and switch to it.
    pub fn add_studio(&mut self, name: String) {
        self.studios.push(Studio { name, scenes: vec![] });
        self.switch_studio(self.studios.len() - 1);
    }

    /// Delete studio `i` (the last one stays).
    pub fn remove_studio(&mut self, i: usize) {
        if self.studios.len() <= 1 || i >= self.studios.len() {
            return;
        }
        if let Some(cur) = self.studios.get_mut(self.studio) {
            cur.scenes = self.pinned.clone();
        }
        self.studios.remove(i);
        self.studio = if self.studio > i { self.studio - 1 } else { self.studio.min(self.studios.len() - 1) };
        self.pinned = self.studios[self.studio].scenes.clone();
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
        LiveState {
            scene: self.scene,
            pip: self.pip(),
            gain_db: [self.system_gain_db, self.mic_gain_db],
            muted: [self.system_muted, self.mic_muted],
            media_gain_db: self.media_gain_db,
            media_muted: self.media_muted,
            layout: self.layout.clone(),
            brand: self.brand.clone(),
            vbg: self.vbg.clone(),
            vertical: self.vertical_layout(),
            vertical_preview: self.vertical_preview,
            output: self.output,
            transition: self.transition,
            switch: self.switch,
            chat_pin: self.chat_pin.clone(),
            media_loop: self.media.iter().filter(|m| m.looped).map(|m| m.path.clone()).collect(),
        }
    }

    /// The 9:16 layout in use: the scene's own, or made from the 16:9 one.
    pub fn vertical_layout(&self) -> Layout {
        self.layout_v.clone().unwrap_or_else(|| self.auto_vertical())
    }

    /// Which scene is on, as a key for its own 9:16 layout.
    pub fn scene_key(&self) -> String {
        match (&self.template, &self.layout) {
            (Some(n), _) => format!("t:{n}"),
            (None, Some(_)) => "custom".into(),
            _ => format!("s:{}", serde_json::to_string(&self.scene).unwrap_or_default()),
        }
    }

    fn store_vertical(&mut self, key: String, v: Layout) {
        match self.verticals.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = v,
            None => self.verticals.push((key, v)),
        }
    }

    /// Before the scene changes: keep its own 9:16 layout (if it has one).
    pub fn keep_vertical(&mut self) {
        if let Some(v) = self.layout_v.clone() {
            let k = self.scene_key();
            self.store_vertical(k, v);
        }
    }

    /// After the scene changed: its own 9:16 layout, else the template's.
    /// Fixed scenes (not editable) without one follow their 16:9 picture.
    pub fn load_vertical(&mut self, template: Option<Layout>) {
        let k = self.scene_key();
        self.layout_v = self.verticals.iter().find(|(key, _)| *key == k).map(|(_, v)| v.clone()).or(template);
        if let Some(v) = self.layout_v.clone() {
            self.store_vertical(k, v);
        }
    }

    /// The 9:16 layout made from the 16:9 one.
    pub fn auto_vertical(&self) -> Layout {
        // Without a camera the fixed scenes show the screen only (like 16:9).
        let scene = if self.camera.is_some() { self.scene } else { Scene::Screen };
        let h = self.layout.clone().unwrap_or_else(|| Layout::from_scene(scene, &self.pip(), 1920.0, 1080.0, 16.0 / 9.0, 16.0 / 9.0));
        h.to_vertical()
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

#[cfg(test)]
mod studio_tests {
    use super::*;

    #[test]
    fn studios_keep_their_scenes() {
        let mut s = Settings::default();
        s.pinned = vec![Pin::Scene(Scene::Screen)];
        s.ensure_studios(&[]);
        assert_eq!(s.studios.len(), 5);
        assert_eq!(s.studios[0].scenes, vec![Pin::Scene(Scene::Screen)], "your scenes become the first studio");
        s.switch_studio(3);
        assert_eq!(s.pinned, s.studios[3].scenes);
        s.pinned.push(Pin::Scene(Scene::Cam));
        s.switch_studio(0);
        assert_eq!(s.studios[3].scenes.last(), Some(&Pin::Scene(Scene::Cam)), "edits stay with their studio");
        s.add_studio("New".into());
        assert_eq!((s.studio, s.pinned.len()), (5, 0));
        s.remove_studio(5);
        assert_eq!(s.studios.len(), 5);
        assert!(s.studio < 5);
    }
}

#[cfg(test)]
mod vertical_tests {
    use super::*;

    #[test]
    fn each_editable_scene_keeps_its_own_vertical() {
        let mut s = Settings::default();
        s.layout = Some(Layout::default());
        s.template = Some("A".into());
        let mut mine = Layout::default();
        mine.background = [1, 2, 3];
        s.layout_v = Some(mine.clone());
        // Switch to template B, then back to A.
        s.keep_vertical();
        s.template = Some("B".into());
        s.load_vertical(None);
        assert_eq!(s.layout_v, None, "B has none of its own yet: made from its 16:9");
        s.keep_vertical();
        s.template = Some("A".into());
        s.load_vertical(None);
        assert_eq!(s.layout_v, Some(mine), "A gets its own back");
        // A fixed scene follows its 16:9 picture.
        s.keep_vertical();
        s.template = None;
        s.layout = None;
        s.load_vertical(None);
        assert_eq!(s.layout_v, None);
    }
}
