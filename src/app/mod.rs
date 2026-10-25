//! The Chisme app: eframe/egui window with a live preview, record controls,
//! audio meters, camera overlay controls, recordings and settings. A tray
//! icon and global hotkeys (Ctrl+Shift+R record/stop, Ctrl+Shift+P pause)
//! work while the window is hidden.

mod editor;
mod groups;
mod guests_ui;
pub mod hotkeys;
mod media_ui;
mod prompter;
mod templates_ui;
mod theme;
mod ui;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, anyhow};
use eframe::egui;
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::engine::{Cmd, Config, Engine, Status};
use crate::live::{Platform, StreamSpec};
use crate::i18n::l;
use crate::settings::Settings;
use crate::t;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Studio,
    Live,
    Templates,
    Guests,
    Media,
    Prompter,
    Library,
    Settings,
}

/// Start the engine with the saved replay setting.
fn start_engine(settings: &Settings, cfg: Config) -> Engine {
    let e = Engine::start(cfg, settings.live());
    if settings.replay_enabled {
        e.send(Cmd::Replay(Some(settings.replay_secs)));
    }
    e
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    /// Whisper subtitles (.srt).
    Subtitles,
    /// A vertical 9:16 copy.
    Vertical,
}

/// Work on a recording running in the background, one at a time.
pub struct Job {
    pub kind: JobKind,
    pub file: PathBuf,
    pub stage: String,
    pub progress: f32,
    pub done: Option<std::result::Result<PathBuf, String>>,
}

/// "Save scene": its name and group, asked in a small window.
#[derive(Default)]
pub struct SaveDialog {
    pub name: String,
    pub group: String,
}

pub struct Recording {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub modified: SystemTime,
}

/// Devices for the settings pickers (enumerated on demand).
#[derive(Default)]
pub struct Devices {
    pub displays: Vec<String>,
    pub windows: Vec<(usize, String)>,
    pub cameras: Vec<String>,
    pub mics: Vec<String>,
    /// Apps with an audio session (for per-app audio).
    pub audio_apps: Vec<String>,
}

struct Tray {
    _icon: TrayIcon,
    record: MenuItem,
    pause: MenuItem,
    marker: MenuItem,
    replay: MenuItem,
    show: MenuItem,
    folder: MenuItem,
    quit: MenuItem,
}

pub struct App {
    pub settings: Settings,
    pub engine: Option<Engine>,
    pub config: Config,
    pub status: Status,
    pub page: Page,
    pub preview: Option<egui::TextureHandle>,
    pub preview_seq: u64,
    /// The 9:16 preview.
    pub preview_v: Option<egui::TextureHandle>,
    pub preview_v_seq: u64,
    /// Level history (dBFS) for the audio chart: (system, mic).
    pub levels: VecDeque<(f32, f32)>,
    last_level: Instant,
    pub recordings: Vec<Recording>,
    last_scan: Option<Instant>,
    last_finished: u64,
    pub devices: Devices,
    pub toast: Option<(String, Instant)>,
    tray: Option<Tray>,
    /// The tray texts' language and shortcuts (refreshed when they change).
    tray_for: Option<(crate::i18n::Lang, crate::settings::Hotkeys)>,
    /// Global shortcuts, and the one being typed in Settings.
    pub hotkeys: hotkeys::Registry,
    pub capturing: Option<hotkeys::Action>,
    /// Media Board image thumbnails, by path.
    pub thumbs: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    pub prompter: prompter::State,
    pub templates: Vec<crate::templates::Template>,
    /// The name typed for "save the current layout as a template".
    pub template_name: String,
    pub hidden: bool,
    quitting: bool,
    /// Whether the window is currently hidden from captures.
    excluded: Option<bool>,
    /// Stream keys being typed (loaded from Credential Manager).
    pub keys: Vec<(Platform, String)>,
    last_notices: u64,
    /// Background work (subtitles, vertical exports): the running job and
    /// the ones waiting.
    pub job: std::sync::Arc<std::sync::Mutex<Option<Job>>>,
    pub jobs: VecDeque<(JobKind, PathBuf)>,
    /// Make a vertical copy of the next replay saved.
    replay_vertical: bool,
    last_replay: Option<PathBuf>,
    /// Layout editor: the selected layer, a drag in progress and the snap
    /// guides to draw (x, y as fractions).
    pub sel_layer: Option<usize>,
    /// The selection and drag are on the 9:16 layout.
    pub edit_v: bool,
    pub drag: Option<editor::Drag>,
    pub guides: (Option<f64>, Option<f64>),
    pub save_dialog: Option<SaveDialog>,
    /// Recordings page: the ones ticked, and a delete waiting for "Yes".
    pub selected: std::collections::HashSet<PathBuf>,
    pub confirm_delete: bool,
    /// Camera card tab: 0 look, 1 controls.
    pub cam_tab: u8,
    /// The "new group" name being typed in a Move-to-group menu.
    pub group_input: String,
    /// Guests: the open room, the API secret being typed, the invite links
    /// made, and camera thumbnails (identity -> frame counter, texture).
    pub guest_room: Option<crate::guests::Room>,
    pub guest_secret: String,
    pub guest_links: Vec<String>,
    pub guest_thumbs: std::collections::HashMap<String, (u64, egui::TextureHandle)>,
}

