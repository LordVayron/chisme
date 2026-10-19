//! `chisme`: the command line, and the app when run without arguments.

use chisme::{app, compose, engine, live, settings, win};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use live::{Platform, StreamSpec};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};

use compose::{Corner, PipSpec, Place, Scene, Shape};
use engine::{AudioLayout, Cmd, Config, Engine, LiveState, TargetSpec};
use settings::Settings;
use win::control::{self, Command};
use win::encoder::Codec;

#[derive(Parser)]
#[command(name = "chisme", version, about = "Chisme: fast screen recorder for Windows. With no command, opens the app.")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd_>,
}

#[derive(Subcommand)]
enum Cmd_ {
    /// Open the app.
    Gui,
    /// Record the screen or a window to an MP4.
    Record(Box<RecordArgs>),
    /// List displays, cameras, microphones and windows.
    Devices,
    /// Stop the running recording.
    Stop,
    /// Pause the running recording.
    Pause,
    /// Resume a paused recording.
    Resume,
    /// Save the running replay buffer (`record --replay`) to an MP4.
    SaveReplay,
    /// Mark a chapter in the running recording (saved as <video>.capitulos.txt).
    Marker,
    /// Export a recording as a vertical 1080x1920 clip (TikTok, Reels, Shorts).
    Vertical { file: PathBuf },
    /// Write subtitles (.srt next to the file) with Whisper, on this PC.
    Subtitles {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = chisme::subtitles::Model::Small)]
        model: chisme::subtitles::Model,
        /// Spoken language: auto, es, en, pt, fr…
        #[arg(long, default_value = "auto")]
        lang: String,
    },
    /// Store a platform's stream key in Windows Credential Manager (read
    /// from standard input), or delete it.
    StreamKey {
        #[arg(value_enum)]
        platform: Platform,
        #[arg(long)]
        delete: bool,
    },
}

#[derive(clap::Args)]
struct RecordArgs {
    /// Output file (default: Videos\Chisme\Chisme <date>.mp4).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Display number from `chisme devices` (1 = primary).
    #[arg(short, long, default_value_t = 1)]
    display: usize,
    /// Record a window instead: part of its title, or its handle.
    #[arg(long)]
    window: Option<String>,
    /// 720p, 1080p, 1440p, 4k, native or WxH. Keeps the source's shape and never upscales.
    #[arg(short, long, default_value = "1080p")]
    resolution: String,
    #[arg(short, long, default_value_t = 60)]
    fps: u32,
    #[arg(long, value_enum, default_value_t = Codec::H264)]
    codec: Codec,
    /// Video bitrate in Mbit/s (default: picked from size and frame rate).
    #[arg(long)]
    bitrate: Option<f64>,
    /// Hide the mouse pointer.
    #[arg(long)]
    no_cursor: bool,
    /// Stop after this many seconds of video.
    #[arg(short = 't', long)]
    duration: Option<f64>,
    /// No status output.
    #[arg(short, long)]
    quiet: bool,

    /// Do not record system audio (what you hear).
    #[arg(long)]
    no_system_audio: bool,
    /// Record only this app's audio (part of its .exe name, e.g. discord).
    #[arg(long, value_name = "APP", conflicts_with = "exclude_app_audio")]
    app_audio: Option<String>,
    /// Record all system audio except this app's (e.g. spotify).
    #[arg(long, value_name = "APP")]
    exclude_app_audio: Option<String>,
    /// Microphone: `default` or part of its name (default: the system default mic).
    #[arg(long, value_name = "DEVICE")]
    mic: Option<String>,
    /// Do not record the microphone.
    #[arg(long, conflicts_with = "mic")]
    no_mic: bool,
    /// Keep the microphone's background noise (no RNNoise).
    #[arg(long)]
    no_denoise: bool,
    /// Audio tracks when recording both system audio and mic.
    #[arg(long, value_enum, default_value_t = AudioLayout::Both)]
    audio_tracks: AudioLayout,
    /// System audio gain in the mix track (dB).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    system_gain: f32,
    /// Microphone gain in the mix track (dB).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    mic_gain: f32,

