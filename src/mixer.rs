//! Real-time audio mixer: system audio + microphone -> one stereo track.
//!
//! A `floor` keeps tracks continuous when a source sends nothing (WASAPI
//! loopback is silent, not zero-filled, while nothing plays).
//!
//! Each source's buffers are 48 kHz stereo f32 (resampled here if needed),
//! placed on a common sample timeline derived from their host-clock
//! timestamps, so the mix stays in sync with video. Per-source continuity
//! snapping avoids clicks from sub-millisecond timestamp jitter.

use std::collections::VecDeque;

pub const RATE: f64 = 48000.0;
/// Emit even if a source is silent/stalled for longer than this.
const MAX_WAIT: i64 = 24000; // 0.5 s
/// Treat timestamp jumps up to this many samples as continuous.
const SNAP: i64 = 96; // 2 ms

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioKind {
    System,
    Mic,
    /// Media Board sounds and clips: mixed in when present, never waited for.
    Media,
}

pub struct Mixer {
    t0: i64, // session start in samples on the host clock
    base: i64,
    buf: VecDeque<[f32; 2]>,
    horizon: [Option<i64>; 3],
    /// Sources the mix waits for (system, mic); media is never waited for.
    active: [bool; 3],
    /// Takes Media Board audio.
    media: bool,
    gain: [f32; 3],
    /// Emit at least up to here (samples from t0), filling silence.
    floor: i64,
    /// Samples that would have clipped (the limiter held them back).
    pub clipped: u64,
    resamplers: [Option<Resampler>; 3],
    /// Noise suppression per source (used for the mic).
    denoise: [Option<Denoiser>; 3],
    limiter: Limiter,
}

/// Microphone noise suppression with RNNoise (`nnnoiseless`): a small
/// recurrent network that removes fans, hum, keyboard and room noise and
/// keeps voices. It works on 10 ms blocks of 48 kHz mono; blocks keep their
/// capture times, so sync is unchanged (output trails input by < 10 ms).
pub struct Denoiser {
    st: Box<nnnoiseless::DenoiseState<'static>>,
    pending: Vec<f32>,
    /// Host time of `pending[0]`.
    t: f64,
}

const DN_FRAME: usize = nnnoiseless::DenoiseState::FRAME_SIZE;

impl Default for Denoiser {
    fn default() -> Self {
        Denoiser { st: nnnoiseless::DenoiseState::new(), pending: Vec::with_capacity(DN_FRAME * 2), t: 0.0 }
    }
}

impl Denoiser {
    /// 48 kHz frames starting at host time `t` → cleaned 10 ms blocks with
    /// their start times.
    pub fn push(&mut self, t: f64, frames: &[[f32; 2]]) -> Vec<(f64, Vec<[f32; 2]>)> {
        let expected = self.t + self.pending.len() as f64 / RATE;
        if self.pending.is_empty() || (t - expected).abs() > SNAP as f64 / RATE {
            // Start (or a gap in the stream): restart the block grid here.
            self.pending.clear();
            self.t = t;
        }
        // RNNoise works on 16-bit-scaled samples.
        self.pending.extend(frames.iter().map(|f| (f[0] + f[1]) * 0.5 * 32768.0));
        let mut out = vec![];
        let mut buf = [0f32; DN_FRAME];
        while self.pending.len() >= DN_FRAME {
            self.st.process_frame(&mut buf, &self.pending[..DN_FRAME]);
            out.push((self.t, buf.iter().map(|s| { let v = s / 32768.0; [v, v] }).collect()));
            self.pending.drain(..DN_FRAME);
            self.t += DN_FRAME as f64 / RATE;
        }
        out
    }
}

fn idx(k: AudioKind) -> usize {
    match k {
        AudioKind::System => 0,
        AudioKind::Mic => 1,
        AudioKind::Media => 2,
    }
}

/// Taps on each side of a resampled output sample.
const HALF_TAPS: usize = 24;

