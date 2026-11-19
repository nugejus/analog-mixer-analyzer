//! Knob recommendation engine (spec "Recommendation engine").

use crate::mixers::{ChannelGroup, Control, EQ_RANGE_DB, KNOB_TRAVEL_DEG};
use crate::speech::Summary;
use crate::target::{BANDS_HZ, TONE_BANDS, TONE_HOT_DB, hpf_db, level_match_db, tone_excess};

const LAMBDA: f32 = 0.01;
/// λ for the restore low-cut decision. Larger penalizes LOW and low MID turning a lot to mimic the switch.
/// ponytail: picked by ±1 dB noise simulation (0.02 is noise-fragile, 0.05 is wrong with no noise). Re-pick once real recordings accumulate.
const SWITCH_LAMBDA: f32 = 0.03;
const FIT_BANDS: std::ops::RangeInclusive<usize> = 2..=23; // 100 Hz–12.5 kHz
const EQ_LIMIT_DB: f32 = 15.0;
const LOW_BOOST_MAX_DB: f32 = 3.0;
const HIGH_BOOST_MAX_DB: f32 = 6.0;
const OK_BELOW_DB: f32 = 1.5;
const OK_RELEASE_DB: f32 = 2.5;
/// ✓ range when restoring to the entered position: per knob max(OK_RELEASE_DB, NOISE_Z·σ). σ is that knob's gain std dev when TAKE_NOISE_DB
/// noise, independent per band, is on the difference d.
/// ponytail: picked by uniform ±1 dB noise simulation, not real recordings (a difference of two recordings, saved and today, so √(2/3) dB; Z is the
/// smallest value at which re-recording the same sound gives a non-✓ knob about 5% of the time per group kind (two sweeps excluded), higher
/// at times depending on K·FREQ). Re-pick once real recordings accumulate. To vary per recording, use the fit's weighted residual (third value of `fit_eq_in`).
const TAKE_NOISE_DB: f32 = 0.8;
const NOISE_Z: f32 = 2.75;
/// χ² 99.9% value per degrees of freedom (EQ knob count): if d² of the whole turn amount exceeds it, it is not noise.
const CHI2_999: [f32; 5] = [0.0, 10.83, 13.82, 16.27, 18.47];
pub const NO_INPUT: &str = "입력 신호가 없습니다. 케이블과 입력장치를 확인하세요. macOS에서는 시스템 설정 > 개인정보 보호 및 보안 > 마이크에서 이 앱을 허용하세요.";
const KEEP_TALKING: &str = "말소리가 부족합니다. [분석]을 다시 누르고 녹음이 끝날 때까지 평소처럼 말씀해 주세요.";
pub const OVERLOAD: &str = "과부하 의심: 아날로그 단에서 눌리거나 COMP가 과한 경우입니다.";

fn weight(b: usize) -> f32 {
    if b == 23 { 0.5 } else { 1.0 }
}

/// dB change at each frequency per 1 dB of knob.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    LowShelf(f32),
    HighShelf(f32),
    Bell(f32),
}

impl Shape {
    pub fn response(&self, hz: f32) -> f32 {
        match *self {
            Shape::LowShelf(fc) => 1.0 / (1.0 + (hz / fc).powi(2)),
            Shape::HighShelf(fc) => 1.0 / (1.0 + (fc / hz).powi(2)),
            Shape::Bell(fc) => (-(hz / fc).log2().powi(2) / (2.0 * 0.6 * 0.6)).exp(),
        }
    }
}

/// Minimizes Σ w(−d − c − Σ g·R)² + λΣg². Returns (gains, c, weighted residual Σ w·r²).
pub fn fit_eq(deviation: &[f32; 25], shapes: &[Shape]) -> (Vec<f32>, f32, f32) {
    fit_eq_in(deviation, shapes, FIT_BANDS, LAMBDA)
}

fn fit_eq_in(deviation: &[f32; 25], shapes: &[Shape], bands: std::ops::RangeInclusive<usize>, lambda: f32) -> (Vec<f32>, f32, f32) {
    let n = shapes.len() + 1;
    let row = |b: usize| -> Vec<f32> {
        std::iter::once(1.0)
            .chain(shapes.iter().map(|s| s.response(BANDS_HZ[b])))
            .collect()
    };
    // augmented matrix [AᵀWA + λD | AᵀWy], y = −d
    let mut m = vec![vec![0.0; n + 1]; n];
    for b in bands.clone() {
        let (a, w) = (row(b), weight(b));
        for (mi, &ai) in m.iter_mut().zip(&a) {
            for (mij, &aj) in mi.iter_mut().zip(&a) {
                *mij += w * ai * aj;
            }
            mi[n] -= w * ai * deviation[b];
        }
    }
    for (i, mi) in m.iter_mut().enumerate().skip(1) {
        mi[i] += lambda;
    }
    let mut x = solve(m);
    let resid = bands
        .map(|b| {
            let fit: f32 = row(b).iter().zip(&x).map(|(a, x)| a * x).sum();
            weight(b) * (-deviation[b] - fit).powi(2)
        })
        .sum();
    let c = x.remove(0);
    (x, c, resid)
}

/// x = M⁻¹y from the augmented matrix [M | y]. M is symmetric positive definite (Σw > 0, λ > 0 for the fit), so Gaussian elimination needs no pivoting.
fn solve(mut m: Vec<Vec<f32>>) -> Vec<f32> {
    let n = m.len();
    for k in 0..n {
        let (top, bot) = m.split_at_mut(k + 1);
        let p = &top[k];
        for r in bot {
            let f = r[k] / p[k];
            for (rj, &pj) in r.iter_mut().zip(p).skip(k) {
                *rj -= f * pj;
            }
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let s: f32 = (i + 1..n).map(|j| m[i][j] * x[j]).sum();
        x[i] = (m[i][n] - s) / m[i][i];
    }
    x
}

/// Covariance of the fitted gains under independent 1 dB noise per band: M⁻¹AᵀW²AM⁻¹, M = AᵀWA + λD.
/// The fit is linear in d, so sum the products of the gains from fits with a single 1 dB in one band each.
fn gain_cov(shapes: &[Shape]) -> Vec<Vec<f32>> {
    let mut cov = vec![vec![0.0; shapes.len()]; shapes.len()];
    for b in FIT_BANDS {
        let mut e = [0.0; 25];
        e[b] = 1.0;
        let g = fit_eq_in(&e, shapes, FIT_BANDS, LAMBDA).0;
        for (ci, gi) in cov.iter_mut().zip(&g) {
            for (cij, gj) in ci.iter_mut().zip(&g) {
                *cij += gi * gj;
            }
        }
    }
    cov
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Wait,
    Ok,
    /// Clockwise if degrees > 0.
    Rotate {
        degrees: f32,
        db: f32,
    },
    SetFreq {
        hz: f32,
    },
    SwitchOn,
}

#[derive(Clone, Debug)]
pub struct Advice {
    pub label: String,
    pub control: Control,
    pub action: Action,
}

pub struct AdviceSet {
    pub knobs: Vec<Advice>,
    pub notes: Vec<String>,
    /// −c (the c in the spec formula): measured ≈ target + level_offset_db. 0 if there is no recommendation.
    pub level_offset_db: f32,
}

pub struct Inputs<'a> {
    pub summary: Option<&'a Summary>,
    pub target: &'a [f32; 25],
    pub clipped_recently: bool,
    pub feedback_hz: Option<f32>,
    pub silent: bool,
    /// `Some(saved DR)` means restore mode.
    pub restore_dynamic_range_db: Option<f32>,
    /// Current knob positions. Center if `None` or not matching the group (`fits`).
    pub current: Option<&'a Positions>,
}

/// Rotation angle rounded to 5° steps.
pub fn rotate_degrees(db: f32, deg_per_db: f32) -> f32 {
    (db * deg_per_db / 5.0).round() * 5.0
}

