//! Automatic subtitles with Whisper (whisper.cpp, on this PC: no audio
//! leaves the machine). The recording's first audio track (the mix) is
//! decoded with Media Foundation, resampled to 16 kHz mono, transcribed, and
//! written as an .srt next to the video.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::HSTRING;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    /// Fastest, rougher (~60 MB).
    Base,
    /// Good balance for Spanish and English (~190 MB).
    Small,
    /// Better accuracy, slower (~540 MB).
    Medium,
    /// Best accuracy (~550 MB, slowest on CPU).
    Turbo,
}

impl Model {
    pub const ALL: [Model; 4] = [Model::Base, Model::Small, Model::Medium, Model::Turbo];

    pub fn file(self) -> &'static str {
        match self {
            Model::Base => "ggml-base-q5_1.bin",
            Model::Small => "ggml-small-q5_1.bin",
            Model::Medium => "ggml-medium-q5_0.bin",
            Model::Turbo => "ggml-large-v3-turbo-q5_0.bin",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Model::Base => crate::i18n::l("Base · fast · 60 MB", "Base · rápido · 60 MB"),
            Model::Small => crate::i18n::l("Small · balanced · 190 MB", "Small · equilibrado · 190 MB"),
            Model::Medium => crate::i18n::l("Medium · accurate · 540 MB", "Medium · preciso · 540 MB"),
            Model::Turbo => crate::i18n::l("Large v3 Turbo · the best · 550 MB", "Large v3 Turbo · el mejor · 550 MB"),
        }
    }

    /// SHA-256 of the file on Hugging Face, checked after every download.
    pub fn sha256(self) -> &'static str {
        match self {
            Model::Base => "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
            Model::Small => "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
            Model::Medium => "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
            Model::Turbo => "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        }
    }

    pub fn url(self) -> String {
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}", self.file())
    }

    pub fn path(self) -> PathBuf {
        models_dir().join(self.file())
    }

    pub fn downloaded(self) -> bool {
        self.path().is_file()
    }
}

pub fn models_dir() -> PathBuf {
    let base = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None).ok().and_then(|p| p.to_string().ok()) };
    PathBuf::from(base.unwrap_or_default()).join("Chisme").join("models")
}

/// Download the model if needed. `progress` gets 0..1.
pub fn ensure_model(model: Model, progress: &dyn Fn(f32)) -> Result<PathBuf> {
    let path = model.path();
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(models_dir())?;
    let part = path.with_extension("part");
    let resp = ureq::get(&model.url()).call().with_context(|| format!("downloading {}", model.file()))?;
    let total: Option<u64> = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    let mut reader = resp.into_body().into_reader();
    let mut file = std::io::BufWriter::new(std::fs::File::create(&part)?);
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    let mut hash = <sha2::Sha256 as sha2::Digest>::new();
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hash, &buf[..n]);
        file.write_all(&buf[..n])?;
        done += n as u64;
        if let Some(t) = total {
            progress(done as f32 / t as f32);
        }
    }
    file.flush()?;
    drop(file);
    if total.is_some_and(|t| t != done) || done < 1_000_000 {
        let _ = std::fs::remove_file(&part);
        bail!("the model download was incomplete");
    }
    // Only the exact published file is ever loaded.
    let got: String = sha2::Digest::finalize(hash).iter().map(|b| format!("{b:02x}")).collect();
    if got != model.sha256() {
        let _ = std::fs::remove_file(&part);
        bail!("the downloaded model doesn't match the published one (SHA-256 {got})");
    }
    std::fs::rename(&part, &path)?;
    Ok(path)
}

