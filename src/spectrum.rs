use crate::target::BANDS_HZ;
use realfft::{RealFftPlanner, RealToComplex, num_complex::Complex};
use std::{f32::consts::TAU, sync::Arc};

pub const FFT_SIZE: usize = 8192;
pub const HOP: usize = 4096;
pub const FLOOR_DB: f32 = -150.0;

pub struct Frame {
    pub bin_power: Vec<f32>,
    pub rms_db: f32,
    pub peak: f32,
}

pub struct Spectrum {
    sr: f32,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    norm: f32,
    buf: Vec<f32>,
}

impl Spectrum {
    pub fn new(sample_rate: f32) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        let sw2: f32 = window.iter().map(|w| w * w).sum();
        Self {
            sr: sample_rate,
            fft,
            window,
            norm: 1.0 / (FFT_SIZE as f32 * sw2),
            buf: Vec::new(),
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sr
    }

    pub fn push(&mut self, mono: &[f32]) -> Vec<Frame> {
        self.buf.extend_from_slice(mono);
        let mut frames = Vec::new();
        while self.buf.len() >= FFT_SIZE {
            frames.push(self.frame());
            self.buf.drain(..HOP);
        }
        frames
    }

    fn frame(&self) -> Frame {
        let win = &self.buf[..FFT_SIZE];
        let mut input: Vec<f32> = win.iter().zip(&self.window).map(|(x, w)| x * w).collect();
        let mut out = vec![Complex::new(0.0, 0.0); FFT_SIZE / 2 + 1];
        self.fft
            .process(&mut input, &mut out)
            .expect("fft size is fixed");
        let last = out.len() - 1;
        let bin_power = out
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let scale = if k == 0 || k == last { 1.0 } else { 2.0 };
                scale * c.norm_sqr() * self.norm
            })
            .collect();
        let hop = &win[FFT_SIZE - HOP..];
        let ms = hop.iter().map(|x| x * x).sum::<f32>() / HOP as f32;
        let peak = hop.iter().fold(0.0, |m: f32, x| m.max(x.abs()));
        Frame {
            bin_power,
            rms_db: db(ms),
            peak,
        }
    }
}

pub fn db(power: f32) -> f32 {
    (10.0 * power.log10()).max(FLOOR_DB)
}

/// Sum of bin power with centre frequency in [lo_hz, hi_hz).
pub fn band_db(bin_power: &[f32], sample_rate: f32, lo_hz: f32, hi_hz: f32) -> f32 {
    let bw = sample_rate / FFT_SIZE as f32;
    let sum: f32 = bin_power
        .iter()
        .enumerate()
        .filter(|&(k, _)| (lo_hz..hi_hz).contains(&(k as f32 * bw)))
        .map(|(_, p)| p)
        .sum();
    db(sum)
}

pub fn third_octave_db(bin_power: &[f32], sample_rate: f32) -> [f32; 25] {
    let edge = 2f32.powf(1.0 / 6.0);
    BANDS_HZ.map(|fc| band_db(bin_power, sample_rate, fc / edge, fc * edge))
}

/// dB to subtract from a third-octave band level to compare with a display point.
pub fn display_to_band_db(hz: f32, sample_rate: f32, per_oct: usize) -> f32 {
    let w3 = hz * (2f32.powf(1.0 / 6.0) - 2f32.powf(-1.0 / 6.0));
    let h = 2f32.powf(0.5 / per_oct as f32);
    let wn = hz * (h - 1.0 / h);
    10.0 * (w3 / wn.max(sample_rate / FFT_SIZE as f32)).log10()
}

/// 20 Hz upward at `per_oct` points per octave, below 20 kHz.
pub fn display_freqs(per_oct: usize) -> Vec<f32> {
    let n = per_oct as f32;
    (0..=(n * 1000f32.log2()).floor() as usize)
        .map(|i| 20.0 * 2f32.powf(i as f32 / n))
        .collect()
}