/// Streaming sinc resampler to 48 kHz for one source (e.g. a 44.1 kHz USB
/// mic or a 16/24 kHz Bluetooth headset). Kaiser-windowed sinc, low-passed
/// below the lower Nyquist rate, with its filter history and fractional
/// position carried across buffers, so buffer boundaries are seamless.
struct Resampler {
    from: f64,
    /// Input samples not yet fully used, starting at absolute input index
    /// `first`.
    hist: Vec<[f32; 2]>,
    first: i64,
    /// Absolute input position of the next output sample.
    pos: f64,
    /// Host time (seconds) of input sample 0.
    t0: f64,
    /// Absolute input index the next buffer is expected to start at.
    next_in: i64,
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..32 {
        term *= (x / (2.0 * k as f64)).powi(2);
        sum += term;
    }
    sum
}

impl Resampler {
    fn new(from: f64, t0: f64) -> Self {
        Resampler { from, hist: vec![], first: 0, pos: 0.0, t0, next_in: 0 }
    }

    /// Resample a buffer starting at host time `pts`. Returns the output
    /// frames and the host time of the first one. A buffer that does not
    /// continue the stream (gap, overlap, rate change) restarts it.
    fn push(&mut self, pts: f64, rate: f64, src: Vec<[f32; 2]>) -> (f64, Vec<[f32; 2]>) {
        let expected = self.t0 + self.next_in as f64 / self.from;
        if rate != self.from || (pts - expected).abs() > SNAP as f64 / RATE {
            *self = Resampler::new(rate, pts);
        }
        self.next_in += src.len() as i64;
        self.hist.extend(src);
        let step = self.from / RATE;
        // Cutoff (cycles per input sample): below both Nyquist rates.
        let fc = 0.5 * (RATE / self.from).min(1.0) * 0.94;
        let beta = 8.0;
        let norm = bessel_i0(beta);
        let mut out = vec![];
        let t_out = self.t0 + self.pos / self.from;
        let h = HALF_TAPS as f64 / (2.0 * fc).min(1.0);
        let reach = h.ceil() as i64;
        // Taps before the stream's start read as silence.
        while (self.pos.floor() as i64) + reach < self.first + self.hist.len() as i64 {
            let c = self.pos.floor() as i64;
            let (mut l, mut r, mut wsum) = (0.0f64, 0.0f64, 0.0f64);
            for j in (c - reach + 1)..=(c + reach) {
                let x = j as f64 - self.pos;
                if x.abs() >= h {
                    continue;
                }
                let arg = 2.0 * fc * x;
                let sinc = if arg.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * arg).sin() / (std::f64::consts::PI * arg) };
                let w = bessel_i0(beta * (1.0 - (x / h).powi(2)).max(0.0).sqrt()) / norm;
                let k = 2.0 * fc * sinc * w;
                wsum += k;
                if j >= self.first {
                    let s = self.hist[(j - self.first) as usize];
                    l += k * s[0] as f64;
                    r += k * s[1] as f64;
                }
            }
            // Unity gain at DC whatever the fractional phase.
            let g = if wsum > 0.0 { 1.0 / wsum } else { 0.0 };
            out.push([(l * g) as f32, (r * g) as f32]);
            self.pos += step;
        }
        let keep_from = (self.pos.floor() as i64 - reach).max(self.first);
        let drop = (keep_from - self.first) as usize;
        self.hist.drain(..drop.min(self.hist.len()));
        self.first += drop as i64;
        (t_out, out)
    }
}

/// Mix bus limiter: keeps summed sources under the ceiling with smooth gain
/// changes (fast attack, slow release) and a soft knee for the rare peak
/// the envelope misses, instead of clipping them flat.
struct Limiter {
    gain: f32,
}

/// -1 dBFS: headroom for the AAC encoder, whose output overshoots its
/// input peaks slightly.
const CEILING: f32 = 0.891;
const ATTACK: f32 = 0.35; // per sample, ~0.06 ms
const RELEASE: f32 = 1.0 / (0.15 * RATE as f32); // full recovery in ~150 ms

