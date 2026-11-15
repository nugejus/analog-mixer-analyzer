//! 노브 추천 엔진(spec "추천 엔진").

use crate::mixers::{ChannelGroup, Control, EQ_RANGE_DB, KNOB_TRAVEL_DEG};
use crate::speech::Summary;
use crate::target::BANDS_HZ;

const LAMBDA: f32 = 0.01;
const FIT_BANDS: std::ops::RangeInclusive<usize> = 2..=23; // 100 Hz–12.5 kHz
const EQ_LIMIT_DB: f32 = 15.0;
const LOW_BOOST_MAX_DB: f32 = 3.0;
const OK_BELOW_DB: f32 = 1.5;
const OK_RELEASE_DB: f32 = 2.5;
pub const NO_INPUT: &str = "입력 신호가 없습니다. 케이블과 입력장치를 확인하세요. macOS에서는 시스템 설정 > 개인정보 보호 및 보안 > 마이크에서 이 앱을 허용하세요.";
const KEEP_TALKING: &str = "말소리가 부족합니다. [분석]을 다시 누르고 녹음이 끝날 때까지 평소처럼 말씀해 주세요.";
pub const OVERLOAD: &str = "과부하 의심: 아날로그 단에서 눌리거나 COMP가 과한 경우입니다.";

fn weight(b: usize) -> f32 {
    if b == 23 { 0.5 } else { 1.0 }
}

/// 노브 1 dB당 각 주파수에서의 dB 변화.
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

/// Σ w(−d − c − Σ g·R)² + λΣg² 최소화. 반환: (gains, c, 가중 잔차 Σ w·r²).
pub fn fit_eq(deviation: &[f32; 25], shapes: &[Shape]) -> (Vec<f32>, f32, f32) {
    let n = shapes.len() + 1;
    let row = |b: usize| -> Vec<f32> {
        std::iter::once(1.0)
            .chain(shapes.iter().map(|s| s.response(BANDS_HZ[b])))
            .collect()
    };
    // 확대 행렬 [AᵀWA + λD | AᵀWy], y = −d
    let mut m = vec![vec![0.0; n + 1]; n];
    for b in FIT_BANDS {
        let (a, w) = (row(b), weight(b));
        for (mi, &ai) in m.iter_mut().zip(&a) {
            for (mij, &aj) in mi.iter_mut().zip(&a) {
                *mij += w * ai * aj;
            }
            mi[n] -= w * ai * deviation[b];
        }
    }
    for (i, mi) in m.iter_mut().enumerate().skip(1) {
        mi[i] += LAMBDA;
    }
    // 대칭 양정치(Σw > 0, λ > 0)라 피벗 없이 가우스 소거해도 됩니다.
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
    let resid = FIT_BANDS
        .map(|b| {
            let fit: f32 = row(b).iter().zip(&x).map(|(a, x)| a * x).sum();
            weight(b) * (-deviation[b] - fit).powi(2)
        })
        .sum();
    let c = x.remove(0);
    (x, c, resid)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Wait,
    Ok,
    /// degrees > 0이면 시계방향.
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
    /// −c(spec 식의 c): 측정 ≈ 목표 + level_offset_db. 추천이 없으면 0.
    pub level_offset_db: f32,
}

pub struct Inputs<'a> {
    pub summary: Option<&'a Summary>,
    pub target: &'a [f32; 25],
    pub clipped_recently: bool,
    pub feedback_hz: Option<f32>,
    pub silent: bool,
}

/// 5° 단위로 반올림한 회전 각도.
pub fn rotate_degrees(db: f32, deg_per_db: f32) -> f32 {
    (db * deg_per_db / 5.0).round() * 5.0
}

/// 유효숫자 3자리: 400Hz, 2.5kHz, 12kHz, 1.25kHz.
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

/// 그룹 순서대로 라벨을 붙입니다. `act(i, is_freq)`가 controls[i]의 동작을 줍니다.
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
                // 팩토리가 높은 대역부터 넣으므로 순서대로 1, 2.
                n += 1;
                let sfx = if sweeps > 1 { format!(" {n}") } else { String::new() };
                push(format!("FREQ{sfx}"), true);
                push(format!("MID{sfx}"), false);
            }
        }
    }
    out
}

