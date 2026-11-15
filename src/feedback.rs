use crate::spectrum::{FFT_SIZE, db};

const RUN: u32 = 6;

pub struct FeedbackDetector {
    lo: usize,
    hi: usize,
    bw: f32,
    k: usize,
    k_db: f32,
    run: u32,
    miss: u32,
    active: bool,
}

impl FeedbackDetector {
    pub fn new(sample_rate: f32) -> Self {
        let bw = sample_rate / FFT_SIZE as f32;
        Self {
            lo: (100.0 / bw).ceil() as usize,
            hi: (10_000.0 / bw) as usize,
            bw,
            k: 0,
            k_db: 0.0,
            run: 0,
            miss: 0,
            active: false,
        }
    }

    /// Returns the suspected howl frequency (Hz) if one is currently held.
    pub fn push(&mut self, bin_power: &[f32]) -> Option<f32> {
        let hi = self.hi.min(bin_power.len().saturating_sub(13));
        let peak = (self.lo..=hi).max_by(|&a, &b| bin_power[a].total_cmp(&bin_power[b]));
        let mut broken = true;
        if let Some(k) = peak
            && k >= 12
            && self.is_howl_like(bin_power, k)
        {
            let d = db(bin_power[k]);
            let steady = self.run > 0 && k.abs_diff(self.k) <= 2 && (d - self.k_db).abs() <= 2.0;
            self.run = if steady { self.run + 1 } else { 1 };
            (self.k, self.k_db) = (k, d);
            broken = !steady;
        }
        if broken {
            self.miss += 1;
            if self.miss >= RUN {
                self.run = 0;
                self.active = false;
            }
        } else {
            self.miss = 0;
            self.active |= self.run >= RUN;
        }
        self.active.then_some(self.k as f32 * self.bw)
    }

    fn is_howl_like(&self, p: &[f32], k: usize) -> bool {
        let pk = db(p[k]);
        let side: f32 = (4..=12).flat_map(|o| [p[k - o], p[k + o]]).sum::<f32>() / 18.0;
        let harm = [2 * k, 3 * k]
            .into_iter()
            .filter(|&h| h + 1 < p.len())
            .map(|h| db(p[h - 1].max(p[h]).max(p[h + 1])))
            .fold(f32::NEG_INFINITY, f32::max);
        pk - db(side) >= 15.0 && pk - harm >= 20.0 && pk > -60.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectrum::Spectrum;
    use crate::testsig::{noise, sine};
    use std::f32::consts::TAU;

    fn run(x: &[f32]) -> Vec<Option<f32>> {
        let mut sp = Spectrum::new(48_000.0);
        let mut fd = FeedbackDetector::new(48_000.0);
        sp.push(x).iter().map(|f| fd.push(&f.bin_power)).collect()
    }
    fn mix(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b).map(|(x, y)| x + y).collect()
    }
    fn voiced(sr: f32, secs: f32) -> Vec<f32> {
        (0..(sr * secs) as usize)
            .map(|i| {
                let t = i as f32 / sr;
                let phase = TAU * 120.0 * (t - 0.05 / (TAU * 5.0) * (TAU * 5.0 * t).cos());
                (1..=10)
                    .map(|h| (0.1 / h as f32) * (h as f32 * phase).sin())
                    .sum()
            })
            .collect()
    }
    #[test]
    fn steady_tone_is_feedback() {
        let x = mix(&sine(48_000.0, 2500.0, 0.3, 1.5), &noise(72_000, 0.001));
        let hz = run(&x).last().copied().flatten().expect("detected");
        assert!((hz - 2500.0).abs() < 25.0);
    }
    #[test]
    fn noise_is_not_feedback() {
        assert!(run(&noise(96_000, 0.3)).iter().all(Option::is_none));
    }
    #[test]
    fn voiced_vibrato_is_not_feedback() {
        let x = voiced(48_000.0, 2.0);
        assert!(run(&x).iter().all(Option::is_none));
    }
    #[test]
    fn released_after_tone_stops() {
        let mut x = mix(&sine(48_000.0, 2500.0, 0.3, 1.5), &noise(72_000, 0.001));
        x.extend(noise(48_000, 0.001));
        assert_eq!(*run(&x).last().unwrap(), None);
    }
}
