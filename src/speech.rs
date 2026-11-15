use crate::spectrum::{Frame, db};
use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Summary {
    pub bands_db: [f32; 25],
    pub peak_db: f32,
    pub rms_db: f32,
    pub noise_floor_db: f32,
    pub snr_db: f32,
    pub dynamic_range_db: f32,
    pub crest_db: f32,
    pub active_seconds: f32,
}

pub struct SpeechStats {
    hop: f32,
    win: usize,
    keep: usize,
    // (rms_db, peak_db, active, bands_db)
    frames: VecDeque<(f32, f32, bool, [f32; 25])>,
}

const MIN_ACTIVE_S: f32 = 1.5;

fn pow(d: f32) -> f32 {
    10f32.powf(d / 10.0)
}

impl SpeechStats {
    pub fn new(hop_seconds: f32) -> Self {
        Self {
            hop: hop_seconds,
            win: (4.0 / hop_seconds).round() as usize,
            keep: (10.0 / hop_seconds).round() as usize,
            frames: VecDeque::new(),
        }
    }

    fn floor(&self) -> f32 {
        self.frames.iter().map(|f| f.0).fold(f32::INFINITY, f32::min)
    }

    pub fn push(&mut self, frame: &Frame, bands_db: &[f32; 25]) {
        if self.frames.len() == self.keep {
            self.frames.pop_front();
        }
        let peak_db = db(frame.peak * frame.peak);
        // Floor includes the current frame, so the first frame is never active.
        self.frames.push_back((frame.rms_db, peak_db, false, *bands_db));
        let floor = self.floor();
        let last = self.frames.back_mut().expect("just pushed");
        last.2 = frame.rms_db > -60.0 && frame.rms_db > floor + 12.0;
    }

    pub fn summary(&self) -> Option<Summary> {
        let skip = self.frames.len().saturating_sub(self.win);
        let act: Vec<_> = self.frames.iter().skip(skip).filter(|f| f.2).collect();
        let n = act.len();
        let active_seconds = n as f32 * self.hop;
        if active_seconds < MIN_ACTIVE_S {
            return None;
        }
        let nf = n as f32;
        let mut bands_db = [0.0; 25];
        for (b, out) in bands_db.iter_mut().enumerate() {
            *out = db(act.iter().map(|f| pow(f.3[b])).sum::<f32>() / nf);
        }
        let rms_db = db(act.iter().map(|f| pow(f.0)).sum::<f32>() / nf);
        let peak_db = act.iter().map(|f| f.1).fold(f32::NEG_INFINITY, f32::max);
        let noise_floor_db = self.floor();
        let mut r: Vec<f32> = act.iter().map(|f| f.0).collect();
        r.sort_by(f32::total_cmp);
        let at = |p: f32| r[(p * (n - 1) as f32).round() as usize];
        Some(Summary {
            bands_db,
            peak_db,
            rms_db,
            noise_floor_db,
            snr_db: rms_db - noise_floor_db,
            dynamic_range_db: at(0.9) - at(0.1),
            crest_db: peak_db - rms_db,
            active_seconds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectrum::FLOOR_DB;

    const HOP_S: f32 = 4096.0 / 48_000.0;
    fn feed(s: &mut SpeechStats, n: usize, rms: f32, peak_db: f32, band0: f32) {
        for _ in 0..n {
            let mut b = [-100.0; 25];
            b[0] = band0;
            s.push(
                &Frame { bin_power: vec![], rms_db: rms, peak: 10f32.powf(peak_db / 20.0) },
                &b,
            );
        }
    }
    #[test]
    fn silence_gives_none() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 100, FLOOR_DB, FLOOR_DB, FLOOR_DB);
        assert!(s.summary().is_none());
    }
    #[test]
    fn speech_after_quiet_gives_summary() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 20, -70.0, -60.0, -90.0);
        for i in 0..30 {
            feed(&mut s, 1, if i % 2 == 0 { -20.0 } else { -30.0 }, -6.0, -10.0);
        }
        let m = s.summary().unwrap();
        assert!((m.noise_floor_db + 70.0).abs() < 0.01);
        assert!((m.peak_db + 6.0).abs() < 0.01);
        assert!((m.bands_db[0] + 10.0).abs() < 0.01);
        assert!((m.rms_db - 10.0 * (0.011f32 / 2.0).log10()).abs() < 0.1);
        assert!((m.snr_db - (m.rms_db + 70.0)).abs() < 0.01);
        assert!((m.crest_db - (m.peak_db - m.rms_db)).abs() < 0.01);
        assert!((m.active_seconds - 30.0 * HOP_S).abs() < 0.01);
    }
    #[test]
    fn only_last_4s_counts() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 20, -70.0, -60.0, -90.0);
        feed(&mut s, 30, -20.0, -6.0, -10.0);
        feed(&mut s, 60, -70.0, -60.0, -90.0);
        assert!(s.summary().is_none());
    }
    #[test]
    fn bands_are_power_averaged() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 20, -70.0, -60.0, -90.0);
        feed(&mut s, 15, -20.0, -6.0, -10.0);
        feed(&mut s, 15, -20.0, -6.0, -20.0);
        assert!((s.summary().unwrap().bands_db[0] - 10.0 * (0.11f32 / 2.0).log10()).abs() < 0.05);
    }
    #[test]
    fn dynamic_range_is_p90_minus_p10() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 10, -70.0, -60.0, -90.0);
        for i in 0..31 {
            feed(&mut s, 1, -40.0 + i as f32, -6.0, -10.0);
        }
        assert!((s.summary().unwrap().dynamic_range_db - 24.0).abs() < 1.5);
    }
}