/// Three significant digits: 400Hz, 2.5kHz, 12kHz, 1.25kHz.
pub fn fmt_hz(hz: f32) -> String {
    let (v, unit) = if hz >= 1000.0 { (hz / 1000.0, "kHz") } else { (hz, "Hz") };
    let p = 10f32.powi(2 - v.log10().floor() as i32);
    format!("{}{unit}", (v * p).round() / p)
}

fn sweep(c: Control) -> Option<(f32, f32)> {
    match c {
        Control::Mid { lo_hz, hi_hz } if lo_hz != hi_hz => Some((lo_hz, hi_hz)),
        _ => None,
    }
}

/// Labels in group order. `act(i, is_freq)` gives the action of controls[i].
fn knobs(group: &ChannelGroup, act: impl Fn(usize, bool) -> Action) -> Vec<Advice> {
    let sweeps = group.controls.iter().filter(|&&c| sweep(c).is_some()).count();
    let mut n = 0;
    let mut out = Vec::new();
    for (i, &control) in group.controls.iter().enumerate() {
        let mut push = |label: String, freq: bool| {
            out.push(Advice {
                label,
                control,
                action: act(i, freq),
            })
        };
        match control {
            Control::Gain { .. } => push("GAIN".into(), false),
            Control::LowCut { hz } => push(format!("저역컷 {}", fmt_hz(hz)), false),
            Control::HighShelf { hz } => push(format!("HIGH {}", fmt_hz(hz)), false),
            Control::LowShelf { hz } => push(format!("LOW {}", fmt_hz(hz)), false),
            Control::Comp => push("COMP".into(), false),
            Control::Mid { lo_hz, hi_hz } if lo_hz == hi_hz => push(format!("MID {}", fmt_hz(lo_hz)), false),
            Control::Mid { .. } => {
                // the factory inserts from the highest band, so 1, 2 in order.
                n += 1;
                let sfx = if sweeps > 1 { format!(" {n}") } else { String::new() };
                push(format!("FREQ{sfx}"), true);
                push(format!("MID{sfx}"), false);
            }
        }
    }
    out
}

/// Knob labels of the group (actions all `Wait`).
pub fn labels(group: &ChannelGroup) -> Vec<Advice> {
    knobs(group, |_, _| Action::Wait)
}

/// Knob positions: one value per label in `labels` (GAIN excluded), in order. EQ is dB from center, FREQ is Hz (0 = unknown),
/// low cut 1 on·0 off, COMP is degrees from the left end.
pub type Positions = Vec<(String, f32)>;

/// All knobs centered (EQ 0, low cut and COMP off, FREQ unknown).
pub fn centre(group: &ChannelGroup) -> Positions {
    labels(group).into_iter().filter(|k| !matches!(k.control, Control::Gain { .. })).map(|k| (k.label, 0.0)).collect()
}

/// True if the labels are exactly the same as the group's, in the same order.
pub fn fits(group: &ChannelGroup, p: &Positions) -> bool {
    p.iter().map(|e| &e.0).eq(centre(group).iter().map(|e| &e.0))
}

/// FREQ candidates for a sweep MID (1/3-octave nominal values within range). Empty list if not a sweep.
pub fn freq_choices(control: Control) -> Vec<f32> {
    sweep(control).map_or(vec![], |(lo, hi)| BANDS_HZ.into_iter().filter(|f| (lo..=hi).contains(f)).collect())
}

/// In controls order (value, FREQ Hz): EQ dB, low cut, COMP value; for a sweep MID its FREQ too. All 0 if not matching or absent.
fn per_control(group: &ChannelGroup, p: Option<&Positions>) -> Vec<(f32, f32)> {
    let mut v = p.filter(|p| fits(group, p)).into_iter().flatten().map(|e| e.1);
    group
        .controls
        .iter()
        .map(|&c| match c {
            Control::Gain { .. } => (0.0, 0.0),
            c if sweep(c).is_some() => {
                let hz = v.next().unwrap_or(0.0);
                (v.next().unwrap_or(0.0), hz)
            }
            _ => (v.next().unwrap_or(0.0), 0.0),
        })
        .collect()
}

/// Positions after following the recommendation (GAIN excluded). ✓ and Wait keep the value from `from`.
pub fn targets(knobs: &[Advice], from: &Positions) -> Positions {
    knobs
        .iter()
        .filter(|k| !matches!(k.control, Control::Gain { .. }))
        .map(|k| {
            let was = from.iter().find(|e| e.0 == k.label).map_or(0.0, |e| e.1);
            let v = match k.action {
                Action::Rotate { degrees, .. } if k.control == Control::Comp => was + degrees,
                Action::Rotate { db, .. } => db,
                Action::SetFreq { hz } => hz,
                Action::SwitchOn => 1.0,
                _ => was,
            };
            (k.label.clone(), v)
        })
        .collect()
}

/// Saved positions as actions to draw from center (GAIN and centered knobs are ✓).
pub fn as_advice(group: &ChannelGroup, p: &Positions) -> Vec<Advice> {
    let k = per_control(group, Some(p));
    knobs(group, |i, freq| {
        let (v, hz) = k[i];
        match group.controls[i] {
            Control::Gain { .. } => Action::Ok,
            _ if freq => if v != 0.0 && hz > 0.0 { Action::SetFreq { hz } } else { Action::Ok },
            Control::LowCut { .. } if v > 0.0 => Action::SwitchOn,
            Control::Comp if v > 0.0 => Action::Rotate { degrees: v, db: 0.0 },
            Control::HighShelf { .. } | Control::Mid { .. } | Control::LowShelf { .. } if v != 0.0 => Action::Rotate {
                degrees: rotate_degrees(v, KNOB_TRAVEL_DEG / (2.0 * EQ_RANGE_DB)),
                db: v,
            },
            _ => Action::Ok,
        }
    })
}

#[derive(Default)]
pub struct AdviceEngine {
    controls: Vec<Control>,
    /// ✓ state of controls[i] (EQ knobs only).
    ok: Vec<bool>,
    /// Previous recommended frequency of a sweep MID.
    freq: Vec<Option<f32>>,
}

impl AdviceEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advise(&mut self, group: &ChannelGroup, inp: &Inputs) -> AdviceSet {
        if self.controls != group.controls {
            let n = group.controls.len();
            *self = Self {
                controls: group.controls.clone(),
                ok: vec![true; n],
                freq: vec![None; n],
            };
        }
        let s = match inp.summary {
            Some(s) if !inp.silent => s,
            _ => {
                let note = if inp.silent { NO_INPUT } else { KEEP_TALKING };
                return AdviceSet {
                    knobs: labels(group),
                    notes: vec![note.into()],
                    level_offset_db: 0.0,
                };
            }
        };
        let mut d: [f32; 25] = std::array::from_fn(|b| s.bands_db[b] - inp.target[b]);
        let ctl = &group.controls;
        let restore = inp.restore_dynamic_range_db;
        // Current position K ((value, FREQ) in controls order). Put K's FREQ in as the previous recommendation; if the sound is the same, leave it.
        let k = per_control(group, inp.current);
        for (f, &(_, kf)) in self.freq.iter_mut().zip(&k) {
            if kf > 0.0 {
                *f = Some(kf);
            }
        }