/// The first audio track of a media file as mono f32 at its own rate.
pub fn decode_audio(path: &Path) -> Result<(Vec<f32>, u32)> {
    unsafe {
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None).with_context(|| format!("can't open {}", path.display()))?;
        const AUDIO: u32 = 0xFFFF_FFFD; // MF_SOURCE_READER_FIRST_AUDIO_STREAM
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
        reader.SetCurrentMediaType(AUDIO, None, &t).context("this file has no audio track")?;
        let cur = reader.GetCurrentMediaType(AUDIO)?;
        let rate = cur.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?;
        let ch = cur.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?.max(1) as usize;
        let mut mono = Vec::new();
        loop {
            let (mut flags, mut sample) = (0u32, None);
            reader.ReadSample(AUDIO, 0, None, Some(&mut flags), None, Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(s) = sample else { continue };
            let buf = s.ConvertToContiguousBuffer()?;
            let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
            buf.Lock(&mut p, None, Some(&mut len))?;
            let v = std::slice::from_raw_parts(p as *const f32, len as usize / 4);
            mono.extend(v.chunks_exact(ch).map(|f| f.iter().sum::<f32>() / ch as f32));
            buf.Unlock()?;
        }
        Ok((mono, rate))
    }
}

/// Resample to 16 kHz (what Whisper takes): a windowed-sinc low-pass below
/// 8 kHz, then linear interpolation at the new rate.
pub fn to_16k(input: &[f32], rate: u32) -> Vec<f32> {
    if rate == 16000 {
        return input.to_vec();
    }
    let fc = 7200.0 / rate as f64; // cycles per input sample
    let half = 32i64;
    let taps: Vec<f32> = (-half..=half)
        .map(|k| {
            let x = k as f64;
            let sinc = if k == 0 { 2.0 * fc } else { (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x) };
            let w = 0.42 + 0.5 * (std::f64::consts::PI * x / half as f64).cos() + 0.08 * (2.0 * std::f64::consts::PI * x / half as f64).cos(); // Blackman
            (sinc * w) as f32
        })
        .collect();
    let gain: f32 = taps.iter().sum();
    let step = rate as f64 / 16000.0;
    let n = (input.len() as f64 / step) as usize;
    let filtered = |i: i64| -> f32 {
        let mut acc = 0.0;
        for (j, t) in taps.iter().enumerate() {
            let k = i + j as i64 - half;
            if k >= 0 && (k as usize) < input.len() {
                acc += input[k as usize] * t;
            }
        }
        acc / gain
    };
    (0..n)
        .map(|o| {
            let pos = o as f64 * step;
            let i = pos.floor() as i64;
            let f = (pos - i as f64) as f32;
            if f == 0.0 { filtered(i) } else { filtered(i) * (1.0 - f) + filtered(i + 1) * f }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Transcribe 16 kHz mono audio. `language`: None = detect. `progress`: 0..1.
pub fn transcribe(audio: &[f32], model_path: &Path, language: Option<&str>, progress: impl FnMut(i32) + 'static) -> Result<Vec<Segment>> {
    let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default()).context("loading the Whisper model")?;
    let mut state = ctx.create_state()?;
    let mut p = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 });
    // One thread per physical core: with every SMT thread busy, ggml's
    // threads wait on each other and it gets ~10x slower.
    let threads = std::env::var("CHISME_WHISPER_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or((std::thread::available_parallelism().map_or(4, |n| n.get()) / 2).clamp(1, 8) as i32);
    p.set_n_threads(threads);
    p.set_language(Some(language.unwrap_or("auto")));
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_special(false);
    p.set_print_timestamps(false);
    p.set_suppress_blank(true);
    // Short subtitle lines, split between words.
    p.set_token_timestamps(true);
    p.set_max_len(48);
    p.set_split_on_word(true);
    p.set_progress_callback_safe(progress);
    state.full(p, audio).context("transcribing")?;
    let mut out = vec![];
    for seg in state.as_iter() {
        let text = seg.to_str_lossy()?.trim().to_string();
        if text.is_empty() || (text.starts_with('[') && text.ends_with(']')) {
            continue; // [Música], [BLANK_AUDIO]…
        }
        out.push(Segment { start: seg.start_timestamp() as f64 / 100.0, end: seg.end_timestamp() as f64 / 100.0, text });
    }
    Ok(out)
}

fn srt_time(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

pub fn to_srt(segments: &[Segment]) -> String {
    let mut s = String::new();
    for (i, seg) in segments.iter().enumerate() {
        s += &format!("{}\n{} --> {}\n{}\n\n", i + 1, srt_time(seg.start), srt_time(seg.end.max(seg.start + 0.3)), seg.text);
    }
    s
}

/// `video.mp4` → `video.srt`.
pub fn srt_path(video: &Path) -> PathBuf {
    video.with_extension("srt")
}

/// The whole job: model, audio, transcription, .srt. `stage` reports what
/// is happening and its progress (0..1).
pub fn subtitle_file(video: &Path, model: Model, language: Option<&str>, stage: impl Fn(&str, f32) + Send + Sync + 'static) -> Result<PathBuf> {
    let stage = std::sync::Arc::new(stage);
    let s2 = stage.clone();
    let model_path = ensure_model(model, &move |p| s2(crate::i18n::l("Downloading the model", "Descargando el modelo"), p))?;
    stage(crate::i18n::l("Reading the audio", "Leyendo el audio"), 0.0);
    let (audio, rate) = decode_audio(video)?;
    if audio.is_empty() {
        bail!("the recording has no audio");
    }
    let audio = to_16k(&audio, rate);
    let s3 = stage.clone();
    stage("Transcribiendo", 0.0);
    let segments = transcribe(&audio, &model_path, language, move |p| s3("Transcribiendo", p as f32 / 100.0))?;
    let out = srt_path(video);
    // UTF-8 with BOM: some Windows players need it for accents.
    std::fs::write(&out, format!("\u{feff}{}", to_srt(&segments)))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_format() {
        let segs = vec![
            Segment { start: 0.0, end: 1.5, text: "Hola, ¿qué tal?".into() },
            Segment { start: 3661.25, end: 3661.3, text: "Adiós".into() },
        ];
        assert_eq!(to_srt(&segs), "1\n00:00:00,000 --> 00:00:01,500\nHola, ¿qué tal?\n\n2\n01:01:01,250 --> 01:01:01,550\nAdiós\n\n");
        assert_eq!(srt_path(Path::new("C:/v/Chisme 1.mp4")), PathBuf::from("C:/v/Chisme 1.srt"));
    }

    #[test]
    fn resampling_keeps_speech_and_drops_ultrasound() {
        let tone = |f: f64, rate: u32| (0..rate).map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / rate as f64).sin() as f32 * 0.5).collect::<Vec<_>>();
        let rms = |v: &[f32]| (v[200..v.len() - 200].iter().map(|x| x * x).sum::<f32>() / (v.len() - 400) as f32).sqrt();
        for rate in [48000, 44100] {
            let out = to_16k(&tone(1000.0, rate), rate);
            assert!((out.len() as i64 - 16000).abs() <= 1, "{rate}: {}", out.len());
            assert!((rms(&out) - 0.3536).abs() < 0.01, "1 kHz passes at {rate}: {}", rms(&out));
            // 12 kHz would alias to 4 kHz; it must be filtered out.
            assert!(rms(&to_16k(&tone(12000.0, rate), rate)) < 0.01);
        }
    }

    /// CHISME_PROBE=file.mp4 cargo test --release --lib probe_audio -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_audio() {
        crate::win::encoder::startup().unwrap();
        let f = std::env::var("CHISME_PROBE").unwrap();
        let (a, rate) = decode_audio(Path::new(&f)).unwrap();
        let peak = a.iter().fold(0f32, |m, x| m.max(x.abs()));
        let nans = a.iter().filter(|x| !x.is_finite()).count();
        let r = to_16k(&a, rate);
        println!("rate {rate}, {} samples ({:.2} s), peak {peak}, non-finite {nans}; 16k: {} samples ({:.2} s)", a.len(), a.len() as f64 / rate as f64, r.len(), r.len() as f64 / 16000.0);
    }

    #[test]
    fn models() {
        assert!(Model::Small.url().ends_with("/ggml-small-q5_1.bin"));
        assert!(Model::Turbo.path().ends_with("Chisme\\models\\ggml-large-v3-turbo-q5_0.bin"));
    }
}
