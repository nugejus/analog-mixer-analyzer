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

pub fn display_freqs() -> Vec<f32> {
    (0..=60).map(|i| 20.0 * 2f32.powf(i as f32 / 6.0)).collect()
}

pub fn display_db(bin_power: &[f32], sample_rate: f32) -> Vec<f32> {
    let half = 2f32.powf(1.0 / 12.0);
    let bw = sample_rate / FFT_SIZE as f32;
    display_freqs()
        .into_iter()
        .map(|f| {
            let (lo, hi) = (f / half, f * half);
            let mut hits = bin_power
                .iter()
                .enumerate()
                .filter(|&(k, _)| (lo..=hi).contains(&(k as f32 * bw)))
                .map(|(_, p)| *p)
                .peekable();
            if hits.peek().is_some() {
                db(hits.sum())
            } else {
                let k = ((f / bw).round() as usize).min(bin_power.len() - 1);
                db(bin_power[k])
            }
        })
        .collect()
}

const TAU_SMOOTH: f32 = 0.3;
const PEAK_HOLD_S: f32 = 1.5;
const PEAK_FALL_DB_S: f32 = 12.0;

pub struct DisplayCurve {
    pub smooth_db: Vec<f32>,
    pub peak_db: Vec<f32>,
    held_s: Vec<f32>,
}

impl DisplayCurve {
    pub fn new(len: usize) -> Self {
        Self {
            smooth_db: vec![FLOOR_DB; len],
            peak_db: vec![FLOOR_DB; len],
            held_s: vec![0.0; len],
        }
    }

    pub fn update(&mut self, frame_db: &[f32], dt: f32) {
        let a = 1.0 - (-dt / TAU_SMOOTH).exp();
        for (i, &f) in frame_db.iter().enumerate() {
            let (s, p) = (10f32.powf(self.smooth_db[i] / 10.0), 10f32.powf(f / 10.0));
            self.smooth_db[i] = db(s + a * (p - s));
            if f >= self.peak_db[i] {
                self.peak_db[i] = f;
                self.held_s[i] = 0.0;
            } else {
                self.held_s[i] += dt;
                if self.held_s[i] > PEAK_HOLD_S {
                    self.peak_db[i] = (self.peak_db[i] - PEAK_FALL_DB_S * dt).max(f);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{target::BANDS_HZ, testsig::*};

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
            display_db(&f.bin_power, 48_000.0)
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
    fn peak_hold_then_decay() {
        let mut d = DisplayCurve::new(1);
        d.update(&[-10.0], 0.1);
        for _ in 0..10 {
            d.update(&[-50.0], 0.1);
        }
        assert!((d.peak_db[0] + 10.0).abs() < 0.01);
        for _ in 0..14 {
            d.update(&[-50.0], 0.1);
        }
        assert!((d.peak_db[0] + 21.0).abs() < 1.5);
        assert!(d.smooth_db[0] < -40.0);
    }
}
