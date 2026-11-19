//! Analog mixer profiles (spec "Mixer data"). Only channel groups with mic inputs.

pub const EQ_RANGE_DB: f32 = 15.0;
pub const KNOB_TRAVEL_DEG: f32 = 300.0;

const MACKIE_USB: &str = "USB sends only the main mix to the PC (not PFL). Bring up only the channel you are adjusting; pull down the other channels and FX. Set its channel fader to U (0 dB).";
const YAMAHA_MGP: &str = "This mixer has no USB audio to the PC, so you need an audio interface. Press PFL on the channel you are adjusting and connect MONITOR OUT L to the interface. Leave the MONITOR knob in one position.";
const YAMAHA_XU: &str = "USB carries the STEREO bus (after the channel faders). Send only the channel you are adjusting to ST and turn the other channels off. Set its channel fader to 0 dB.";
const BEHRINGER_USB: &str = "USB carries the main mix. Bring up only the channel you are adjusting; pull down the other channels and FX.";
const SOUNDCRAFT_USB: &str = "With USB SEND on MST, USB carries the main mix. Bring up only the channel you are adjusting; pull down the other channels.";
const AH_USB: &str = "With all USB output select switches released, USB carries the main L-R. Bring up only the channel you are adjusting; pull down the other channels.";
const NO_USB_SOLO: &str = "This mixer has no USB. SOLO/PFL the channel you are adjusting and connect the CONTROL ROOM (or PHONES) output to the PC through an audio interface.";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Gain { range_db: f32 },
    LowCut { hz: f32 },
    HighShelf { hz: f32 },
    /// If `lo_hz == hi_hz`, a fixed MID.
    Mid { lo_hz: f32, hi_hz: f32 },
    LowShelf { hz: f32 },
    Comp,
}

pub struct ChannelGroup {
    pub label: &'static str,
    pub controls: Vec<Control>,
}

