//! Replay buffer: the last N seconds of encoded video and audio, kept in
//! memory from a keyframe on, saved to MP4 on demand without stopping.
//! The MP4 is written with the `mp4` crate.

use std::collections::VecDeque;
use std::path::Path;

use anyhow::{Context, Result, bail};
use mp4::{AacConfig, AudioObjectType, AvcConfig, ChannelConfig, MediaConfig, Mp4Config, Mp4Sample, Mp4Writer, SampleFreqIndex, TrackConfig, TrackType};

use super::{AudioPacket, VideoPacket};

/// What the ring needs to know about a packet.
pub trait Timed {
    fn pts(&self) -> f64;
    fn key(&self) -> bool {
        true
    }
    fn size(&self) -> usize;
}

impl Timed for VideoPacket {
    fn pts(&self) -> f64 {
        self.pts
    }
    fn key(&self) -> bool {
        self.key
    }
    fn size(&self) -> usize {
        self.avcc.len()
    }
}

impl Timed for AudioPacket {
    fn pts(&self) -> f64 {
        self.pts
    }
    fn size(&self) -> usize {
        self.data.len()
    }
}

/// Ring of the last `secs` seconds, always starting at a video keyframe.
pub struct Ring<V, A> {
    pub secs: f64,
    video: VecDeque<V>,
    audio: VecDeque<A>,
    bytes: usize,
}

impl<V: Timed + Clone, A: Timed + Clone> Ring<V, A> {
    pub fn new(secs: f64) -> Self {
        Ring { secs, video: VecDeque::new(), audio: VecDeque::new(), bytes: 0 }
    }

    pub fn push_video(&mut self, v: V) {
        // Wait for a keyframe to start.
        if self.video.is_empty() && !v.key() {
            return;
        }
        self.bytes += v.size();
        self.video.push_back(v);
        self.trim();
    }

    pub fn push_audio(&mut self, a: A) {
        if self.video.front().is_none_or(|v| a.pts() < v.pts()) {
            return;
        }
        self.bytes += a.size();
        self.audio.push_back(a);
    }