const HISTORY: usize = 200; // 10 s at 20 Hz

impl App {
    fn new(cc: &eframe::CreationContext) -> App {
        theme::install(&cc.egui_ctx);
        let mut settings = Settings::load();
        // CHISME_WINDOW=0x… records that window (for screenshots).
        if let Some(h) = std::env::var("CHISME_WINDOW").ok().and_then(|v| usize::from_str_radix(v.trim_start_matches("0x"), 16).ok()) {
            settings.window = Some((h, "Demo".into()));
        }
        crate::i18n::set(settings.language);
        let config = settings.config();
        let engine = start_engine(&settings, config.clone());
        let tray = Self::tray();
        let mut hotkeys = hotkeys::Registry::new();
        hotkeys.apply(&settings.hotkeys);
        crate::templates::ensure_starters(&settings.pip());
        crate::templates::ensure_guest_scenes();
        let names: Vec<String> = crate::templates::list().iter().map(|t| t.name.clone()).collect();
        settings.ensure_studios(&names);
        let _ = settings.save();
        // The starter sound pad, and the new pack in place of the old one.
        if settings.media_pack < 2 {
            settings.media.retain(|m| !crate::media::is_old_starter(m));
            settings.media.extend(crate::media::starter_sounds());
            settings.media_seeded = true;
            settings.media_pack = 2;
            let _ = settings.save();
        }
        let keys = Platform::ALL.iter().map(|p| (*p, crate::win::creds::get(p.id()).unwrap_or_default())).collect();
        App {
            settings,
            engine: Some(engine),
            config,
            status: Status::default(),
            // CHISME_PAGE opens on a page (for screenshots).
            page: match std::env::var("CHISME_PAGE").as_deref() {
                Ok("live") => Page::Live,
                Ok("templates") => Page::Templates,
                Ok("prompter") => Page::Prompter,
                Ok("media") => Page::Media,
                Ok("guests") => Page::Guests,
                Ok("library") => Page::Library,
                Ok("settings") => Page::Settings,
                _ => Page::Studio,
            },
            preview: None,
            preview_seq: 0,
            preview_v: None,
            preview_v_seq: 0,
            levels: VecDeque::from(vec![(-90.0, -90.0); HISTORY]),
            last_level: Instant::now(),
            recordings: vec![],
            last_scan: None,
            last_finished: 0,
            devices: Devices::default(),
            toast: None,
            tray,
            tray_for: None,
            hotkeys,
            capturing: None,
            thumbs: Default::default(),
            prompter: Default::default(),
            templates: crate::templates::list(),
            template_name: String::new(),
            hidden: false,
            quitting: false,
            excluded: None,
            keys,
            last_notices: 0,
            job: Default::default(),
            jobs: VecDeque::new(),
            replay_vertical: false,
            last_replay: None,
            sel_layer: None,
            edit_v: false,
            drag: None,
            guides: (None, None),
            save_dialog: None,
            selected: Default::default(),
            confirm_delete: false,
            cam_tab: 0,
            group_input: String::new(),
            guest_room: None,
            guest_secret: crate::win::creds::get("livekit").unwrap_or_default(),
            guest_links: vec![],
            guest_thumbs: Default::default(),
        }
    }

