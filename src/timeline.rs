//! Pause timeline on the constant-frame-rate grid.

/// Pause spans on the real frame grid, in slot indices `[pause, resume)`.
/// Everything captured inside a span is dropped and everything after it is
/// shifted earlier by the span's length, so the output has no gap. Spans are
/// whole slots, which keeps the video grid exact across pauses.
#[derive(Default, Debug, Clone)]
pub struct Pauses {
    spans: Vec<(i64, Option<i64>)>,
}

impl Pauses {
    pub fn is_paused(&self) -> bool {
        self.spans.last().is_some_and(|s| s.1.is_none())
    }

    /// Pause from `slot` on. Returns false if already paused.
    pub fn pause(&mut self, slot: i64) -> bool {
        if self.is_paused() {
            return false;
        }
        let floor = self.spans.last().and_then(|s| s.1).unwrap_or(0);
        self.spans.push((slot.max(floor), None));
        true
    }

    /// Resume at `slot`. Returns false if not paused.
    pub fn resume(&mut self, slot: i64) -> bool {
        let Some(last) = self.spans.last_mut() else { return false };
        if last.1.is_some() {
            return false;
        }
        if slot <= last.0 {
            self.spans.pop(); // resumed before the pause took effect
        } else {
            last.1 = Some(slot);
        }
        true
    }

    /// Slots to shift a capture at real grid position `slot` (fractional for
    /// audio), or None if it falls inside a pause and must be dropped.
    pub fn offset_at(&self, slot: f64) -> Option<i64> {
        let mut off = 0;
        for &(p, r) in &self.spans {
            if slot < p as f64 {
                break;
            }
            match r {
                Some(r) if slot >= r as f64 => off += r - p,
                _ => return None,
            }
        }
        Some(off)
    }

    /// The longest part of a capture spanning real slots `[a, b)` that lies
    /// outside pauses: `(from, to, offset)`. None if it is entirely paused.
    #[allow(dead_code)] // used by the audio path (phase 2)
    pub fn keep(&self, a: f64, b: f64) -> Option<(f64, f64, i64)> {
        let mut best: Option<(f64, f64, i64)> = None;
        let mut from = f64::NEG_INFINITY;
        let mut off = 0;
        let mut consider = |lo: f64, hi: f64, off: i64| {
            let (x, y) = (lo.max(a), hi.min(b));
            if y > x && best.is_none_or(|(p, q, _)| y - x > q - p) {
                best = Some((x, y, off));
            }
        };
        for &(p, r) in &self.spans {
            consider(from, p as f64, off);
            match r {
                Some(r) => {
                    off += r - p;
                    from = r as f64;
                }
                None => return best,
            }
        }
        consider(from, f64::INFINITY, off);
        best
    }

    /// Total paused slots (closed spans only).
    #[cfg(test)]
    pub fn total(&self) -> i64 {
        self.spans.iter().filter_map(|&(p, r)| r.map(|r| r - p)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::Pauses;

    #[test]
    fn no_pauses_means_no_offset() {
        let p = Pauses::default();
        assert_eq!(p.offset_at(0.0), Some(0));
        assert_eq!(p.offset_at(1e6), Some(0));
        assert!(!p.is_paused());
    }

    #[test]
    fn pause_drops_span_and_shifts_later_captures() {
        let mut p = Pauses::default();
        assert!(p.pause(100));
        assert!(!p.pause(120), "double pause is a no-op");
        assert!(p.is_paused());
        assert_eq!(p.offset_at(99.9), Some(0));
        assert_eq!(p.offset_at(100.0), None);
        assert_eq!(p.offset_at(5000.0), None, "open span drops everything after it");
        assert!(p.resume(160));
        assert_eq!(p.offset_at(159.5), None);
        assert_eq!(p.offset_at(160.0), Some(60));
        assert_eq!(p.total(), 60);
        assert!(p.pause(200));
        assert!(p.resume(230));
        assert_eq!(p.offset_at(180.0), Some(60));
        assert_eq!(p.offset_at(231.0), Some(90));
        assert_eq!(p.total(), 90);
    }

    #[test]
    fn output_slots_stay_contiguous_across_pauses() {
        let mut p = Pauses::default();
        p.pause(10);
        p.resume(25);
        p.pause(30);
        p.resume(31);
        let out: Vec<i64> = (0..40).filter_map(|s| p.offset_at(s as f64).map(|o| s - o)).collect();
        assert_eq!(out, (0..out.len() as i64).collect::<Vec<_>>());
        assert_eq!(out.len(), 40 - 15 - 1);
    }

    #[test]
    fn keep_cuts_captures_at_pause_boundaries() {
        let mut p = Pauses::default();
        assert_eq!(p.keep(1.0, 2.0), Some((1.0, 2.0, 0)));
        p.pause(10);
        assert_eq!(p.keep(9.4, 10.6), Some((9.4, 10.0, 0)), "tail inside the pause is cut");
        assert_eq!(p.keep(10.0, 11.0), None);
        p.resume(20);
        assert_eq!(p.keep(19.5, 20.7), Some((20.0, 20.7, 10)), "head inside the pause is cut");
        assert_eq!(p.keep(30.0, 31.0), Some((30.0, 31.0, 10)));
        // A capture spanning a whole pause keeps its longer side.
        assert_eq!(p.keep(9.0, 20.5), Some((9.0, 10.0, 0)));
    }

    #[test]
    fn instant_resume_cancels_the_pause() {
        let mut p = Pauses::default();
        p.pause(50);
        assert!(p.resume(50));
        assert!(!p.is_paused());
        assert_eq!(p.offset_at(60.0), Some(0));
        assert!(!p.resume(70), "resume without pause is a no-op");
    }
}
