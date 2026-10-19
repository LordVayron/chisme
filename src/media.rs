//! Media Board: sounds, video clips and images fired with a click or a
//! hotkey while recording or streaming.
//!
//! - A **sound** or a **clip's audio** is decoded by Media Foundation to
//!   48 kHz stereo and goes into the mix (the recording and the streams) on
//!   the same host clock as everything else. It also plays on your speakers
//!   (`Monitor`), timed to match.
//! - A **clip's picture** covers the scene while it plays.
//! - An **image** stays over the scene until fired again (a "be right back"
//!   card, a logo, a meme).

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Com::StructuredStorage::PropVariantToUInt64;
use windows::core::HSTRING;

use crate::mixer::RATE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Sound,
    Video,
    Image,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Sound => crate::i18n::l("Sound", "Sonido"),
            Kind::Video => "Video",
            Kind::Image => crate::i18n::l("Image", "Imagen"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub name: String,
    pub path: String,
    pub kind: Kind,
    /// Linear gain, 0–2.
    pub volume: f32,
    /// The group it is shown in ("" = ungrouped).
    pub group: String,
}

impl Default for Item {
    fn default() -> Self {
        Item { name: String::new(), path: String::new(), kind: Kind::Sound, volume: 1.0, group: String::new() }
    }
}

/// Sound, clip or image, by extension (what Media Foundation and the image
/// decoder read on a stock Windows 10).
pub fn kind_of(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    Some(match ext.as_str() {
        "mp3" | "wav" | "m4a" | "aac" | "wma" | "flac" => Kind::Sound,
        "mp4" | "mov" | "m4v" | "wmv" | "avi" => Kind::Video,
        "png" | "jpg" | "jpeg" => Kind::Image,
        _ => return None,
    })
}

pub fn item(path: &Path) -> Option<Item> {
    Some(Item { name: path.file_stem()?.to_string_lossy().into_owned(), path: path.to_string_lossy().into_owned(), kind: kind_of(path)?, volume: 1.0, group: String::new() })
}

pub const EXTENSIONS: [&str; 14] = ["mp3", "wav", "m4a", "aac", "wma", "flac", "mp4", "mov", "m4v", "wmv", "avi", "png", "jpg", "jpeg"];

/// What the engine asks the decoder for.
pub enum Chunk {
    /// The file's length in seconds (sent first, when known).
    Duration(f64),
    /// Seconds from the start, 48 kHz stereo.
    Audio(f64, Vec<[f32; 2]>),
    /// Seconds from the start, size, BGRX rows top-down.
    Video(f64, (u32, u32), Vec<u8>),
    Failed(String),
}

const ANY_STREAM: u32 = 0xFFFF_FFFE;
const ALL_STREAMS: u32 = 0xFFFF_FFFE;
const FIRST_VIDEO: u32 = 0xFFFF_FFFC;
const FIRST_AUDIO: u32 = 0xFFFF_FFFD;
const MEDIASOURCE: u32 = 0xFFFF_FFFF;

/// Decodes a file a little ahead of playback (bounded channels, so it
/// waits for the engine). Audio and picture have a reader each: a file
/// interleaved in one-second runs would otherwise hold the sound back
/// behind a second of frames.
pub struct Decoder {
    /// Audio first, then the picture (for a clip).
    pub streams: Vec<Receiver<Chunk>>,
    stop: Arc<AtomicBool>,
}

impl Decoder {
    /// `video_max`: decode the picture too, scaled to fit this size.
    pub fn open(path: &str, video_max: Option<(u32, u32)>) -> Decoder {
        let stop = Arc::new(AtomicBool::new(false));
        let mut streams = vec![];
        // A clip without sound is fine; a sound file without one isn't.
        let parts = [(true, None, 48, video_max.is_some()), (false, video_max, 6, false)];
        for (audio, video, depth, quiet) in parts {
            if !audio && video.is_none() {
                continue;
            }
            let (tx, rx) = sync_channel(depth);
            let (path, stop2) = (path.to_string(), stop.clone());
            let _ = std::thread::Builder::new().name("media-decode".into()).spawn(move || {
                crate::win::audio::com();
                if crate::win::encoder::startup().is_err() {
                    let _ = tx.send(Chunk::Failed(crate::i18n::l("Media Foundation isn't available", "Media Foundation no está disponible").into()));
                    return;
                }
                match decode(&path, audio, video, &tx, &stop2) {
                    Err(e) if !(quiet && e.to_string() == NOTHING) => {
                        let _ = tx.send(Chunk::Failed(format!("{e:#}")));
                    }
                    _ => {}
                }
                crate::win::encoder::shutdown();
            });
            streams.push(rx);
        }
        Decoder { streams, stop }
    }
}