    /// Queue background work on a recording.
    pub fn queue(&mut self, kind: JobKind, path: PathBuf) {
        let busy = self.job.lock().unwrap().as_ref().is_some_and(|j| j.kind == kind && j.file == path);
        if !busy && !self.jobs.contains(&(kind, path.clone())) {
            self.jobs.push_back((kind, path));
        }
    }

    /// The running job's (stage, progress) for this file and kind, if any.
    pub fn job_for(&self, kind: JobKind, path: &std::path::Path) -> Option<(String, f32)> {
        self.job.lock().unwrap().as_ref().filter(|j| j.kind == kind && j.file == path).map(|j| (j.stage.clone(), j.progress))
    }

    /// Finish the current job and start the next one.
    fn pump_jobs(&mut self) {
        let finished = {
            let mut j = self.job.lock().unwrap();
            if j.as_ref().is_some_and(|j| j.done.is_some()) { j.take() } else { None }
        };
        if let Some(j) = finished {
            let what = if j.kind == JobKind::Subtitles { l("Subtitles", "Subtítulos") } else { l("Vertical clip", "Clip vertical") };
            match j.done.unwrap() {
                Ok(p) => self.toast(t!("{} ready: {}", "{} listo: {}", what, p.file_name().unwrap_or_default().to_string_lossy())),
                Err(e) => self.toast(format!("{what}: {e}")),
            }
            self.scan_recordings();
        }
        if self.job.lock().unwrap().is_some() {
            return;
        }
        let Some((kind, file)) = self.jobs.pop_front() else { return };
        *self.job.lock().unwrap() = Some(Job { kind, file: file.clone(), stage: l("Preparing", "Preparando").into(), progress: 0.0, done: None });
        let (job, model) = (self.job.clone(), self.settings.subtitle_model);
        let lang = Some(self.settings.subtitle_lang.clone()).filter(|l| l != "auto");
        std::thread::spawn(move || {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            let _ = crate::win::encoder::startup();
            let j2 = job.clone();
            let report = move |stage: &str, p: f32| {
                if let Some(j) = j2.lock().unwrap().as_mut() {
                    j.stage = stage.to_string();
                    j.progress = p;
                }
            };
            let r = match kind {
                JobKind::Subtitles => crate::subtitles::subtitle_file(&file, model, lang.as_deref(), report),
                JobKind::Vertical => crate::export::vertical(&file, |p| report(l("Exporting", "Exportando"), p)),
            };
            if let Some(j) = job.lock().unwrap().as_mut() {
                j.done = Some(r.map_err(|e| format!("{e:#}")));
            }
        });
    }

    /// Save the replay buffer and make a vertical copy of it.
    pub fn save_replay_vertical(&mut self) {
        if self.status.replay.is_some() {
            self.replay_vertical = true;
        }
        self.save_replay();
    }

    fn tray() -> Option<Tray> {
        (|| -> Result<Tray> {
            // The texts are set by `refresh_tray`.
            let record = MenuItem::new("", true, None);
            let pause = MenuItem::new("", false, None);
            let marker = MenuItem::new("", false, None);
            let replay = MenuItem::new("", false, None);
            let show = MenuItem::new("", true, None);
            let folder = MenuItem::new("", true, None);
            let quit = MenuItem::new("", true, None);
            let menu = Menu::new();
            menu.append_items(&[&record, &pause, &marker, &replay, &PredefinedMenuItem::separator(), &show, &folder, &PredefinedMenuItem::separator(), &quit])?;
            let icon = tray_icon::Icon::from_rgba(theme::icon_rgba(32), 32, 32)?;
            let _icon = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("Chisme").with_icon(icon).build()?;
            Ok(Tray { _icon, record, pause, marker, replay, show, folder, quit })
        })()
        .ok()
    }