        // EQ knobs and each knob's Bell frequency (sweep: previous recommendation, else middle candidate).
        let eq: Vec<usize> = (0..ctl.len())
            .filter(|&i| {
                matches!(
                    ctl[i],
                    Control::HighShelf { .. } | Control::Mid { .. } | Control::LowShelf { .. }
                )
            })
            .collect();
        let hz: Vec<f32> = ctl
            .iter()
            .zip(&self.freq)
            .map(|(&c, f)| match (c, sweep(c)) {
                (_, Some(r)) => f
                    .or_else(|| {
                        let v = freq_choices(c);
                        v.get(v.len() / 2).copied()
                    })
                    .unwrap_or(r.0),
                (Control::Mid { lo_hz, .. }, None) => lo_hz,
                _ => 0.0,
            })
            .collect();
        let shapes = |hz: &[f32]| -> Vec<Shape> {
            eq.iter()
                .map(|&i| match ctl[i] {
                    Control::HighShelf { hz } => Shape::HighShelf(hz),
                    Control::LowShelf { hz } => Shape::LowShelf(hz),
                    _ => Shape::Bell(hz[i]),
                })
                .collect()
        };
        // The fit finds the turn amount from K (λ applies to that amount too): pulling to center makes nearly overlapping knobs (MID 2 80 Hz and
        // LOW 60 Hz in Signature 22) split into different combinations/frequencies than K even for the same sound. Fit by adding K's shape back onto the centered sound d.
        let k_eq = |sh: &[Shape], f: f32| -> f32 { eq.iter().zip(sh).map(|(&i, s)| k[i].0 * s.response(f)).sum() };
        let fit = |d: &[f32; 25], sh: &[Shape], bands: std::ops::RangeInclusive<usize>, lambda: f32| {
            let dk: [f32; 25] = std::array::from_fn(|b| d[b] + k_eq(sh, BANDS_HZ[b]));
            fit_eq_in(&dk, sh, bands, lambda)
        };

        // Sweep MID: minimum residual among candidates, change only when 20% or more better than the previous frequency. For two, 2 rounds of coordinate descent.
        // But two sweeps with no previous frequency (the app always has a new engine) check all pairs. Coordinate descent gets stuck in a wrong pair
        // and can turn another knob to the ±15 dB end (Signature 22). Candidates are at most 14×15 pairs.
        let sweeps: Vec<usize> = eq.iter().copied().filter(|&i| sweep(ctl[i]).is_some()).collect();
        let pick = |d: &[f32; 25]| -> Vec<f32> {
            let mut hz = hz.clone();
            if let [a, b] = sweeps[..]
                && self.freq[a].is_none()
                && self.freq[b].is_none()
            {
                let cb = freq_choices(ctl[b]);
                let best = freq_choices(ctl[a])
                    .into_iter()
                    .flat_map(|fa| cb.iter().map(move |&fb| (fa, fb)))
                    .filter(|(fa, fb)| (fa / fb).log2().abs() >= 1.0) // 1-octave guard same as below
                    .map(|(fa, fb)| {
                        let mut h = hz.clone();
                        (h[a], h[b]) = (fa, fb);
                        let r = fit(d, &shapes(&h), FIT_BANDS, LAMBDA).2;
                        (h, r)
                    })
                    .min_by(|p, q| p.1.total_cmp(&q.1));
                if let Some((h, _)) = best {
                    hz = h;
                }
            } else {
                for _ in 0..sweeps.len().min(2) {
                    for &i in &sweeps {
                        let resid = |f: f32| {
                            let mut h = hz.clone();
                            h[i] = f;
                            fit(d, &shapes(&h), FIT_BANDS, LAMBDA).2
                        };
                        // If two sweeps split the gain in half at the same frequency, both become ✓, so keep them 1 octave or more apart.
                        let best = freq_choices(ctl[i])
                            .into_iter()
                            .filter(|&f| sweeps.iter().all(|&j| j == i || (f / hz[j]).log2().abs() >= 1.0))
                            .map(|f| (f, resid(f)))
                            .min_by(|a, b| a.1.total_cmp(&b.1));
                        if let Some((f, r)) = best
                            && (self.freq[i].is_none() || r < 0.8 * resid(hz[i]))
                        {
                            hz[i] = f;
                        }
                    }
                }
            }
            hz
        };

        // Judge low cut by d before applying (analysis starts from off, so only turning on); if on, apply that switch response to d so
        // LOW does not chase the same low difference again.
        let cut = ctl.iter().find_map(|&c| match c {
            Control::LowCut { hz } => Some(hz),
            _ => None,
        });
        let with_cut = |d: &[f32; 25], fc: f32| -> [f32; 25] { std::array::from_fn(|b| d[b] + hpf_db(fc, BANDS_HZ[b])) };
        // Subtract K's EQ shape (and its response if low cut is on) from the measurement and treat it as the sound measured at center. K's low cut on is
        // always ✓ and the fit treats it as on.
        let k_cut = ctl.iter().zip(&k).any(|(c, k)| matches!(c, Control::LowCut { .. }) && k.0 > 0.0);
        let k_shapes = shapes(&hz);
        d = std::array::from_fn(|b| {
            let f = BANDS_HZ[b];
            d[b] - k_eq(&k_shapes, f) - cut.filter(|_| k_cut).map_or(0.0, |fc| hpf_db(fc, f))
        });
        let switch = k_cut || match (restore, cut) {
            // Restore: of the fit with the switch response and the fit without it, the one fitting better over 63 Hz–12.5 kHz. For 75·80 Hz switches the
            // 63·80 Hz vs 125·160 Hz difference is only a bit over 3 dB, so comparing their mean gets buried in noise or low-mid cuts.
            (Some(_), Some(fc)) => {
                let cost = |d: &[f32; 25]| {
                    let (g, _, r) = fit(d, &shapes(&pick(d)), 0..=*FIT_BANDS.end(), SWITCH_LAMBDA);
                    r + SWITCH_LAMBDA * g.iter().map(|g| g * g).sum::<f32>()
                };
                cost(&with_cut(&d, fc)) < cost(&d)
            }
            // Normal: if 63·80 Hz exceed c0 (250 Hz–4 kHz) by more than 3 dB.
            _ => {
                let mean = |r: std::ops::RangeInclusive<usize>| d[r.clone()].iter().sum::<f32>() / r.count() as f32;
                mean(0..=1) - mean(6..=18) > 3.0
            }
        };
        if let (true, Some(fc)) = (switch, cut) {
            d = with_cut(&d, fc);
        }
        let hz = pick(&d);
        for &i in &sweeps {
            self.freq[i] = Some(hz[i]);
        }
        let sh = shapes(&hz);
        let (turn, c, _) = fit(&d, &sh, FIT_BANDS, LAMBDA);
        let gains: Vec<f32> = turn.iter().zip(&eq).map(|(g, &i)| g + k[i].0).collect();
        // Restoring to the entered position does not chase per-recording differences: 60·80 Hz LOW and 12 kHz HIGH touch only a few fit bands,
        // so band noise becomes a large gain difference; widen the ✓ range to that knob's noise size (center analysis and normal mode unchanged).
        // But if d² summed over the turn amounts of the knobs left in range is outside noise, the sound changed, so do not widen: if FREQ
        // overlaps LOW·HIGH only their share is uncertain, so each knob's range is as wide as 10 dB, but the combined difference is distinguishable from noise.
        // A knob already past its own range is removed from d², so one knob really changing does not narrow another knob's range.
        let release: Vec<f32> = if restore.is_some() && inp.current.is_some_and(|p| fits(group, p)) {
            let cov = gain_cov(&sh);
            let wide: Vec<f32> =
                (0..eq.len()).map(|i| OK_RELEASE_DB.max(NOISE_Z * TAKE_NOISE_DB * cov[i][i].sqrt())).collect();
            let q: Vec<usize> = (0..eq.len()).filter(|&i| turn[i].abs() <= wide[i]).collect();
            let aug = q.iter().map(|&i| q.iter().map(|&j| cov[i][j]).chain([turn[i]]).collect()).collect();
            let d2: f32 = q.iter().zip(solve(aug)).map(|(&i, x)| turn[i] * x).sum::<f32>() / TAKE_NOISE_DB.powi(2);
            let changed = d2 > CHI2_999[q.len()];
            wide.into_iter().map(|w| if changed { OK_RELEASE_DB } else { w }).collect()
        } else {
            vec![OK_RELEASE_DB; eq.len()]
        };