pub struct MixerProfile {
    pub brand: &'static str,
    pub name: &'static str,
    pub groups: Vec<ChannelGroup>,
    pub pc_tip: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixerModel {
    MackieProFx22,
    MackieProFx22v2,
    MackieProFx22v3,
    MackieProFx10v3,
    MackieProFx12v3,
    MackieProFx16v3,
    MackieProFx30v3,
    Mackie1202Vlz4,
    Mackie1604Vlz4,
    YamahaMgp24x,
    YamahaMgp32x,
    YamahaMgp16x,
    YamahaMgp12x,
    YamahaMg20xu,
    YamahaMg16xu,
    YamahaMg12xu,
    YamahaMg10xu,
    YamahaMg06x,
    BehringerX1222Usb,
    BehringerQx1222Usb,
    BehringerX1832Usb,
    BehringerX2222Usb,
    BehringerQx2222Usb,
    BehringerX2442Usb,
    SoundcraftSignature12,
    SoundcraftSignature22,
    SoundcraftEpm,
    AhZed10,
    AhZed12Fx,
    AhZed14,
    AhZedi10,
    AhZed6,
}

impl MixerModel {
    pub const ALL: [MixerModel; 32] = {
        use MixerModel::*;
        [
            MackieProFx22, MackieProFx22v2, MackieProFx22v3, MackieProFx10v3, MackieProFx12v3, MackieProFx16v3, MackieProFx30v3, Mackie1202Vlz4, Mackie1604Vlz4, YamahaMgp24x, YamahaMgp32x, YamahaMgp16x, YamahaMgp12x, YamahaMg20xu, YamahaMg16xu, YamahaMg12xu, YamahaMg10xu, YamahaMg06x, BehringerX1222Usb, BehringerQx1222Usb, BehringerX1832Usb, BehringerX2222Usb, BehringerQx2222Usb, BehringerX2442Usb, SoundcraftSignature12, SoundcraftSignature22, SoundcraftEpm, AhZed10, AhZed12Fx, AhZed14, AhZedi10, AhZed6,
        ]
    };
}

/// Order: Gain, LowCut?, HighShelf, Mid (from the highest band), LowShelf, Comp?
fn strip(
    label: &'static str,
    gain: f32,
    cut: Option<f32>,
    high: f32,
    mids: &[(f32, f32)],
    low: f32,
    comp: bool,
) -> ChannelGroup {
    let mut controls = vec![Control::Gain { range_db: gain }];
    controls.extend(cut.map(|hz| Control::LowCut { hz }));
    controls.push(Control::HighShelf { hz: high });
    controls.extend(mids.iter().map(|&(lo_hz, hi_hz)| Control::Mid { lo_hz, hi_hz }));
    controls.push(Control::LowShelf { hz: low });
    if comp {
        controls.push(Control::Comp);
    }
    ChannelGroup { label, controls }
}

pub struct MixerFactory;

impl MixerFactory {
    // Medium confidence (docs disagree on values or could not be confirmed from docs): Behringer low cut 75/80 Hz,
    // Soundcraft Signature gain range, EPM LOW 60/80 Hz, ZEDi gain, QX1222 low cut,
    // Signature stereo HPF, X2442 stereo mic gain. Values below are exactly the spec table.
    pub fn create(model: MixerModel) -> MixerProfile {
        use MixerModel::*;
        let (brand, name, pc_tip, groups) = match model {
        MackieProFx22 => ("Mackie", "ProFX22 (1st gen, 2011)", MACKIE_USB, vec![
            strip("CH1-10", 50.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, false),
            strip("CH11-14 (COMP)", 50.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, true),
            strip("CH15-18 (stereo mic)", 50.0, Some(100.0), 12000.0, &[(2500.0, 2500.0)], 80.0, false),
        ]),
        MackieProFx22v2 => ("Mackie", "ProFX22v2 (2015, VITA preamps)", MACKIE_USB, vec![
            strip("CH1-10", 50.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, false),
            strip("CH11-14 (COMP)", 50.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, true),
            strip("CH15-18 (stereo mic)", 50.0, Some(100.0), 12000.0, &[(2500.0, 2500.0)], 80.0, false),
        ]),
        MackieProFx22v3 => ("Mackie", "ProFX22v3 (2019, LEVEL SET LED)", MACKIE_USB, vec![
            strip("CH1-12 (COMP)", 60.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, true),
            strip("CH13-14", 60.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, false),
            strip("CH15-20 (stereo mic)", 60.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, false),
        ]),
        MackieProFx10v3 => ("Mackie", "ProFX10v3", MACKIE_USB, vec![
            strip("CH1-2 (COMP)", 60.0, Some(100.0), 12000.0, &[(2500.0, 2500.0)], 80.0, true),
            strip("CH3-4", 60.0, Some(100.0), 12000.0, &[(2500.0, 2500.0)], 80.0, false),
        ]),
        MackieProFx12v3 => ("Mackie", "ProFX12v3", MACKIE_USB, vec![
            strip("CH1-4 (COMP)", 60.0, Some(100.0), 12000.0, &[(2500.0, 2500.0)], 80.0, true),
            strip("CH5-10 (stereo mic)", 60.0, Some(100.0), 12000.0, &[(2500.0, 2500.0)], 80.0, false),
        ]),
        MackieProFx16v3 => ("Mackie", "ProFX16v3", MACKIE_USB, vec![
            strip("CH1-8 (COMP)", 60.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, true),
            strip("CH9-14 (stereo mic)", 60.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, false),
        ]),
        MackieProFx30v3 => ("Mackie", "ProFX30v3", MACKIE_USB, vec![
            strip("CH1-16 (COMP)", 60.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, true),
            strip("CH17-22", 60.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, false),
            strip("CH23-28 (stereo mic)", 60.0, Some(100.0), 12000.0, &[(100.0, 8000.0)], 80.0, false),
        ]),
        Mackie1202Vlz4 => ("Mackie", "1202VLZ4 / 1402VLZ4", NO_USB_SOLO, vec![
            strip("Mono CH", 60.0, Some(75.0), 12000.0, &[(2500.0, 2500.0)], 80.0, false),
        ]),
        Mackie1604Vlz4 => ("Mackie", "1604VLZ4", NO_USB_SOLO, vec![
            strip("CH1-16", 60.0, Some(75.0), 12000.0, &[(100.0, 8000.0)], 80.0, false),
        ]),
        YamahaMgp24x => ("Yamaha", "MGP24X", YAMAHA_MGP, vec![
            strip("CH1-8", 44.0, Some(100.0), 8000.0, &[(250.0, 5000.0)], 125.0, false),
            strip("CH9-16 (COMP)", 44.0, Some(100.0), 8000.0, &[(250.0, 5000.0)], 125.0, true),
        ]),
        YamahaMgp32x => ("Yamaha", "MGP32X", YAMAHA_MGP, vec![
            strip("CH1-8", 44.0, Some(100.0), 8000.0, &[(250.0, 5000.0)], 125.0, false),
            strip("CH9-24 (COMP)", 44.0, Some(100.0), 8000.0, &[(250.0, 5000.0)], 125.0, true),
        ]),
        YamahaMgp16x => ("Yamaha", "MGP16X", YAMAHA_MGP, vec![
            strip("CH1-8 (COMP)", 44.0, Some(100.0), 8000.0, &[(250.0, 5000.0)], 125.0, true),
            strip("CH9-12 (mono/stereo)", 44.0, Some(100.0), 8000.0, &[(2500.0, 2500.0)], 125.0, false),
        ]),
        YamahaMgp12x => ("Yamaha", "MGP12X", YAMAHA_MGP, vec![
            strip("CH1-4 (COMP)", 44.0, Some(100.0), 8000.0, &[(250.0, 5000.0)], 125.0, true),
            strip("CH5-8 (mono/stereo)", 44.0, Some(100.0), 8000.0, &[(2500.0, 2500.0)], 125.0, false),
        ]),
        YamahaMg20xu => ("Yamaha", "MG20XU", YAMAHA_XU, vec![
            strip("CH1-8 (COMP)", 44.0, Some(80.0), 10000.0, &[(250.0, 5000.0)], 100.0, true),
            strip("CH9-12", 44.0, Some(80.0), 10000.0, &[(250.0, 5000.0)], 100.0, false),
            strip("CH13-20 (mono/stereo)", 44.0, Some(80.0), 10000.0, &[(2500.0, 2500.0)], 100.0, false),
        ]),
        YamahaMg16xu => ("Yamaha", "MG16XU", YAMAHA_XU, vec![
            strip("CH1-8 (COMP)", 44.0, Some(80.0), 10000.0, &[(250.0, 5000.0)], 100.0, true),
            strip("CH9-12 (mono/stereo)", 44.0, Some(80.0), 10000.0, &[(2500.0, 2500.0)], 100.0, false),
        ]),
        YamahaMg12xu => ("Yamaha", "MG12XU", YAMAHA_XU, vec![
            strip("CH1-4 (COMP)", 44.0, Some(80.0), 10000.0, &[(2500.0, 2500.0)], 100.0, true),
            strip("CH5-8 (mono/stereo)", 44.0, Some(80.0), 10000.0, &[(2500.0, 2500.0)], 100.0, false),
        ]),
        YamahaMg10xu => ("Yamaha", "MG10XU", YAMAHA_XU, vec![
            strip("CH1-2 (COMP)", 44.0, Some(80.0), 10000.0, &[(2500.0, 2500.0)], 100.0, true),
            strip("CH3-4", 44.0, Some(80.0), 10000.0, &[(2500.0, 2500.0)], 100.0, false),
        ]),
        YamahaMg06x => ("Yamaha", "MG06X", NO_USB_SOLO, vec![
            strip("CH1-2", 44.0, Some(80.0), 10000.0, &[], 100.0, false),
        ]),
        BehringerX1222Usb => ("Behringer", "X1222USB", BEHRINGER_USB, vec![
            strip("CH1-4 (COMP)", 50.0, Some(80.0), 12000.0, &[(2500.0, 2500.0)], 80.0, true),
            strip("CH5-8 (stereo mic)", 40.0, None, 12000.0, &[(2500.0, 2500.0)], 80.0, false),
        ]),
        BehringerQx1222Usb => ("Behringer", "QX1222USB", BEHRINGER_USB, vec![
            strip("CH1-4 (COMP)", 50.0, Some(75.0), 12000.0, &[(2500.0, 2500.0)], 80.0, true),
            strip("CH5-8 (stereo mic)", 40.0, None, 12000.0, &[(2500.0, 2500.0)], 80.0, false),
        ]),
        BehringerX1832Usb => ("Behringer", "X1832USB", BEHRINGER_USB, vec![
            strip("CH1-6 (COMP)", 50.0, Some(75.0), 12000.0, &[(100.0, 8000.0)], 80.0, true),
        ]),
        BehringerX2222Usb => ("Behringer", "X2222USB", BEHRINGER_USB, vec![
            strip("CH1-8 (COMP)", 50.0, Some(75.0), 12000.0, &[(100.0, 8000.0)], 80.0, true),
        ]),
        BehringerQx2222Usb => ("Behringer", "QX2222USB", BEHRINGER_USB, vec![
            strip("CH1-8 (COMP)", 50.0, Some(75.0), 12000.0, &[(100.0, 8000.0)], 80.0, true),
        ]),
        BehringerX2442Usb => ("Behringer", "X2442USB", BEHRINGER_USB, vec![
            strip("CH1-8 (COMP)", 50.0, Some(75.0), 12000.0, &[(100.0, 8000.0)], 80.0, true),
            strip("CH9-12 (stereo mic)", 40.0, Some(75.0), 12000.0, &[(3000.0, 3000.0), (500.0, 500.0)], 80.0, false),
        ]),
        SoundcraftSignature12 => ("Soundcraft", "Signature 12", SOUNDCRAFT_USB, vec![
            strip("CH1-6", 50.0, Some(100.0), 12000.0, &[(140.0, 3000.0)], 60.0, false),
            strip("CH7-10 (stereo mic)", 50.0, None, 12000.0, &[(1000.0, 1000.0)], 60.0, false),
        ]),
        SoundcraftSignature22 => ("Soundcraft", "Signature 22", SOUNDCRAFT_USB, vec![
            strip("Mono CH", 50.0, Some(100.0), 12000.0, &[(400.0, 8000.0), (80.0, 2000.0)], 60.0, false),
            strip("CH15-18 (stereo mic)", 50.0, None, 12000.0, &[(140.0, 3000.0)], 60.0, false),
        ]),
        SoundcraftEpm => ("Soundcraft", "EPM 6/8/12", NO_USB_SOLO, vec![
            strip("Mono CH", 55.0, None, 12000.0, &[(150.0, 3500.0)], 80.0, false),
        ]),
        AhZed10 => ("Allen & Heath", "ZED-10 / 10FX", AH_USB, vec![
            strip("M1-M4", 50.0, Some(100.0), 12000.0, &[(120.0, 4000.0)], 80.0, false),
        ]),
        AhZed12Fx => ("Allen & Heath", "ZED-12FX / 16FX / 22FX", AH_USB, vec![
            strip("Mono CH", 69.0, Some(100.0), 12000.0, &[(120.0, 4000.0)], 80.0, false),
        ]),
        AhZed14 => ("Allen & Heath", "ZED-14 / 18 / 24", AH_USB, vec![
            strip("Mono CH", 69.0, Some(100.0), 12000.0, &[(120.0, 4000.0)], 80.0, false),
        ]),
        AhZedi10 => ("Allen & Heath", "ZEDi-10 / 10FX", AH_USB, vec![
            strip("M1-M4", 55.0, Some(100.0), 12000.0, &[(600.0, 600.0)], 80.0, false),
        ]),
        AhZed6 => ("Allen & Heath", "ZED-6 / 6FX / ZEDi-8", NO_USB_SOLO, vec![
            strip("M1-M2", 55.0, Some(100.0), 12000.0, &[], 80.0, false),
        ]),
        };
        MixerProfile { brand, name, groups, pc_tip }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
use Control::*;
fn has(g: &ChannelGroup, c: Control) -> bool { g.controls.contains(&c) }
#[test] fn every_model_builds_a_sane_profile() {
    let mut names = std::collections::HashSet::new();
    for m in MixerModel::ALL {
        let p = MixerFactory::create(m);
        assert!(names.insert(format!("{} {}", p.brand, p.name)));
        assert!(!p.groups.is_empty() && !p.pc_tip.is_empty());
        for g in &p.groups { assert!(matches!(g.controls[0], Gain { .. }), "{}", p.name); }
        let labels: std::collections::HashSet<_> = p.groups.iter().map(|g| g.label).collect();
        assert_eq!(labels.len(), p.groups.len());
    }
}
#[test] fn profx22v3_strip() {
    let p = MixerFactory::create(MixerModel::MackieProFx22v3);
    assert_eq!(p.groups[0].label, "CH1-12 (COMP)");
    for c in [Gain { range_db: 60.0 }, LowCut { hz: 100.0 }, HighShelf { hz: 12_000.0 },
              Mid { lo_hz: 100.0, hi_hz: 8_000.0 }, LowShelf { hz: 80.0 }, Comp] {
        assert!(has(&p.groups[0], c), "{c:?}");
    }
    assert!(!has(&p.groups[1], Comp));
}
#[test] fn mgp24x_strip() {
    let p = MixerFactory::create(MixerModel::YamahaMgp24x);
    assert_eq!(p.groups.iter().map(|g| g.label).collect::<Vec<_>>(), ["CH1-8", "CH9-16 (COMP)"]);
    for c in [Gain { range_db: 44.0 }, LowCut { hz: 100.0 }, HighShelf { hz: 8_000.0 },
              Mid { lo_hz: 250.0, hi_hz: 5_000.0 }, LowShelf { hz: 125.0 }] {
        assert!(has(&p.groups[0], c) && has(&p.groups[1], c), "{c:?}");
    }
    assert!(!has(&p.groups[0], Comp) && has(&p.groups[1], Comp));
    assert!(p.pc_tip.contains("MONITOR OUT"));
}
#[test] fn odd_strips() {
    let sig = MixerFactory::create(MixerModel::SoundcraftSignature22);
    assert!(has(&sig.groups[0], Mid { lo_hz: 80.0, hi_hz: 2_000.0 }) && has(&sig.groups[0], Mid { lo_hz: 400.0, hi_hz: 8_000.0 }));
    let no_cut = |m| !MixerFactory::create(m).groups[0].controls.iter().any(|c| matches!(c, LowCut { .. }));
    let no_mid = |m| !MixerFactory::create(m).groups[0].controls.iter().any(|c| matches!(c, Mid { .. }));
    assert!(no_cut(MixerModel::SoundcraftEpm));
    assert!(no_mid(MixerModel::YamahaMg06x) && no_mid(MixerModel::AhZed6));
    assert!(has(&MixerFactory::create(MixerModel::MackieProFx22).groups[2], Mid { lo_hz: 2_500.0, hi_hz: 2_500.0 }));
    assert!(has(&MixerFactory::create(MixerModel::AhZedi10).groups[0], Mid { lo_hz: 600.0, hi_hz: 600.0 }));
}
#[test] fn stereo_mic_groups() {
    let mg = MixerFactory::create(MixerModel::YamahaMg16xu);
    assert_eq!(mg.groups[1].label, "CH9-12 (mono/stereo)");
    assert!(has(&mg.groups[1], Mid { lo_hz: 2_500.0, hi_hz: 2_500.0 }) && has(&mg.groups[1], LowCut { hz: 80.0 }));
    assert!(!has(&mg.groups[1], Comp));
    let x = MixerFactory::create(MixerModel::BehringerX2442Usb);
    assert_eq!(x.groups[1].label, "CH9-12 (stereo mic)");
    let mids: Vec<_> = x.groups[1].controls.iter().filter(|c| matches!(c, Mid { .. })).collect();
    assert_eq!(mids, [&Mid { lo_hz: 3_000.0, hi_hz: 3_000.0 }, &Mid { lo_hz: 500.0, hi_hz: 500.0 }]);
    for m in [MixerModel::BehringerX1222Usb, MixerModel::BehringerQx1222Usb, MixerModel::SoundcraftSignature12, MixerModel::SoundcraftSignature22] {
        let g = MixerFactory::create(m).groups.pop().unwrap();
        assert!(g.label.contains("stereo mic") && !g.controls.iter().any(|c| matches!(c, LowCut { .. } | Comp)), "{m:?}");
    }
    assert!(has(&MixerFactory::create(MixerModel::SoundcraftSignature12).groups[1], Mid { lo_hz: 1_000.0, hi_hz: 1_000.0 }));
    assert!(has(&MixerFactory::create(MixerModel::SoundcraftSignature22).groups[1], Mid { lo_hz: 140.0, hi_hz: 3_000.0 }));
    assert_eq!(MixerFactory::create(MixerModel::MackieProFx22v3).groups[2].label, "CH15-20 (stereo mic)");
}
}
