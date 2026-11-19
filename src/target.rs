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

/// 2nd-order high-pass (12 dB/octave) response: −10·log10(1 + (fc/f)^4).
pub fn hpf_db(fc: f32, hz: f32) -> f32 {
    -10.0 * (1.0 + (fc / hz).powi(4)).log10()
}

/// Half-width in octaves of a Bell (sigma 0.6 octave).
pub const BELL_OCT: f32 = 0.7;

/// EQ change from the settings table: (hz, gain_db, width_oct). With Bell width 0, σ = 0.6 octave.
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

/// spec "Settings table". Index 0 = default.
pub const PRESETS: [Preset; 6] = [
    Preset { category: "Default", name: "Natural voice", when: "Keeps the voice natural, as it is.", moves: &[] },
    Preset {
        category: "Worship leader",
        name: "Clear lead",
        when: "When the band buries the leader's voice and the lyrics are hard to hear",
        moves: &[Move::Hpf(100.0), Move::Bell(4000.0, 3.0, 0.0), Move::HighShelf(10000.0, 2.5)],
    },
    Preset {
        category: "Worship leader",
        name: "Lighter lead",
        when: "When the voice sounds thick and boomy in the band mix (especially male leaders)",
        moves: &[Move::Hpf(100.0), Move::Bell(300.0, -4.5, 0.0), Move::HighShelf(10000.0, 2.5)],
    },
    Preset {
        category: "Preacher",
        name: "Clear sermon",
        when: "When you want the sermon to come through more clearly",
        moves: &[Move::Hpf(100.0), Move::Bell(3000.0, 4.0, 0.0)],
    },
    Preset {
        category: "Backing vocal",
        name: "Behind the lead",
        when: "When a backing vocal competes with the leader and jumps out front",
        moves: &[Move::Hpf(100.0), Move::Bell(3750.0, -3.0, 0.0)],
    },
    Preset {
        category: "Backing vocal",
        name: "Male backing",
        when: "When a male backing vocal's thick low end covers the leader or the instruments",
        moves: &[Move::Hpf(135.0), Move::Bell(300.0, -4.5, 0.0)],
    },
];

/// dB that `m` changes at `hz`.
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

/// LTASS + the setting's moves. hpf is changed to the group's low-cut switch hz; moves with no knob to handle them (including hpf with no switch) are removed and their hz returned.
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
            Move::Hpf(_) => (Move::Hpf(cut.unwrap_or(hz)), cut.is_some()),
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
    ("Boom", 80.0, 250.0),
    ("Mud", 250.0, 500.0),
    ("Boxy", 500.0, 800.0),
    ("Nasal", 800.0, 1500.0),
    ("Harsh", 1500.0, 3000.0),
    ("Sharp", 3000.0, 5000.0),
    ("Sibilance", 5000.0, 10000.0),
    ("Air", 10000.0, 20000.0),
];

/// A tone band this far above (red) or below (blue) the reference is flagged on the graph.
pub const TONE_HOT_DB: f32 = 3.0;

/// Per tone band: 10·log10(Σ measured power / Σ reference power), in dB, rounded to 0.1 so the
/// graph and the knob advice see the same value (+ 0.0 keeps -0.0 out).
pub fn tone_excess(measured: &[f32; 25], reference: &[f32; 25]) -> [f32; 8] {
    TONE_BANDS.map(|(_, lo, hi)| {
        let (mut m, mut r) = (0.0f32, 0.0f32);
        for i in (0..25).filter(|&i| (lo..hi).contains(&BANDS_HZ[i])) {
            m += 10f32.powf(measured[i] / 10.0);
            r += 10f32.powf(reference[i] / 10.0);
        }
        (10.0 * (m / r).log10() * 10.0).round() / 10.0 + 0.0
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
        assert_eq!(c, ["Default", "Worship leader", "Worship leader", "Preacher", "Backing vocal", "Backing vocal"]);
        let n: Vec<_> = PRESETS.iter().map(|p| p.name).collect();
        assert_eq!(n, ["Natural voice", "Clear lead", "Lighter lead", "Clear sermon", "Behind the lead", "Male backing"]);
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
    }
    #[test] fn hpf_dropped_without_switch() {
        // With no switch, remove it so LOW does not mimic the low cut and cut hard.
        let (t, dropped) = preset_target(&PRESETS[3], Voice::Male, &g(MixerModel::SoundcraftEpm));
        let want: [f32; 25] = std::array::from_fn(|i| ltass_db(Voice::Male)[i] + move_db(&Move::Bell(3000.0, 4.0, 0.0), BANDS_HZ[i]));
        assert!(t.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-4) && dropped == [100.0], "{t:?} {dropped:?}");
        let stereo = MixerFactory::create(MixerModel::SoundcraftSignature12).groups.remove(1);
        assert_eq!(preset_target(&PRESETS[3], Voice::Male, &stereo), (ltass_db(Voice::Male), vec![100.0, 3000.0]));
    }
    #[test] fn moves_without_knob_are_dropped() {
        let dropped = |i: usize, m| preset_target(&PRESETS[i], Voice::Male, &g(m)).1;
        assert_eq!(dropped(2, MixerModel::MackieProFx12v3), [300.0]); // fixed MID 2.5k
        assert_eq!(dropped(4, MixerModel::MackieProFx12v3), Vec::<f32>::new()); // 3750 is within 0.7 octave
        assert_eq!(dropped(3, MixerModel::AhZedi10), [3000.0]); // fixed MID 600
        assert_eq!(dropped(4, MixerModel::YamahaMg06x), [3750.0]); // no MID
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
        m[14..=16].iter_mut().for_each(|v| *v += 6.0); // 1600, 2000, 2500 = harshness
        let e = tone_excess(&m, &reference);
        assert_eq!(TONE_BANDS[4].0, "Harsh");
        assert!((e[4] - 6.0).abs() < 0.01 && e[3].abs() < 1e-4 && e[5].abs() < 1e-4);
    }
}