        // A knob recommendation must not disagree with the graph's red/blue bands: if the knob center is a red band, do not raise;
        // if a blue band, do not cut (same calculation as the graph).
        let lvl = level_match_db(&s.bands_db, inp.target);
        let reference: [f32; 25] = std::array::from_fn(|b| inp.target[b] + lvl);
        let tone = tone_excess(&s.bands_db, &reference);

        let mut main = vec![Action::Ok; ctl.len()];
        let mut freq_act = vec![Action::Ok; ctl.len()];
        for ((&i, &g), &rel) in eq.iter().zip(&gains).zip(&release) {
            // In normal mode, if turning on is recommended, no LOW boost: 63·80 Hz outside the fit range (100 Hz~) still overflow, but
            // if LOW refills what the switch cut at 100–160 Hz, the two chase the same low end in opposite directions.
            let max = if restore.is_none() && matches!(ctl[i], Control::LowShelf { .. }) {
                if switch { 0.0 } else { LOW_BOOST_MAX_DB }
            } else if restore.is_none() && matches!(ctl[i], Control::HighShelf { .. }) {
                HIGH_BOOST_MAX_DB
            } else {
                EQ_LIMIT_DB
            };
            let centre = match ctl[i] {
                Control::LowShelf { hz } | Control::HighShelf { hz } => hz,
                _ => hz[i],
            };
            // The limit applies at the target position, the band guard at raising/cutting from the current position K (the guard wins even if K is outside the limit).
            let (kv, kf) = k[i];
            let mut g = g.clamp(-EQ_LIMIT_DB, max);
            // LOW below the first range (80 Hz) (Signature 60 Hz) also follows the boom range.
            let centre = centre.max(TONE_BANDS[0].1);
            if let Some(t) = TONE_BANDS.iter().position(|&(_, lo, hi)| (lo..hi).contains(&centre)) {
                if tone[t] >= TONE_HOT_DB {
                    g = g.min(kv);
                } else if tone[t] <= -TONE_HOT_DB {
                    g = g.max(kv);
                }
            }
            let ok = &mut self.ok[i];
            if *ok && (g - kv).abs() > rel {
                *ok = false;
            } else if !*ok && (g - kv).abs() < OK_BELOW_DB {
                *ok = true;
            }
            if !*ok {
                main[i] = Action::Rotate {
                    degrees: rotate_degrees(g - kv, KNOB_TRAVEL_DEG / (2.0 * EQ_RANGE_DB)),
                    db: g,
                };
            }
            // FREQ: only if the chosen frequency differs from K and the MID target is outside the ✓ range. If K's MID is 0, that ✓ is the ✓ above.
            let mid_off = if kv == 0.0 { !*ok } else { g.abs() > rel };
            if hz[i] != kf && mid_off {
                freq_act[i] = Action::SetFreq { hz: hz[i] };
            }
        }

        // ponytail: COMP is a fixed +30° heuristic looking only at dynamic range/crest (analysis starts with COMP off,
        // so clockwise only). Actual threshold/ratio and compression amount are unknown. Compute the angle once gain reduction can be estimated.
        let comp = match restore {
            Some(saved) => s.dynamic_range_db - saved > 4.0,
            None => s.crest_db >= 9.0 && s.dynamic_range_db > 18.0,
        };
        let overload = s.crest_db < 9.0 && s.peak_db > -20.0;
        for (i, &control) in ctl.iter().enumerate() {
            main[i] = match control {
                Control::Gain { range_db } => {
                    let db = if inp.clipped_recently || s.peak_db > -6.0 {
                        (-12.0 - s.peak_db).min(-3.0)
                    } else if s.peak_db < -24.0 {
                        (-12.0 - s.peak_db).min(20.0)
                    } else {
                        continue;
                    };
                    Action::Rotate {
                        degrees: rotate_degrees(db, KNOB_TRAVEL_DEG / range_db),
                        db,
                    }
                }
                Control::LowCut { .. } if switch && !k_cut => Action::SwitchOn,
                Control::Comp if comp && k[i].0 < 30.0 => Action::Rotate {
                    degrees: 30.0 - k[i].0,
                    db: 0.0,
                },
                _ => continue,
            };
        }