    /// Drop whole GOPs that are older than the window (keeping the keyframe
    /// that starts it, so the saved clip always decodes).
    fn trim(&mut self) {
        let Some(latest) = self.video.back().map(|v| v.pts()) else { return };
        let horizon = latest - self.secs;
        let cut = self.video.iter().enumerate().filter(|(_, v)| v.key() && v.pts() <= horizon).map(|(i, _)| i).last();
        if let Some(cut) = cut.filter(|&c| c > 0) {
            for v in self.video.drain(..cut) {
                self.bytes -= v.size();
            }
            let start = self.video[0].pts();
            while self.audio.front().is_some_and(|a| a.pts() < start) {
                let a = self.audio.pop_front().unwrap();
                self.bytes -= a.size();
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Seconds of video held.
    pub fn held(&self) -> f64 {
        match (self.video.front(), self.video.back()) {
            (Some(a), Some(b)) => b.pts() - a.pts(),
            _ => 0.0,
        }
    }

    /// The last `secs` (or everything held), starting at a keyframe.
    pub fn snapshot(&self) -> (Vec<V>, Vec<A>) {
        let latest = self.video.back().map_or(0.0, |v| v.pts());
        let horizon = latest - self.secs;
        let start = self
            .video
            .iter()
            .enumerate()
            .filter(|(_, v)| v.key() && v.pts() <= horizon.max(self.video[0].pts()))
            .map(|(i, _)| i)
            .last()
            .unwrap_or(0);
        let v: Vec<V> = self.video.iter().skip(start).cloned().collect();
        let t0 = v.first().map_or(0.0, |v| v.pts());
        let a = self.audio.iter().filter(|a| a.pts() >= t0).cloned().collect();
        (v, a)
    }

    #[cfg(test)]
    pub fn clear(&mut self) {
        self.video.clear();
        self.audio.clear();
        self.bytes = 0;
    }
}

const VIDEO_SCALE: u32 = 90_000;

/// Write a clip (video from a keyframe, AAC frames) as an MP4.
pub fn write_mp4(path: &Path, video: &[VideoPacket], audio: &[AudioPacket], sps: &[u8], pps: &[u8], size: (u32, u32), fps: u32) -> Result<()> {
    if video.is_empty() {
        bail!("the replay buffer is still empty");
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path).with_context(|| format!("can't create {}", path.display()))?);
    let cfg = Mp4Config {
        major_brand: "isom".parse()?,
        minor_version: 512,
        compatible_brands: vec!["isom".parse()?, "iso2".parse()?, "avc1".parse()?, "mp41".parse()?],
        timescale: 1000,
    };
    let mut w = Mp4Writer::write_start(file, &cfg)?;
    w.add_track(&TrackConfig {
        track_type: TrackType::Video,
        timescale: VIDEO_SCALE,
        language: "und".into(),
        media_conf: MediaConfig::AvcConfig(AvcConfig { width: size.0 as u16, height: size.1 as u16, seq_param_set: sps.to_vec(), pic_param_set: pps.to_vec() }),
    })?;
    let has_audio = !audio.is_empty();
    if has_audio {
        w.add_track(&TrackConfig {
            track_type: TrackType::Audio,
            timescale: 48_000,
            language: "und".into(),
            media_conf: MediaConfig::AacConfig(AacConfig {
                bitrate: 160_000,
                profile: AudioObjectType::AacLowComplexity,
                freq_index: SampleFreqIndex::Freq48000,
                chan_conf: ChannelConfig::Stereo,
            }),
        })?;
    }
    let t0 = video[0].pts;
    let ticks = |t: f64| ((t - t0) * VIDEO_SCALE as f64).round().max(0.0) as u64;
    let frame = VIDEO_SCALE / fps.max(1);
    for (i, v) in video.iter().enumerate() {
        let start = ticks(v.pts);
        let duration = video.get(i + 1).map_or(frame as u64, |n| ticks(n.pts).saturating_sub(start).max(1)) as u32;
        w.write_sample(1, &Mp4Sample { start_time: start, duration, rendering_offset: 0, is_sync: v.key, bytes: bytes::Bytes::copy_from_slice(&v.avcc) })?;
    }
    if has_audio {
        // AAC frames are back to back from the first one at or after t0.
        for (i, a) in audio.iter().enumerate() {
            w.write_sample(2, &Mp4Sample { start_time: i as u64 * 1024, duration: 1024, rendering_offset: 0, is_sync: true, bytes: bytes::Bytes::copy_from_slice(&a.data) })?;
        }
    }
    w.write_end()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct P(f64, bool);
    impl Timed for P {
        fn pts(&self) -> f64 {
            self.0
        }
        fn key(&self) -> bool {
            self.1
        }
        fn size(&self) -> usize {
            100
        }
    }

    #[test]
    fn ring_keeps_whole_gops_inside_the_window() {
        let mut r: Ring<P, P> = Ring::new(10.0);
        // Starts at a keyframe.
        r.push_video(P(0.0, false));
        assert_eq!(r.held(), 0.0);
        // 30 s at 30 fps, a keyframe every 2 s.
        for i in 0..900 {
            let t = 1.0 + i as f64 / 30.0;
            r.push_video(P(t, i % 60 == 0));
            r.push_audio(P(t, true));
        }
        let held = r.held();
        assert!((10.0..12.1).contains(&held), "held {held}");
        let (v, a) = r.snapshot();
        assert!(v[0].1, "the clip starts at a keyframe");
        assert!(a[0].0 >= v[0].0);
        assert!(v.last().unwrap().0 - v[0].0 <= 12.0);
        r.clear();
        assert_eq!((r.held(), r.bytes()), (0.0, 0));
    }
}