impl Limiter {
    fn process(&mut self, fr: [f32; 2]) -> [f32; 2] {
        let peak = fr[0].abs().max(fr[1].abs());
        let want = if peak > CEILING { CEILING / peak } else { 1.0 };
        self.gain = if want < self.gain { self.gain + (want - self.gain) * ATTACK } else { (self.gain + RELEASE).min(want) };
        fr.map(|s| soft_clip(s * self.gain))
    }
}

/// Identity below the knee, then a smooth curve that never passes 1.0.
fn soft_clip(s: f32) -> f32 {
    const KNEE: f32 = 0.9;
    let a = s.abs();
    if a <= KNEE {
        return s;
    }
    let room = 1.0 - KNEE;
    (KNEE + room * ((a - KNEE) / room).tanh()).copysign(s)
}

impl Mixer {
    pub fn new(t0_secs: f64, system: bool, mic: bool, gain_db: [f32; 2]) -> Self {
        Mixer {
            t0: (t0_secs * RATE).round() as i64,
            base: 0,
            buf: VecDeque::new(),
            horizon: [None, None, None],
            active: [system, mic, false],
            media: true,
            gain: [gain_db[0], gain_db[1], 0.0].map(|db| 10f32.powf(db / 20.0)),
            floor: 0,
            clipped: 0,
            resamplers: [None, None, None],
            denoise: [None, None, None],
            limiter: Limiter { gain: 1.0 },
        }
    }

    /// Clean the microphone with RNNoise.
    pub fn with_mic_denoise(mut self, on: bool) -> Self {
        self.denoise[idx(AudioKind::Mic)] = on.then(Denoiser::default);
        self
    }

    /// Whether this track gets Media Board audio.
    pub fn with_media(mut self, on: bool) -> Self {
        self.media = on;
        self
    }

    /// Whether this mixer takes `kind` at all.
    pub fn takes(&self, kind: AudioKind) -> bool {
        match kind {
            AudioKind::Media => self.media,
            k => self.active[idx(k)],
        }
    }

    /// Add frames that start at `pts` (host seconds) at sample `rate`.
    pub fn push(&mut self, kind: AudioKind, pts: f64, rate: f64, frames: Vec<[f32; 2]>) {
        let i = idx(kind);
        let (pts, frames) = if (rate - RATE).abs() < 0.5 || rate <= 0.0 {
            self.resamplers[i] = None;
            (pts, frames)
        } else {
            self.resamplers[i].get_or_insert_with(|| Resampler::new(rate, pts)).push(pts, rate, frames)
        };
        if frames.is_empty() {
            return;
        }
        match self.denoise[i].as_mut() {
            Some(d) => {
                for (t, block) in d.push(pts, &frames) {
                    self.place(i, t, block);
                }
            }
            None => self.place(i, pts, frames),
        }
    }

    /// Put 48 kHz frames from source `i` on the timeline at host time `pts`.
    fn place(&mut self, i: usize, pts: f64, frames: Vec<[f32; 2]>) {
        let mut start = (pts * RATE).round() as i64 - self.t0;
        if let Some(h) = self.horizon[i]
            && (start - h).abs() <= SNAP
        {
            start = h;
        }
        let end = start + frames.len() as i64;
        self.horizon[i] = Some(self.horizon[i].map_or(end, |h| h.max(end)));
        let need = (end - self.base).max(0) as usize;
        if self.buf.len() < need {
            self.buf.resize(need, [0.0, 0.0]);
        }
        let g = self.gain[i];
        for (n, fr) in frames.iter().enumerate() {
            let pos = start + n as i64 - self.base;
            if pos < 0 {
                continue; // already emitted
            }
            let slot = &mut self.buf[pos as usize];
            slot[0] += fr[0] * g;
            slot[1] += fr[1] * g;
        }
    }

    /// Emit at least up to host time `secs` on the next pull, filling silence
    /// where no source has sent anything.
    pub fn set_floor(&mut self, secs: f64) {
        self.floor = self.floor.max((secs * RATE).round() as i64 - self.t0);
    }