        let mut notes = Vec::new();
        if let Some(f) = inp.feedback_hz {
            notes.push(format!(
                "하울링 의심({}: 마이크와 스피커 거리 확인, FREQ를 해당 눈금에 두고 MID를 줄이세요)",
                fmt_hz(f)
            ));
        }
        if overload {
            notes.push(OVERLOAD.into());
        }
        if s.snr_db < 30.0 {
            notes.push(format!(
                "주변 잡음이 큽니다(SNR {:.0}dB). GAIN보다 마이크를 입에 더 가깝게 하세요.",
                s.snr_db
            ));
        }
        AdviceSet {
            knobs: knobs(group, |i, f| if f { freq_act[i].clone() } else { main[i].clone() }),
            notes,
            level_offset_db: -c,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
use crate::{mixers::*, speech::Summary, target::*};
fn summary(dev: impl Fn(f32) -> f32) -> Summary {
    let t = ltass_db(Voice::Male);
    let mut bands = [0.0; 25];
    for (i, b) in bands.iter_mut().enumerate() { *b = t[i] - 40.0 + dev(BANDS_HZ[i]); }
    Summary { bands_db: bands, bin_power: vec![], peak_db: -12.0, rms_db: -30.0, noise_floor_db: -80.0, snr_db: 50.0,
              dynamic_range_db: 10.0, crest_db: 18.0, active_seconds: 4.0 }
}
fn run(e: &mut AdviceEngine, g: &ChannelGroup, s: &Summary) -> AdviceSet {
    let t = ltass_db(Voice::Male);
    e.advise(g, &Inputs { summary: Some(s), target: &t, clipped_recently: false, feedback_hz: None, silent: false, restore_dynamic_range_db: None, current: None })
}
fn act(a: &AdviceSet, label: &str) -> Action { a.knobs.iter().find(|k| k.label == label).unwrap().action.clone() }
fn db_of(a: Action) -> f32 { match a { Action::Rotate { db, .. } => db, x => panic!("{x:?}") } }
fn group(m: MixerModel, i: usize) -> ChannelGroup { MixerFactory::create(m).groups.remove(i) }
const V3: MixerModel = MixerModel::MackieProFx22v3;
const FX12: MixerModel = MixerModel::MackieProFx12v3;

#[test] fn level_offset_maps_target_onto_measured() {
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|_| 0.0));
    assert!((a.level_offset_db + 40.0).abs() < 0.01, "{}", a.level_offset_db);
}
#[test] fn flat_gives_all_ok() {
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|_| 0.0));
    assert_eq!(a.knobs.len(), 7);
    assert!(a.knobs.iter().all(|k| k.action == Action::Ok), "{:?}", a.knobs);
}
#[test] fn low_boom_turns_low_ccw() {
    // Boom below the low-cut threshold (mean(63,80) − c0 > 3). 6 dB gives 3.23, so the switch turns on.
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|f| 5.0 * Shape::LowShelf(80.0).response(f)));
    assert_eq!(act(&a, "저역컷 100Hz"), Action::Ok);
    let d = db_of(act(&a, "LOW 80Hz"));
    assert!((-7.5..=-3.0).contains(&d), "{d}");
    assert_eq!(act(&a, "HIGH 12kHz"), Action::Ok);
}
#[test] fn mid_bump_finds_freq() {
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|f| 6.0 * Shape::Bell(400.0).response(f)));
    match act(&a, "FREQ") { Action::SetFreq { hz } => assert!((315.0..=500.0).contains(&hz), "{hz}"), x => panic!("{x:?}") }
    assert!((-7.5..=-4.5).contains(&db_of(act(&a, "MID"))));
}
#[test] fn dull_top_turns_high_cw() {
    let a = run(&mut AdviceEngine::new(), &group(MixerModel::YamahaMgp24x, 0), &summary(|f| -6.0 * Shape::HighShelf(8_000.0).response(f)));
    assert!((4.0..=7.5).contains(&db_of(act(&a, "HIGH 8kHz"))));
}
#[test] fn rumble_recommends_lowcut() {
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|f| if f < 90.0 { 12.0 } else { 0.0 }));
    assert_eq!(act(&a, "저역컷 100Hz"), Action::SwitchOn);
    assert_eq!(act(&a, "LOW 80Hz"), Action::Ok);
}
#[test] fn boom_over_switch_threshold_switches_on_without_low() {
    // A 6 dB boom has mean(63,80) − c0 = 3.23 > 3, so the switch turns on. LOW must not refill it with a boost (+2.9) on top.
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|f| 6.0 * Shape::LowShelf(80.0).response(f)));
    assert_eq!(act(&a, "저역컷 100Hz"), Action::SwitchOn);
    assert_eq!(act(&a, "LOW 80Hz"), Action::Ok);
}
#[test] fn clip_turns_gain_down() {
    let mut s = summary(|f| 20.0 * Shape::Bell(3_000.0).response(f));
    s.peak_db = -2.0;
    let t = ltass_db(Voice::Male);
    let a = AdviceEngine::new().advise(&group(V3, 0), &Inputs { summary: Some(&s), target: &t, clipped_recently: true, feedback_hz: None, silent: false, restore_dynamic_range_db: None, current: None });
    assert_eq!(act(&a, "GAIN"), Action::Rotate { degrees: -50.0, db: -10.0 });
    for k in &a.knobs { if let Action::Rotate { degrees, db } = k.action { assert!(degrees.is_finite() && db.is_finite() && db.abs() <= 20.0); } }
    for k in a.knobs.iter().filter(|k| k.label != "GAIN") { if let Action::Rotate { db, .. } = k.action { assert!(db.abs() <= 15.0); } }
}
#[test] fn quiet_turns_gain_up() {
    let mut s = summary(|_| 0.0);
    s.peak_db = -40.0;
    assert_eq!(act(&run(&mut AdviceEngine::new(), &group(V3, 0), &s), "GAIN"), Action::Rotate { degrees: 100.0, db: 20.0 });
    assert_eq!(act(&run(&mut AdviceEngine::new(), &group(MixerModel::YamahaMgp24x, 0), &s), "GAIN"), Action::Rotate { degrees: 135.0, db: 20.0 });
}
#[test] fn deadband_hysteresis() {
    let mut e = AdviceEngine::new();
    let g = group(FX12, 0);
    let boom = |x: f32| summary(move |f| x * Shape::LowShelf(80.0).response(f));
    assert_eq!(act(&run(&mut e, &g, &boom(2.2)), "LOW 80Hz"), Action::Ok);
    assert!(matches!(act(&run(&mut e, &g, &boom(5.0)), "LOW 80Hz"), Action::Rotate { .. }));
    assert!(matches!(act(&run(&mut e, &g, &boom(2.2)), "LOW 80Hz"), Action::Rotate { .. }));
    assert_eq!(act(&run(&mut e, &g, &boom(0.0)), "LOW 80Hz"), Action::Ok);
}
#[test] fn low_boost_capped() {
    let a = run(&mut AdviceEngine::new(), &group(FX12, 0), &summary(|f| -10.0 * Shape::LowShelf(80.0).response(f)));
    assert!(db_of(act(&a, "LOW 80Hz")) <= 3.0);
}
#[test] fn comp_rules() {
    let mut s = summary(|_| 0.0);
    s.dynamic_range_db = 22.0;
    assert_eq!(act(&run(&mut AdviceEngine::new(), &group(V3, 0), &s), "COMP"), Action::Rotate { degrees: 30.0, db: 0.0 });
    // Low crest: analysis starts with COMP off, so no counterclockwise; only the overload notice remains.
    s.crest_db = 7.0; s.peak_db = -10.0;
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &s);
    assert_eq!(act(&a, "COMP"), Action::Ok);
    assert!(a.notes.iter().any(|n| n == OVERLOAD));
}
#[test] fn notes_for_feedback_and_snr() {
    let mut s = summary(|_| 0.0);
    s.snr_db = 20.0;
    let t = ltass_db(Voice::Male);
    let a = AdviceEngine::new().advise(&group(V3, 0), &Inputs { summary: Some(&s), target: &t, clipped_recently: false, feedback_hz: Some(2_500.0), silent: false, restore_dynamic_range_db: None, current: None });
    assert!(a.notes.iter().any(|n| n.contains("2.5kHz")));
    assert!(a.notes.iter().any(|n| n.contains("마이크를")));
}
#[test] fn silent_input_waits() {
    let t = ltass_db(Voice::Male);
    let mut e = AdviceEngine::new();
    let a = e.advise(&group(V3, 0), &Inputs { summary: None, target: &t, clipped_recently: false, feedback_hz: None, silent: true, restore_dynamic_range_db: None, current: None });
    assert!(a.knobs.iter().all(|k| k.action == Action::Wait));
    assert!(a.notes.iter().any(|n| n.contains("입력 신호가 없습니다")));
    let b = e.advise(&group(V3, 0), &Inputs { summary: None, target: &t, clipped_recently: false, feedback_hz: None, silent: false, restore_dynamic_range_db: None, current: None });
    assert!(b.notes.iter().any(|n| n.contains("말소리가 부족합니다")));
}
#[test] fn group_change_resets_state() {
    let mut e = AdviceEngine::new();
    assert_eq!(run(&mut e, &group(V3, 0), &summary(|_| 0.0)).knobs.len(), 7);
    assert_eq!(run(&mut e, &group(MixerModel::YamahaMg06x, 0), &summary(|_| 0.0)).knobs.len(), 4);
}
#[test] fn signature22_sweeps_do_not_split_one_bump() {
    let a = run(&mut AdviceEngine::new(), &group(MixerModel::SoundcraftSignature22, 0), &summary(|f| 4.0 * Shape::Bell(1_000.0).response(f)));
    let mids: Vec<f32> = ["MID 1", "MID 2"].iter().filter_map(|&l| match act(&a, l) { Action::Rotate { degrees, .. } => Some(degrees), _ => None }).collect();
    assert_eq!(mids.len(), 1, "{:?}", a.knobs);
    assert!((-45.0..=-35.0).contains(&mids[0]), "{mids:?}");
}
fn run_t(g: &ChannelGroup, s: &Summary, target: [f32; 25], restore: Option<f32>) -> AdviceSet { run_k(g, s, target, restore, None) }
fn run_k(g: &ChannelGroup, s: &Summary, target: [f32; 25], restore: Option<f32>, current: Option<&Positions>) -> AdviceSet {
    AdviceEngine::new().advise(g, &Inputs { summary: Some(s), target: &target, clipped_recently: false, feedback_hz: None, silent: false, restore_dynamic_range_db: restore, current })
}
fn with(dev: impl Fn(f32) -> f32) -> [f32; 25] { let t = ltass_db(Voice::Male); std::array::from_fn(|b| t[b] + dev(BANDS_HZ[b])) }
#[test] fn hpf_db_values() {
    assert!((hpf_db(100.0, 100.0) + 3.01).abs() < 0.01 && (hpf_db(100.0, 50.0) + 12.30).abs() < 0.01 && hpf_db(100.0, 1000.0) > -0.001);
}
#[test] fn preset_hpf_switch_not_chased_by_low() {
    let a = run_t(&group(V3, 0), &summary(|_| 0.0), with(|f| hpf_db(100.0, f)), None);
    assert_eq!(act(&a, "저역컷 100Hz"), Action::SwitchOn);
    assert_eq!(act(&a, "LOW 80Hz"), Action::Ok);
}
#[test] fn switch_already_on_is_quiet() {
    let a = run_t(&group(V3, 0), &summary(|f| hpf_db(100.0, f)), with(|f| hpf_db(100.0, f)), None);
    assert!(a.knobs.iter().all(|k| k.action == Action::Ok), "{:?}", a.knobs);
}
#[test] fn normal_mode_never_switches_off() {
    assert_eq!(act(&run_t(&group(V3, 0), &summary(|f| hpf_db(100.0, f)), with(|_| 0.0), None), "저역컷 100Hz"), Action::Ok);
}
#[test] fn restore_switch_only_on() {
    // Off when saved, on now: do not say to turn it off (analysis starts from off).
    let off = run_t(&group(V3, 0), &summary(|f| hpf_db(100.0, f)), with(|_| 0.0), Some(10.0));
    assert_eq!(act(&off, "저역컷 100Hz"), Action::Ok);
    let on = run_t(&group(V3, 0), &summary(|_| 0.0), with(|f| hpf_db(100.0, f)), Some(10.0));
    assert_eq!(act(&on, "저역컷 100Hz"), Action::SwitchOn);
    assert_eq!(act(&on, "LOW 80Hz"), Action::Ok);
}
#[test] fn restore_lifts_low_boost_cap() {
    let t = with(|f| 6.0 * Shape::LowShelf(80.0).response(f));
    assert!(db_of(act(&run_t(&group(FX12, 0), &summary(|_| 0.0), t, Some(10.0)), "LOW 80Hz")) >= 4.5);
    assert!(db_of(act(&run_t(&group(FX12, 0), &summary(|_| 0.0), t, None), "LOW 80Hz")) <= 3.0);
}
#[test] fn restore_comp_follows_saved_dynamic_range() {
    let mut s = summary(|_| 0.0); // dynamic_range_db 10
    s.crest_db = 7.0; s.peak_db = -10.0; // restore ignores crest
    let comp = |saved| act(&run_t(&group(V3, 0), &s, with(|_| 0.0), Some(saved)), "COMP");
    assert_eq!(comp(4.0), Action::Rotate { degrees: 30.0, db: 0.0 });
    assert_eq!(comp(16.0), Action::Ok);
    assert_eq!(comp(12.0), Action::Ok);
}
#[test] fn rounding_and_labels() {
    assert_eq!(rotate_degrees(1.6, 10.0), 15.0);
    assert_eq!(rotate_degrees(-3.2, 10.0), -30.0);
    assert_eq!(rotate_degrees(2.4, 10.0), 25.0);
    assert_eq!([fmt_hz(400.0), fmt_hz(2_500.0), fmt_hz(12_000.0), fmt_hz(1_250.0), fmt_hz(80.0)],
               ["400Hz", "2.5kHz", "12kHz", "1.25kHz", "80Hz"]);
}