const NOTHING: &str = "no playable audio or video";

impl Drop for Decoder {
    fn drop(&mut self) {
        // The thread ends at its next send (the channel closes with us).
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

fn decode(path: &str, audio: bool, video_max: Option<(u32, u32)>, tx: &SyncSender<Chunk>, stop: &AtomicBool) -> Result<()> {
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs: IMFAttributes = attrs.context("no attributes")?;
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path), &attrs).with_context(|| crate::t!("couldn't open {}", "no se pudo abrir {}", path))?;
        let _ = reader.SetStreamSelection(ALL_STREAMS, false);
        let has_audio = audio && {
            let at = MFCreateMediaType()?;
            at.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            at.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
            at.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000)?;
            at.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
            reader.SetCurrentMediaType(FIRST_AUDIO, None, &at).is_ok() && reader.SetStreamSelection(FIRST_AUDIO, true).is_ok()
        };
        // The picture, scaled down by the reader to fit the canvas.
        let mut size = (0, 0);
        let has_video = match video_max {
            Some(max) => (|| -> Result<()> {
                let vt = MFCreateMediaType()?;
                vt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                vt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                reader.SetCurrentMediaType(FIRST_VIDEO, None, &vt)?;
                let native = unpack(reader.GetCurrentMediaType(FIRST_VIDEO)?.GetUINT64(&MF_MT_FRAME_SIZE)?);
                let (_, _, w, h) = crate::size::fit(native, max);
                let (w, h) = (w.min(native.0).max(2) & !1, h.min(native.1).max(2) & !1);
                if (w, h) != native {
                    vt.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
                    let _ = reader.SetCurrentMediaType(FIRST_VIDEO, None, &vt);
                }
                size = unpack(reader.GetCurrentMediaType(FIRST_VIDEO)?.GetUINT64(&MF_MT_FRAME_SIZE)?);
                reader.SetStreamSelection(FIRST_VIDEO, true)?;
                Ok(())
            })()
            .is_ok(),
            None => false,
        };
        if !has_audio && !has_video {
            return Err(anyhow!(NOTHING));
        }
        if let Some(d) = reader.GetPresentationAttribute(MEDIASOURCE, &MF_PD_DURATION).ok().and_then(|v| PropVariantToUInt64(&v).ok()) {
            tx.send(Chunk::Duration(d as f64 * 1e-7))?;
        }
        let video_index = if has_video {
            (0..16u32)
                .find(|&i| reader.GetStreamSelection(i).unwrap_or_default().as_bool() && reader.GetCurrentMediaType(i).ok().and_then(|t| t.GetGUID(&MF_MT_MAJOR_TYPE).ok()) == Some(MFMediaType_Video))
        } else {
            None
        };
        let mut streams = has_audio as u32 + has_video as u32;
        let row = size.0 as usize * 4;
        // Each stream starts at 0: Media Foundation ignores the edit list
        // that shifts B-frame video (most phone and editor exports) back,
        // which would put the picture a few frames behind the sound.
        let mut first: Option<i64> = None;
        while streams > 0 && !stop.load(Ordering::Relaxed) {
            let (mut index, mut flags, mut ts, mut sample) = (0u32, 0u32, 0i64, None);
            reader.ReadSample(ANY_STREAM, 0, Some(&mut index), Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                return Err(anyhow!(crate::i18n::l("decoding failed", "falló la decodificación")));
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                streams -= 1;
            }
            let Some(s) = sample else { continue };
            let secs = ((ts - *first.get_or_insert(ts)) as f64 * 1e-7).max(0.0);
            let buf = s.ConvertToContiguousBuffer()?;
            let chunk = if Some(index) == video_index {
                let mut px = vec![0u8; row * size.1 as usize];
                crate::win::copy_rows(&buf, &mut px, row, size.1 as usize)?;
                Chunk::Video(secs, size, px)
            } else {
                let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                buf.Lock(&mut p, None, Some(&mut len))?;
                let v = std::slice::from_raw_parts(p as *const f32, len as usize / 4);
                let frames: Vec<[f32; 2]> = v.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
                buf.Unlock()?;
                Chunk::Audio(secs, frames)
            };
            if tx.send(chunk).is_err() {
                break;
            }
        }
    }
    Ok(())
}