    /// Mixed frames ready with their start time in host seconds: everything
    /// all active sources have reached, or anything older than MAX_WAIT
    /// behind the newest source (or the floor).
    pub fn pull_frames(&mut self, flush: bool) -> Option<(f64, Vec<[f32; 2]>)> {
        let hs: Vec<i64> = (0..3).filter(|&i| self.active[i]).map(|i| self.horizon[i].unwrap_or(0)).collect();
        let max_h = hs.iter().copied().max().unwrap_or(0);
        let until = if flush { max_h } else { hs.iter().copied().min().unwrap_or(0).max(max_h - MAX_WAIT) };
        let until = until.max(self.floor);
        let need = (until - self.base).max(0) as usize;
        if self.buf.len() < need {
            self.buf.resize(need, [0.0, 0.0]);
        }
        let n = (until - self.base).min(self.buf.len() as i64);
        if n <= 0 {
            return None;
        }
        let t = (self.t0 + self.base) as f64 / RATE;
        self.base += n;
        let mut frames: Vec<[f32; 2]> = self.buf.drain(..n as usize).collect();
        for fr in &mut frames {
            self.clipped += fr.iter().filter(|s| s.abs() > 1.0).count() as u64;
            *fr = self.limiter.process(*fr);
        }
        Some((t, frames))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pull_all(m: &mut Mixer, flush: bool) -> (Option<f64>, Vec<[f32; 2]>) {
        let mut first = None;
        let mut out = vec![];
        while let Some((pts, frames)) = m.pull_frames(flush) {
            first.get_or_insert(pts);
            out.extend(frames);
        }
        (first, out)
    }

    #[test]
    fn sums_sources_on_a_shared_timeline() {
        let mut m = Mixer::new(10.0, true, true, [0.0, 0.0]);
        m.push(AudioKind::System, 10.0, RATE, vec![[0.25, 0.25]; 480]);
        // Nothing until the mic reaches the same point (it may still arrive).
        assert!(m.pull_frames(false).is_none());
        // Mic starts 5 ms later, mono duplicated to both channels.
        m.push(AudioKind::Mic, 10.005, RATE, vec![[0.5, 0.5]; 480]);
        let (pts, out) = pull_all(&mut m, false);
        assert_eq!(pts, Some(10.0));
        assert_eq!(out.len(), 480, "only up to the slower source's horizon");
        assert_eq!(out[0], [0.25, 0.25]);
        assert_eq!(out[240], [0.75, 0.75], "overlap is summed");
        let (pts, rest) = pull_all(&mut m, true);
        assert_eq!(pts, Some(10.01));
        assert_eq!(rest.len(), 240);
        assert_eq!(rest[0], [0.5, 0.5]);
    }

    #[test]
    fn gain_and_limiting() {
        // +6 dB: 0.2 -> 0.399, untouched (under the ceiling).
        let mut m = Mixer::new(0.0, false, true, [0.0, 6.0]);
        m.push(AudioKind::Mic, 0.0, RATE, vec![[0.2, -0.3]; 100]);
        let (_, out) = pull_all(&mut m, true);
        assert!((out[50][0] - 0.399).abs() < 0.01);
        assert!((out[50][1] + 0.599).abs() < 0.01);
        assert_eq!(m.clipped, 0);
        // Too loud: held under full scale (both channels by the same gain,
        // so the stereo image keeps), never flattened at 1.0.
        let mut m = Mixer::new(0.0, false, true, [0.0, 6.0]);
        m.push(AudioKind::Mic, 0.0, RATE, vec![[0.4, -0.6]; 4800]);
        let (_, out) = pull_all(&mut m, true);
        assert!(out.iter().all(|f| f[0].abs() < 1.0 && f[1].abs() < 1.0));
        let last = out[4799];
        assert!((last[1].abs() - CEILING).abs() < 0.01, "{last:?}");
        assert!((last[0] / last[1] + 0.4 / 0.6).abs() < 0.01);
        assert_eq!(m.clipped, 4800);
    }

    /// A sine through the resampler in uneven buffers: the right length and
    /// pitch, seamless across buffers (error against the ideal 48 kHz sine).
    fn resampled_sine_error(from: f64, freq: f64, chunk: usize) -> (usize, f64) {
        let mut m = Mixer::new(0.0, false, true, [0.0, 0.0]);
        let n = from as usize; // 1 s
        let sine = |t: f64| (2.0 * std::f64::consts::PI * freq * t).sin() as f32 * 0.5;
        let mut i = 0;
        while i < n {
            let len = chunk + (i / chunk) % 7; // uneven buffer sizes
            let end = (i + len).min(n);
            let frames: Vec<[f32; 2]> = (i..end)
                .map(|k| {
                    let v = sine(k as f64 / from);
                    [v, v]
                })
                .collect();
            m.push(AudioKind::Mic, i as f64 / from, from, frames);
            i = end;
        }
        let (pts, out) = pull_all(&mut m, true);
        let t0 = pts.unwrap();
        // Compare away from the stream's start and end (filter ramp).
        let err = (2400..out.len() - 2400).map(|k| (out[k][0] - sine(t0 + k as f64 / RATE)).abs() as f64).fold(0.0, f64::max);
        (out.len(), err)
    }

    #[test]
    fn resamples_seamlessly() {
        for (from, chunk) in [(44100.0, 441), (16000.0, 160), (24000.0, 512), (96000.0, 1024)] {
            let (len, err) = resampled_sine_error(from, 1000.0, chunk);
            // The last ~1.6 ms wait in the filter for input that never came.
            assert!((len as i64 - 48000).abs() <= 100, "{from}: {len} samples");
            assert!(err < 0.002, "{from}: max error {err}");
        }
    }

    #[test]
    fn resampler_removes_content_above_nyquist() {
        // 30 kHz at 96 kHz must be filtered out rather than fold to 18 kHz.
        let mut m = Mixer::new(0.0, false, true, [0.0, 0.0]);
        let frames: Vec<[f32; 2]> = (0..96000)
            .map(|k| {
                let v = (2.0 * std::f64::consts::PI * 30000.0 * k as f64 / 96000.0).sin() as f32 * 0.5;
                [v, v]
            })
            .collect();
        m.push(AudioKind::Mic, 0.0, 96000.0, frames);
        let (_, out) = pull_all(&mut m, true);
        let peak = out[2400..out.len() - 2400].iter().fold(0f32, |a, f| a.max(f[0].abs()));
        assert!(peak < 0.005, "aliased residue {peak}");
    }

    #[test]
    fn small_jitter_snaps_to_continuous() {
        let mut m = Mixer::new(0.0, true, false, [0.0, 0.0]);
        m.push(AudioKind::System, 0.0, RATE, vec![[0.1, 0.1]; 480]);
        // 1 ms late: within SNAP, so no gap of silence is inserted.
        m.push(AudioKind::System, 0.011, RATE, vec![[0.1, 0.1]; 480]);
        let (_, out) = pull_all(&mut m, true);
        assert_eq!(out.len(), 960);
        assert!(out.iter().all(|f| f[0] == 0.1));
    }

    /// Room-like noise: white noise through a one-pole low-pass (more
    /// energy low, like fans and hum), `secs` long at amplitude `amp`.
    fn room_noise(secs: f64, amp: f32) -> Vec<[f32; 2]> {
        let mut x = 12345u32;
        let mut lp = 0f32;
        (0..(secs * RATE) as usize)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                lp += 0.15 * ((x as f32 / u32::MAX as f32 - 0.5) - lp);
                let v = lp * amp;
                [v, v]
            })
            .collect()
    }

    #[test]
    #[ignore]
    fn denoise_probe() {
        let rms = |v: &[[f32; 2]]| (v.iter().map(|f| f[0] * f[0]).sum::<f32>() / v.len() as f32).sqrt();
        for amp in [0.02f32, 0.05, 0.2, 0.6, 2.0] {
            let n = room_noise(4.0, amp);
            let mut d = Denoiser::default();
            let out: Vec<[f32; 2]> = d.push(0.0, &n).into_iter().skip(100).flat_map(|(_, b)| b).collect();
            println!("noise {:.1} dBFS -> reduced {:.1} dB", 20.0 * rms(&n).log10(), 20.0 * (rms(&n[48000..]) / rms(&out).max(1e-9)).log10());
        }
    }

    #[test]
    fn denoiser_removes_steady_noise_and_keeps_timing() {
        let frames = room_noise(2.0, 0.2);
        let rms = |v: &[[f32; 2]]| (v.iter().map(|f| f[0] * f[0]).sum::<f32>() / v.len() as f32).sqrt();
        let mut d = Denoiser::default();
        let mut out = vec![];
        // Uneven buffers, like a microphone delivers them.
        let mut t = 100.0;
        for chunk in frames.chunks(441) {
            for (bt, block) in d.push(t, chunk) {
                if let Some((lt, lb)) = out.last().map(|(lt, lb): &(f64, Vec<[f32; 2]>)| (*lt, lb.len())) {
                    assert!((bt - (lt + lb as f64 / RATE)).abs() < 1e-9, "blocks are back to back");
                }
                out.push((bt, block));
            }
            t += chunk.len() as f64 / RATE;
        }
        assert_eq!(out[0].0, 100.0, "the first block keeps the capture time");
        let cleaned: Vec<[f32; 2]> = out.iter().skip(50).flat_map(|(_, b)| b.clone()).collect();
        let reduction = 20.0 * (rms(&frames) / rms(&cleaned).max(1e-9)).log10();
        assert!(reduction > 15.0, "noise reduced by {reduction:.1} dB");
    }

    #[test]
    fn mixer_denoises_only_the_mic() {
        let tone: Vec<[f32; 2]> = (0..4800).map(|i| { let v = (i as f32 * 0.13).sin() * 0.3; [v, v] }).collect();
        let mut m = Mixer::new(0.0, true, false, [0.0, 0.0]).with_mic_denoise(true);
        m.push(AudioKind::System, 0.0, RATE, tone.clone());
        let (_, out) = pull_all(&mut m, true);
        assert_eq!(out, tone, "system audio passes untouched");
    }

    #[test]
    fn media_is_mixed_but_never_waited_for() {
        let mut m = Mixer::new(0.0, false, true, [0.0, 0.0]);
        // A sound fired ahead of the mic's horizon doesn't hold the mix back…
        m.push(AudioKind::Media, 0.0, RATE, vec![[0.25, 0.25]; 960]);
        assert!(m.pull_frames(false).is_none(), "nothing until the mic arrives");
        m.push(AudioKind::Mic, 0.0, RATE, vec![[0.5, 0.5]; 480]);
        let (_, out) = pull_all(&mut m, false);
        assert_eq!(out.len(), 480, "only up to the mic");
        assert_eq!(out[0], [0.75, 0.75], "…but is summed in");
        assert!(Mixer::new(0.0, true, true, [0.0; 2]).takes(AudioKind::Media));
        assert!(!Mixer::new(0.0, true, true, [0.0; 2]).with_media(false).takes(AudioKind::Media));
    }

    #[test]
    fn floor_fills_silence_when_a_source_is_quiet() {
        // Loopback sends nothing while nothing plays: the track still advances.
        let mut m = Mixer::new(5.0, true, false, [0.0, 0.0]);
        assert!(m.pull_frames(false).is_none());
        m.set_floor(5.5);
        let (pts, out) = pull_all(&mut m, false);
        assert_eq!(pts, Some(5.0));
        assert_eq!(out.len(), 24000);
        assert!(out.iter().all(|f| *f == [0.0, 0.0]));
        // Sound arriving later lands after the silence.
        m.push(AudioKind::System, 5.5, RATE, vec![[0.2, 0.2]; 480]);
        let (pts, out) = pull_all(&mut m, true);
        assert_eq!(pts, Some(5.5));
        assert_eq!(out, vec![[0.2, 0.2]; 480]);
    }
}
