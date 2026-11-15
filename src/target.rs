use crate::advice::Shape;
use crate::mixers::{ChannelGroup, Control};

pub const BANDS_HZ: [f32; 25] = [
    63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0, 630.0, 800.0, 1000.0,
    1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0, 10000.0, 12500.0,
    16000.0,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Voice {
    Male,
    Female,
}

/// 2차 하이패스(12 dB/옥타브) 응답: −10·log10(1 + (fc/f)^4).
pub fn hpf_db(fc: f32, hz: f32) -> f32 {
    -10.0 * (1.0 + (fc / hz).powi(4)).log10()
}

/// Half-width in octaves of a Bell (sigma 0.6 octave).
pub const BELL_OCT: f32 = 0.7;

/// 세팅 표의 EQ 변화: (hz, gain_db, width_oct). Bell 폭 0이면 σ = 0.6옥타브.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    Hpf(f32),
    LowShelf(f32, f32),
    HighShelf(f32, f32),
    Bell(f32, f32, f32),
}

pub struct Preset {
    pub category: &'static str,
    pub name: &'static str,
    pub when: &'static str,
    pub moves: &'static [Move],
}

/// spec "세팅 표". 0번 = 기본.
pub const PRESETS: [Preset; 6] = [
    Preset { category: "기본", name: "자연스러운 목소리", when: "목소리를 있는 그대로 자연스럽게 맞춥니다.", moves: &[] },
    Preset {
        category: "찬양인도자",
        name: "또렷한 찬양 인도",
        when: "밴드 소리에 인도자 목소리가 묻히고 가사가 잘 안 들릴 때",
        moves: &[Move::Hpf(100.0), Move::Bell(4000.0, 3.0, 0.0), Move::HighShelf(10000.0, 2.5)],
    },
    Preset {
        category: "찬양인도자",
        name: "가벼운 찬양 인도",
        when: "밴드와 섞일 때 목소리가 두껍고 웅웅거릴 때(특히 남성 인도자)",
        moves: &[Move::Hpf(100.0), Move::Bell(300.0, -4.5, 0.0), Move::HighShelf(10000.0, 2.5)],
    },
    Preset {
        category: "설교자",
        name: "명료한 설교",
        when: "설교 말씀을 더 또렷하게 들리게 하고 싶을 때",
        moves: &[Move::Hpf(100.0), Move::Bell(3000.0, 4.0, 0.0)],
    },
    Preset {
        category: "보조보컬",
        name: "리드 뒤로",
        when: "보조보컬이 인도자와 겹쳐 같이 앞으로 튀어나올 때",
        moves: &[Move::Hpf(100.0), Move::Bell(3750.0, -3.0, 0.0)],
    },
    Preset {
        category: "보조보컬",
        name: "남성 보조보컬",
        when: "남성 보조보컬의 저음이 두꺼워 인도자나 악기를 덮을 때",
        moves: &[Move::Hpf(135.0), Move::Bell(300.0, -4.5, 0.0)],
    },
];

/// `m`이 `hz`에서 바꾸는 dB.
pub fn move_db(m: &Move, hz: f32) -> f32 {
    match *m {
        Move::Hpf(fc) => hpf_db(fc, hz),
        Move::LowShelf(fc, g) => g * Shape::LowShelf(fc).response(hz),
        Move::HighShelf(fc, g) => g * Shape::HighShelf(fc).response(hz),
        Move::Bell(fc, g, w) => {
            let s = if w == 0.0 { 0.6 } else { w / (2.0 * (2.0 * std::f32::consts::LN_2).sqrt()) };
            g * (-(hz / fc).log2().powi(2) / (2.0 * s * s)).exp()
        }
    }
}

/// LTASS + 세팅의 move. hpf는 그룹의 저역컷 스위치 hz로 바꾸고, 맡을 노브가 없는 move는 빼서 그 hz를 돌려줍니다.
pub fn preset_target(p: &Preset, voice: Voice, group: &ChannelGroup) -> ([f32; 25], Vec<f32>) {
    let ctl = &group.controls;
    let cut = ctl.iter().find_map(|&c| match c {
        Control::LowCut { hz } => Some(hz),
        _ => None,
    });
    let mut t = ltass_db(voice);
    let mut dropped = Vec::new();
    for &m in p.moves {
        let (Move::Hpf(hz) | Move::LowShelf(hz, _) | Move::HighShelf(hz, _) | Move::Bell(hz, ..)) = m;
        let (m, keep) = match m {
            Move::Hpf(_) => (Move::Hpf(cut.unwrap_or(hz)), true),
            Move::LowShelf(..) => (m, ctl.iter().any(|c| matches!(c, Control::LowShelf { .. }))),
            Move::HighShelf(..) => (m, ctl.iter().any(|c| matches!(c, Control::HighShelf { .. }))),
            Move::Bell(..) => (
                m,
                ctl.iter().any(|&c| match c {
                    Control::Mid { lo_hz, hi_hz } if lo_hz == hi_hz => (hz / lo_hz).log2().abs() <= BELL_OCT,
                    Control::Mid { lo_hz, hi_hz } => (lo_hz..=hi_hz).contains(&hz),
                    _ => false,
                }),
            ),
        };
        if keep {
            for (v, f) in t.iter_mut().zip(BANDS_HZ) {
                *v += move_db(&m, f);
            }
        } else {
            dropped.push(hz);
        }
    }
    (t, dropped)
}