    /// Run a shortcut's action.
    fn run(&mut self, a: hotkeys::Action) {
        use hotkeys::Action;
        match a {
            Action::Record => self.toggle_record(),
            Action::Pause => self.toggle_pause(),
            Action::Marker => self.marker(),
            Action::Replay => self.save_replay(),
            Action::Prompter => self.toggle_prompter(),
            Action::PrompterPlay => self.prompter_play(),
            Action::Template(i) => {
                if let Some(t) = self.templates.get(i).cloned() {
                    self.apply_template(&t);
                }
            }
            Action::Media(i) => self.media_fire(i),
        }
    }

    pub fn key(&self, p: Platform) -> String {
        self.keys.iter().find(|(q, _)| *q == p).map(|(_, k)| k.clone()).unwrap_or_default()
    }

    pub fn key_mut(&mut self, p: Platform) -> &mut String {
        let i = self.keys.iter().position(|(q, _)| *q == p).expect("every platform has a key slot");
        &mut self.keys[i].1
    }

    pub fn stream_id(&self, p: Platform) -> Option<u64> {
        self.status.streams.iter().find(|s| s.platform == Some(p)).map(|s| s.id)
    }

    /// Go live on `p` with its saved URL and the typed key (saved to
    /// Credential Manager).
    pub fn start_stream(&mut self, p: Platform) {
        let key = self.key(p).trim().to_string();
        if key.is_empty() {
            self.toast(t!("The {} stream key is missing", "Falta la clave de transmisión de {}", p.label()));
            return;
        }
        if let Err(e) = crate::win::creds::set(p.id(), &key) {
            self.toast(t!("Couldn't save the key: {:#}", "No pude guardar la clave: {:#}", e));
        }
        let url = self.settings.stream_url(p);
        if url.is_empty() {
            self.toast(t!("The {} server URL is missing", "Falta la URL del servidor de {}", p.label()));
            return;
        }
        if let Some(e) = &self.engine {
            e.send(Cmd::StartStream(StreamSpec { platform: p, url, key, vertical: self.settings.is_vertical(p) || p == Platform::Instagram, kbps: None }));
        }
    }

    pub fn stop_stream(&mut self, p: Platform) {
        if let (Some(e), Some(id)) = (&self.engine, self.stream_id(p)) {
            e.send(Cmd::StopStream(id));
        }
    }

    pub fn set_replay(&mut self, on: bool) {
        self.settings.replay_enabled = on;
        let _ = self.settings.save();
        if let Some(e) = &self.engine {
            e.send(Cmd::Replay(on.then_some(self.settings.replay_secs)));
        }
    }

    /// Switch to a template's layout (and brand, if it has one), live.
    pub fn apply_template(&mut self, t: &crate::templates::Template) {
        self.settings.switch += 1;
        self.settings.layout = Some(t.layout.clone());
        self.settings.layout_v = t.vertical.clone();
        if let Some(b) = &t.brand {
            self.settings.brand = b.clone();
        }
        self.settings.template = Some(t.name.clone());
        self.sel_layer = None;
        self.settings_changed();
        self.toast(t!("Template: {}", "Plantilla: {}", t.name));
    }

    /// Save the current layout (and brand) as a template named `name`.
    pub fn save_template(&mut self, name: &str, group: &str) {
        if self.settings.layout.is_none() {
            self.customize_layout();
        }
        let Some(layout) = self.settings.layout.clone() else { return };
        let t = crate::templates::Template { name: name.trim().to_string(), layout, brand: Some(self.settings.brand.clone()), assets: vec![], group: group.trim().to_string(), vertical: self.settings.layout_v.clone() };
        match crate::templates::save(&t) {
            Ok(_) => {
                self.settings.template = Some(t.name.clone());
                let _ = self.settings.save();
                self.toast(t!("Template saved: {}", "Plantilla guardada: {}", t.name));
            }
            Err(e) => self.toast(t!("Couldn't save the template: {:#}", "No pude guardar la plantilla: {:#}", e)),
        }
        self.templates = crate::templates::list();
    }

