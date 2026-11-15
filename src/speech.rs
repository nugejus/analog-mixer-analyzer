use crate::spectrum::{Frame, db};

#[derive(Clone, Debug)]
pub struct Summary {
    pub bands_db: [f32; 25],
    pub display_db: Vec<f32>,
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
    // (rms_db, peak_db, bands_db, display_db)
    frames: Vec<(f32, f32, [f32; 25], Vec<f32>)>,
}

const MIN_ACTIVE_S: f32 = 5.0;

fn pow(d: f32) -> f32 {
    10f32.powf(d / 10.0)
}

impl SpeechStats {
    pub fn new(hop_seconds: f32) -> Self {
        Self { hop: hop_seconds, frames: Vec::new() }
    }

    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    pub fn push(&mut self, frame: &Frame, bands_db: &[f32; 25], display_db: &[f32]) {
        let peak_db = db(frame.peak * frame.peak);
        self.frames.push((frame.rms_db, peak_db, *bands_db, display_db.to_vec()));
    }

    /// Judged at the end: the floor is the whole recording's minimum RMS.
    pub fn summary(&self) -> Option<Summary> {
        let floor = self.frames.iter().map(|f| f.0).fold(f32::INFINITY, f32::min);
        let act: Vec<_> = self
            .frames
            .iter()
            .filter(|f| f.0 > -60.0 && f.0 > floor + 12.0)
            .collect();
        let n = act.len();
        let active_seconds = n as f32 * self.hop;
        if active_seconds < MIN_ACTIVE_S {
            return None;
        }
        let nf = n as f32;
        let mut bands_db = [0.0; 25];
        for (b, out) in bands_db.iter_mut().enumerate() {
            *out = db(act.iter().map(|f| pow(f.2[b])).sum::<f32>() / nf);
        }
        let display_db = (0..act[0].3.len())
            .map(|i| db(act.iter().map(|f| pow(f.3[i])).sum::<f32>() / nf))
            .collect();
        let rms_db = db(act.iter().map(|f| pow(f.0)).sum::<f32>() / nf);
        let peak_db = act.iter().map(|f| f.1).fold(f32::NEG_INFINITY, f32::max);
        let mut r: Vec<f32> = act.iter().map(|f| f.0).collect();
        r.sort_by(f32::total_cmp);
        let at = |p: f32| r[(p * (n - 1) as f32).round() as usize];
        Some(Summary {
            bands_db,
            display_db,
            peak_db,
            rms_db,
            noise_floor_db: floor,
            snr_db: rms_db - floor,
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
    fn feed(s: &mut SpeechStats, n: usize, rms: f32, peak_db: f32, band0: f32, display: &[f32]) {
        for _ in 0..n {
            let mut b = [-100.0; 25];
            b[0] = band0;
            s.push(
                &Frame { bin_power: vec![], rms_db: rms, peak: 10f32.powf(peak_db / 20.0) },
                &b,
                display,
            );
        }
    }
    #[test]
    fn silence_gives_none() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 100, FLOOR_DB, FLOOR_DB, FLOOR_DB, &[]);
        assert!(s.summary().is_none());
    }
    #[test]
    fn speech_after_quiet_gives_summary() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 20, -70.0, -60.0, -90.0, &[]);
        for i in 0..60 {
            feed(&mut s, 1, if i % 2 == 0 { -20.0 } else { -30.0 }, -6.0, -10.0, &[]);
        }
        let m = s.summary().unwrap();
        assert!((m.noise_floor_db + 70.0).abs() < 0.01);
        assert!((m.peak_db + 6.0).abs() < 0.01);
        assert!((m.bands_db[0] + 10.0).abs() < 0.01);
        assert!((m.rms_db - 10.0 * (0.011f32 / 2.0).log10()).abs() < 0.1);
        assert!((m.snr_db - (m.rms_db + 70.0)).abs() < 0.01);
        assert!((m.crest_db - (m.peak_db - m.rms_db)).abs() < 0.01);
        assert!((m.active_seconds - 60.0 * HOP_S).abs() < 0.01);
    }
    #[test]
    fn under_5s_of_speech_gives_none() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 20, -70.0, -60.0, -90.0, &[]);
        feed(&mut s, 58, -20.0, -6.0, -10.0, &[]);
        assert!(s.summary().is_none());
    }
    #[test]
    fn activity_judged_against_whole_recording_floor() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 60, -20.0, -6.0, -10.0, &[]);
        feed(&mut s, 20, -70.0, -60.0, -90.0, &[]);
        let m = s.summary().unwrap();
        assert!((m.active_seconds - 60.0 * HOP_S).abs() < 0.01);
    }
    #[test]
    fn bands_are_power_averaged() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 20, -70.0, -60.0, -90.0, &[]);
        feed(&mut s, 30, -20.0, -6.0, -10.0, &[]);
        feed(&mut s, 30, -20.0, -6.0, -20.0, &[]);
        assert!((s.summary().unwrap().bands_db[0] - 10.0 * (0.11f32 / 2.0).log10()).abs() < 0.05);
    }
    #[test]
    fn display_is_power_averaged_over_active_frames() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 20, -70.0, -60.0, -90.0, &[-90.0, -90.0]);
        feed(&mut s, 30, -20.0, -6.0, -10.0, &[-10.0, -30.0]);
        feed(&mut s, 30, -20.0, -6.0, -10.0, &[-20.0, -30.0]);
        let d = s.summary().unwrap().display_db;
        assert!((d[0] - 10.0 * 0.055f32.log10()).abs() < 0.05);
        assert!((d[1] + 30.0).abs() < 0.05);
    }
    #[test]
    fn dynamic_range_is_p90_minus_p10() {
        let mut s = SpeechStats::new(HOP_S);
        feed(&mut s, 10, -70.0, -60.0, -90.0, &[]);
        for i in 0..=60 {
            feed(&mut s, 1, -40.0 + 0.5 * i as f32, -6.0, -10.0, &[]);
        }
        assert!((s.summary().unwrap().dynamic_range_db - 24.0).abs() < 1.5);
    }
}