fn sum_of(bands: [f32; 25]) -> Summary {
    Summary { bands_db: bands, ..summary(|_| 0.0) }
}
/// Band difference seen on screen (measured − target): boom is red, 63–100 Hz is short.
const SHOT: [f32; 25] = [-2.3, -6.5, -10.5, 1.6, 7.0, 11.4, 1.7, 1.5, 3.4, 1.0, 4.0, 2.8, -1.0, 5.9, 0.6, -3.9, -3.9, -1.1, -3.4, -4.0, -0.5, -2.7, -7.7, -13.0, -16.0];
#[test] fn screenshot_case_follows_the_bands() {
    let g = group(V3, 0);
    let (t, _) = preset_target(&PRESETS[3], Voice::Male, &g);
    let s = sum_of(std::array::from_fn(|i| t[i] + SHOT[i] - 20.0));
    let a = run_t(&g, &s, t, None);
    assert_eq!(act(&a, "LOW 80Hz"), Action::Ok);
    assert!((db_of(act(&a, "HIGH 12kHz")) - 6.0).abs() < 1e-3);
    assert!(db_of(act(&a, "MID")) < -10.0);
    assert_eq!(act(&a, "FREQ"), Action::SetFreq { hz: 160.0 });
    // Restore mode has no LOW limit, but this is a red boom range, so do not raise here either.
    let r = run_t(&g, &s, t, Some(s.dynamic_range_db));
    assert!(!matches!(act(&r, "LOW 80Hz"), Action::Rotate { db, .. } if db > 0.0), "{:?}", act(&r, "LOW 80Hz"));
}
#[test] fn low_below_the_hum_band_follows_it() {
    // Signature LOW 60 Hz is below boom (80–250 Hz), but if that range is red, do not raise.
    let g = group(MixerModel::SoundcraftSignature12, 0);
    let (t, _) = preset_target(&PRESETS[3], Voice::Male, &g);
    let s = sum_of(std::array::from_fn(|i| t[i] + SHOT[i] - 20.0));
    let lvl = level_match_db(&s.bands_db, &t);
    assert!(tone_excess(&s.bands_db, &std::array::from_fn(|i| t[i] + lvl))[0] >= TONE_HOT_DB);
    for restore in [None, Some(s.dynamic_range_db)] {
        let low = act(&run_t(&g, &s, t, restore), "LOW 60Hz");
        assert!(!matches!(low, Action::Rotate { db, .. } if db > 0.0), "{restore:?}: {low:?}");
    }
}
#[test] fn high_boost_capped_only_in_normal_mode() {
    let g = group(V3, 0);
    let s = summary(|f| if f >= 8000.0 { -12.0 } else { 0.0 });
    let t = ltass_db(Voice::Male);
    assert!((db_of(act(&run_t(&g, &s, t, None), "HIGH 12kHz")) - 6.0).abs() < 1e-3);
    assert!(db_of(act(&run_t(&g, &s, t, Some(s.dynamic_range_db)), "HIGH 12kHz")) > 6.0);
}
#[test] fn no_cut_into_a_blue_band() {
    // Case found by random search: HIGH told to cut -4.6 dB in the 10–20 kHz band (blue).
    let g = group(V3, 0);
    let t = ltass_db(Voice::Male);
    let d = [6.66, -10.65, 11.98, 10.44, -7.73, 1.32, 3.12, -6.10, 2.73, 7.70, 7.77, -9.03, 3.92, 2.54, -9.35, -2.70, -5.65, 3.92, -6.72, -6.00, 10.27, -9.60, 0.39, -5.73, -9.54];
    let s = sum_of(std::array::from_fn(|i| t[i] + d[i] - 20.0));
    let lvl = level_match_db(&s.bands_db, &t);
    let reference: [f32; 25] = std::array::from_fn(|i| t[i] + lvl);
    assert!(tone_excess(&s.bands_db, &reference)[7] <= -TONE_HOT_DB);
    assert_eq!(act(&run_t(&g, &s, t, None), "HIGH 12kHz"), Action::Ok);
}

