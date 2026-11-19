//! UI language. Data (mixer names, channel groups, knob labels, saved.tsv) stays English in both;
//! `local` turns those labels into Korean only where they are drawn.
use std::cell::Cell;
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Ko,
}

// Per thread, not global: the UI and the engine run on the UI thread, and each test thread starts in English.
thread_local!(static LANG: Cell<Lang> = const { Cell::new(Lang::En) });

pub fn set(l: Lang) {
    LANG.set(l);
}

pub fn get() -> Lang {
    LANG.get()
}

pub fn korean() -> bool {
    get() == Lang::Ko
}

/// The current language's text.
pub fn tr(en: &'static str, ko: &'static str) -> &'static str {
    if korean() { ko } else { en }
}

/// An (English, Korean) pair for consts and tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct L(pub &'static str, pub &'static str);

impl L {
    pub fn get(self) -> &'static str {
        tr(self.0, self.1)
    }
}

/// `format!` of the current language: `tr!("Peak {:.0} dBFS", "피크 {:.0} dBFS", x)`.
/// Both strings must use every argument; use `{0}`/named arguments where the word order differs.
#[macro_export]
macro_rules! tr {
    ($en:literal, $ko:literal $(, $($arg:tt)*)?) => {
        if $crate::lang::korean() { format!($ko $(, $($arg)*)?) } else { format!($en $(, $($arg)*)?) }
    };
}

/// 0.5.0 and older wrote Korean mixer, group and knob labels: (Korean, English).
const LEGACY: [(&str, &str); 6] = [
    ("1세대", "1st gen"),
    ("VITA 프리앰프", "VITA preamps"),
    ("스테레오 마이크", "stereo mic"),
    ("모노/스테레오", "mono/stereo"),
    ("모노 CH", "Mono CH"),
    ("저역컷 ", "Low cut "),
];

/// Korean labels from old saved.tsv files to the English ones the profiles use.
pub fn english(s: &str) -> String {
    LEGACY.iter().fold(s.to_owned(), |s, (ko, en)| s.replace(ko, en))
}

/// A mixer name, channel group or knob label as drawn: Korean in Korean mode, else unchanged.
pub fn local(s: &str) -> String {
    if korean() { LEGACY.iter().fold(s.to_owned(), |s, (ko, en)| s.replace(en, ko)) } else { s.to_owned() }
}

const REMEMBER: &str = "language.txt";

/// The language last picked, from `language.txt` in the default folder; missing or anything but `ko` = English.
pub fn remembered(default: &Path) -> Lang {
    match std::fs::read_to_string(default.join(REMEMBER)).as_deref().map(str::trim) {
        Ok("ko") => Lang::Ko,
        _ => Lang::En,
    }
}

/// Remembers `l` as the UI language.
pub fn remember(default: &Path, l: Lang) -> std::io::Result<()> {
    std::fs::create_dir_all(default)?;
    std::fs::write(default.join(REMEMBER), if l == Lang::Ko { "ko\n" } else { "en\n" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advice::{AdviceEngine, Inputs, as_advice, centre, fits};
    use crate::mixers::{MixerFactory, MixerModel};
    use crate::saved::{Saved, to_line};
    use crate::target::{Voice, ltass_db};

    #[test] fn korean_display() {
        assert_eq!((tr("Peak", "피크"), crate::tr!("Peak {:.0} dBFS", "피크 {:.0} dBFS", -12.4)), ("Peak", "Peak -12 dBFS".into()));
        set(Lang::Ko);
        assert_eq!((tr("Peak", "피크"), crate::tr!("Peak {:.0} dBFS", "피크 {:.0} dBFS", -12.4)), ("피크", "피크 -12 dBFS".into()));
        assert_eq!(crate::tr!("{0:.0}° {1}", "{1} {0:.0}°", 30.0, "시계방향"), "시계방향 30°");
        assert_eq!(local("Low cut 100Hz"), "저역컷 100Hz");
        let g = &MixerFactory::create(MixerModel::BehringerX2442Usb).groups[1];
        assert_eq!((g.label, local(g.label)), ("CH9-12 (stereo mic)", "CH9-12 (스테레오 마이크)".into()));
        assert_eq!(local("Mackie ProFX22 (1st gen, 2011)"), "Mackie ProFX22 (1세대, 2011)");
        // Engine notes come out in Korean; knob labels stay English.
        let t = ltass_db(Voice::Male);
        let a = AdviceEngine::new().advise(g, &Inputs { summary: None, target: &t, clipped_recently: false, feedback_hz: None,
                                                         silent: false, restore_dynamic_range_db: None, current: None });
        assert!(a.notes[0].starts_with("말소리가 부족합니다"), "{:?}", a.notes);
        assert!(a.knobs.iter().any(|k| k.label == "LOW 80Hz"));
        set(Lang::En);
        assert_eq!(local("Low cut 100Hz"), "Low cut 100Hz");
    }

    #[test] fn korean_mode_saves_english() {
        set(Lang::Ko);
        let p = MixerFactory::create(MixerModel::MackieProFx22);
        let g = &p.groups[2];
        let mut knobs = centre(g);
        knobs[0].1 = 1.0;
        // Entry and saving work with the labels, not what is drawn, so the setting still fits its group.
        assert!(fits(g, &knobs));
        assert_eq!(as_advice(g, &knobs)[1].label, "Low cut 100Hz");
        let s = Saved { name: "홍길동".into(), mixer: format!("{} {}", p.brand, p.name), group: g.label.into(), dynamic_range_db: 9.0,
                        bands_db: [0.0; 25], sample_rate: Some(48_000), knobs: Some(knobs) };
        let line = to_line(&s);
        assert!(line.starts_with("홍길동\tMackie ProFX22 (1st gen, 2011)\tCH15-18 (stereo mic)\t"), "{line}");
        assert!(line.contains("\tLow cut 100Hz=1;") && !line.contains("저역컷"), "{line}");
    }

    #[test] fn language_file_roundtrip() {
        let d = std::env::temp_dir().join(format!("mixer-analyzer-lang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(remembered(&d), Lang::En);
        remember(&d, Lang::Ko).unwrap();
        assert_eq!(remembered(&d), Lang::Ko);
        remember(&d, Lang::En).unwrap();
        assert_eq!(remembered(&d), Lang::En);
        std::fs::write(d.join(REMEMBER), "fr\n").unwrap();
        assert_eq!(remembered(&d), Lang::En);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
