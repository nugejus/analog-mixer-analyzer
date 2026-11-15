use crate::spectrum::{FLOOR_DB, db};
use std::collections::VecDeque;

const CLIP_LEVEL: f32 = 0.999;
const CLIP_RUN: u32 = 3;
const HIST_MAX: usize = 300;
const SILENT_DB: f32 = -140.0;

#[derive(Clone, Copy)]
pub struct HistPoint {
    pub peak_db: f32,
    pub clipped: bool,
}

pub struct LevelMeter {
    channels: usize,
    block: usize,
    runs: Vec<u32>,
    clips: u32,
    hist: VecDeque<HistPoint>,
    frames: usize,
    peak: f32,
    clipped: bool,
}

impl LevelMeter {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        Self {
            channels: channels.max(1),
            block: ((sample_rate / 10.0) as usize).max(1),
            runs: vec![0; channels.max(1)],
            clips: 0,
            hist: VecDeque::new(),
            frames: 0,
            peak: 0.0,
            clipped: false,
        }
    }

    pub fn push_interleaved(&mut self, data: &[f32]) {
        for frame in data.chunks_exact(self.channels) {
            for (run, &x) in self.runs.iter_mut().zip(frame) {
                let a = x.abs();
                self.peak = self.peak.max(a);
                if a >= CLIP_LEVEL {
                    *run += 1;
                    if *run == CLIP_RUN {
                        self.clips += 1;
                        self.clipped = true;
                    }
                } else {
                    *run = 0;
                }
            }
            self.frames += 1;
            if self.frames == self.block {
                if self.hist.len() == HIST_MAX {
                    self.hist.pop_front();
                }
                self.hist.push_back(HistPoint {
                    peak_db: db(self.peak * self.peak),
                    clipped: self.clipped,
                });
                (self.frames, self.peak, self.clipped) = (0, 0.0, false);
            }
        }
    }

    pub fn peak_db(&self) -> f32 {
        self.hist.back().map_or(FLOOR_DB, |p| p.peak_db)
    }

    pub fn clip_count(&self) -> u32 {
        self.clips
    }

    pub fn reset_clips(&mut self) {
        self.clips = 0;
    }

    pub fn history(&self) -> &VecDeque<HistPoint> {
        &self.hist
    }

    fn recent(&self, seconds: f32) -> impl Iterator<Item = &HistPoint> {
        let n = (seconds * 10.0).round() as usize;
        self.hist.iter().skip(self.hist.len().saturating_sub(n))
    }

    pub fn clipped_within(&self, seconds: f32) -> bool {
        self.recent(seconds).any(|p| p.clipped)
    }

    pub fn silent_for(&self, seconds: f32) -> bool {
        let n = (seconds * 10.0).round() as usize;
        self.hist.len() >= n && self.recent(seconds).all(|p| p.peak_db < SILENT_DB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_needs_three_in_a_row() {
        let count = |x: &[f32]| {
            let mut m = LevelMeter::new(48_000.0, 1);
            m.push_interleaved(x);
            m.clip_count()
        };
        assert_eq!(count(&[0.0, 1.0, 1.0, 1.0, 0.0]), 1);
        assert_eq!(count(&[1.0, 1.0, 0.0, 1.0, 1.0]), 0);
        assert_eq!(count(&[-0.9995, -0.9995, -0.9995]), 1);
        assert_eq!(count(&[1.0; 10]), 1);
    }
    #[test]
    fn clip_run_spans_buffers_and_channels() {
        let mut m = LevelMeter::new(48_000.0, 1);
        m.push_interleaved(&[1.0, 1.0]);
        m.push_interleaved(&[1.0]);
        assert_eq!(m.clip_count(), 1);
        let mut s = LevelMeter::new(48_000.0, 2);
        s.push_interleaved(&[0.0, 1.0, 0.0, 1.0, 0.0, 1.0]);
        assert_eq!(s.clip_count(), 1);
        let mut t = LevelMeter::new(48_000.0, 2);
        t.push_interleaved(&[1.0, 0.0, 0.0, 1.0, 1.0, 0.0]);
        assert_eq!(t.clip_count(), 0);
    }
    #[test]
    fn history_is_100ms_points_capped_at_30s() {
        let mut m = LevelMeter::new(48_000.0, 1);
        m.push_interleaved(&vec![0.5; 48_000]);
        assert_eq!(m.history().len(), 10);
        assert!((m.peak_db() + 6.02).abs() < 0.05);
        m.push_interleaved(&vec![0.5; 48_000 * 31]);
        assert_eq!(m.history().len(), 300);
    }
    #[test]
    fn clip_recency_reset_and_silence() {
        let mut m = LevelMeter::new(48_000.0, 1);
        m.push_interleaved(&[1.0; 3]);
        m.push_interleaved(&vec![0.0; 4800]);
        assert!(m.clipped_within(4.0));
        m.push_interleaved(&vec![0.0; 48_000 * 5]);
        assert!(!m.clipped_within(4.0));
        assert!(m.silent_for(3.0));
        m.reset_clips();
        assert_eq!(m.clip_count(), 0);
        m.push_interleaved(&vec![0.1; 4800]);
        assert!(!m.silent_for(3.0));
    }
}