/// Position in group order: only the labels in `set` change from center.
fn pos(g: &ChannelGroup, set: &[(&str, f32)]) -> Positions {
    let mut p = centre(g);
    for &(l, v) in set { p.iter_mut().find(|e| e.0 == l).unwrap().1 = v; }
    p
}
/// dB that position p adds to the centered sound (the engine's Shape and hpf_db).
fn model(g: &ChannelGroup, p: &Positions) -> [f32; 25] {
    let v = |l: &str| p.iter().find(|e| e.0 == l).map_or(0.0, |e| e.1);
    let ks = labels(g);
    std::array::from_fn(|b| {
        let f = BANDS_HZ[b];
        ks.iter().map(|k| match k.control {
            Control::HighShelf { hz } => v(&k.label) * Shape::HighShelf(hz).response(f),
            Control::LowShelf { hz } => v(&k.label) * Shape::LowShelf(hz).response(f),
            Control::LowCut { hz } if v(&k.label) > 0.0 => hpf_db(hz, f),
            Control::Mid { lo_hz, hi_hz } if !k.label.starts_with("FREQ") => {
                let hz = if lo_hz == hi_hz { lo_hz } else { v(&k.label.replace("MID", "FREQ")) };
                v(&k.label) * Shape::Bell(hz).response(f)
            }
            _ => 0.0,
        }).sum()
    })
}
fn plus(a: [f32; 25], b: [f32; 25]) -> [f32; 25] { std::array::from_fn(|i| a[i] + b[i]) }
#[test] fn same_sound_with_entered_positions_is_all_ok() {
    let raw = summary(|_| 0.0).bands_db;
    for m in MixerModel::ALL {
        for g in MixerFactory::create(m).groups {
            let (mut eq, mut sweeps) = ([6.0, -6.0, -3.0, 9.0, 4.0].into_iter().cycle(), 0);
            let k: Positions = labels(&g).into_iter().filter(|a| !matches!(a.control, Control::Gain { .. })).map(|a| {
                let v = match a.control {
                    Control::LowCut { .. } => 1.0,
                    Control::Comp => 30.0,
                    c if a.label.starts_with("FREQ") => {
                        sweeps += 1;
                        let f = freq_choices(c);
                        if sweeps == 1 { f[f.len() - 1] } else { f[0] }
                    }
                    _ => eq.next().unwrap(),
                };
                (a.label, v)
            }).collect();
            let s = sum_of(plus(raw, model(&g, &k)));
            let a = run_k(&g, &s, s.bands_db, Some(s.dynamic_range_db), Some(&k));
            assert!(a.knobs.iter().filter(|x| x.label != "GAIN").all(|x| x.action == Action::Ok), "{m:?} {}: {k:?} {:?}", g.label, a.knobs);
        }
    }
}
#[test] fn entered_matches_centre_on_fixed_mid() {
    let g = group(FX12, 0);
    let t = ltass_db(Voice::Male);
    let set = model(&g, &pos(&g, &[("HIGH 12kHz", 5.0), ("MID 2.5kHz", -7.0), ("LOW 80Hz", 3.0)]));
    let raw = sum_of(std::array::from_fn(|b| summary(|_| 0.0).bands_db[b] - set[b]));
    let c = run_t(&g, &raw, t, None);
    let k = pos(&g, &[("HIGH 12kHz", -2.0), ("MID 2.5kHz", 2.0), ("LOW 80Hz", -3.0)]);
    let a = run_k(&g, &sum_of(plus(raw.bands_db, model(&g, &k))), t, None, Some(&k));
    for (l, kv) in [("HIGH 12kHz", -2.0), ("MID 2.5kHz", 2.0), ("LOW 80Hz", -3.0)] {
        let centre_db = db_of(act(&c, l));
        match act(&a, l) {
            Action::Rotate { degrees, db } => {
                assert!((db - centre_db).abs() <= 0.5, "{l}: {db} vs {centre_db}");
                assert_eq!(degrees, rotate_degrees(db - kv, 10.0), "{l}");
            }
            x => panic!("{l}: {x:?}"),
        }
    }
}
#[test] fn normal_mode_entered_keeps_the_fixed_ok_range() {
    // The noise ✓ range is used only when restoring (difference of the saved and today recordings). Normal mode with K at center is the same as center analysis.
    let (g, t) = (group(FX12, 0), ltass_db(Voice::Male));
    let s = summary(|f| 4.0 * Shape::LowShelf(80.0).response(f));
    let c = act(&run_t(&g, &s, t, None), "LOW 80Hz");
    assert!(matches!(c, Action::Rotate { .. }), "{c:?}");
    assert_eq!(act(&run_k(&g, &s, t, None, Some(&centre(&g))), "LOW 80Hz"), c);
}
const V3_LABEL: [(&str, f32); 5] = [("저역컷 100Hz", 1.0), ("HIGH 12kHz", 6.0), ("FREQ", 800.0), ("MID", -6.0), ("LOW 80Hz", -3.0)];
/// Set to label k and re-record: today's sound = saved sound (centered sound + k) + today, restore mode, K = k.
fn retake(g: &ChannelGroup, k: &Positions, today: impl Fn(f32) -> f32) -> AdviceSet {
    let saved = plus(summary(|_| 0.0).bands_db, model(g, k));
    let s = sum_of(std::array::from_fn(|b| saved[b] + today(BANDS_HZ[b])));
    run_k(g, &s, saved, Some(s.dynamic_range_db), Some(k))
}
#[test] fn small_change_turns_one_knob() {
    // 3 dB is within the 12 kHz HIGH noise ✓ range (about 3.9 dB here), so ✓.
    let g = group(V3, 0);
    let a = retake(&g, &pos(&g, &V3_LABEL), |f| -5.0 * Shape::HighShelf(12_000.0).response(f));
    match act(&a, "HIGH 12kHz") {
        Action::Rotate { degrees, db } => assert!((10.0..=12.0).contains(&db) && (40.0..=60.0).contains(&degrees), "{db} {degrees}"),
        x => panic!("{x:?}"),
    }
    for l in ["FREQ", "MID", "LOW 80Hz", "저역컷 100Hz", "COMP"] { assert_eq!(act(&a, l), Action::Ok, "{l}: {:?}", a.knobs); }
}
/// Fixed noise of ±1 dB per band.
const NOISE: [f32; 25] = [-1., 1., -1., -1., -1., -1., 1., 1., 1., 1., 1., -1., -1., 1., -1., 1., 1., -1., -1., -1., 1., -1., -1., 1., -1.];
fn noise(f: f32) -> f32 { NOISE[BANDS_HZ.iter().position(|&b| b == f).unwrap()] }
#[test] fn take_noise_alone_is_all_ok() {
    // With a fixed 2.5 dB ✓ range, it said to turn LOW 35° (80 Hz), 55° (60 Hz of Signature 12).
    let sig = [("저역컷 100Hz", 1.0), ("HIGH 12kHz", 4.0), ("FREQ", 2500.0), ("MID", -4.0), ("LOW 60Hz", -3.0)];
    for (m, set) in [(V3, V3_LABEL), (MixerModel::SoundcraftSignature12, sig)] {
        let g = group(m, 0);
        let a = retake(&g, &pos(&g, &set), noise);
        assert!(a.knobs.iter().filter(|x| x.label != "GAIN").all(|x| x.action == Action::Ok), "{m:?}: {:?}", a.knobs);
    }
}
#[test] fn one_real_change_keeps_the_others_in_noise_range() {
    // If only HIGH got 6 dB darker, turn only HIGH; LOW with just noise stays (d² is among the knobs left in range).
    let g = group(V3, 0);
    let a = retake(&g, &pos(&g, &V3_LABEL), |f| noise(f) - 6.0 * Shape::HighShelf(12_000.0).response(f));
    assert!(matches!(act(&a, "HIGH 12kHz"), Action::Rotate { degrees, .. } if degrees > 0.0), "{:?}", a.knobs);
    assert_eq!(act(&a, "LOW 80Hz"), Action::Ok, "{:?}", a.knobs);
}
#[test] fn real_low_change_is_still_advised() {
    // If LOW was set 6 dB below the label, have it turned back clockwise.
    let g = group(V3, 0);
    let a = retake(&g, &pos(&g, &V3_LABEL), |f| -6.0 * Shape::LowShelf(80.0).response(f));
    assert!(matches!(act(&a, "LOW 80Hz"), Action::Rotate { degrees, .. } if degrees > 0.0), "{:?}", a.knobs);
    for l in ["HIGH 12kHz", "FREQ", "MID", "저역컷 100Hz"] { assert_eq!(act(&a, l), Action::Ok, "{l}: {:?}", a.knobs); }
    // LOW overlapping a FREQ 125 Hz MID has a noise ✓ range of a bit over 10 dB, but a 9 dB boom puts the whole turn amount (d²) outside noise.
    let k = pos(&g, &[("저역컷 100Hz", 1.0), ("FREQ", 125.0), ("MID", -6.0), ("LOW 80Hz", -3.0)]);
    let a = retake(&g, &k, |f| 9.0 * Shape::LowShelf(80.0).response(f));
    assert!(matches!(act(&a, "LOW 80Hz"), Action::Rotate { degrees, .. } if degrees < 0.0), "{:?}", a.knobs);
}
#[test] fn freq_only_move() {
    let g = group(V3, 0);
    let raw = summary(|_| 0.0).bands_db;
    let target = plus(raw, model(&g, &pos(&g, &[("FREQ", 2000.0), ("MID", -6.0)])));
    let k = pos(&g, &[("FREQ", 800.0), ("MID", -6.0)]);
    let s = sum_of(plus(raw, model(&g, &k)));
    let a = run_k(&g, &s, target, Some(s.dynamic_range_db), Some(&k));
    assert!(matches!(act(&a, "FREQ"), Action::SetFreq { hz } if [1600.0, 2000.0, 2500.0].contains(&hz)), "{:?}", a.knobs);
    assert_eq!(act(&a, "MID"), Action::Ok, "{:?}", a.knobs);
}
#[test] fn entered_cut_and_comp_stay() {
    let g = group(V3, 0);
    let mut s = summary(|_| 0.0);
    s.dynamic_range_db = 22.0; // normal COMP rule (crest ≥ 9, DR > 18) true, low-cut threshold false
    let t = ltass_db(Voice::Male);
    let c = run_t(&g, &s, t, None);
    assert_eq!((act(&c, "저역컷 100Hz"), act(&c, "COMP")), (Action::Ok, Action::Rotate { degrees: 30.0, db: 0.0 }));
    let a = run_k(&g, &s, t, None, Some(&pos(&g, &[("저역컷 100Hz", 1.0), ("COMP", 30.0)])));
    assert_eq!((act(&a, "저역컷 100Hz"), act(&a, "COMP")), (Action::Ok, Action::Ok));
    let comp = |v| act(&run_k(&g, &s, t, None, Some(&pos(&g, &[("COMP", v)]))), "COMP");
    assert_eq!(comp(0.0), Action::Rotate { degrees: 30.0, db: 0.0 });
    assert_eq!(comp(15.0), Action::Rotate { degrees: 15.0, db: 0.0 });
}
#[test] fn positions_helpers() {
    let g = group(V3, 0);
    assert!(fits(&g, &centre(&g)));
    let mut short = centre(&g);
    short.pop();
    assert!(!fits(&g, &short));
    let mut renamed = centre(&g);
    renamed[1].0 = "HIGH".into();
    assert!(!fits(&g, &renamed));
    let p = pos(&g, &[("저역컷 100Hz", 1.0), ("HIGH 12kHz", 6.0), ("FREQ", 800.0), ("MID", -6.0), ("LOW 80Hz", -3.0), ("COMP", 30.0)]);
    assert_eq!(targets(&as_advice(&g, &p), &centre(&g)), p);
    assert_eq!(freq_choices(g.controls[3]), BANDS_HZ[2..=21]); // 100 Hz–8 kHz
    assert!(freq_choices(group(FX12, 0).controls[3]).is_empty());
}