    /// Move recordings to the Recycle Bin (with their subtitles and chapters).
    pub fn delete_recordings(&mut self, paths: &[PathBuf]) {
        let mut files: Vec<PathBuf> = vec![];
        for p in paths {
            files.push(p.clone());
            for side in [crate::subtitles::srt_path(p), crate::chapters::path_for(p), p.with_extension("capitulos.txt")] {
                if side.is_file() {
                    files.push(side);
                }
            }
        }
        let n = paths.len();
        match crate::win::recycle(&files) {
            Ok(()) => self.toast(t!("{} recording(s) moved to the Recycle Bin", "{} grabación(es) enviadas a la Papelera", n)),
            Err(e) => self.toast(t!("Couldn't delete: {:#}", "No pude eliminar: {:#}", e)),
        }
        self.selected.clear();
        self.scan_recordings();
    }

    pub fn delete_template(&mut self, name: &str) {
        let _ = crate::templates::delete(name);
        self.templates = crate::templates::list();
    }

    /// Export a template (with its images inside) where the user chooses.
    /// Mark a chapter in the running recording.
    pub fn marker(&mut self) {
        if let Some(e) = &self.engine {
            e.send(Cmd::Marker(None));
        }
    }

    pub fn save_replay(&mut self) {
        match &self.engine {
            Some(e) if self.status.replay.is_some() => e.send(Cmd::SaveReplay(None)),
            _ => self.toast(l("Turn on the replay buffer on the Live page", "Activa el replay buffer en la página En vivo")),
        }
    }

    /// Something is going out live (no capture restarts meanwhile).
    pub fn busy(&self) -> bool {
        self.status.recording || !self.status.streams.is_empty()
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn toggle_record(&mut self) {
        let Some(e) = &self.engine else { return };
        if self.status.recording {
            e.send(Cmd::Stop);
        } else if self.status.ready {
            match self.settings.new_output_file() {
                Ok(path) => e.send(Cmd::Record { path, duration: None }),
                Err(err) => self.toast(t!("Can't create the file: {:#}", "No puedo crear el archivo: {:#}", err)),
            }
        }
    }

    pub fn toggle_pause(&mut self) {
        if let Some(e) = &self.engine
            && self.status.recording
        {
            e.send(if self.status.paused { Cmd::Resume } else { Cmd::Pause });
        }
    }

    /// Save settings; restart the capture if needed (never mid-recording).
    pub fn settings_changed(&mut self) {
        let _ = self.settings.save();
        if let Some(e) = &self.engine {
            e.set_live(self.settings.live());
        }
        let cfg = self.settings.config();
        if cfg != self.config && !self.busy() {
            self.restart(cfg);
        }
    }

    pub fn restart(&mut self, cfg: Config) {
        if let Some(e) = self.engine.take() {
            e.quit();
        }
        self.config = cfg.clone();
        self.preview = None;
        self.status = Status::default();
        self.engine = Some(start_engine(&self.settings, cfg));
    }

    pub fn refresh_devices(&mut self) {
        let _ = crate::win::encoder::startup();
        self.devices = Devices {
            displays: crate::win::sources::displays()
                .iter()
                .map(|d| format!("{}  ·  {}x{}", d.name.trim_start_matches("\\\\.\\"), d.rect.right - d.rect.left, d.rect.bottom - d.rect.top))
                .collect(),
            windows: crate::win::sources::windows().into_iter().map(|w| (w.hwnd.0 as usize, w.title)).collect(),
            cameras: crate::win::camera::cameras().unwrap_or_default(),
            mics: crate::win::audio::microphones().map(|m| m.into_iter().map(|d| d.name).collect()).unwrap_or_default(),
            audio_apps: crate::win::apps::audio_apps().unwrap_or_default(),
        };
    }

    fn scan_recordings(&mut self) {
        let mut list: Vec<Recording> = std::fs::read_dir(&self.settings.output_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some(Recording {
                    path: e.path(),
                    name: e.path().file_stem()?.to_string_lossy().into_owned(),
                    bytes: m.len(),
                    modified: m.modified().ok()?,
                })
            })
            .collect();
        list.sort_by(|a, b| b.modified.cmp(&a.modified));
        self.recordings = list;
        self.last_scan = Some(Instant::now());
    }

