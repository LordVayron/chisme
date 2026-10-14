//! Chapter markers added while recording, saved next to the video in
//! YouTube's description format ("00:00 Intro", one per line), which turns
//! into chapters when pasted into a video's description.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    /// Seconds into the recording (pauses removed).
    pub at: f64,
    pub label: String,
}

/// `m:ss`, or `h:mm:ss` from an hour on (YouTube's style).
pub fn stamp(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

/// The chapter list. YouTube needs the first chapter at 0:00, so one is
/// added ("Intro") unless a marker is already within the first second.
/// Markers closer than a second to the previous one are dropped.
pub fn to_text(markers: &[Marker]) -> String {
    to_text_with(markers, crate::i18n::l("Intro", "Inicio"))
}

fn to_text_with(markers: &[Marker], start: &str) -> String {
    let mut list: Vec<Marker> = vec![];
    if markers.first().is_none_or(|m| m.at >= 1.0) {
        list.push(Marker { at: 0.0, label: start.into() });
    }
    for m in markers {
        if list.last().is_none_or(|l| m.at - l.at >= 1.0) {
            list.push(Marker { at: if list.is_empty() { 0.0 } else { m.at }, label: m.label.clone() });
        }
    }
    list.iter().map(|m| format!("{} {}\n", stamp(m.at), m.label)).collect()
}

/// `video.mp4` → `video.chapters.txt`.
pub fn path_for(video: &Path) -> PathBuf {
    video.with_extension("chapters.txt")
}

/// Write the chapters file (nothing when there are no markers).
pub fn save(video: &Path, markers: &[Marker]) -> std::io::Result<Option<PathBuf>> {
    if markers.is_empty() {
        return Ok(None);
    }
    let p = path_for(video);
    std::fs::write(&p, to_text(markers))?;
    Ok(Some(p))
}

/// How many chapters a video has (0 without a chapters file).
pub fn count(video: &Path) -> usize {
    // Recordings from before 0.2 have `video.capitulos.txt`.
    std::fs::read_to_string(path_for(video))
        .or_else(|_| std::fs::read_to_string(video.with_extension("capitulos.txt")))
        .map_or(0, |t| t.lines().filter(|l| !l.trim().is_empty()).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(at: f64, label: &str) -> Marker {
        Marker { at, label: label.into() }
    }

    #[test]
    fn youtube_format() {
        let t = to_text_with(&[m(83.4, "Demo"), m(83.9, "double"), m(3725.0, "Q&A")], "Intro");
        assert_eq!(t, "00:00 Intro\n01:23 Demo\n1:02:05 Q&A\n");
        // A marker right at the start becomes the 0:00 chapter.
        assert_eq!(to_text_with(&[m(0.4, "Hello"), m(30.0, "Topic")], "Intro"), "00:00 Hello\n00:30 Topic\n");
    }

    #[test]
    fn files() {
        assert_eq!(path_for(Path::new("C:/v/Chisme 1.mp4")), PathBuf::from("C:/v/Chisme 1.chapters.txt"));
        let dir = std::env::temp_dir().join(format!("chisme-ch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let v = dir.join("a.mp4");
        assert_eq!(save(&v, &[]).unwrap(), None);
        save(&v, &[m(10.0, "Uno"), m(20.0, "Dos")]).unwrap();
        assert_eq!(count(&v), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