/// Engine behavior the 0.4.0 knob position calculation used: restore recommendation from the centered sound (measured) to the finished sound (target).
fn restore_from_centre(g: &ChannelGroup, fin: [f32; 25]) -> AdviceSet { run_t(g, &sum_of(ltass_db(Voice::Male)), fin, Some(10.0)) }
#[test] fn recovers_known_settings() {
    let g = group(V3, 0);
    let flat = ltass_db(Voice::Male);
    let k = restore_from_centre(&g, std::array::from_fn(|b| {
        let f = BANDS_HZ[b];
        flat[b] + 6.0 * Shape::HighShelf(12_000.0).response(f) - 6.0 * Shape::Bell(800.0).response(f)
            - 3.0 * Shape::LowShelf(80.0).response(f) + hpf_db(100.0, f)
    }));
    assert!((5.0..=7.0).contains(&db_of(act(&k, "HIGH 12kHz"))), "{:?}", k.knobs);
    assert!(matches!(act(&k, "FREQ"), Action::SetFreq { hz } if [630.0, 800.0, 1000.0].contains(&hz)), "{:?}", k.knobs);
    assert!((-7.0..=-5.0).contains(&db_of(act(&k, "MID"))), "{:?}", k.knobs);
    assert!((-4.0..=-2.0).contains(&db_of(act(&k, "LOW 80Hz"))), "{:?}", k.knobs);
    assert_eq!(act(&k, "저역컷 100Hz"), Action::SwitchOn);
    assert_eq!(act(&k, "GAIN"), Action::Ok);
}
#[test] fn recovers_signature22_two_sweeps() {
    // Coordinate descent got stuck at (3150, 400) and gave HIGH +15, LOW +14.
    let g = group(MixerModel::SoundcraftSignature22, 0);
    let flat = ltass_db(Voice::Male);
    let k = restore_from_centre(&g, std::array::from_fn(|b| {
        let f = BANDS_HZ[b];
        flat[b] + 6.0 * Shape::HighShelf(12_000.0).response(f) - 6.0 * Shape::Bell(1_000.0).response(f)
            - 4.0 * Shape::Bell(200.0).response(f) + hpf_db(100.0, f)
    }));
    assert!((5.0..=7.0).contains(&db_of(act(&k, "HIGH 12kHz"))), "{:?}", k.knobs);
    assert!(matches!(act(&k, "FREQ 1"), Action::SetFreq { hz } if [800.0, 1000.0, 1250.0].contains(&hz)), "{:?}", k.knobs);
    assert!((-7.0..=-5.0).contains(&db_of(act(&k, "MID 1"))), "{:?}", k.knobs);
    assert!(matches!(act(&k, "FREQ 2"), Action::SetFreq { hz } if [160.0, 200.0, 250.0].contains(&hz)), "{:?}", k.knobs);
    assert!((-5.0..=-3.0).contains(&db_of(act(&k, "MID 2"))), "{:?}", k.knobs);
    assert_eq!(act(&k, "LOW 60Hz"), Action::Ok);
    assert_eq!(act(&k, "저역컷 100Hz"), Action::SwitchOn);
}
#[test] fn finds_75hz_low_cut_under_a_low_mid_cut() {
    // The 75 Hz switch differs only 3.3 dB between 63·80 Hz and 125·160 Hz, so a FREQ 200 MID cut lowering 125·160 Hz buried it.
    let g = group(MixerModel::Mackie1604Vlz4, 0);
    let flat = ltass_db(Voice::Male);
    for (bell, db) in [(160.0, -4.0), (200.0, -5.0), (250.0, -6.0)] {
        let k = restore_from_centre(&g, std::array::from_fn(|b| flat[b] + hpf_db(75.0, BANDS_HZ[b]) + db * Shape::Bell(bell).response(BANDS_HZ[b])));
        assert_eq!(act(&k, "저역컷 75Hz"), Action::SwitchOn, "{bell} {db}: {:?}", k.knobs);
        assert_eq!(act(&k, "LOW 80Hz"), Action::Ok, "{bell} {db}: {:?}", k.knobs);
    }
    // Cutting only LOW is not the switch.
    let k = restore_from_centre(&g, std::array::from_fn(|b| flat[b] - 6.0 * Shape::LowShelf(80.0).response(BANDS_HZ[b])));
    assert_eq!(act(&k, "저역컷 75Hz"), Action::Ok);
}
#[test] fn same_centre_sound_restores_all_ok() {
    let k = restore_from_centre(&group(V3, 0), ltass_db(Voice::Male));
    assert!(k.knobs.iter().all(|k| k.action == Action::Ok), "{:?}", k.knobs);
}
}