    pub fn open_path(path: &std::path::Path) {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }

    pub fn reveal(path: &std::path::Path) {
        let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        // Engine status and preview.
        if let Some(e) = &self.engine {
            self.status = e.status();
            let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
            e.shared.preview_on.store(!self.hidden && !minimized && self.page == Page::Studio, std::sync::atomic::Ordering::Relaxed);
            let p = e.shared.preview.lock().unwrap();
            if p.seq != self.preview_seq && p.size.0 > 0 {
                self.preview_seq = p.seq;
                let img = egui::ColorImage::from_rgba_unmultiplied([p.size.0 as usize, p.size.1 as usize], &p.rgba);
                match &mut self.preview {
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => self.preview = Some(ctx.load_texture("preview", img, egui::TextureOptions::LINEAR)),
                }
            }
            drop(p);
            let p = e.shared.preview_v.lock().unwrap();
            if p.seq != self.preview_v_seq && p.size.0 > 0 {
                self.preview_v_seq = p.seq;
                let img = egui::ColorImage::from_rgba_unmultiplied([p.size.0 as usize, p.size.1 as usize], &p.rgba);
                match &mut self.preview_v {
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => self.preview_v = Some(ctx.load_texture("preview-v", img, egui::TextureOptions::LINEAR)),
                }
            }
        }
        if self.last_level.elapsed() >= Duration::from_millis(50) {
            self.last_level = Instant::now();
            let db = |p: f32| if p <= 1e-5 { -90.0 } else { 20.0 * p.log10() };
            self.levels.push_back((db(self.status.peak[0]), db(self.status.peak[1])));
            while self.levels.len() > HISTORY {
                self.levels.pop_front();
            }
        }
        // A recording just finished.
        if self.status.finished != self.last_finished {
            self.last_finished = self.status.finished;
            if let Some((p, s)) = self.status.last.clone() {
                let msg = t!("Saved: {}  ·  {:.1} MB", "Guardado: {}  ·  {:.1} MB", p.file_name().unwrap_or_default().to_string_lossy(), s.bytes as f64 / 1e6);
                self.toast(msg);
                if self.settings.subtitles_auto && !s.tracks.is_empty() {
                    self.queue(JobKind::Subtitles, p);
                }
            }
            self.scan_recordings();
            // Settings changed during the recording apply now.
            let cfg = self.settings.config();
            if cfg != self.config && !self.busy() {
                self.restart(cfg);
            }
        }
        // Engine messages (stream errors, replay saved…).
        if self.status.notices != self.last_notices {
            self.last_notices = self.status.notices;
            if let Some(n) = self.status.notice.clone() {
                self.toast(n);
            }
            self.scan_recordings();
        }
        if self.last_scan.is_none_or(|t| t.elapsed() > Duration::from_secs(5)) {
            self.scan_recordings();
        }
        // A replay was saved: make its vertical copy if asked.
        if self.status.last_replay != self.last_replay {
            self.last_replay = self.status.last_replay.clone();
            if let (Some(p), true) = (self.last_replay.clone(), std::mem::take(&mut self.replay_vertical)) {
                self.queue(JobKind::Vertical, p);
            }
        }
        self.pump_jobs();

        // Hotkeys and tray.
        // Shortcuts: none while a new one is being typed in Settings;
        // re-registered when they change.
        if self.capturing.is_some() {
            if self.hotkeys.applied.is_some() {
                self.hotkeys.clear();
            }
        } else if self.hotkeys.applied.as_ref() != Some(&self.settings.hotkeys) {
            self.hotkeys.apply(&self.settings.hotkeys);
        }
        while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
            if ev.state == HotKeyState::Pressed
                && self.capturing.is_none()
                && let Some(a) = self.hotkeys.action(ev.id)
            {
                self.run(a);
            }
        }
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::DoubleClick { .. } = ev {
                self.show_window(ctx);
            }
        }
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            let Some(t) = &self.tray else { continue };
            let id = ev.id;
            if id == *t.record.id() {
                self.toggle_record();
            } else if id == *t.pause.id() {
                self.toggle_pause();
            } else if id == *t.replay.id() {
                self.save_replay();
            } else if id == *t.marker.id() {
                self.marker();
            } else if id == *t.show.id() {
                self.show_window(ctx);
            } else if id == *t.folder.id() {
                let _ = std::fs::create_dir_all(&self.settings.output_dir);
                Self::open_path(&self.settings.output_dir);
            } else if id == *t.quit.id() {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if let Some(t) = &self.tray {
            let h = &self.settings.hotkeys;
            let with = |label: &str, combo: &str| if combo.is_empty() { label.to_string() } else { format!("{label}  ({combo})") };
            t.record.set_text(with(if self.status.recording { l("Stop", "Detener") } else { l("Record", "Grabar") }, &h.record));
            t.pause.set_enabled(self.status.recording);
            t.pause.set_text(with(if self.status.paused { l("Resume", "Reanudar") } else { l("Pause", "Pausar") }, &h.pause));
            let now = (self.settings.language, h.clone());
            if self.tray_for.as_ref() != Some(&now) {
                t.marker.set_text(with(l("Mark a chapter", "Marcar capítulo"), &h.marker));
                t.replay.set_text(with(l("Save replay", "Guardar replay"), &h.replay));
                t.show.set_text(l("Show Chisme", "Mostrar Chisme"));
                t.folder.set_text(l("Open the recordings folder", "Abrir carpeta de grabaciones"));
                t.quit.set_text(l("Quit", "Salir"));
                self.tray_for = Some(now);
            }
            t.replay.set_enabled(self.status.replay.is_some());
            t.marker.set_enabled(self.status.recording);
        }
    }
}