/// `smooth_n` > 0 averages each point over ±1/(2N) octave (cos² weights, power domain)
/// and scales by the point's band bin count; 0 keeps the plain whole-bin sum.
pub fn display_db(bin_power: &[f32], sample_rate: f32, per_oct: usize, smooth_n: usize) -> Vec<f32> {
    let half = 2f32.powf(0.5 / per_oct as f32);
    let bw = sample_rate / FFT_SIZE as f32;
    let last = bin_power.len() - 1;
    display_freqs(per_oct)
        .into_iter()
        .map(|f| {
            if smooth_n > 0 {
                let hw = (f * (2f32.powf(0.5 / smooth_n as f32) - 1.0)).max(bw);
                let k_lo = ((f - hw) / bw).ceil().max(0.0) as usize;
                let k_hi = (((f + hw) / bw).floor() as usize).min(last);
                let (mut num, mut den) = (0.0f32, 0.0f32);
                for (k, &p) in bin_power.iter().enumerate().take(k_hi + 1).skip(k_lo) {
                    let c = (std::f32::consts::FRAC_PI_2 * (k as f32 * bw - f) / hw).cos();
                    num += c * c * p;
                    den += c * c;
                }
                let h = 2f32.powf(0.5 / per_oct as f32);
                return db(num / den * (f * (h - 1.0 / h) / bw).max(1.0));
            }
            let k_lo = (f / half / bw).ceil() as usize;
            let k_hi = ((f * half / bw).floor() as usize).min(last);
            if k_lo <= k_hi {
                db(bin_power[k_lo..=k_hi].iter().sum())
            } else {
                db(bin_power[((f / bw).round() as usize).min(last)])
            }
        })
        .collect()
}

/// Narrow bumps of the analysis curve: (Hz, dB above the 1-octave average), largest first, at most 5.
/// A point counts when its 1/6-octave curve is >= 3 dB over the 1-octave curve and the highest
/// within ±1/6 octave, 100 Hz–10 kHz.
pub fn peaks(bin_power: &[f32], sample_rate: f32, per_oct: usize) -> Vec<(f32, f32)> {
    let fine = display_db(bin_power, sample_rate, per_oct, 6);
    let wide = display_db(bin_power, sample_rate, per_oct, 1);
    let r = (per_oct / 6).max(1);
    let mut out: Vec<(f32, f32)> = display_freqs(per_oct)
        .into_iter()
        .enumerate()
        .filter(|&(i, f)| {
            let lo = i.saturating_sub(r);
            let hi = (i + r).min(fine.len() - 1);
            (100.0..=10_000.0).contains(&f)
                && fine[i] - wide[i] >= 3.0
                && (lo..i).all(|j| fine[j] < fine[i])
                && (i + 1..=hi).all(|j| fine[j] <= fine[i])
        })
        .map(|(i, f)| (f, fine[i] - wide[i]))
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out.truncate(5);
    out
}

pub struct DisplayCurve {
    pub smooth_db: Vec<f32>,
    pub peak_db: Vec<f32>,
}

impl DisplayCurve {
    pub fn new(len: usize) -> Self {
        Self {
            smooth_db: vec![FLOOR_DB; len],
            peak_db: vec![FLOOR_DB; len],
        }
    }

    pub fn update(&mut self, frame_db: &[f32], dt: f32, tau_s: f32) {
        let a = 1.0 - (-dt / tau_s).exp();
        for (i, &f) in frame_db.iter().enumerate() {
            let (s, p) = (10f32.powf(self.smooth_db[i] / 10.0), 10f32.powf(f / 10.0));
            self.smooth_db[i] = db(s + a * (p - s));
            self.peak_db[i] = self.peak_db[i].max(f);
        }
    }