#[derive(Default)]
pub struct AdviceEngine {
    controls: Vec<Control>,
    /// controls[i]의 ✓ 상태(EQ 노브만 씀).
    ok: Vec<bool>,
    /// 스윕 MID의 지난 추천 주파수.
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
                    knobs: knobs(group, |_, _| Action::Wait),
                    notes: vec![note.into()],
                    level_offset_db: 0.0,
                };
            }
        };
        let d: [f32; 25] = std::array::from_fn(|b| s.bands_db[b] - inp.target[b]);
        let ctl = &group.controls;

        // EQ 노브와 각 노브의 Bell 주파수(스윕은 지난 추천, 없으면 후보 가운데).
        let eq: Vec<usize> = (0..ctl.len())
            .filter(|&i| {
                matches!(
                    ctl[i],
                    Control::HighShelf { .. } | Control::Mid { .. } | Control::LowShelf { .. }
                )
            })
            .collect();
        let cands =
            |(lo, hi): (f32, f32)| -> Vec<f32> { BANDS_HZ.into_iter().filter(|f| (lo..=hi).contains(f)).collect() };
        let mut hz: Vec<f32> = ctl
            .iter()
            .zip(&self.freq)
            .map(|(&c, f)| match (c, sweep(c)) {
                (_, Some(r)) => f
                    .or_else(|| {
                        let v = cands(r);
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

        // 스윕 MID: 후보 중 잔차 최소, 지난 주파수보다 20% 이상 좋을 때만 바꿈. 둘이면 좌표 하강 2회.
        let sweeps: Vec<usize> = eq.iter().copied().filter(|&i| sweep(ctl[i]).is_some()).collect();
        for _ in 0..sweeps.len().min(2) {
            for &i in &sweeps {
                let resid = |f: f32| {
                    let mut h = hz.clone();
                    h[i] = f;
                    fit_eq(&d, &shapes(&h)).2
                };
                // 두 스윕이 같은 주파수에서 게인을 반씩 나누면 둘 다 ✓가 되므로 1옥타브 이상 떨어뜨립니다.
                let best = cands(sweep(ctl[i]).expect("sweep"))
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
        for &i in &sweeps {
            self.freq[i] = Some(hz[i]);
        }
        let (gains, c, _) = fit_eq(&d, &shapes(&hz));

        let mut main = vec![Action::Ok; ctl.len()];
        let mut freq_act = vec![Action::Ok; ctl.len()];
        for (&i, &g) in eq.iter().zip(&gains) {
            let max = if matches!(ctl[i], Control::LowShelf { .. }) {
                LOW_BOOST_MAX_DB
            } else {
                EQ_LIMIT_DB
            };
            let g = g.clamp(-EQ_LIMIT_DB, max);
            let ok = &mut self.ok[i];
            if *ok && g.abs() > OK_RELEASE_DB {
                *ok = false;
            } else if !*ok && g.abs() < OK_BELOW_DB {
                *ok = true;
            }
            if !*ok {
                main[i] = Action::Rotate {
                    degrees: rotate_degrees(g, KNOB_TRAVEL_DEG / (2.0 * EQ_RANGE_DB)),
                    db: g,
                };
                freq_act[i] = Action::SetFreq { hz: hz[i] };
            }
        }

        let mean = |r: std::ops::RangeInclusive<usize>| d[r.clone()].iter().sum::<f32>() / r.count() as f32;
        let c0 = mean(6..=18); // 250 Hz–4 kHz
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
                Control::LowCut { .. } if mean(0..=1) - c0 > 3.0 => Action::SwitchOn,
                // ponytail: COMP는 다이내믹 레인지/크레스트만 보는 ±30° 고정 휴리스틱입니다.
                // 실제 threshold/ratio와 압축량은 모릅니다. 게인 리덕션을 추정할 수 있게 되면 각도를 계산하세요.
                Control::Comp if s.crest_db < 9.0 => Action::Rotate {
                    degrees: -30.0,
                    db: 0.0,
                },
                Control::Comp if s.dynamic_range_db > 18.0 => Action::Rotate { degrees: 30.0, db: 0.0 },
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
    Summary { bands_db: bands, display_db: vec![], peak_db: -12.0, rms_db: -30.0, noise_floor_db: -80.0, snr_db: 50.0,
              dynamic_range_db: 10.0, crest_db: 18.0, active_seconds: 4.0 }
}
fn run(e: &mut AdviceEngine, g: &ChannelGroup, s: &Summary) -> AdviceSet {
    let t = ltass_db(Voice::Male);
    e.advise(g, &Inputs { summary: Some(s), target: &t, clipped_recently: false, feedback_hz: None, silent: false })
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
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &summary(|f| 6.0 * Shape::LowShelf(80.0).response(f)));
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
#[test] fn clip_turns_gain_down() {
    let mut s = summary(|f| 20.0 * Shape::Bell(3_000.0).response(f));
    s.peak_db = -2.0;
    let t = ltass_db(Voice::Male);
    let a = AdviceEngine::new().advise(&group(V3, 0), &Inputs { summary: Some(&s), target: &t, clipped_recently: true, feedback_hz: None, silent: false });
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
    s.crest_db = 7.0; s.peak_db = -10.0;
    let a = run(&mut AdviceEngine::new(), &group(V3, 0), &s);
    assert_eq!(act(&a, "COMP"), Action::Rotate { degrees: -30.0, db: 0.0 });
    assert!(a.notes.iter().any(|n| n.contains("과부하")));
}
#[test] fn notes_for_feedback_and_snr() {
    let mut s = summary(|_| 0.0);
    s.snr_db = 20.0;
    let t = ltass_db(Voice::Male);
    let a = AdviceEngine::new().advise(&group(V3, 0), &Inputs { summary: Some(&s), target: &t, clipped_recently: false, feedback_hz: Some(2_500.0), silent: false });
    assert!(a.notes.iter().any(|n| n.contains("2.5kHz")));
    assert!(a.notes.iter().any(|n| n.contains("마이크를")));
}
#[test] fn silent_input_waits() {
    let t = ltass_db(Voice::Male);
    let mut e = AdviceEngine::new();
    let a = e.advise(&group(V3, 0), &Inputs { summary: None, target: &t, clipped_recently: false, feedback_hz: None, silent: true });
    assert!(a.knobs.iter().all(|k| k.action == Action::Wait));
    assert!(a.notes.iter().any(|n| n.contains("입력 신호가 없습니다")));
    let b = e.advise(&group(V3, 0), &Inputs { summary: None, target: &t, clipped_recently: false, feedback_hz: None, silent: false });
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
#[test] fn rounding_and_labels() {
    assert_eq!(rotate_degrees(1.6, 10.0), 15.0);
    assert_eq!(rotate_degrees(-3.2, 10.0), -30.0);
    assert_eq!(rotate_degrees(2.4, 10.0), 25.0);
    assert_eq!([fmt_hz(400.0), fmt_hz(2_500.0), fmt_hz(12_000.0), fmt_hz(1_250.0), fmt_hz(80.0)],
               ["400Hz", "2.5kHz", "12kHz", "1.25kHz", "80Hz"]);
}
}