/// How far the monitor's timeline may wander before it is corrected.
const SLACK: f64 = 0.03;

/// Plays media audio on the default speakers, each buffer at its host time
/// (so you hear it when it lands in the recording), mixing overlaps.
pub struct Monitor {
    tx: Sender<(f64, Vec<[f32; 2]>)>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Monitor {
    pub fn start() -> Result<Monitor> {
        let (tx, rx) = channel::<(f64, Vec<[f32; 2]>)>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let (ready_tx, ready_rx) = sync_channel::<Result<()>>(1);
        let thread = std::thread::Builder::new().name("media-monitor".into()).spawn(move || {
            crate::win::audio::com();
            let opened = (|| -> Result<(IAudioClient, IAudioRenderClient, u32)> {
                unsafe {
                    let dev = crate::win::audio::enumerator()?.GetDefaultAudioEndpoint(eRender, eConsole).context(crate::i18n::l("no speakers", "no hay bocinas"))?;
                    let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
                    let wfx = WAVEFORMATEX {
                        wFormatTag: 3, // IEEE float
                        nChannels: 2,
                        nSamplesPerSec: 48000,
                        nAvgBytesPerSec: 48000 * 8,
                        nBlockAlign: 8,
                        wBitsPerSample: 32,
                        cbSize: 0,
                    };
                    let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
                    client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 1_000_000, 0, &wfx, None)?;
                    let render: IAudioRenderClient = client.GetService()?;
                    let size = client.GetBufferSize()?;
                    client.Start()?;
                    Ok((client, render, size))
                }
            })();
            let (client, render, size) = match opened {
                Ok(o) => {
                    let _ = ready_tx.send(Ok(()));
                    o
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            // Samples waiting to play; `base` is the host time of q[0].
            let mut q: VecDeque<[f32; 2]> = VecDeque::new();
            let mut base = 0.0f64;
            while !stop2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
                for (pts, frames) in rx.try_iter() {
                    if q.is_empty() {
                        base = pts;
                    }
                    let mut at = ((pts - base) * RATE).round() as i64;
                    if at < 0 {
                        // Earlier than the queue start: grow it backwards.
                        for _ in 0..(-at) {
                            q.push_front([0.0; 2]);
                        }
                        base = pts;
                        at = 0;
                    }
                    let end = at as usize + frames.len();
                    if q.len() < end {
                        q.resize(end, [0.0; 2]);
                    }
                    for (i, f) in frames.iter().enumerate() {
                        let s = &mut q[at as usize + i];
                        s[0] += f[0];
                        s[1] += f[1];
                    }
                }
                if q.is_empty() {
                    continue;
                }
                unsafe {
                    let padding = client.GetCurrentPadding().unwrap_or(size);
                    let room = size.saturating_sub(padding) as usize;
                    // When the next written sample will be heard.
                    let play = crate::win::now() + padding as f64 / RATE;
                    // The device clock moves in ~10 ms steps: within 30 ms
                    // the queue just continues; beyond, catch up or wait.
                    let drift = base - play;
                    if drift < -SLACK {
                        let n = ((-drift * RATE).round() as usize).min(q.len());
                        q.drain(..n);
                        base += n as f64 / RATE;
                    }
                    if q.is_empty() || room == 0 {
                        continue;
                    }
                    let lead = if drift > SLACK { (drift * RATE).round() as usize } else { 0 };
                    let n = room.min(lead + q.len());
                    let Ok(p) = render.GetBuffer(n as u32) else { continue };
                    let out = std::slice::from_raw_parts_mut(p as *mut [f32; 2], n);
                    for (i, o) in out.iter_mut().enumerate() {
                        *o = if i < lead { [0.0; 2] } else { q.pop_front().unwrap_or([0.0; 2]).map(|s| s.clamp(-1.0, 1.0)) };
                    }
                    base += n.saturating_sub(lead) as f64 / RATE;
                    let _ = render.ReleaseBuffer(n as u32, 0);
                }
            }
            unsafe {
                let _ = client.Stop();
            }
        })?;
        ready_rx.recv().unwrap_or_else(|_| Err(anyhow!("monitor thread ended")))?;
        Ok(Monitor { tx, stop, thread: Some(thread) })
    }

    pub fn play(&self, pts: f64, frames: Vec<[f32; 2]>) {
        let _ = self.tx.send((pts, frames));
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The starter sound pad: short effects synthesized by Chisme (so they
/// carry nobody's copyright), written once to `%APPDATA%\Chisme\sonidos`.
pub fn starter_sounds() -> Vec<Item> {
    let dir = crate::settings::Settings::path().parent().map(|p| p.join("sonidos")).unwrap_or_default();
    let _ = std::fs::create_dir_all(&dir);
    let group = crate::i18n::l("Sound pad", "Efectos").to_string();
    let sounds: [(&str, &str, fn() -> Vec<f32>); 6] = [
        ("applause", crate::i18n::l("Applause", "Aplausos"), synth::applause),
        ("drum-roll", crate::i18n::l("Drum roll", "Redoble"), synth::drum_roll),
        ("ding", "Ding", synth::ding),
        ("whoosh", "Whoosh", synth::whoosh),
        ("buzzer", crate::i18n::l("Buzzer", "Chicharra"), synth::buzzer),
        ("rimshot", "Ba-dum-tss", synth::rimshot),
    ];
    sounds
        .iter()
        .filter_map(|(file, name, make)| {
            let path = dir.join(format!("{file}.wav"));
            if !path.is_file() {
                synth::write_wav(&path, &make()).ok()?;
            }
            Some(Item { name: name.to_string(), path: path.to_string_lossy().into_owned(), kind: Kind::Sound, volume: 1.0, group: group.clone() })
        })
        .collect()
}

mod synth {
    use std::f32::consts::TAU;

    const RATE: f32 = 48000.0;

    /// A tiny deterministic noise source.
    struct Noise(u32);
    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
        }
    }

    fn len(secs: f32) -> usize {
        (secs * RATE) as usize
    }

    /// Mono samples to a 48 kHz 16-bit stereo WAV.
    pub fn write_wav(path: &std::path::Path, mono: &[f32]) -> std::io::Result<()> {
        let peak = mono.iter().fold(0.0f32, |a, v| a.max(v.abs())).max(1e-6);
        let k = 0.8 / peak;
        let data_len = (mono.len() * 4) as u32;
        let mut b = Vec::with_capacity(44 + data_len as usize);
        b.extend(b"RIFF");
        b.extend((36 + data_len).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(48000u32.to_le_bytes());
        b.extend((48000u32 * 4).to_le_bytes());
        b.extend(4u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(data_len.to_le_bytes());
        for v in mono {
            let s = ((v * k).clamp(-1.0, 1.0) * 32767.0) as i16;
            b.extend(s.to_le_bytes());
            b.extend(s.to_le_bytes());
        }
        std::fs::write(path, b)
    }

    pub fn ding() -> Vec<f32> {
        (0..len(1.6))
            .map(|i| {
                let t = i as f32 / RATE;
                let env = (-t * 3.2).exp() * (t * 400.0).min(1.0);
                env * ((TAU * 1318.5 * t).sin() + 0.5 * (TAU * 1975.5 * t).sin() + 0.25 * (TAU * 2637.0 * t).sin())
            })
            .collect()
    }

    pub fn whoosh() -> Vec<f32> {
        let mut n = Noise(7);
        let mut lp = 0.0;
        let total = len(0.9);
        (0..total)
            .map(|i| {
                let x = i as f32 / total as f32;
                let env = (x * std::f32::consts::PI).sin().powf(2.0);
                let cutoff = 0.02 + 0.25 * env;
                lp += cutoff * (n.next() - lp);
                lp * env
            })
            .collect()
    }

    /// One drum hit: a noise burst through a decay, with some body.
    fn hit(out: &mut [f32], at: usize, gain: f32, decay: f32, tone: f32, n: &mut Noise) {
        for k in 0..len(0.25) {
            let Some(o) = out.get_mut(at + k) else { break };
            let t = k as f32 / RATE;
            let env = (-t * decay).exp() * gain;
            *o += env * (0.7 * n.next() + 0.5 * (TAU * tone * t).sin());
        }
    }

    pub fn drum_roll() -> Vec<f32> {
        let mut out = vec![0.0; len(2.2)];
        let mut n = Noise(11);
        let mut t = 0.0;
        while t < 1.7 {
            let gain = 0.25 + 0.75 * (t / 1.7);
            hit(&mut out, len(t), gain, 38.0, 190.0, &mut n);
            t += 0.042;
        }
        hit(&mut out, len(1.75), 1.6, 9.0, 120.0, &mut n);
        out
    }

    pub fn applause() -> Vec<f32> {
        let total = len(3.0);
        let mut out = vec![0.0; total];
        let mut n = Noise(23);
        let mut r = Noise(99);
        for _ in 0..240 {
            let at = ((r.next() * 0.5 + 0.5) * (total as f32 - RATE * 0.05)) as usize;
            let x = at as f32 / total as f32;
            let gain = (x * std::f32::consts::PI).sin().powf(0.6) * (0.5 + 0.5 * (r.next() * 0.5 + 0.5));
            for k in 0..len(0.012) {
                out[at + k] += gain * n.next() * (-(k as f32) / (RATE * 0.004)).exp();
            }
        }
        // Soften the top end.
        let mut lp = 0.0;
        out.iter().map(|v| { lp += 0.45 * (v - lp); lp }).collect()
    }

    pub fn buzzer() -> Vec<f32> {
        (0..len(0.8))
            .map(|i| {
                let t = i as f32 / RATE;
                let env = (t * 200.0).min(1.0) * ((0.8 - t) * 40.0).clamp(0.0, 1.0);
                let sq = |f: f32| if (t * f).fract() < 0.5 { 1.0 } else { -1.0 };
                env * (0.6 * sq(110.0) + 0.4 * sq(116.0))
            })
            .collect()
    }

    pub fn rimshot() -> Vec<f32> {
        let mut out = vec![0.0; len(1.6)];
        let mut n = Noise(5);
        // Ba-dum: two toms with a falling pitch.
        for (at, f0) in [(0.0, 180.0), (0.18, 140.0)] {
            let start = len(at);
            for k in 0..len(0.35) {
                let t = k as f32 / RATE;
                let f = f0 * (1.0 - 0.4 * (t * 8.0).min(1.0));
                out[start + k] += (-t * 9.0).exp() * (TAU * f * t).sin();
            }
        }
        // Tss: a bright, long cymbal.
        let start = len(0.4);
        let mut prev = 0.0;
        for k in 0..len(1.15) {
            let t = k as f32 / RATE;
            let x = n.next();
            let hp = x - prev;
            prev = x;
            out[start + k] += 0.6 * hp * (-t * 3.0).exp();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(kind_of(Path::new("C:/a/aplausos.MP3")), Some(Kind::Sound));
        assert_eq!(kind_of(Path::new("intro.mov")), Some(Kind::Video));
        assert_eq!(kind_of(Path::new("vuelvo.png")), Some(Kind::Image));
        assert_eq!(kind_of(Path::new("notas.txt")), None);
        assert_eq!(item(Path::new("C:/x/Risas grabadas.wav")).unwrap().name, "Risas grabadas");
    }
}