/// Tone-colour ranges (name, lo Hz, hi Hz); a third-octave centre in [lo, hi) belongs to it.
pub const TONE_BANDS: [(&str, f32, f32); 8] = [
    ("웅웅거림", 80.0, 250.0),
    ("먹먹함", 250.0, 500.0),
    ("통 울림", 500.0, 800.0),
    ("비음", 800.0, 1500.0),
    ("까랑함", 1500.0, 3000.0),
    ("쏘는 소리", 3000.0, 5000.0),
    ("치찰음", 5000.0, 10000.0),
    ("공기감", 10000.0, 20000.0),
];

/// Per tone band: 10·log10(Σ measured power / Σ reference power), in dB.
pub fn tone_excess(measured: &[f32; 25], reference: &[f32; 25]) -> [f32; 8] {
    TONE_BANDS.map(|(_, lo, hi)| {
        let (mut m, mut r) = (0.0f32, 0.0f32);
        for i in (0..25).filter(|&i| (lo..hi).contains(&BANDS_HZ[i])) {
            m += 10f32.powf(measured[i] / 10.0);
            r += 10f32.powf(reference[i] / 10.0);
        }
        10.0 * (m / r).log10()
    })
}

/// Level that lines the goal up with the measurement on the graph: the median of measured − goal
/// over 100 Hz–12.5 kHz, so a few loud bands (what the graph should reveal) do not drag it up.
pub fn level_match_db(measured: &[f32; 25], goal: &[f32; 25]) -> f32 {
    let mut d: Vec<f32> = (2..=23).map(|i| measured[i] - goal[i]).collect();
    d.sort_by(f32::total_cmp);
    (d[10] + d[11]) / 2.0
}