impl eframe::App for App {
    /// Runs even while the window is hidden, so the tray and hotkeys work.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        // Closing the window while recording keeps recording in the tray.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.status.recording && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
        }
        ctx.request_repaint_after(Duration::from_millis(if self.hidden { 100 } else { 33 }));
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Hidden from capture only while recording or streaming: on some PCs
        // the hidden window is drawn offset from where clicks land.
        let hide = !self.settings.show_self && self.busy() && std::env::var_os("CHISME_SHOW_IN_CAPTURE").is_none();
        if self.excluded != Some(hide) {
            self.excluded = Some(hide);
            exclude_from_capture(frame, hide);
        }
        ui::draw(self, ui);
        prompter::window(self, &ui.ctx().clone());
    }

    /// Transparent, so the teleprompter window can be see-through (the main
    /// window paints its own background).
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
}

/// Keep Chisme's own window out of every recording (and out of its preview,
/// which would otherwise show itself endlessly).
/// The "Show Chisme in recordings" switch (or CHISME_SHOW_IN_CAPTURE=1)
/// turns this off, to record a tutorial of the app itself.
fn exclude_from_capture(frame: &eframe::Frame, hide: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};
    if let Ok(h) = frame.window_handle()
        && let RawWindowHandle::Win32(w) = h.as_raw()
    {
        unsafe {
            let _ = SetWindowDisplayAffinity(windows::Win32::Foundation::HWND(w.hwnd.get() as _), if hide { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE });
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Finishes any recording so the file plays.
        if let Some(e) = self.engine.take() {
            e.quit();
        }
    }
}

pub fn run() -> Result<()> {
    let icon = egui::IconData { rgba: theme::icon_rgba(64), width: 64, height: 64 };
    // Direct3D 12 presenting through DirectComposition: the only Windows
    // swapchain with an alpha channel (plain HWND swapchains are opaque).
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(c) = &mut wgpu_options.wgpu_setup {
        c.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        c.instance_descriptor.backend_options.dx12.presentation_system = eframe::wgpu::Dx12SwapchainKind::DxgiFromVisual;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Chisme")
            .with_inner_size([1320.0, 880.0])
            .with_min_inner_size([720.0, 540.0])
            .with_icon(icon)
            // With a transparent back buffer, so the teleprompter window
            // can be see-through.
            .with_transparent(true),
        wgpu_options,
        ..Default::default()
    };
    eframe::run_native("Chisme", options, Box::new(|cc| Ok(Box::new(App::new(cc))))).map_err(|e| anyhow!("{e}"))
}