    /// Add the webcam (optionally part of its name).
    #[arg(short = 'w', long, visible_alias = "cam", value_name = "DEVICE", num_args = 0..=1, default_missing_value = "")]
    webcam: Option<String>,
    /// screen, screen-cam (picture-in-picture) or cam (webcam full frame).
    /// Default: screen-cam with a webcam.
    #[arg(long, value_enum)]
    scene: Option<Scene>,
    /// Webcam overlay width as a fraction of the frame width.
    #[arg(long, default_value_t = 0.22)]
    pip_size: f64,
    /// Snap the webcam overlay to a corner.
    #[arg(long, value_enum, default_value_t = Corner::BottomRight)]
    pip_corner: Corner,
    /// Place the overlay's center anywhere: X,Y as fractions of the frame
    /// from the top-left (e.g. 0.5,0.5). Overrides --pip-corner.
    #[arg(long, value_name = "X,Y")]
    pip_pos: Option<String>,
    #[arg(long, value_enum, default_value_t = Shape::Rounded)]
    pip_shape: Shape,
    /// Corner roundness 0..1 (rounded, square and squircle shapes).
    #[arg(long, default_value_t = 0.2)]
    pip_roundness: f64,
    /// Overlay border width in pixels at 1080p (0 = none).
    #[arg(long, default_value_t = 0.0)]
    pip_border: f64,
    /// Border color as hex RGB.
    #[arg(long, default_value = "ffffff", value_name = "RRGGBB")]
    pip_border_color: String,
    /// Soft drop shadow under the overlay.
    #[arg(long)]
    pip_shadow: bool,
    /// Do not mirror the webcam.
    #[arg(long)]
    no_mirror: bool,
    /// A custom layout (JSON, as saved in settings) instead of --scene.
    #[arg(long, value_name = "FILE")]
    layout: Option<PathBuf>,
    /// Use a saved template (its name, or part of it).
    #[arg(long, value_name = "NAME", conflicts_with = "layout")]
    template: Option<String>,
    /// Blur the background behind you on camera (0..1).
    #[arg(long, value_name = "AMOUNT", num_args = 0..=1, default_missing_value = "0.6")]
    bg_blur: Option<f32>,
    /// Replace the background behind you on camera with a picture.
    #[arg(long, value_name = "FILE", conflicts_with = "bg_blur")]
    bg_image: Option<PathBuf>,

    /// Go live: youtube, twitch, tiktok=URL, x=URL or an rtmp(s):// URL
    /// (repeatable). Keys come from $CHISME_STREAM_KEY or `chisme stream-key`.
    #[arg(long, value_name = "DEST")]
    stream: Vec<String>,
    /// Crop streams to vertical 9:16 (TikTok always is).
    #[arg(long)]
    vertical: bool,
    /// Keep the last SECS seconds (15-300) in memory; `chisme save-replay` saves them.
    #[arg(long, value_name = "SECS")]
    replay: Option<f64>,
    /// Do not write a recording (stream / replay buffer only).
    #[arg(long)]
    no_record: bool,
    /// Record a flash + beep every second instead of the screen and system
    /// audio, to check A/V sync with scripts/synccheck.py.
    #[arg(long)]
    test_pattern: bool,
}

/// `youtube`, `twitch`, `tiktok=URL`, `x=URL`, `custom=URL` or a bare URL.
fn parse_dest(d: &str) -> Result<(Platform, String)> {
    let (name, url) = d.split_once('=').map_or((d, ""), |(n, u)| (n, u));
    let platform = match name.trim().to_lowercase().as_str() {
        "youtube" => Platform::Youtube,
        "twitch" => Platform::Twitch,
        "tiktok" => Platform::Tiktok,
        "x" | "twitter" => Platform::X,
        "custom" => Platform::Custom,
        _ if d.starts_with("rtmp://") || d.starts_with("rtmps://") => return Ok((Platform::Custom, d.to_string())),
        _ => bail!("unknown stream destination {d:?}: use youtube, twitch, tiktok=URL, x=URL or rtmp(s)://…"),
    };
    if platform.default_url().is_none() && url.is_empty() {
        bail!("{} needs its server URL: --stream {}=rtmps://…", platform.label(), platform.id());
    }
    Ok((platform, url.to_string()))
}