    pub fn reset_peak(&mut self) {
        self.peak_db.fill(FLOOR_DB);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{target::BANDS_HZ, testsig::*};

    /// Flat power keeps the display scale: mean power × the display band's (fractional, ≥ 1) bin count.
    /// (Not compared to `smooth_n = 0`: that one counts whole bins, which is the sawtooth being removed.)
    #[test]
    fn smoothing_keeps_flat_level() {
        let h = 2f32.powf(0.5 / 48.0);
        for sr in [48_000.0f32, 44_100.0] {
            let flat = vec![1e-6f32; FFT_SIZE / 2 + 1];
            let bw = sr / FFT_SIZE as f32;
            for n in [24, 12, 6, 3] {
                for (f, d) in display_freqs(48).iter().zip(display_db(&flat, sr, 48, n)) {
                    let want = db(1e-6 * (f * (h - 1.0 / h) / bw).max(1.0));
                    assert!((d - want).abs() < 0.01, "sr {sr} n {n} f {f}: {d} vs {want}");
                }
            }
        }
    }

    #[test]
    fn smoothing_removes_low_steps() {
        let sr = 48_000.0;
        let rising: Vec<f32> = (0..=FFT_SIZE / 2).map(|k| 1e-6 * (1.0 + k as f32)).collect();
        let fs = display_freqs(48);
        let low: Vec<usize> = (0..fs.len()).filter(|&i| fs[i] > 40.0 && fs[i] < 150.0).collect();
        let none = display_db(&rising, sr, 48, 0);
        assert!(low.windows(2).any(|w| none[w[0]] == none[w[1]]), "old method has steps");
        let six = display_db(&rising, sr, 48, 6);
        assert!(low.windows(2).all(|w| six[w[1]] > six[w[0]]), "smoothed rises every point");
    }

    #[test]
    fn display_to_band_db_matches_widths() {
        let near = |a: f32, b: f32| assert!((a - b).abs() < 0.05, "{a} vs {b}");
        near(display_to_band_db(1000.0, 48000.0, 24), 9.03);
        near(display_to_band_db(1000.0, 48000.0, 12), 6.03);
        near(display_to_band_db(1000.0, 48000.0, 48), 12.05);
        near(display_to_band_db(1000.0, 48000.0, 96), 15.06);
        near(
            display_to_band_db(63.0, 48000.0, 24),
            10.0 * (63.0f32 * 0.23156 / 5.859375).log10(),
        );
        near(
            display_to_band_db(63.0, 96000.0, 24),
            10.0 * (63.0f32 * 0.23156 / 11.71875).log10(),
        );
    }
    #[test]
    fn target_shift_tracks_white_noise() {
        let sr = 48_000.0;
        let frames = Spectrum::new(sr).push(&noise(48_000 * 5, 0.5));
        let n = frames.len() as f32;
        let mut disp = vec![0.0f32; 240];
        let mut bands = [0.0f32; 25];
        for f in &frames {
            for (a, d) in disp.iter_mut().zip(display_db(&f.bin_power, sr, 24, 0)) {
                *a += 10f32.powf(d / 10.0) / n;
            }
            for (a, d) in bands.iter_mut().zip(third_octave_db(&f.bin_power, sr)) {
                *a += 10f32.powf(d / 10.0) / n;
            }
        }
        let freqs = display_freqs(24);
        for hz in [63.0, 125.0, 1000.0, 4000.0] {
            let i = BANDS_HZ.iter().position(|&b| b == hz).unwrap();
            let j = (0..240)
                .min_by(|&a, &b| (freqs[a] - hz).abs().total_cmp(&(freqs[b] - hz).abs()))
                .unwrap();
            let got = 10.0 * disp[j].log10();
            let want = 10.0 * bands[i].log10() - display_to_band_db(hz, sr, 24);
            println!(
                "{hz} Hz: point {} Hz display {got:.2} expected {want:.2} diff {:.2}",
                freqs[j],
                got - want
            );
            assert!((got - want).abs() < 1.5, "{hz}: {got} vs {want}");
        }
    }
    #[test]
    fn sine_reads_rms_level_at_any_rate() {
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            let f = Spectrum::new(sr)
                .push(&sine(sr, 1000.0, 0.5, 1.0))
                .pop()
                .unwrap();
            let bands = third_octave_db(&f.bin_power, sr);
            let i = BANDS_HZ.iter().position(|&b| b == 1000.0).unwrap();
            assert!((bands[i] + 9.03).abs() < 0.5, "{sr}: {}", bands[i]);
            assert!(bands.iter().all(|&b| b <= bands[i]));
            assert!((f.rms_db + 9.03).abs() < 0.1);
            assert!((f.peak - 0.5).abs() < 1e-3);
        }
    }
    #[test]
    fn noise_power_sums_to_mean_square() {
        let x = noise(48_000, 0.5);
        let ms = x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32;
        let f = Spectrum::new(48_000.0).push(&x).pop().unwrap();
        assert!((db(f.bin_power.iter().sum()) - db(ms)).abs() < 0.5);
    }
    #[test]
    fn silence_is_finite_floor() {
        let f = Spectrum::new(48_000.0)
            .push(&vec![0.0; FFT_SIZE])
            .pop()
            .unwrap();
        assert!(
            third_octave_db(&f.bin_power, 48_000.0)
                .iter()
                .all(|&b| b == FLOOR_DB)
        );
        assert_eq!(f.rms_db, FLOOR_DB);
        assert!(
            display_db(&f.bin_power, 48_000.0, 24, 0)
                .iter()
                .all(|b| b.is_finite())
        );
    }
    #[test]
    fn one_frame_per_hop_across_chunks() {
        let mut s = Spectrum::new(48_000.0);
        let mut n = 0;
        for _ in 0..8 {
            n += s.push(&vec![0.1; 1024]).len();
        }
        assert_eq!(n, 1);
        n += s.push(&vec![0.1; HOP]).len();
        assert_eq!(n, 2);
    }
    #[test]
    fn display_point_counts_per_resolution() {
        for (n, len) in [(12, 120), (24, 240), (48, 479), (96, 957)] {
            let f = display_freqs(n);
            assert_eq!(f.len(), len, "{n}");
            assert_eq!(f[0], 20.0);
            assert!((f[n] - 40.0).abs() < 1e-2, "{n}");
            assert!(*f.last().unwrap() < 20_000.0);
        }
    }
    #[test]
    fn display_of_mean_bins_equals_mean_of_displays() {
        let sr = 48_000.0;
        let a = Spectrum::new(sr).push(&sine(sr, 1000.0, 0.5, 1.0)).pop().unwrap().bin_power;
        let b = Spectrum::new(sr).push(&noise(48_000, 0.3)).pop().unwrap().bin_power;
        let mean: Vec<f32> = a.iter().zip(&b).map(|(x, y)| (x + y) / 2.0).collect();
        let (da, db_, dm) = (display_db(&a, sr, 48, 0), display_db(&b, sr, 48, 0), display_db(&mean, sr, 48, 0));
        for i in 0..dm.len() {
            let want = db((10f32.powf(da[i] / 10.0) + 10f32.powf(db_[i] / 10.0)) / 2.0);
            assert!((dm[i] - want).abs() < 0.01, "{i}: {} vs {want}", dm[i]);
        }
    }
    #[test]
    fn display_point_holds_tone_power() {
        let sr = 48_000.0;
        let hz = 20.0 * 2f32.powf(136.0 / 24.0);
        let f = Spectrum::new(sr)
            .push(&sine(sr, hz, 0.5, 1.0))
            .pop()
            .unwrap();
        let d = display_db(&f.bin_power, sr, 24, 0);
        assert!((d[136] + 9.03).abs() < 1.0, "{}", d[136]);
    }
    #[test]
    fn low_points_without_bins_use_nearest_bin() {
        let sr = 48_000.0;
        let f = Spectrum::new(sr)
            .push(&sine(sr, 50.0, 0.5, 1.0))
            .pop()
            .unwrap();
        let d = display_db(&f.bin_power, sr, 24, 0);
        assert!(d.iter().all(|v| v.is_finite()));
        assert!(d[32] > -15.0, "{}", d[32]); // 20*2^(32/24) = 50.4 Hz
        let hi = display_freqs(24).iter().position(|&x| x > 200.0).unwrap();
        assert!(d[hi..].iter().all(|&v| v < -40.0));
    }
    #[test]
    fn peak_holds_until_reset() {
        let mut d = DisplayCurve::new(1);
        d.update(&[-10.0], 0.1, 1.0);
        for _ in 0..100 {
            d.update(&[-50.0], 0.1, 1.0);
        }
        assert_eq!(d.peak_db[0], -10.0);
        d.reset_peak();
        assert_eq!(d.peak_db[0], FLOOR_DB);
        d.update(&[-50.0], 0.1, 1.0);
        assert_eq!(d.peak_db[0], -50.0);
    }
    #[test]
    fn ema_tau_sets_speed() {
        let step = |tau: f32| {
            let mut d = DisplayCurve::new(1);
            for _ in 0..10 {
                d.update(&[-10.0], 0.1, tau);
            }
            d.smooth_db[0]
        };
        assert!((step(0.3) + 10.16).abs() < 0.1, "{}", step(0.3));
        assert!((step(3.0) + 15.47).abs() < 0.1, "{}", step(3.0));
    }