/// Byrne 1994 LTASS, 1/3-octave dB.
pub fn ltass_db(voice: Voice) -> [f32; 25] {
    match voice {
        Voice::Male => [
            38.6, 43.5, 54.4, 57.7, 56.8, 58.2, 59.7, 60.0, 62.4, 62.6, 60.6, 55.7, 53.1, 53.7,
            52.3, 48.7, 48.9, 47.0, 46.0, 44.4, 43.3, 42.4, 41.9, 39.8, 40.4,
        ],
        Voice::Female => [
            37.0, 36.0, 37.5, 40.1, 53.4, 62.2, 60.9, 58.1, 61.7, 61.7, 60.4, 58.0, 54.3, 52.3,
            51.7, 48.8, 47.3, 46.7, 45.3, 44.6, 45.2, 44.9, 45.0, 42.8, 41.1,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::mixers::{MixerFactory, MixerModel};
    fn g(m: MixerModel) -> crate::mixers::ChannelGroup { MixerFactory::create(m).groups.remove(0) }
    #[test] fn preset_table_matches_spec() {
        let c: Vec<_> = PRESETS.iter().map(|p| p.category).collect();
        assert_eq!(c, ["기본", "찬양인도자", "찬양인도자", "설교자", "보조보컬", "보조보컬"]);
        let n: Vec<_> = PRESETS.iter().map(|p| p.name).collect();
        assert_eq!(n, ["자연스러운 목소리", "또렷한 찬양 인도", "가벼운 찬양 인도", "명료한 설교", "리드 뒤로", "남성 보조보컬"]);
        assert_eq!(PRESETS[1].moves, [Move::Hpf(100.0), Move::Bell(4000.0, 3.0, 0.0), Move::HighShelf(10000.0, 2.5)]);
        assert_eq!(PRESETS[5].moves, [Move::Hpf(135.0), Move::Bell(300.0, -4.5, 0.0)]);
    }
    #[test] fn move_formulas() {
        let near = |a: f32, b: f32| assert!((a - b).abs() < 0.01, "{a} vs {b}");
        near(move_db(&Move::Hpf(100.0), 100.0), -3.01);
        near(move_db(&Move::LowShelf(80.0, -6.0), 80.0), -3.0);
        near(move_db(&Move::HighShelf(10000.0, 2.5), 10000.0), 1.25);
        near(move_db(&Move::Bell(1000.0, 4.0, 0.0), 2000.0), 0.997);
        near(move_db(&Move::Bell(1000.0, 4.0, 1.0), 1000.0 * 2f32.sqrt()), 2.0);
    }
    #[test] fn natural_preset_is_ltass() {
        assert_eq!(preset_target(&PRESETS[0], Voice::Female, &g(MixerModel::MackieProFx22v3)), (ltass_db(Voice::Female), vec![]));
    }
    #[test] fn hpf_uses_group_switch_hz() {
        let (t, _) = preset_target(&PRESETS[3], Voice::Male, &g(MixerModel::Mackie1202Vlz4)); // LowCut 75
        let want = ltass_db(Voice::Male)[0] + hpf_db(75.0, 63.0) + move_db(&Move::Bell(3000.0, 4.0, 0.0), 63.0);
        assert!((t[0] - want).abs() < 1e-4);
        let (t, _) = preset_target(&PRESETS[5], Voice::Male, &g(MixerModel::SoundcraftEpm)); // 스위치 없음
        assert!((t[0] - (ltass_db(Voice::Male)[0] + hpf_db(135.0, 63.0) + move_db(&Move::Bell(300.0, -4.5, 0.0), 63.0))).abs() < 1e-4);
    }
    #[test] fn moves_without_knob_are_dropped() {
        let dropped = |i: usize, m| preset_target(&PRESETS[i], Voice::Male, &g(m)).1;
        assert_eq!(dropped(2, MixerModel::MackieProFx12v3), [300.0]); // 고정 MID 2.5k
        assert_eq!(dropped(4, MixerModel::MackieProFx12v3), Vec::<f32>::new()); // 3750은 0.7옥타브 안
        assert_eq!(dropped(3, MixerModel::AhZedi10), [3000.0]); // 고정 MID 600
        assert_eq!(dropped(4, MixerModel::YamahaMg06x), [3750.0]); // MID 없음
        assert_eq!(dropped(2, MixerModel::MackieProFx22v3), Vec::<f32>::new());
        let (t, _) = preset_target(&PRESETS[2], Voice::Male, &g(MixerModel::MackieProFx12v3));
        let want = ltass_db(Voice::Male)[7] + hpf_db(100.0, 315.0) + move_db(&Move::HighShelf(10000.0, 2.5), 315.0);
        assert!((t[7] - want).abs() < 1e-4);
    }

    #[test]
    fn table_spot_checks() {
        assert_eq!(ltass_db(Voice::Male)[12], 53.1);
        assert_eq!(ltass_db(Voice::Female)[5], 62.2);
        assert_eq!((BANDS_HZ[0], BANDS_HZ[24]), (63.0, 16000.0));
    }

    #[test]
    fn tone_bands_partition_third_octaves() {
        for (i, &hz) in BANDS_HZ.iter().enumerate() {
            let n = TONE_BANDS.iter().filter(|&&(_, lo, hi)| (lo..hi).contains(&hz)).count();
            assert_eq!(n, if i == 0 { 0 } else { 1 }, "{hz} Hz");
        }
    }

    #[test]
    fn tone_excess_sums_power() {
        let reference = [0.0f32; 25];
        assert!(tone_excess(&reference, &reference).iter().all(|d| d.abs() < 1e-4));
        let mut m = reference;
        m[15] = 6.0; // 2 kHz only: 10·log10((1 + 10^0.6 + 1) / 3) ≈ 3.0, not the dB mean 2.0
        let e = tone_excess(&m, &reference);
        assert!((e[4] - 2.996).abs() < 0.01, "{}", e[4]);
        assert!(e.iter().enumerate().all(|(i, d)| i == 4 || d.abs() < 1e-4));
    }

    #[test]
    fn level_match_ignores_a_few_loud_bands() {
        let goal = [-40.0f32; 25];
        let mut m = goal.map(|g| g + 5.0);
        assert!((level_match_db(&m, &goal) - 5.0).abs() < 1e-4);
        for v in &mut m[11..=18] {
            *v += 8.0; // 800 Hz–4 kHz: 8 of the 22 matched bands
        }
        assert!((level_match_db(&m, &goal) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn tone_excess_follows_reference_level() {
        let reference = [-40.0f32; 25];
        let mut m = reference;
        m[14..=16].iter_mut().for_each(|v| *v += 6.0); // 1600, 2000, 2500 = 까랑함
        let e = tone_excess(&m, &reference);
        assert_eq!(TONE_BANDS[4].0, "까랑함");
        assert!((e[4] - 6.0).abs() < 0.01 && e[3].abs() < 1e-4 && e[5].abs() < 1e-4);
    }
}