fn clock(secs: f64) -> String {
    let s = secs as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

fn db(peak: f32) -> String {
    if peak <= 1e-5 { "  -∞".into() } else { format!("{:4.0}", 20.0 * peak.log10()) }
}

fn record(a: RecordArgs) -> Result<()> {
    let target = match &a.window {
        Some(q) => {
            let Some(w) = win::sources::find_window(q) else { bail!("no window matches {q:?}; see `chisme devices`") };
            TargetSpec::Window(w.hwnd.0 as usize, w.title)
        }
        None => TargetSpec::Display(a.display),
    };
    if !(1..=240).contains(&a.fps) {
        bail!("--fps must be 1-240");
    }
    let output = match a.output {
        Some(o) => o,
        None => Settings::default().new_output_file()?,
    };
    let pip = PipSpec {
        size: a.pip_size.clamp(0.05, 1.0),
        place: match &a.pip_pos {
            Some(p) => Place::At(compose::parse_pos(p)?),
            None => Place::Corner(a.pip_corner),
        },
        shape: a.pip_shape,
        roundness: a.pip_roundness.clamp(0.0, 1.0),
        border: a.pip_border.max(0.0),
        border_color: compose::parse_hex(&a.pip_border_color)?,
        shadow: a.pip_shadow,
        mirror: !a.no_mirror,
    };
    // A camera scene turns the webcam on.
    let cam = a.webcam.or_else(|| a.scene.filter(|s| s.uses_cam()).map(|_| String::new()));
    let streams = a.stream.iter().map(|d| parse_dest(d)).collect::<Result<Vec<_>>>()?;
    let cfg = Config {
        target,
        resolution: a.resolution,
        fps: a.fps,
        codec: a.codec,
        bitrate: a.bitrate,
        cursor: !a.no_cursor,
        system_audio: !a.no_system_audio,
        mic_denoise: !a.no_denoise,
        app_audio: a.app_audio.clone().map(|n| (n, true)).or_else(|| a.exclude_app_audio.clone().map(|n| (n, false))),
        mic: if a.no_mic { None } else { Some(a.mic.unwrap_or_default()) },
        audio_tracks: a.audio_tracks,
        cam: cam.clone(),
        test_pattern: a.test_pattern,
    };
    let mut brand = Settings::load().brand;
    let layout = match (&a.layout, &a.template) {
        (Some(f), _) => Some(serde_json::from_str::<chisme::layout::Layout>(std::fs::read_to_string(f)?.trim_start_matches('\u{feff}'))?),
        (None, Some(name)) => {
            let Some(t) = chisme::templates::find(name) else { bail!("no template matches {name:?}") };
            if let Some(b) = t.brand {
                brand = b;
            }
            Some(t.layout)
        }
        (None, None) => None,
    };
    let mut vbg = chisme::vbg::VirtualBackground::default();
    if let Some(amount) = a.bg_blur {
        vbg.mode = chisme::vbg::Mode::Blur;
        vbg.blur = amount;
    }
    if let Some(p) = &a.bg_image {
        vbg.pick(p.to_string_lossy().into_owned());
    }
    let live = LiveState { scene: a.scene.unwrap_or_default(), pip, gain_db: [a.system_gain, a.mic_gain], muted: [false; 2], layout, brand, vbg };
    let engine = Engine::start(cfg, live);
    engine.shared.preview_on.store(false, std::sync::atomic::Ordering::Relaxed);

    // Wait for the capture to come up.
    let st = loop {
        let st = engine.status();
        if let Some(e) = st.error {
            bail!(e);
        }
        if st.ready {
            break st;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    for w in &st.warnings {
        eprintln!("warning: {w}");
    }
    if cam.is_some() && st.cam.is_none() {
        bail!("the camera didn't start");
    }
    for (platform, url) in &streams {
        let key = std::env::var("CHISME_STREAM_KEY").ok().filter(|k| !k.is_empty()).or_else(|| win::creds::get(platform.id()));
        let Some(key) = key else { bail!("no stream key for {}: set CHISME_STREAM_KEY or run `chisme stream-key {}`", platform.label(), platform.id()) };
        let vertical = a.vertical || *platform == Platform::Tiktok;
        engine.send(Cmd::StartStream(StreamSpec { platform: *platform, url: url.clone(), key, vertical, kbps: None }));
    }
    if let Some(secs) = a.replay {
        engine.send(Cmd::Replay(Some(secs.clamp(5.0, 300.0))));
    }
    let record = !a.no_record;
    if record {
        engine.send(Cmd::Record { path: output.clone(), duration: a.duration });
    } else if streams.is_empty() && a.replay.is_none() {
        bail!("--no-record needs --stream or --replay");
    }
    let stop = engine.sender();
    let stopping = Arc::new(AtomicBool::new(false));
    let s2 = stopping.clone();
    let _ = ctrlc::set_handler(move || {
        s2.store(true, Ordering::SeqCst);
        let _ = stop.send(Cmd::Stop);
    });
    if !record {
        // Streaming / replay only: run until Ctrl+C, `chisme stop` or -t.
        let started = std::time::Instant::now();
        let mut notices = 0;
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let st = engine.status();
            if let Some(e) = st.error {
                bail!(e);
            }
            if st.notices != notices {
                notices = st.notices;
                if let Some(n) = &st.notice {
                    eprintln!("\n{n}");
                }
            }
            if !a.quiet {
                let mut line = String::new();
                for s in &st.streams {
                    line += &format!("  {} {} {:.1} Mb/s {}  ", s.name, s.state, s.bitrate / 1e6, clock(s.duration));
                }
                if let Some((held, bytes)) = st.replay {
                    line += &format!("  replay {held:.0} s ({:.0} MB)", bytes as f64 / 1e6);
                }
                eprint!("\r{line}   ");
                let _ = std::io::stderr().flush();
            }
            if stopping.load(Ordering::SeqCst) || st.finished > 0 || a.duration.is_some_and(|d| started.elapsed().as_secs_f64() >= d) {
                break;
            }
        }
        engine.quit();
        eprintln!();
        return Ok(());
    }
    if !a.quiet {
        eprintln!("Recording {} → {}", st.label, output.display());
        eprintln!("  video: {}x{} @ {} fps, {:?}, {:.1} Mbit/s", st.out_size.0, st.out_size.1, st.fps, a.codec, st.bitrate / 1e6);
        if let Some((name, size, fps)) = &st.cam {
            eprintln!("  camera: {name} ({}x{} @ {fps:.0} fps)", size.0, size.1);
        }
        if !st.audio.is_empty() {
            eprintln!("  audio: {}", st.audio.join(", "));
        }
        eprintln!("  Ctrl+C or `chisme stop` to finish, `chisme pause` / `chisme resume`.");
    }
    let (path, stats) = loop {
        std::thread::sleep(Duration::from_millis(250));
        let st = engine.status();
        if let Some(e) = st.error {
            bail!(e);
        }
        if st.finished > 0 {
            break st.last.unwrap();
        }
        if !a.quiet && st.recording {
            let state = if st.paused { "❚❚ PAUSED" } else { "● REC" };
            let mut levels = String::new();
            if st.has_system {
                levels += &format!("  sys {} dB", db(st.peak[0]));
            }
            if st.has_mic {
                levels += &format!("  mic {} dB", db(st.peak[1]));
            }
            let marks = if st.stats.markers > 0 { format!("  chapters {}", st.stats.markers) } else { String::new() };
            eprint!("\r  {state} {}  frames {}  repeated {}  dropped {}{levels}{marks}   ", clock(st.rec_secs), st.stats.frames_out, st.stats.dups, st.stats.drops);
            let _ = std::io::stderr().flush();
        }
    };
    // An error stops the recording first (so the file plays), then shows.
    for _ in 0..20 {
        if engine.status().error.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Some(e) = engine.status().error {
        eprintln!("
error: {e}");
    }
    engine.quit();
    if !a.quiet {
        eprintln!(
            "\nSaved {}\n  {} frames written ({} repeated, {} dropped, {} paused), {:.1} MB",
            path.display(),
            stats.frames_out,
            stats.dups,
            stats.drops,
            stats.paused_slots,
            stats.bytes as f64 / 1e6
        );
        if !stats.tracks.is_empty() {
            eprintln!("  audio: {:.1} s on {} track(s): {}", stats.audio_secs, stats.tracks.len(), stats.tracks.join(", "));
        }
        if stats.markers > 0 {
            eprintln!("  chapters: {}", chisme::chapters::path_for(&path).display());
        }
    }
    println!("{}", path.display());
    Ok(())
}

fn devices() {
    println!("Displays:");
    for (i, d) in win::sources::displays().iter().enumerate() {
        let r = d.rect;
        println!("  {}  {}  {}x{} at {},{}{}", i + 1, d.name, r.right - r.left, r.bottom - r.top, r.left, r.top, if d.primary { "  (primary)" } else { "" });
    }
    println!("Cameras:");
    let _ = win::encoder::startup();
    match win::camera::cameras() {
        Ok(c) if c.is_empty() => println!("  (none)"),
        Ok(c) => c.iter().for_each(|n| println!("  {n}")),
        Err(e) => println!("  error: {e:#}"),
    }
    println!("Microphones:");
    match win::audio::microphones() {
        Ok(m) if m.is_empty() => println!("  (none)"),
        Ok(m) => m.iter().for_each(|d| println!("  {}", d.name)),
        Err(e) => println!("  error: {e:#}"),
    }
    if let Ok(o) = win::audio::default_output() {
        println!("System audio from: {o}");
    }
    println!("Apps with audio (--app-audio / --exclude-app-audio):");
    match win::apps::audio_apps() {
        Ok(a) if a.is_empty() => println!("  (none)"),
        Ok(a) => a.iter().for_each(|n| println!("  {n}")),
        Err(e) => println!("  error: {e:#}"),
    }
    println!("Windows:");
    for w in win::sources::windows() {
        println!("  0x{:x}  {}", w.hwnd.0 as usize, w.title);
    }
}

fn main() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    match Cli::parse().cmd {
        None | Some(Cmd_::Gui) => {
            // Started from Explorer: don't keep a console window open.
            let mut pids = [0u32; 2];
            if unsafe { GetConsoleProcessList(&mut pids) } <= 1 {
                unsafe {
                    let _ = FreeConsole();
                }
            }
            app::run()
        }
        Some(Cmd_::Record(a)) => record(*a),
        Some(Cmd_::Devices) => {
            devices();
            Ok(())
        }
        Some(Cmd_::Stop) => control::send(Command::Stop),
        Some(Cmd_::Pause) => control::send(Command::Pause),
        Some(Cmd_::Resume) => control::send(Command::Resume),
        Some(Cmd_::SaveReplay) => control::send(Command::SaveReplay),
        Some(Cmd_::Marker) => control::send(Command::Marker),
        Some(Cmd_::Vertical { file }) => {
            win::encoder::startup()?;
            let started = std::time::Instant::now();
            let last = std::sync::atomic::AtomicI32::new(-1);
            let out = chisme::export::vertical(&file, |p| {
                let pct = (p * 100.0) as i32;
                if pct / 5 != last.swap(pct, std::sync::atomic::Ordering::Relaxed) / 5 {
                    eprint!("\r  Exporting vertical… {pct:>3}%   ");
                }
            })?;
            eprintln!("\nSaved {} in {:.1} s", out.display(), started.elapsed().as_secs_f64());
            println!("{}", out.display());
            Ok(())
        }
        Some(Cmd_::Subtitles { file, model, lang }) => {
            win::encoder::startup()?;
            let started = std::time::Instant::now();
            let last = std::sync::Mutex::new((String::new(), -1i32));
            let out = chisme::subtitles::subtitle_file(&file, model, Some(lang.as_str()).filter(|l| *l != "auto"), move |stage, p| {
                let pct = (p * 100.0) as i32;
                let mut l = last.lock().unwrap();
                // One line per stage, updated in place every 5%.
                if l.0 != stage {
                    eprint!("{}  {stage}…", if l.0.is_empty() { "" } else { "\n" });
                    *l = (stage.to_string(), -1);
                }
                if pct / 5 != l.1 / 5 {
                    eprint!("\r  {stage}… {pct:>3}%   ");
                    l.1 = pct;
                }
            })?;
            eprintln!("\nSaved {} in {:.0} s", out.display(), started.elapsed().as_secs_f64());
            println!("{}", out.display());
            Ok(())
        }
        Some(Cmd_::StreamKey { platform, delete }) => {
            if delete {
                win::creds::delete(platform.id())?;
                eprintln!("Deleted the {} stream key.", platform.label());
            } else {
                eprintln!("Paste the {} stream key and press Enter:", platform.label());
                let mut key = String::new();
                std::io::stdin().read_line(&mut key)?;
                let key = key.trim();
                if key.is_empty() {
                    bail!("no key given");
                }
                win::creds::set(platform.id(), key)?;
                eprintln!("Saved in Windows Credential Manager.");
            }
            Ok(())
        }
    }
}