    fn shaped(gain_db: impl Fn(f32) -> f32) -> Vec<f32> {
        let bw = 48_000.0 / FFT_SIZE as f32;
        (0..=FFT_SIZE / 2).map(|k| 1e-6 * 10f32.powf(gain_db((k as f32 * bw).max(1.0)) / 10.0)).collect()
    }

    #[test]
    fn flat_has_no_peaks() {
        assert!(peaks(&shaped(|_| 0.0), 48_000.0, 48).is_empty());
    }

    #[test]
    fn narrow_bump_is_a_peak() {
        let p = peaks(&shaped(|f| 10.0 * (-(f / 1000.0).log2().powi(2) / (2.0 * 0.04f32.powi(2))).exp()), 48_000.0, 48);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!((p[0].0 / 1000.0).log2().abs() < 1.0 / 12.0 && p[0].1 >= 3.0, "{p:?}");
    }

    #[test]
    fn broad_rise_is_not_a_peak() {
        let p = peaks(&shaped(|f| 8.0 * (-(f / 3200.0).log2().powi(2) / (2.0 * 0.5f32.powi(2))).exp()), 48_000.0, 48);
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn at_most_five_largest_first() {
        let centres = [150.0f32, 300.0, 600.0, 1200.0, 2400.0, 4800.0, 9000.0];
        let p = peaks(&shaped(|f| centres.iter().enumerate()
            .map(|(i, c)| (4.0 + i as f32) * (-(f / c).log2().powi(2) / (2.0 * 0.04f32.powi(2))).exp())
            .sum()), 48_000.0, 48);
        assert_eq!(p.len(), 5, "{p:?}");
        assert!(p.windows(2).all(|w| w[0].1 >= w[1].1), "{p:?}");
    }
}
