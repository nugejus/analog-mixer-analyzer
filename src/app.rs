use eframe::egui::{
    self, Align, Align2, Color32, Layout, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, vec2,
};
use egui_plot::{GridMark, Line, LineStyle, Plot, PlotPoint, PlotPoints, MarkerShape, Points, Polygon, Text, VLine};
use mixer_analyzer::{
    advice::{self, Action, Advice, AdviceEngine, AdviceSet, Inputs, fmt_hz},
    audio::{self, Input},
    feedback::FeedbackDetector,
    levels::{HistPoint, LevelMeter},
    mixers::{ChannelGroup, Control, KNOB_TRAVEL_DEG, MixerFactory, MixerModel, MixerProfile},
    saved::{self, Flat, Saved},
    spectrum::{self, DisplayCurve, Frame, HOP, Spectrum},
    speech::{SpeechStats, Summary},
    target::{BANDS_HZ, BELL_OCT, PRESETS, Voice, preset_target},
    theme::{self, mocha},
    update::{self, Latest},
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const DEMO: &str = "데모 신호";
const CHANNELS: [&str; 2] = ["1(L)", "2(R)"];
const TICKS_HZ: [f64; 10] = [20.0, 50.0, 100.0, 200.0, 500.0, 1e3, 2e3, 5e3, 1e4, 2e4];
const SMOOTHING: [(&str, f32); 3] = [("빠름 0.3초", 0.3), ("보통 1초", 1.0), ("느림 3초", 3.0)];
const RESOLUTIONS: [(&str, usize); 4] =
    [("1/12옥타브", 12), ("1/24옥타브", 24), ("1/48옥타브", 48), ("1/96옥타브", 96)];
/// 주파수 평활: (label, N of 1/N octave; 0 = off).
const FREQ_SMOOTH: [(&str, usize); 5] =
    [("없음", 0), ("1/24옥타브", 24), ("1/12옥타브", 12), ("1/6옥타브", 6), ("1/3옥타브", 3)];
const KNOB_W: f32 = 96.0;
/// Left control panel width.
const SIDE_W: f32 = 270.0;
const KNOB_R: f32 = 20.0;

/// One press of `분석`: collects audio frames until `needed` have arrived.
struct Recording {
    stats: SpeechStats,
    needed: usize,
    /// Last howl frequency seen during the recording.
    howl: Option<f32>,
    /// 가운데 분석(안내창의 첫 버튼); false면 지금 세팅 그대로.
    flat: bool,
}

impl Recording {
    fn new(seconds: u32, hop_s: f32) -> Self {
        Self {
            stats: SpeechStats::new(hop_s),
            needed: (seconds as f32 / hop_s).round() as usize,
            howl: None,
            flat: true,
        }
    }

    fn push(&mut self, f: &Frame, bands: &[f32; 25], howl: Option<f32>) {
        self.stats.push(f, bands);
        self.howl = howl.or(self.howl);
    }

    fn progress(&self) -> f32 {
        (self.stats.frames() as f32 / self.needed as f32).min(1.0)
    }

    fn done(&self) -> bool {
        self.stats.frames() >= self.needed
    }
}

/// A finished recording, kept until the next one so advice can be recomputed without re-recording.
struct Analysis {
    summary: Option<Summary>,
    /// Rate the recording was analysed at; the device may change afterwards.
    sample_rate: f32,
    /// Recording length; `clipped` and `silent` cover this window.
    #[expect(dead_code, reason = "brief interface; no spec UI shows it")]
    seconds: u32,
    clipped: bool,
    howl: Option<f32>,
    silent: bool,
    /// EQ 가운데·저역컷과 COMP 꺼짐에서 잰 분석; false면 지금 세팅 그대로.
    flat: bool,
}

/// The analysed spectrum on the display points of the chosen resolution.
fn result_db(a: &Analysis, per_oct: usize, smooth_n: usize) -> Option<Vec<f32>> {
    let s = a.summary.as_ref()?;
    Some(spectrum::display_db(&s.bin_power, a.sample_rate, per_oct, smooth_n))
}

/// What the advice aims at: a 1/3-octave target, notes about it, and the saved DR in restore mode.
struct Goal {
    db: [f32; 25],
    notes: Vec<String>,
    restore_dr: Option<f32>,
}

fn preset_goal(i: usize, voice: Voice, group: &ChannelGroup) -> Goal {
    let (db, dropped) = preset_target(&PRESETS[i], voice, group);
    let notes = dropped
        .into_iter()
        .map(|hz| format!("이 채널에는 {} 부근을 바꿀 노브가 없어 그 부분은 빼고 맞춥니다.", fmt_hz(hz)))
        .collect();
    Goal { db, notes, restore_dr: None }
}

/// kHz to one decimal without a trailing .0: 48000 → "48", 44100 → "44.1".
fn khz(hz: f32) -> String {
    ((hz / 100.0).round() / 10.0).to_string()
}

/// Restore mode: aim at the saved bands and compare DR with the saved one.
/// `sample_rate` is the current analysis rate (`None` before the first analysis).
fn saved_goal(s: &Saved, mixer: &str, group: &str, sample_rate: Option<f32>) -> Goal {
    let mut notes = vec![];
    // 0.3.1의 ProFX22v3 `CH15-18 (스테레오 마이크)`는 지금 `CH15-20`입니다.
    let same_group = s.group == group || (s.group.as_str(), group) == ("CH15-18 (스테레오 마이크)", "CH15-20 (스테레오 마이크)");
    if s.mixer != mixer || !same_group {
        notes.push(format!(
            "저장할 때와 믹서/채널이 다릅니다(저장: {} {}). 같은 채널에서 분석해야 맞습니다.",
            s.mixer, s.group
        ));
    }
    match (s.sample_rate, sample_rate) {
        (None, _) => notes.push("이 소리는 이전 버전에서 저장해 저음 쪽 값이 조금 다릅니다. 같은 마이크·채널로 다시 저장해 주세요.".into()),
        (Some(was), Some(now)) if was != now.round() as u32 => notes.push(format!(
            "저장할 때와 샘플레이트가 다릅니다(저장 {}kHz, 지금 {}kHz). 같은 입력장치 설정에서 분석해야 정확합니다.",
            khz(was as f32),
            khz(now)
        )),
        _ => {}
    }
    Goal { db: s.bands_db, notes, restore_dr: Some(s.dynamic_range_db) }
}

const FLAT_NOTE: &str = "추천 각도는 가운데에서 돌릴 양입니다. 다시 분석할 때도 먼저 가운데로 돌려 주세요. 그다음 미세 조정은 귀로 하세요. 예배 중에는 소리가 바뀌니 사운드 체크 때 하세요.";

/// 분석 전 안내창의 줄: 그룹에 있는 노브만(팩토리 순서라 MID가 여럿이어도 dedup으로 한 번).
fn flat_lines(group: &ChannelGroup) -> Vec<String> {
    let (mut eq, mut off) = (vec![], vec![]);
    for c in &group.controls {
        match c {
            Control::HighShelf { .. } => eq.push("HIGH"),
            Control::Mid { .. } => eq.push("MID"),
            Control::LowShelf { .. } => eq.push("LOW"),
            Control::LowCut { .. } => off.push("저역컷 끄기"),
            Control::Comp => off.push("COMP는 왼쪽 끝(꺼짐)"),
            Control::Gain { .. } => {}
        }
    }
    eq.dedup();
    let mut v = vec![format!("{} 노브는 가운데(12시)로", eq.join("·"))];
    v.extend((!off.is_empty()).then(|| off.join(", ")));
    v.push("GAIN은 그대로".into());
    v
}

const AS_IS_NOTE: &str = "세팅을 저장하거나 지금 노브를 확인할 때는 '지금 세팅 그대로 분석'을 누르세요.";

/// 분석 전 안내창: Some(Some(true)) = 가운데 분석(Enter), Some(Some(false)) = 그대로 분석,
/// Some(None) = 취소(Esc, 바깥 클릭), None = 열려 있음.
fn flat_dialog(ctx: &egui::Context, group: &ChannelGroup) -> Option<Option<bool>> {
    let m = egui::Modal::new(egui::Id::unique("flat_start")).show(ctx, |ui| {
        ui.set_max_width(360.0);
        ui.heading("분석 전에 맞춰 주세요");
        for l in flat_lines(group) {
            ui.label(l);
        }
        for n in [FLAT_NOTE, AS_IS_NOTE] {
            ui.add(egui::Label::new(egui::RichText::new(n).color(mocha::SUBTEXT0)).wrap());
        }
        ui.horizontal_wrapped(|ui| {
            let flat = ui.button(egui::RichText::new("가운데로 맞췄어요, 분석").strong()).clicked();
            (flat, ui.button("지금 세팅 그대로 분석").clicked(), ui.button("취소").clicked())
        })
        .inner
    });
    let (flat, as_is, cancel) = m.inner;
    let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    if cancel || m.should_close() {
        Some(None)
    } else if flat || enter {
        Some(Some(true))
    } else {
        as_is.then_some(Some(false))
    }
}

/// The `목표` choice: a preset or a saved sound.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pick {
    Preset(usize),
    Saved(usize),
}

/// The same pick in a new list: a saved sound is found again by name, else the first preset.
fn repick(pick: Pick, old: &[Saved], new: &[Saved]) -> Pick {
    match pick {
        Pick::Saved(i) => new.iter().position(|s| s.name == old[i].name).map_or(Pick::Preset(0), Pick::Saved),
        p => p,
    }
}

/// `읽기`: a gone folder (USB pulled) is an error, not an empty list, so its file is not replaced once it is back.
fn read_list(dir: &Path) -> std::io::Result<(Vec<Saved>, Option<usize>)> {
    if !dir.is_dir() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "저장 폴더를 찾을 수 없습니다"));
    }
    saved::load(&dir.join(saved::FILE))
}

/// A `saved.tsv` line that did not parse: the rest is used but the file is not written until 읽기/쓰기.
fn bad_line(n: usize) -> String {
    format!("saved.tsv의 {n}번째 줄을 읽지 못했습니다. 파일을 고치거나 [읽기]/[쓰기]로 다른 파일을 고르기 전에는 저장·삭제하지 않습니다.")
}

/// Writes `list` to `dir/saved.tsv`, unless there is no folder or its file could not be read.
fn write_list(dir: Option<&Path>, blocked: bool, list: &[Saved]) -> Result<(), String> {
    let dir = dir.ok_or("저장 폴더를 찾을 수 없습니다")?;
    if blocked {
        return Err("읽지 못한 파일은 덮어쓰지 않습니다. [읽기]나 [쓰기]를 누르세요.".to_owned());
    }
    saved::store(&dir.join(saved::FILE), list).map_err(|e| e.to_string())
}

fn mixer_name(p: &MixerProfile) -> String {
    format!("{} {}", p.brand, p.name)
}

/// Below this the 16 kHz band is not measured: no knob turns and no saving.
const MIN_RATE: f32 = 40_000.0;

/// A fresh engine each time, so every knob starts from ✓ (no deadband state carried over).
/// Below 40 kHz the 16 kHz band is not measured: knob names and one note only (the graph stays).
fn advise(a: &Analysis, group: &ChannelGroup, goal: &Goal) -> AdviceSet {
    if a.sample_rate < MIN_RATE {
        let note = format!(
            "입력 샘플레이트가 {}kHz라 16kHz 대역까지 잴 수 없습니다. 믹서 USB 입력(44.1kHz 이상)을 고르세요.",
            khz(a.sample_rate)
        );
        return AdviceSet { knobs: advice::labels(group), notes: vec![note], level_offset_db: 0.0 };
    }
    let mut s = AdviceEngine::new().advise(
        group,
        &Inputs {
            summary: a.summary.as_ref(),
            target: &goal.db,
            clipped_recently: a.clipped,
            feedback_hz: a.howl,
            silent: a.silent,
            restore_dynamic_range_db: goal.restore_dr,
        },
    );
    s.notes.extend(goal.notes.iter().cloned());
    s
}

/// 마지막 가운데 분석과 그때의 채널(앱을 끄면 사라짐).
struct LastFlat {
    mixer: String,
    group: String,
    sample_rate: f32,
    summary: Summary,
}

impl LastFlat {
    fn fits(&self, mixer: &str, group: &str, sample_rate: f32) -> bool {
        (self.mixer.as_str(), self.group.as_str(), self.sample_rate) == (mixer, group, sample_rate)
    }
}

const DIFFERS: &str = "오늘 소리 기준 추천이 저장된 노브 위치와 다릅니다(마이크·목소리·자리가 바뀌었을 수 있음). 오늘 소리 기준인 지금 추천을 따르세요.";
const NEED_FLAT: &str = "가운데에서 분석하면 노브 추천이 나옵니다. 그다음 세팅을 바꾸고 '지금 세팅 그대로 분석'을 누르면 지금 노브 위치를 추정합니다.";
const NO_FLAT_SAVED: &str = "가운데 분석이 없어 노브 위치는 저장하지 않았습니다(소리만 저장).";

/// 노브 줄에 보일 것(spec 규칙 1–5).
enum KnobView {
    /// 1: 가운데 분석의 추천(저장된 노브 위치와 다르면 노트 하나 더).
    Advice(AdviceSet),
    /// 2: 고른 저장 세팅의 노브 위치.
    Stored(Vec<Advice>),
    /// 3: 그대로 분석과 last_flat으로 추정한 지금 노브 위치.
    Estimate(Vec<Advice>),
    /// 4: 노브 이름과 노트 하나.
    Names(String),
    /// 5: 분석 없음.
    None,
}

/// 규칙 1–5. 2와 3의 GAIN은 ✓이고 엔진 노트는 버립니다(`knob_positions`).
fn knob_view(
    analysis: Option<&Analysis>,
    saved: Option<&Saved>,
    last_flat: Option<&LastFlat>,
    mixer: &str,
    group: &ChannelGroup,
    goal: &Goal,
) -> KnobView {
    let stored = saved.filter(|s| s.mixer == mixer && s.group == group.label).and_then(|s| {
        let f = s.flat.as_ref()?;
        let flat = advice::neutral_summary(f.bands_db, f.dynamic_range_db);
        Some(advice::knob_positions(group, &flat, &s.bands_db, s.dynamic_range_db))
    });
    let Some(a) = analysis else {
        return stored.map_or(KnobView::None, KnobView::Stored);
    };
    if a.flat {
        let mut s = advise(a, group, goal);
        if stored.is_some_and(|k| advice::knobs_differ(&s.knobs, &k)) {
            s.notes.push(DIFFERS.into());
        }
        return KnobView::Advice(s);
    }
    if let Some(k) = stored {
        return KnobView::Stored(k);
    }
    match (last_flat.filter(|l| l.fits(mixer, group.label, a.sample_rate)), &a.summary) {
        // 말소리 부족·낮은 샘플레이트는 엔진처럼 노브 이름과 그 노트로 알립니다.
        _ if a.summary.is_none() || a.sample_rate < MIN_RATE => KnobView::Advice(advise(a, group, goal)),
        (Some(l), Some(s)) => KnobView::Estimate(advice::knob_positions(group, &l.summary, &s.bands_db, s.dynamic_range_db)),
        _ => KnobView::Names(NEED_FLAT.into()),
    }
}

/// 노브 줄 머리글.
fn header(v: &KnobView) -> Option<&'static str> {
    match v {
        KnobView::Advice(_) => Some("추천: 가운데(12시)에서 돌릴 양"),
        KnobView::Stored(_) => Some("저장된 세팅: 가운데(12시)에서 돌릴 양"),
        KnobView::Estimate(_) => Some("지금 노브 추정: 가운데(12시)에서 돌린 양"),
        KnobView::Names(_) | KnobView::None => None,
    }
}

/// 이 소리와 함께 저장할 가운데 소리: 가운데 분석이면 자신, 그대로 분석이면 같은 채널·샘플레이트의 last_flat.
fn flat_for_save(a: &Analysis, last_flat: Option<&LastFlat>, mixer: &str, group: &str) -> Option<Flat> {
    let s = if a.flat {
        a.summary.as_ref()?
    } else {
        &last_flat.filter(|l| l.fits(mixer, group, a.sample_rate))?.summary
    };
    Some(Flat { dynamic_range_db: s.dynamic_range_db, bands_db: s.bands_db })
}

/// An open input and the analysis state built for its sample rate and channel count.
struct Live {
    input: Input,
    levels: LevelMeter,
    spectrum: Spectrum,
    feedback: FeedbackDetector,
    curve: DisplayCurve,
    howl: Option<f32>,
    rec: Option<Recording>,
    /// cpal pauses the stream on unplug or sample-rate change without telling us, so track arrival.
    last_data: Instant,
}

impl Live {
    fn new(input: Input, per_oct: usize) -> Self {
        let sr = input.sample_rate;
        Self {
            levels: LevelMeter::new(sr, input.channels),
            spectrum: Spectrum::new(sr),
            feedback: FeedbackDetector::new(sr),
            curve: DisplayCurve::new(spectrum::display_freqs(per_oct).len()),
            howl: None,
            rec: None,
            last_data: Instant::now(),
            input,
        }
    }

    /// Drains whole frames from the ring and runs every analyser on them.
    fn pump(&mut self, channel: usize, tau_s: f32, per_oct: usize, smooth_n: usize) {
        let (sr, ch) = (self.input.sample_rate, self.input.channels.max(1));
        let n = self.input.rx.slots() / ch * ch;
        let data: Vec<f32> = self
            .input
            .rx
            .read_chunk(n)
            .map(|c| c.into_iter().collect())
            .unwrap_or_default();
        if !data.is_empty() {
            self.last_data = Instant::now();
        }
        self.levels.push_interleaved(&data);
        for f in self.spectrum.push(&audio::pick_channel(&data, ch, channel)) {
            let display = spectrum::display_db(&f.bin_power, sr, per_oct, smooth_n);
            self.howl = self.feedback.push(&f.bin_power);
            self.curve.update(&display, HOP as f32 / sr, tau_s);
            if let Some(r) = self.rec.as_mut().filter(|r| !r.done()) {
                let bands = spectrum::third_octave_db(&f.bin_power, sr);
                r.push(&f, &bands, self.howl);
            }
        }
    }

    fn silent_for(&self, seconds: f32) -> bool {
        self.levels.silent_for(seconds) || self.last_data.elapsed() > Duration::from_secs(3)
    }

    fn silent(&self) -> bool {
        self.silent_for(3.0)
    }
}

fn open(device: &str, per_oct: usize) -> Result<Live, String> {
    if device == DEMO {
        Ok(audio::open_demo())
    } else {
        audio::open(Some(device))
    }
    .map(|l| Live::new(l, per_oct))
    .map_err(|e| format!("입력 장치를 열 수 없습니다: {e}"))
}

enum Update {
    /// Background check still running.
    Checking(mpsc::Receiver<Option<Latest>>),
    /// Nothing to show: not newer, offline, or check failed.
    Hidden,
    Found(Latest),
    Downloading(Latest, mpsc::Receiver<std::io::Result<PathBuf>>),
    Failed(Latest, String),
}

pub struct App {
    devices: Vec<String>,
    device: String,
    channel: usize,
    live: Result<Live, String>,
    model: MixerModel,
    profile: MixerProfile,
    group: usize,
    voice: Voice,
    pick: Pick,
    /// Rebuilt by `readvise` from pick, voice and channel group.
    goal: Goal,
    /// Holds `folder.txt`; `None` when HOME/APPDATA is missing.
    default_dir: Option<PathBuf>,
    /// The save folder holding `saved.tsv`; `None` until one is known.
    dir: Option<PathBuf>,
    /// Its `saved.tsv` could not be read, so only `쓰기` may overwrite it.
    write_blocked: bool,
    saved: Vec<Saved>,
    /// Name being typed after `이 소리 저장`.
    naming: Option<String>,
    /// `삭제` was pressed once and now reads `정말 삭제`.
    confirm_delete: bool,
    save_error: Option<String>,
    rec_seconds: u32,
    /// `분석` was pressed: the flat-start dialog is open.
    flat_start: bool,
    /// Index into `SMOOTHING`.
    smooth: usize,
    /// Index into `RESOLUTIONS`.
    res: usize,
    /// Index into `FREQ_SMOOTH`.
    freq_smooth: usize,
    analysis: Option<Analysis>,
    last_flat: Option<LastFlat>,
    /// Rebuilt by `readvise`.
    view: KnobView,
    show: Show,
    update: Update,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, demo: bool) -> Self {
        theme::install(&cc.egui_ctx);
        let device = if demo {
            DEMO.to_owned()
        } else {
            audio::default_device_name().unwrap_or_default()
        };
        let model = MixerModel::MackieProFx22v3;
        let profile = MixerFactory::create(model);
        let default_dir = saved::default_dir();
        let mut save_error = None;
        // A remembered folder that is gone (USB pulled) keeps folder.txt, so it is used again once back.
        let dir = match default_dir.as_deref().and_then(saved::remembered_dir) {
            Some(d) if d.is_dir() => Some(d),
            Some(d) => {
                save_error = Some(format!("저장 폴더를 찾을 수 없어 기본 폴더를 씁니다: {}", d.display()));
                default_dir.clone()
            }
            None => default_dir.clone(),
        };
        // An existing file we cannot read must not be replaced by a shorter list: blocked until 읽기/쓰기.
        let (saved, write_blocked) = match dir.as_deref().map(|d| saved::load(&d.join(saved::FILE))).transpose() {
            Ok(l) => {
                let (l, bad) = l.unwrap_or_default();
                if let Some(n) = bad {
                    save_error = Some(bad_line(n));
                }
                (l, bad.is_some())
            }
            Err(e) => {
                save_error = Some(format!("읽지 못했습니다: {e}"));
                (vec![], true)
            }
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(update::fetch_latest().filter(|l| update::is_newer(&l.version, update::VERSION)));
        });
        Self {
            update: Update::Checking(rx),
            saved,
            default_dir,
            dir,
            write_blocked,
            naming: None,
            confirm_delete: false,
            save_error,
            devices: audio::device_names(),
            live: open(&device, RESOLUTIONS[2].1),
            device,
            channel: 0,
            model,
            goal: preset_goal(0, Voice::Male, &profile.groups[0]),
            profile,
            group: 0,
            voice: Voice::Male,
            pick: Pick::Preset(0),
            rec_seconds: 20,
            flat_start: false,
            smooth: 1,
            res: 2,
            freq_smooth: 3,
            analysis: None,
            last_flat: None,
            view: KnobView::None,
            show: Show::default(),
        }
    }

    fn readvise(&mut self) {
        let group = &self.profile.groups[self.group];
        self.goal = match self.pick {
            Pick::Preset(i) => preset_goal(i, self.voice, group),
            Pick::Saved(i) => {
                saved_goal(&self.saved[i], &mixer_name(&self.profile), group.label, self.analysis.as_ref().map(|a| a.sample_rate))
            }
        };
        let saved = match self.pick {
            Pick::Saved(i) => Some(&self.saved[i]),
            Pick::Preset(_) => None,
        };
        let mixer = mixer_name(&self.profile);
        self.view = knob_view(self.analysis.as_ref(), saved, self.last_flat.as_ref(), &mixer, group, &self.goal);
    }

    /// Writes the saved list; a failure shows in red under 안내 and the list stays in memory.
    fn persist(&mut self) {
        // 막힌 이유(줄 번호, 읽기 실패)가 떠 있으면 그대로 둡니다.
        let reason = |e: &str| e.starts_with("saved.tsv의") || e.starts_with("읽지 못했습니다");
        if self.write_blocked && self.save_error.as_deref().is_some_and(reason) {
            return;
        }
        let r = write_list(self.dir.as_deref(), self.write_blocked, &self.saved);
        self.save_error = r.err().map(|e| format!("저장하지 못했습니다: {e}"));
    }

    /// A new list (읽기, another folder) keeps the picked saved sound by name.
    /// A read of the save folder: on failure the list stays and that folder is not written until 읽기/쓰기.
    /// A line it could not parse also blocks writing; the parsed sounds are used.
    fn take_list(&mut self, r: std::io::Result<(Vec<Saved>, Option<usize>)>) {
        self.write_blocked = !matches!(r, Ok((_, None)));
        self.save_error = match r {
            Ok((l, bad)) => {
                self.pick = repick(self.pick, &self.saved, &l);
                self.saved = l;
                self.readvise();
                bad.map(bad_line)
            }
            Err(e) => Some(format!("읽지 못했습니다: {e}")),
        };
    }

    /// `저장 폴더`: folder name (full path on hover), 찾아보기…/폴더 열기, 읽기/쓰기.
    fn folder_controls(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        ui.label("저장 폴더");
        match &self.dir {
            Some(d) => {
                let name = d.file_name().map_or(d.display().to_string(), |n| format!("…/{}", n.to_string_lossy()));
                ui.label(name).on_hover_text(d.display().to_string());
            }
            None => {
                ui.label("저장 폴더를 찾을 수 없습니다");
            }
        }
        let on = self.dir.is_some();
        let button = |ui: &mut egui::Ui, enabled, text| ui.add_enabled(enabled, egui::Button::new(text)).clicked();
        let (browse, open) = ui.horizontal(|ui| (button(ui, true, "찾아보기…"), button(ui, on, "폴더 열기"))).inner;
        let (read, write) = ui.horizontal(|ui| (button(ui, on, "읽기"), button(ui, on, "쓰기"))).inner;
        if browse {
            // Parented so the dialog stays in front of the (blocked) main window on Windows.
            let dialog = rfd::FileDialog::new().set_parent(frame);
            let dialog = match &self.dir {
                Some(d) => dialog.set_directory(d),
                None => dialog,
            };
            if let Some(d) = dialog.pick_folder() {
                let remembered = self.default_dir.as_deref().map_or(Ok(()), |def| saved::remember_dir(def, &d));
                let had_file = d.join(saved::FILE).exists();
                let list = saved::adopt(&d, &self.saved);
                self.dir = Some(d);
                match list {
                    // No file there, so the failure was writing the moved list: nothing to protect.
                    Err(e) if !had_file => {
                        self.write_blocked = false;
                        self.save_error = Some(format!("저장하지 못했습니다: {e}"));
                    }
                    list => self.take_list(list),
                }
                // A read error matters more: it is the one that blocks saving.
                if let (Err(e), false) = (remembered, self.write_blocked) {
                    self.save_error = Some(format!("저장하지 못했습니다: {e}"));
                }
            }
        }
        if let (true, Some(d)) = (open, &self.dir)
            && let Err(e) = saved::open_dir(d)
        {
            self.save_error = Some(format!("폴더를 열지 못했습니다: {e}"));
        }
        if let (true, Some(d)) = (read, &self.dir) {
            let list = read_list(d);
            self.take_list(list);
        }
        if write {
            let r = write_list(self.dir.as_deref(), false, &self.saved);
            self.write_blocked &= r.is_err();
            self.save_error = r.err().map(|e| format!("저장하지 못했습니다: {e}"));
        }
    }

    /// `삭제`/`정말 삭제` for a picked saved sound, then `이 소리 저장` or the name field.
    fn save_controls(&mut self, ui: &mut egui::Ui) {
        if let Pick::Saved(i) = self.pick {
            let label = if self.confirm_delete { "정말 삭제" } else { "삭제" };
            if ui.button(label).clicked() {
                if self.confirm_delete {
                    self.saved.remove(i);
                    self.persist();
                    (self.pick, self.confirm_delete) = (Pick::Preset(0), false);
                    self.readvise();
                } else {
                    self.confirm_delete = true;
                }
            }
        }
        let analysis = self.analysis.as_ref().filter(|a| a.sample_rate >= MIN_RATE);
        let summary = analysis.and_then(|a| a.summary.as_ref());
        let Some(name) = &mut self.naming else {
            if ui.add_enabled(summary.is_some(), egui::Button::new("이 소리 저장")).clicked() {
                self.naming = Some(String::new());
            }
            return;
        };
        ui.add(egui::TextEdit::singleline(name).hint_text("홍길동 목사님 설교 마이크").desired_width(180.0));
        let name = saved::clean(name);
        let save = ui.add_enabled(!name.is_empty() && summary.is_some(), egui::Button::new("저장")).clicked();
        if ui.button("취소").clicked() {
            self.naming = None;
        }
        if let (true, Some(a), Some(s)) = (save, analysis, summary) {
            let (mixer, group) = (mixer_name(&self.profile), self.profile.groups[self.group].label);
            let flat = flat_for_save(a, self.last_flat.as_ref(), &mixer, group);
            let no_flat = flat.is_none();
            let s = Saved {
                name,
                mixer,
                group: group.to_owned(),
                dynamic_range_db: s.dynamic_range_db,
                bands_db: s.bands_db,
                sample_rate: self.analysis.as_ref().map(|a| a.sample_rate.round() as u32),
                flat,
            };
            saved::upsert(&mut self.saved, s);
            self.persist();
            if no_flat && self.save_error.is_none() {
                self.save_error = Some(NO_FLAT_SAVED.into());
            }
            self.naming = None;
            self.readvise();
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        // Combos fill the panel; long mixer names are cut with "…" instead of widening it.
        ui.spacing_mut().combo_width = ui.available_width();
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        self.update_notice(ui);
        ui.label("믹서");
        let mut model = self.model;
        egui::ComboBox::from_id_salt("mixer")
            .selected_text(mixer_name(&self.profile))
            .show_ui(ui, |ui| {
                for m in MixerModel::ALL {
                    ui.selectable_value(&mut model, m, mixer_name(&MixerFactory::create(m)));
                }
            });
        if model != self.model {
            (self.model, self.profile, self.group) = (model, MixerFactory::create(model), 0);
            self.readvise();
        }
        ui.label("채널");
        let mut group = self.group;
        egui::ComboBox::from_id_salt("group")
            .selected_text(self.profile.groups[self.group].label)
            .show_ui(ui, |ui| {
                for (i, g) in self.profile.groups.iter().enumerate() {
                    ui.selectable_value(&mut group, i, g.label);
                }
            });
        if group != self.group {
            self.group = group;
            self.readvise();
        }
        ui.separator();
        ui.label("입력장치");
        let mut pick = self.device.clone();
        let combo = egui::ComboBox::from_id_salt("device")
            .selected_text(&self.device)
            .show_ui(ui, |ui| {
                for name in self.devices.iter().map(String::as_str).chain([DEMO]) {
                    ui.selectable_value(&mut pick, name.to_owned(), name);
                }
            });
        if combo.response.clicked() {
            self.devices = audio::device_names(); // pick up devices plugged in after launch
        }
        if pick != self.device {
            self.live = open(&pick, RESOLUTIONS[self.res].1);
            self.device = pick;
        }
        ui.label("분석채널");
        let channel = self.channel;
        egui::ComboBox::from_id_salt("channel")
            .selected_text(CHANNELS[self.channel])
            .show_ui(ui, |ui| {
                for (i, label) in CHANNELS.into_iter().enumerate() {
                    ui.selectable_value(&mut self.channel, i, label);
                }
            });
        if self.channel != channel
            && let Ok(live) = &mut self.live
        {
            live.rec = None;
        }
        ui.separator();
        ui.label("목표");
        let (voice, pick) = (self.voice, self.pick);
        let title = |pick| match pick {
            Pick::Preset(i) => format!("{} · {}", PRESETS[i].category, PRESETS[i].name),
            Pick::Saved(i) => format!("저장한 세팅 · {}", self.saved[i].name),
        };
        egui::ComboBox::from_id_salt("goal")
            .selected_text(title(pick))
            .show_ui(ui, |ui| {
                for (i, p) in PRESETS.iter().enumerate() {
                    if i == 0 || PRESETS[i - 1].category != p.category {
                        ui.colored_label(mocha::SUBTEXT0, p.category);
                    }
                    ui.selectable_value(&mut self.pick, Pick::Preset(i), p.name);
                }
                if !self.saved.is_empty() {
                    ui.colored_label(mocha::SUBTEXT0, "저장한 세팅");
                }
                for (i, s) in self.saved.iter().enumerate() {
                    ui.selectable_value(&mut self.pick, Pick::Saved(i), &s.name);
                }
            });
        let when = match self.pick {
            Pick::Preset(i) => PRESETS[i].when.to_owned(),
            Pick::Saved(i) => {
                let s = &self.saved[i];
                format!("저장: {} {} — 저장할 때와 같은 사람·같은 마이크로 분석하세요.", s.mixer, s.group)
            }
        };
        ui.add(egui::Label::new(egui::RichText::new(when).color(mocha::SUBTEXT0)).wrap_mode(egui::TextWrapMode::Wrap));
        ui.add_enabled_ui(matches!(self.pick, Pick::Preset(_)), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.voice, Voice::Male, "남성");
                ui.selectable_value(&mut self.voice, Voice::Female, "여성");
            });
        });
        if self.pick != pick {
            self.confirm_delete = false;
        }
        if (self.voice, self.pick) != (voice, pick) {
            self.readvise();
        }
        ui.horizontal_wrapped(|ui| self.save_controls(ui));
        ui.separator();
        // 버튼보다 먼저: [분석]을 누른 Enter가 같은 프레임에 안내창까지 확인하지 않게 합니다.
        if self.flat_start {
            let r = flat_dialog(ui.ctx(), &self.profile.groups[self.group]);
            self.flat_start = r.is_none();
            // 그사이 입력장치가 안 열리면(Err) 녹음하지 않고 닫기만 합니다.
            if let Some(Some(flat)) = r
                && let Ok(live) = &mut self.live
            {
                live.curve.reset_peak();
                live.rec = Some(Recording { flat, ..Recording::new(self.rec_seconds, HOP as f32 / live.input.sample_rate) });
            }
        }
        let recording = self.live.as_ref().is_ok_and(|l| l.rec.is_some());
        ui.horizontal(|ui| {
            ui.label("녹음");
            ui.add_enabled(
                !recording,
                egui::Slider::new(&mut self.rec_seconds, 15..=30).suffix("초"),
            );
        });
        let label = if recording { "취소" } else { "분석" };
        let button = egui::Button::new(egui::RichText::new(label).strong()).min_size(vec2(0.0, 32.0));
        // Justified layout stretches the button across the panel and centres its text.
        let enabled = self.live.is_ok();
        if ui
            .vertical_centered_justified(|ui| ui.add_enabled(enabled, button))
            .inner
            .clicked()
            && let Ok(live) = &mut self.live
        {
            if recording {
                live.rec = None;
            } else {
                self.flat_start = true;
            }
        }
        ui.separator();
        ui.label("실시간 선");
        egui::ComboBox::from_id_salt("smooth")
            .selected_text(SMOOTHING[self.smooth].0)
            .show_ui(ui, |ui| {
                for (i, (label, _)) in SMOOTHING.into_iter().enumerate() {
                    ui.selectable_value(&mut self.smooth, i, label);
                }
            });
        ui.label("해상도");
        let before = self.res;
        egui::ComboBox::from_id_salt("res")
            .selected_text(RESOLUTIONS[self.res].0)
            .show_ui(ui, |ui| {
                for (i, (label, _)) in RESOLUTIONS.into_iter().enumerate() {
                    ui.selectable_value(&mut self.res, i, label);
                }
            });
        ui.label("주파수 평활");
        let smooth_before = self.freq_smooth;
        egui::ComboBox::from_id_salt("freq_smooth")
            .selected_text(FREQ_SMOOTH[self.freq_smooth].0)
            .show_ui(ui, |ui| {
                for (i, (label, _)) in FREQ_SMOOTH.into_iter().enumerate() {
                    ui.selectable_value(&mut self.freq_smooth, i, label);
                }
            });
        if (self.res != before || self.freq_smooth != smooth_before)
            && let Ok(live) = &mut self.live
        {
            live.curve = DisplayCurve::new(spectrum::display_freqs(RESOLUTIONS[self.res].1).len());
        }
        ui.horizontal(|ui| {
            if ui.button("피크 리셋").clicked()
                && let Ok(live) = &mut self.live
            {
                live.curve.reset_peak();
            }
            if ui.button("클리핑 리셋").clicked()
                && let Ok(live) = &mut self.live
            {
                live.levels.reset_clips();
            }
        });
        ui.separator();
        self.folder_controls(ui, frame);
        ui.separator();
        ui.colored_label(mocha::SUBTEXT0, format!("버전 {}", update::VERSION));
    }

    /// Polls the background check/download and draws the update notice, if any.
    fn update_notice(&mut self, ui: &mut egui::Ui) {
        self.update = match std::mem::replace(&mut self.update, Update::Hidden) {
            Update::Checking(rx) => match rx.try_recv() {
                Ok(Some(l)) => Update::Found(l),
                Err(mpsc::TryRecvError::Empty) => Update::Checking(rx),
                _ => Update::Hidden,
            },
            Update::Downloading(l, rx) => match rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => Update::Downloading(l, rx),
                Ok(Ok(path)) => match std::process::Command::new(&path).spawn() {
                    Ok(_) => {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        Update::Found(l)
                    }
                    Err(e) => Update::Failed(l, format!("설치 파일을 실행하지 못했습니다: {e}")),
                },
                Ok(Err(e)) => Update::Failed(l, format!("업데이트를 받지 못했습니다: {e}")),
                Err(e) => Update::Failed(l, format!("업데이트를 받지 못했습니다: {e}")),
            },
            u => u,
        };
        let mut next = None;
        match &self.update {
            Update::Checking(_) | Update::Hidden => return,
            Update::Downloading(..) => {
                ui.label("업데이트 받는 중…");
            }
            Update::Found(l) | Update::Failed(l, _) => {
                ui.horizontal(|ui| {
                    ui.label(format!("새 버전이 있습니다: {}", l.version));
                    if cfg!(windows) {
                        if ui.button("업데이트").clicked() {
                            let (tx, rx) = mpsc::channel();
                            let url = l.url.clone();
                            std::thread::spawn(move || {
                                let path = std::env::temp_dir().join("MixerAnalyzer-Setup.exe");
                                let _ = tx.send(update::download(&url, &path).map(|()| path));
                            });
                            next = Some(Update::Downloading(l.clone(), rx));
                        }
                    } else if ui.button("받으러 가기").clicked() {
                        next = Some(match update::open_url(update::FOLDER_URL) {
                            Ok(()) => Update::Found(l.clone()),
                            Err(e) => Update::Failed(l.clone(), format!("드라이브를 열지 못했습니다: {e}")),
                        });
                    }
                });
                if let Update::Failed(_, e) = &self.update {
                    ui.add(egui::Label::new(egui::RichText::new(e).color(mocha::RED)).wrap());
                }
            }
        }
        ui.separator();
        if let Some(n) = next {
            self.update = n;
        }
    }
}

/// A plot with fixed bounds that volunteers cannot drag, zoom or reset by accident.
fn fixed_plot(id: &str, height: f32, x: (f64, f64), y: (f64, f64)) -> Plot<'_> {
    Plot::new(id)
        .height(height)
        .default_x_bounds(x.0, x.1)
        .default_y_bounds(y.0, y.1)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .grid_color(mocha::SURFACE1)
}

struct Show {
    result: bool,
    live: bool,
    peak: bool,
    target: bool,
    knobs: bool,
    tone: bool,
    bumps: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self { result: true, live: true, peak: true, target: true, knobs: true, tone: true, bumps: false }
    }
}

/// An EQ knob's adjustable range in Hz; `mark` is the last recommended FREQ.
struct Span {
    label: String,
    lo: f32,
    hi: f32,
    mark: Option<f32>,
}

fn knob_spans(group: &ChannelGroup, knobs: Option<&[Advice]>) -> Vec<Span> {
    advice::labels(group)
        .into_iter()
        .filter(|k| !k.label.starts_with("FREQ"))
        .filter_map(|k| {
            let (lo, hi) = match k.control {
                Control::LowCut { hz } | Control::LowShelf { hz } => (20.0, hz),
                Control::HighShelf { hz } => (hz, 20_000.0),
                Control::Mid { lo_hz, hi_hz } if lo_hz == hi_hz => {
                    (lo_hz * 2f32.powf(-BELL_OCT), lo_hz * 2f32.powf(BELL_OCT))
                }
                Control::Mid { lo_hz, hi_hz } => (lo_hz, hi_hz),
                _ => return None,
            };
            let freq = k.label.replacen("MID", "FREQ", 1);
            let mark = knobs
                .and_then(|k| k.iter().find(|a| a.label == freq))
                .and_then(|a| match a.action {
                    Action::SetFreq { hz } => Some(hz),
                    _ => None,
                });
            Some(Span { label: k.label, lo, hi, mark })
        })
        .collect()
}

/// `result_db` is the last analysis on the display points; `target_db` is the 1/3-octave
/// target already shifted onto the display scale.
#[allow(clippy::too_many_arguments)]
fn spectrum_plot(
    ui: &mut egui::Ui,
    curve: &DisplayCurve,
    result_db: Option<&[f32]>,
    target_db: Option<[f32; 25]>,
    tone_db: Option<[f32; 8]>,
    bumps: &[(f32, f32)],
    per_oct: usize,
    show: &Show,
    spans: &[Span],
    height: f32,
) {
    let line = |name: &str, hz: &[f32], db: &[f32], color| {
        let pts: PlotPoints = hz
            .iter()
            .zip(db)
            .map(|(f, d)| [f64::from(f.log10()), f64::from(*d)])
            .collect();
        Line::new(name, pts).color(color)
    };
    let freqs = spectrum::display_freqs(per_oct);
    fixed_plot(
        "spectrum",
        height,
        (20f64.log10(), 20_000f64.log10()),
        (-90.0, 0.0),
    )
    .x_grid_spacer(|_| {
        TICKS_HZ
            .iter()
            .map(|hz| GridMark {
                value: hz.log10(),
                step_size: 0.3,
            })
            .collect()
    })
    .x_axis_formatter(|m, _| {
        let hz = 10f64.powf(m.value).round();
        if hz >= 1000.0 {
            format!("{}k", hz / 1000.0)
        } else {
            hz.to_string()
        }
    })
    .show(ui, |p| {
        let tone: Vec<_> = tone_db
            .filter(|_| show.tone)
            .map(|t| {
                let bands = mixer_analyzer::target::TONE_BANDS.iter();
                bands.zip(t).map(|(&(name, lo, hi), d)| (name, lo, hi, d)).collect()
            })
            .unwrap_or_default();
        let band_color = |d: f32| (d.abs() >= mixer_analyzer::target::TONE_HOT_DB).then_some(if d > 0.0 { mocha::RED } else { mocha::BLUE });
        // Bands behind the curves, their labels (below) in front.
        for &(_, lo, hi, d) in &tone {
            if let Some(color) = band_color(d) {
                let (x0, x1) = (f64::from(lo.log10()), f64::from(hi.log10()));
                let alpha = if d > 0.0 { 50 } else { 30 };
                let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
                let pts: PlotPoints = [[x0, -90.0], [x1, -90.0], [x1, 0.0], [x0, 0.0]].into_iter().collect();
                p.polygon(Polygon::new("", pts).fill_color(fill).stroke(egui::Stroke::NONE));
            }
        }
        if show.peak {
            p.line(line("피크", &freqs, &curve.peak_db, mocha::SURFACE2));
        }
        if let (true, Some(db)) = (show.result, result_db) {
            p.line(line("분석 결과", &freqs, db, mocha::GREEN).width(2.5));
        }
        if show.live {
            p.line(line("현재 소리", &freqs, &curve.smooth_db, mocha::BLUE));
        }
        if let (true, Some(t)) = (show.target, target_db) {
            p.line(line("목표", &BANDS_HZ, &t, mocha::SUBTEXT0).style(LineStyle::dotted_dense()));
        }
        for &(name, lo, hi, d) in &tone {
            let pos = PlotPoint::new(f64::from((lo.log10() + hi.log10()) / 2.0), -88.0);
            let col = band_color(d).unwrap_or(mocha::SUBTEXT0);
            p.text(Text::new("", pos, format!("{name}\n{d:+.1} dB")).color(col).anchor(Align2::CENTER_BOTTOM));
        }
        // Markers sit on the analysis curve, so they need it shown; drawn last so no line covers them.
        if let (true, true, Some(db)) = (show.bumps, show.result, result_db) {
            for &(hz, over) in bumps {
                let i = freqs.partition_point(|&f| f < hz).min(freqs.len() - 1);
                let pos = PlotPoint::new(f64::from(freqs[i].log10()), f64::from(db[i]));
                p.points(Points::new("", vec![[pos.x, pos.y]]).shape(MarkerShape::Down).color(mocha::PEACH).radius(5.0));
                let name = if hz >= 1000.0 { format!("{:.1}kHz", hz / 1000.0) } else { format!("{hz:.0}Hz") };
                let label = format!("{name} {over:+.0}dB");
                p.text(Text::new("", PlotPoint::new(pos.x, pos.y + 2.0), label).color(mocha::PEACH).anchor(Align2::CENTER_BOTTOM));
            }
        }
        if show.knobs {
            // Rows start at −5 dB so the first label is not clipped by the top edge.
            let step = (25.0 / spans.len().max(1) as f64).min(4.0);
            for (i, s) in spans.iter().enumerate() {
                let y = -5.0 - step * i as f64;
                let bar = |lo: f32, hi: f32| {
                    let pts: PlotPoints = [lo, hi].iter().map(|f| [f64::from(f.log10()), y]).collect();
                    Line::new("", pts)
                };
                p.line(bar(s.lo, s.hi).color(mocha::OVERLAY1).width(4.0));
                if let Some(hz) = s.mark {
                    p.vline(VLine::new("", f64::from(hz.log10())).color(mocha::YELLOW));
                    let (lo, hi) = (hz * 2f32.powf(-BELL_OCT), hz * 2f32.powf(BELL_OCT));
                    p.line(bar(lo, hi).color(mocha::YELLOW).width(6.0));
                }
                let pos = PlotPoint::new(f64::from(s.lo.log10()), y + step * 0.4);
                p.text(Text::new("", pos, s.label.as_str()).color(mocha::SUBTEXT0).anchor(Align2::LEFT_BOTTOM));
            }
        }
    });
}

fn level_color(db: f64) -> Color32 {
    if db <= -12.0 {
        mocha::GREEN
    } else if db <= -6.0 {
        mocha::YELLOW
    } else {
        mocha::RED
    }
}

/// Last 30 s of 100 ms peaks; x is seconds before now.
fn level_plot(ui: &mut egui::Ui, hist: &std::collections::VecDeque<HistPoint>, height: f32) {
    let n = hist.len();
    let pts = |clipped_only: bool| -> PlotPoints {
        hist.iter()
            .enumerate()
            .filter(|(_, p)| !clipped_only || p.clipped)
            .map(|(i, p)| [(i + 1) as f64 * 0.1 - n as f64 * 0.1, f64::from(p.peak_db)])
            .collect()
    };
    fixed_plot("levels", height, (-30.0, 0.0), (-60.0, 3.0)).show(ui, |p| {
        p.line(
            Line::new("피크", pts(false)).gradient_color(Arc::new(|pt| level_color(pt.y)), false),
        );
        p.points(
            Points::new("클리핑", pts(true))
                .color(mocha::RED)
                .radius(3.5)
                .filled(true),
        );
    });
}

fn status_line(ui: &mut egui::Ui, live: &Live, overload: bool) {
    let dot = |on: bool| if on { "●" } else { "○" };
    let clips = live.levels.clip_count();
    let hz = live
        .howl
        .map_or(String::new(), |hz| format!(" {}", fmt_hz(hz)));
    let warn = |on: bool, c: Color32| if on { c } else { mocha::TEXT };
    ui.horizontal(|ui| {
        ui.label(format!("피크 {:.0} dBFS ·", live.levels.peak_db()));
        ui.colored_label(warn(clips > 0, mocha::RED), format!("클리핑 {clips}회"));
        ui.label("·");
        ui.colored_label(
            warn(overload, mocha::MAROON),
            format!("과부하 의심 {}", dot(overload)),
        );
        ui.label("·");
        let howl = live.howl.is_some();
        ui.colored_label(warn(howl, mocha::RED), format!("하울링 {}{hz}", dot(howl)));
    });
}

/// FREQ dial angle (° from 12 o'clock): `lo` at −150°, `hi` at +150°, log-linear between.
pub fn dial_angle(hz: f32, lo: f32, hi: f32) -> f32 {
    KNOB_TRAVEL_DEG * ((hz / lo).ln() / (hi / lo).ln() - 0.5)
}

/// Text lines under a knob, with their colours.
fn caption(k: &Advice) -> Vec<(String, Color32)> {
    let one = |s: String, c| vec![(s, c)];
    match k.action {
        Action::Wait => one("—".into(), mocha::SUBTEXT0),
        Action::Ok if k.label.starts_with("FREQ") => one("✓ 그대로".into(), mocha::GREEN),
        Action::Ok => one("✓".into(), mocha::GREEN),
        Action::SwitchOn => one("켜기 권장".into(), mocha::SAPPHIRE),
        Action::SetFreq { hz } => one(format!("{} 눈금에", fmt_hz(hz)), mocha::YELLOW),
        Action::Rotate { degrees, db } => {
            let dir = if degrees > 0.0 {
                "시계방향"
            } else {
                "반시계"
            };
            match k.control {
                // COMP는 왼쪽 끝(꺼짐)에서 재고, db는 의미 없는 0입니다.
                Control::Comp => one(format!("왼쪽 끝에서 {dir} {:.0}°", degrees.abs()), mocha::YELLOW),
                _ => {
                    let mut v = one(format!("{dir} {:.0}°", degrees.abs()), mocha::YELLOW);
                    v.push((format!("(≈{db:+.0}dB)"), mocha::YELLOW));
                    // GAIN은 가운데가 아니라 지금 위치에서 잽니다.
                    if let Control::Gain { .. } = k.control {
                        v.push(("(지금 위치에서)".into(), mocha::SUBTEXT0));
                    }
                    v
                }
            }
        }
    }
}

/// Point on a circle at `deg` clockwise from 12 o'clock (screen y points down).
fn at(c: Pos2, r: f32, deg: f32) -> Pos2 {
    let a = deg.to_radians();
    c + r * vec2(a.sin(), -a.cos())
}

fn dial(ui: &mut egui::Ui, k: &Advice) {
    let (resp, p) = ui.allocate_painter(vec2(KNOB_W, 2.0 * KNOB_R + 24.0), Sense::hover());
    let c = resp.rect.center();
    let pointer =
        |deg: f32, color| p.line_segment([c, at(c, KNOB_R - 3.0, deg)], Stroke::new(2.5, color));
    if let Control::LowCut { .. } = k.control {
        let on = k.action == Action::SwitchOn;
        let r = Rect::from_center_size(c, vec2(26.0, 2.0 * KNOB_R));
        let color = if on { mocha::SAPPHIRE } else { mocha::OVERLAY1 };
        p.rect_filled(r, 4.0, mocha::SURFACE0);
        p.rect_stroke(r, 4.0, Stroke::new(2.0, color), StrokeKind::Inside);
        // 눌린 쪽(아래)을 칠합니다: 켜기 권장이면 sapphire.
        let lit = Rect::from_min_max(r.center() + vec2(-8.0, 2.0), r.max - Vec2::splat(5.0));
        p.rect_filled(lit, 2.0, if on { mocha::SAPPHIRE } else { mocha::SURFACE2 });
        return;
    }
    p.circle_filled(c, KNOB_R, mocha::SURFACE0);
    p.circle_stroke(c, KNOB_R, Stroke::new(1.5, mocha::OVERLAY1));
    // COMP는 왼쪽 끝(꺼짐)에서, 나머지는 12시에서 잽니다.
    let start = if k.control == Control::Comp { -150.0 } else { 0.0 };
    match (k.action.clone(), k.control) {
        (Action::Rotate { degrees, .. }, _) => {
            pointer(start, mocha::TEXT);
            let ra = KNOB_R + 7.0;
            let arc: Vec<Pos2> = (0..=24)
                .map(|i| at(c, ra, start + degrees * i as f32 / 24.0))
                .collect();
            p.add(Shape::line(arc, Stroke::new(2.5, mocha::YELLOW)));
            let a = (start + degrees).to_radians();
            let (e, t, n) = (
                at(c, ra, start + degrees),
                degrees.signum() * vec2(a.cos(), a.sin()),
                vec2(a.sin(), -a.cos()),
            );
            p.add(Shape::convex_polygon(
                vec![e + 7.0 * t, e - 2.0 * t + 5.0 * n, e - 2.0 * t - 5.0 * n],
                mocha::YELLOW,
                Stroke::NONE,
            ));
        }
        (Action::SetFreq { hz }, Control::Mid { lo_hz, hi_hz }) => {
            for end in [-150.0, 150.0] {
                p.line_segment(
                    [at(c, KNOB_R + 2.0, end), at(c, KNOB_R + 6.0, end)],
                    Stroke::new(1.5, mocha::OVERLAY1),
                );
            }
            pointer(dial_angle(hz, lo_hz, hi_hz), mocha::YELLOW);
        }
        _ if k.label.starts_with("FREQ") => {} // 지금 눈금은 모르므로 포인터 없음
        _ => {
            pointer(start, mocha::SUBTEXT0);
        }
    }
}

fn knob_row(ui: &mut egui::Ui, knobs: &[Advice]) {
    ui.horizontal_top(|ui| {
        for k in knobs {
            ui.allocate_ui_with_layout(vec2(KNOB_W, 0.0), Layout::top_down(Align::Center), |ui| {
                ui.strong(&k.label);
                dial(ui, k);
                for (s, c) in caption(k) {
                    ui.colored_label(c, s);
                }
            });
        }
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let finished = self.live.as_mut().ok().and_then(|live| {
            live.pump(
                self.channel,
                SMOOTHING[self.smooth].1,
                RESOLUTIONS[self.res].1,
                FREQ_SMOOTH[self.freq_smooth].1,
            );
            let rec = live.rec.take_if(|r| r.done())?;
            let n = self.rec_seconds as f32;
            Some(Analysis {
                summary: rec.stats.summary(),
                sample_rate: live.input.sample_rate,
                seconds: self.rec_seconds,
                clipped: live.levels.clipped_within(n),
                howl: rec.howl,
                silent: live.silent_for(n),
                flat: rec.flat,
            })
        });
        if let Some(a) = finished {
            if let (true, Some(s)) = (a.flat, &a.summary) {
                let (mixer, group) = (mixer_name(&self.profile), self.profile.groups[self.group].label.to_owned());
                self.last_flat = Some(LastFlat { mixer, group, sample_rate: a.sample_rate, summary: s.clone() });
            }
            self.analysis = Some(a);
            self.readvise();
        }
        egui::Panel::left("controls").resizable(false).exact_size(SIDE_W).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.side_panel(ui, frame))
        });
        let live = self.live.as_ref().ok();
        let names = advice::labels(&self.profile.groups[self.group]);
        let (knobs, notes): (Option<&[Advice]>, &[String]) = match &self.view {
            KnobView::Advice(a) => (Some(&a.knobs), &a.notes),
            KnobView::Stored(k) | KnobView::Estimate(k) => (Some(k), &[]),
            KnobView::Names(n) => (Some(&names), std::slice::from_ref(n)),
            KnobView::None => (None, &[]),
        };
        let summary = self.analysis.as_ref().and_then(|a| a.summary.as_ref());
        egui::Panel::bottom("advice").show(ui, |ui| {
            let rec = live.and_then(|l| l.rec.as_ref());
            if let Some(r) = rec {
                let n = self.rec_seconds;
                let text = format!(
                    "녹음 중 {:.0}/{n}초 — 계속 말씀하세요",
                    r.progress() * n as f32
                );
                ui.add(egui::ProgressBar::new(r.progress()).text(text));
            }
            let noted = notes.iter().any(|n| n == advice::NO_INPUT);
            if !noted && live.is_some_and(Live::silent) {
                ui.colored_label(mocha::RED, advice::NO_INPUT);
            }
            if let Some(k) = knobs {
                ui.horizontal(|ui| {
                    ui.strong(summary.map_or("노브 추천".into(), |s| {
                        format!("노브 추천 — 말소리 {:.1}초 분석", s.active_seconds)
                    }));
                    if let Some(h) = header(&self.view) {
                        ui.label(h);
                    }
                });
                knob_row(ui, k);
            } else if rec.is_none() {
                ui.label(format!(
                    "[분석]을 누르고 {}초 동안 평소처럼 말씀하세요.",
                    self.rec_seconds
                ));
            }
            ui.separator();
            ui.strong("안내");
            for n in notes {
                ui.label(n);
            }
            if let Some(e) = &self.save_error {
                ui.colored_label(mocha::RED, e);
            }
            ui.colored_label(mocha::SUBTEXT0, self.profile.pc_tip);
        });
        egui::CentralPanel::default().show(ui, |ui| match &self.live {
            Ok(live) => {
                let offset = summary.map_or(0.0, |s| mixer_analyzer::target::level_match_db(&s.bands_db, &self.goal.db));
                let per_oct = RESOLUTIONS[self.res].1;
                let target = self.analysis.as_ref().filter(|a| a.summary.is_some()).map(|a| {
                    let mut t = self.goal.db;
                    for (v, hz) in t.iter_mut().zip(BANDS_HZ) {
                        *v += offset - spectrum::display_to_band_db(hz, a.sample_rate, per_oct);
                    }
                    t
                });
                let h = (ui.available_height() - 90.0).max(120.0);
                ui.horizontal(|ui| {
                    ui.strong("실시간 주파수 그래프");
                    ui.label("표시");
                    // The coloured names double as the legend (a plot legend would cover the HIGH bar).
                    let sh = &mut self.show;
                    let c = |t: &str, col| egui::RichText::new(t).color(col);
                    ui.checkbox(&mut sh.result, c("분석 결과", mocha::GREEN));
                    ui.checkbox(&mut sh.live, c("현재 소리", mocha::BLUE));
                    ui.checkbox(&mut sh.peak, c("피크", mocha::OVERLAY1));
                    ui.checkbox(&mut sh.target, c("목표", mocha::SUBTEXT0));
                    ui.checkbox(&mut sh.knobs, c("노브 구간", mocha::OVERLAY1));
                    ui.checkbox(&mut sh.tone, c("대역 비교", mocha::RED));
                    ui.checkbox(&mut sh.bumps, c("봉우리 표시", mocha::PEACH));
                });
                let tone = summary.map(|s| {
                    let mut reference = self.goal.db;
                    reference.iter_mut().for_each(|v| *v += offset);
                    mixer_analyzer::target::tone_excess(&s.bands_db, &reference)
                });
                let bumps = match (&self.analysis, summary) {
                    (Some(a), Some(s)) if self.show.bumps => spectrum::peaks(&s.bin_power, a.sample_rate, per_oct),
                    _ => Vec::new(),
                };
                let spans = knob_spans(&self.profile.groups[self.group], knobs);
                let result = self.analysis.as_ref().and_then(|a| result_db(a, per_oct, FREQ_SMOOTH[self.freq_smooth].1));
                spectrum_plot(ui, &live.curve, result.as_deref(), target, tone, &bumps, per_oct, &self.show, &spans, h * 0.6);
                ui.strong("레벨/디스토션 기록 (최근 30초)");
                level_plot(ui, live.levels.history(), h * 0.4);
                status_line(
                    ui,
                    live,
                    notes.iter().any(|n| n == advice::OVERLOAD),
                );
            }
            Err(e) => {
                ui.strong("실시간 주파수 그래프");
                ui.colored_label(mocha::RED, e);
            }
        });
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mixer_analyzer::target::{hpf_db, ltass_db};

    fn spans(m: MixerModel, a: Option<&AdviceSet>) -> Vec<(String, f32, f32, Option<f32>)> {
        knob_spans(&MixerFactory::create(m).groups[0], a.map(|a| &a.knobs[..])).into_iter().map(|s| (s.label, s.lo, s.hi, s.mark)).collect()
    }
    #[test]
    fn knob_spans_profx22v3() {
        assert_eq!(spans(MixerModel::MackieProFx22v3, None), [
            ("저역컷 100Hz".into(), 20.0, 100.0, None), ("HIGH 12kHz".into(), 12_000.0, 20_000.0, None),
            ("MID".into(), 100.0, 8_000.0, None), ("LOW 80Hz".into(), 20.0, 80.0, None)]);
    }
    #[test]
    fn knob_spans_mark_recommended_freq() {
        let g = &MixerFactory::create(MixerModel::MackieProFx22v3).groups[0];
        let mut knobs = advice::labels(g);
        knobs.iter_mut().find(|k| k.label == "FREQ").unwrap().action = Action::SetFreq { hz: 400.0 };
        let a = AdviceSet { knobs, notes: vec![], level_offset_db: 0.0 };
        assert_eq!(spans(MixerModel::MackieProFx22v3, Some(&a))[2].3, Some(400.0));
    }
    #[test]
    fn knob_spans_fixed_mid_and_two_sweeps() {
        let fx12 = spans(MixerModel::MackieProFx12v3, None);
        let m = fx12.iter().find(|s| s.0 == "MID 2.5kHz").unwrap();
        assert!((m.1 - 2500.0 * 2f32.powf(-0.7)).abs() < 0.5 && (m.2 - 2500.0 * 2f32.powf(0.7)).abs() < 0.5);
        let g = &MixerFactory::create(MixerModel::SoundcraftSignature22).groups[0];
        let mut knobs = advice::labels(g);
        knobs.iter_mut().find(|k| k.label == "FREQ 2").unwrap().action = Action::SetFreq { hz: 200.0 };
        let a = AdviceSet { knobs, notes: vec![], level_offset_db: 0.0 };
        let s = spans(MixerModel::SoundcraftSignature22, Some(&a));
        let get = |l: &str| s.iter().find(|x| x.0 == l).unwrap().clone();
        assert_eq!(get("MID 1"), ("MID 1".into(), 400.0, 8_000.0, None));
        assert_eq!(get("MID 2"), ("MID 2".into(), 80.0, 2_000.0, Some(200.0)));
    }

    #[test]
    fn flat_lines_follow_group() {
        let lines = |m, i: usize| flat_lines(&MixerFactory::create(m).groups[i]);
        assert_eq!(lines(MixerModel::MackieProFx22v3, 0),
                   ["HIGH·MID·LOW 노브는 가운데(12시)로", "저역컷 끄기, COMP는 왼쪽 끝(꺼짐)", "GAIN은 그대로"]);
        for m in [MixerModel::YamahaMg06x, MixerModel::AhZed6] {
            assert_eq!(lines(m, 0), ["HIGH·LOW 노브는 가운데(12시)로", "저역컷 끄기", "GAIN은 그대로"]);
        }
        assert_eq!(lines(MixerModel::SoundcraftEpm, 0), ["HIGH·MID·LOW 노브는 가운데(12시)로", "GAIN은 그대로"]);
        assert_eq!(lines(MixerModel::SoundcraftSignature22, 0),
                   ["HIGH·MID·LOW 노브는 가운데(12시)로", "저역컷 끄기", "GAIN은 그대로"]);
    }

    #[test]
    fn dial_angle_is_log_linear() {
        assert!((dial_angle(100.0, 100.0, 8_000.0) + 150.0).abs() < 1e-3);
        assert!((dial_angle(8_000.0, 100.0, 8_000.0) - 150.0).abs() < 1e-3);
        assert!(dial_angle((100.0f32 * 8_000.0).sqrt(), 100.0, 8_000.0).abs() < 1e-2);
    }

    #[test]
    fn knob_captions_use_spec_text() {
        let text = |label: &str, control, action| {
            let k = Advice {
                label: label.into(),
                control,
                action,
            };
            caption(&k).into_iter().map(|(s, _)| s).collect::<Vec<_>>()
        };
        let gain = Control::Gain { range_db: 60.0 };
        let mid = Control::Mid {
            lo_hz: 100.0,
            hi_hz: 8_000.0,
        };
        let high = Control::HighShelf { hz: 12_000.0 };
        assert_eq!(
            text(
                "HIGH 12kHz",
                high,
                Action::Rotate {
                    degrees: 30.0,
                    db: 3.2
                }
            ),
            ["시계방향 30°", "(≈+3dB)"]
        );
        assert_eq!(
            text(
                "GAIN",
                gain,
                Action::Rotate {
                    degrees: -50.0,
                    db: -10.0
                }
            ),
            ["반시계 50°", "(≈-10dB)", "(지금 위치에서)"]
        );
        assert_eq!(
            text(
                "COMP",
                Control::Comp,
                Action::Rotate {
                    degrees: 30.0,
                    db: 0.0
                }
            ),
            ["왼쪽 끝에서 시계방향 30°"]
        );
        assert_eq!(
            text("FREQ", mid, Action::SetFreq { hz: 400.0 }),
            ["400Hz 눈금에"]
        );
        assert_eq!(text("FREQ", mid, Action::Ok), ["✓ 그대로"]);
        assert_eq!(text("MID", mid, Action::Ok), ["✓"]);
        assert_eq!(
            text(
                "저역컷 100Hz",
                Control::LowCut { hz: 100.0 },
                Action::SwitchOn
            ),
            ["켜기 권장"]
        );
        assert_eq!(text("GAIN", gain, Action::Wait), ["—"]);
    }

    #[test]
    fn stalled_stream_reads_as_silent() {
        let mut live = Live::new(audio::open_demo(), 48);
        live.last_data = Instant::now() - Duration::from_secs(4);
        assert!(live.silent());
        std::thread::sleep(Duration::from_millis(100));
        live.pump(0, 1.0, 48, 0);
        assert!(!live.silent());
    }

    #[test]
    fn live_built_at_96_handles_its_frames() {
        let mut live = Live::new(audio::open_demo(), 96);
        std::thread::sleep(Duration::from_millis(600));
        live.pump(0, 1.0, 96, 0);
    }

    #[test]
    fn recording_counts_audio_frames() {
        let f = Frame {
            bin_power: vec![],
            rms_db: -20.0,
            peak: 0.5,
        };
        let mut r = Recording::new(15, 4096.0 / 48_000.0);
        assert_eq!(r.needed, 176);
        for _ in 0..175 {
            r.push(&f, &[-30.0; 25], None);
        }
        assert!(!r.done());
        r.push(&f, &[-30.0; 25], None);
        assert!(r.done());
        assert_eq!(r.progress(), 1.0);
        assert_eq!(Recording::new(20, 4096.0 / 96_000.0).needed, 469);
        assert_eq!(Recording::new(20, 4096.0 / 44_100.0).needed, 215);
    }

    #[test]
    fn recording_keeps_last_howl() {
        let f = Frame {
            bin_power: vec![],
            rms_db: -20.0,
            peak: 0.5,
        };
        let mut r = Recording::new(15, 4096.0 / 48_000.0);
        r.push(&f, &[-30.0; 25], Some(2500.0));
        r.push(&f, &[-30.0; 25], None);
        assert_eq!(r.howl, Some(2500.0));
    }

    #[test]
    fn analysis_without_speech_says_too_little() {
        let a = Analysis {
            summary: None,
            sample_rate: 48_000.0,
            seconds: 20,
            clipped: false,
            howl: None,
            silent: false,
            flat: true,
        };
        let g = &MixerFactory::create(MixerModel::MackieProFx22v3).groups[0];
        let s = advise(&a, g, &preset_goal(0, Voice::Male, g));
        assert!(s.knobs.iter().all(|k| k.action == Action::Wait));
        assert!(s.notes.iter().any(|n| n.contains("말소리가 부족합니다")));
    }

    #[test]
    fn result_uses_analysis_sample_rate() {
        let sr = 48_000.0;
        // testsig is lib cfg(test), unavailable to the bin tests: build the 1 kHz sine here.
        let tone: Vec<f32> = (0..48_000).map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / sr).sin()).collect();
        let bin_power = Spectrum::new(sr).push(&tone).pop().unwrap().bin_power;
        let a = Analysis {
            summary: Some(Summary { bands_db: [0.0; 25], bin_power, peak_db: -12.0, rms_db: -30.0, noise_floor_db: -80.0,
                                    snr_db: 50.0, dynamic_range_db: 10.0, crest_db: 18.0, active_seconds: 10.0 }),
            sample_rate: sr, seconds: 20, clipped: false, howl: None, silent: false, flat: true,
        };
        let d = result_db(&a, 48, 0).unwrap();
        let freqs = spectrum::display_freqs(48);
        let top = (0..d.len()).max_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap();
        assert!((freqs[top] / 1000.0).log2().abs() <= 1.0 / 48.0, "{}", freqs[top]);
    }

    /// Male LTASS − 40 dB shaped by `dev`, otherwise a clean recording.
    fn natural(dev: impl Fn(f32) -> f32) -> Analysis {
        let t = ltass_db(Voice::Male);
        let s = Summary {
            bands_db: std::array::from_fn(|b| t[b] - 40.0 + dev(BANDS_HZ[b])),
            bin_power: vec![],
            peak_db: -12.0,
            rms_db: -30.0,
            noise_floor_db: -80.0,
            snr_db: 50.0,
            dynamic_range_db: 10.0,
            crest_db: 18.0,
            active_seconds: 10.0,
        };
        Analysis {
            summary: Some(s),
            sample_rate: 48_000.0,
            seconds: 20,
            clipped: false,
            howl: None,
            silent: false,
            flat: true,
        }
    }

    fn knob(a: &AdviceSet, l: &str) -> Action {
        a.knobs.iter().find(|k| k.label == l).unwrap().action.clone()
    }

    #[test]
    fn analysis_advice_follows_group() {
        let a = natural(|_| 0.0);
        let knobs = |m| {
            let g = &MixerFactory::create(m).groups[0];
            advise(&a, g, &preset_goal(0, Voice::Male, g)).knobs.len()
        };
        assert_eq!(knobs(MixerModel::MackieProFx22v3), 7);
        assert_eq!(knobs(MixerModel::YamahaMg06x), 4);
    }

    #[test] fn low_sample_rate_gives_names_and_one_note() {
        let g = &MixerFactory::create(MixerModel::MackieProFx12v3).groups[0];
        let goal = preset_goal(2, Voice::Male, g); // 300Hz 노트가 있는 목표
        assert!(!goal.notes.is_empty());
        let s = advise(&Analysis { sample_rate: 24_000.0, ..natural(|_| 6.0) }, g, &goal);
        assert!(s.knobs.iter().all(|k| k.action == Action::Wait), "{:?}", s.knobs);
        assert_eq!(s.notes, ["입력 샘플레이트가 24kHz라 16kHz 대역까지 잴 수 없습니다. 믹서 USB 입력(44.1kHz 이상)을 고르세요."]);
    }
    #[test] fn presets_do_not_chase_switch_with_low() {
        let g = &MixerFactory::create(MixerModel::MackieProFx22v3).groups[0];
        for i in 1..PRESETS.len() {
            let s = advise(&natural(|_| 0.0), g, &preset_goal(i, Voice::Male, g));
            assert_eq!(knob(&s, "저역컷 100Hz"), Action::SwitchOn, "{i}");
            assert_eq!(knob(&s, "LOW 80Hz"), Action::Ok, "{i}");
        }
    }
    #[test] fn preset_on_75hz_switch_settles_when_on() {
        let g = &MixerFactory::create(MixerModel::Mackie1202Vlz4).groups[0];
        let s = advise(&natural(|f| hpf_db(75.0, f)), g, &preset_goal(1, Voice::Male, g));
        assert_eq!(knob(&s, "저역컷 75Hz"), Action::Ok);
        assert_eq!(knob(&s, "LOW 80Hz"), Action::Ok);
    }
    #[test] fn light_leader_on_fixed_mid_does_not_brighten() {
        let g = &MixerFactory::create(MixerModel::MackieProFx12v3).groups[0];
        let s = advise(&natural(|_| 0.0), g, &preset_goal(2, Voice::Male, g));
        match knob(&s, "HIGH 12kHz") { Action::Ok => {}, Action::Rotate { db, .. } => assert!(db <= 4.0, "{db}"), x => panic!("{x:?}") }
        assert!(s.notes.iter().any(|n| n == "이 채널에는 300Hz 부근을 바꿀 노브가 없어 그 부분은 빼고 맞춥니다."));
    }
    #[test] fn preset_low_cut_without_switch_is_a_note() {
        let g = &MixerFactory::create(MixerModel::SoundcraftEpm).groups[0];
        assert_eq!(preset_goal(3, Voice::Male, g).notes, ["이 채널에는 100Hz 부근을 바꿀 노브가 없어 그 부분은 빼고 맞춥니다."]);
    }
    #[test] fn every_preset_on_every_group_is_finite() {
        for m in MixerModel::ALL { for g in &MixerFactory::create(m).groups { for i in 0..PRESETS.len() {
            for k in advise(&natural(|_| 0.0), g, &preset_goal(i, Voice::Female, g)).knobs {
                if let Action::Rotate { degrees, db } = k.action { assert!(degrees.is_finite() && db.is_finite()); }
            }
        }}}
    }

    #[test] fn saved_goal_flags_other_channel() {
        let s = Saved { name: "x".into(), mixer: "Mackie ProFX22v3 (2019, LEVEL SET LED)".into(), group: "CH1-12 (COMP)".into(), dynamic_range_db: 9.0, bands_db: [0.0; 25], sample_rate: Some(48_000), flat: None };
        let same = saved_goal(&s, &s.mixer, &s.group, Some(48_000.0));
        assert!(same.notes.is_empty() && same.restore_dr == Some(9.0) && same.db == s.bands_db);
        let other = saved_goal(&s, &s.mixer, "CH13-14", Some(48_000.0));
        assert_eq!(other.notes, ["저장할 때와 믹서/채널이 다릅니다(저장: Mackie ProFX22v3 (2019, LEVEL SET LED) CH1-12 (COMP)). 같은 채널에서 분석해야 맞습니다."]);
    }
    #[test] fn saved_goal_flags_sample_rate() {
        let s = saved_sample("x");
        assert_eq!(saved_goal(&s, "m", "g", Some(44_100.0)).notes,
                   ["저장할 때와 샘플레이트가 다릅니다(저장 48kHz, 지금 44.1kHz). 같은 입력장치 설정에서 분석해야 정확합니다."]);
        let old = Saved { sample_rate: None, ..s };
        assert_eq!(saved_goal(&old, "m", "g", Some(48_000.0)).notes,
                   ["이 소리는 이전 버전에서 저장해 저음 쪽 값이 조금 다릅니다. 같은 마이크·채널로 다시 저장해 주세요."]);
        assert_eq!([khz(96_000.0), khz(22_050.0)], ["96", "22.1"]);
    }
    #[test] fn old_profx22v3_stereo_label_is_the_same_group() {
        let s = Saved { group: "CH15-18 (스테레오 마이크)".into(), ..saved_sample("x") };
        assert!(saved_goal(&s, &s.mixer.clone(), "CH15-20 (스테레오 마이크)", Some(48_000.0)).notes.is_empty());
        assert_eq!(saved_goal(&s, "Mackie ProFX22 (1세대, 2011)", "CH15-20 (스테레오 마이크)", Some(48_000.0)).notes.len(), 1);
    }
    #[test] fn restore_from_saved_says_switch_on() {
        let g = &MixerFactory::create(MixerModel::MackieProFx22v3).groups[0];
        let saved_now = natural(|f| hpf_db(100.0, f)); // 저장 때: 스위치 켜짐
        let s = Saved { name: "x".into(), mixer: "m".into(), group: g.label.into(),
                        dynamic_range_db: 10.0, bands_db: saved_now.summary.as_ref().unwrap().bands_db, sample_rate: Some(48_000), flat: None };
        let a = advise(&natural(|_| 0.0), g, &saved_goal(&s, "m", g.label, Some(48_000.0))); // 지금: 꺼짐(가운데에서 시작)
        assert_eq!(knob(&a, "저역컷 100Hz"), Action::SwitchOn);
    }

    fn saved_sample(name: &str) -> Saved {
        Saved { name: name.into(), mixer: "m".into(), group: "g".into(), dynamic_range_db: 9.0, bands_db: [0.0; 25], sample_rate: Some(48_000), flat: None }
    }
    #[test] fn repick_follows_name() {
        let (a, b, c) = (saved_sample("a"), saved_sample("b"), saved_sample("c"));
        let old = [a.clone(), b.clone()];
        assert_eq!(repick(Pick::Saved(1), &old, &[b.clone(), c.clone()]), Pick::Saved(0));
        assert_eq!(repick(Pick::Saved(0), &old, &[b, c]), Pick::Preset(0));
        assert_eq!(repick(Pick::Preset(3), &old, &[]), Pick::Preset(3));
    }
    #[test] fn read_gone_folder_is_an_error() {
        let d = std::env::temp_dir().join(format!("mixer-analyzer-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(read_list(&d).unwrap_err().to_string(), "저장 폴더를 찾을 수 없습니다");
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(read_list(&d).unwrap(), (vec![], None));
        std::fs::remove_dir_all(&d).unwrap();
    }
    #[test] fn blocked_write_keeps_file() {
        let d = std::env::temp_dir().join(format!("mixer-analyzer-blocked-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(saved::FILE), "keep me").unwrap();
        assert_eq!(write_list(Some(d.as_path()), true, &[saved_sample("x")]),
                   Err("읽지 못한 파일은 덮어쓰지 않습니다. [읽기]나 [쓰기]를 누르세요.".to_owned()));
        assert_eq!(std::fs::read_to_string(d.join(saved::FILE)).unwrap(), "keep me");
        assert_eq!(write_list(None, false, &[]), Err("저장 폴더를 찾을 수 없습니다".to_owned()));
        assert!(write_list(Some(d.as_path()), false, &[saved_sample("x")]).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn act(k: &[Advice], l: &str) -> Action {
        k.iter().find(|k| k.label == l).unwrap().action.clone()
    }
    fn last_flat(mixer: &str, group: &str, a: &Analysis) -> LastFlat {
        LastFlat { mixer: mixer.into(), group: group.into(), sample_rate: a.sample_rate, summary: a.summary.clone().unwrap() }
    }
    fn as_is(a: Analysis) -> Analysis {
        Analysis { flat: false, ..a }
    }
    /// ProFX22v3 CH1-12에서 가운데 소리(남성 LTASS)로 시작해 저역컷만 켜고 저장한 세팅.
    fn knob_setup() -> (MixerProfile, String, Saved) {
        let p = MixerFactory::create(MixerModel::MackieProFx22v3);
        let m = mixer_name(&p);
        let flat = natural(|_| 0.0).summary.unwrap();
        let done = natural(|f| hpf_db(100.0, f)).summary.unwrap();
        let s = Saved { mixer: m.clone(), group: p.groups[0].label.into(), dynamic_range_db: done.dynamic_range_db, bands_db: done.bands_db,
                        flat: Some(Flat { dynamic_range_db: flat.dynamic_range_db, bands_db: flat.bands_db }), ..saved_sample("x") };
        (p, m, s)
    }
    #[test] fn knob_view_rules() {
        let (p, m, s) = knob_setup();
        let g = &p.groups[0];
        let view = |a: Option<&Analysis>, saved: Option<&Saved>, lf: Option<&LastFlat>| {
            let goal = saved.map_or_else(|| preset_goal(0, Voice::Male, g), |s| saved_goal(s, &m, g.label, Some(48_000.0)));
            knob_view(a, saved, lf, &m, g, &goal)
        };
        let (flat, done) = (natural(|_| 0.0), as_is(natural(|f| hpf_db(100.0, f))));
        let lf = last_flat(&m, g.label, &flat);
        let old = Saved { flat: None, ..s.clone() };
        let names = "가운데에서 분석하면 노브 추천이 나옵니다. 그다음 세팅을 바꾸고 '지금 세팅 그대로 분석'을 누르면 지금 노브 위치를 추정합니다.";
        // 1: 가운데 분석은 저장된 노브 위치가 있어도 오늘 소리 기준 추천.
        let v = view(Some(&flat), Some(&s), Some(&lf));
        assert!(matches!(&v, KnobView::Advice(a) if knob(a, "저역컷 100Hz") == Action::SwitchOn));
        assert_eq!(header(&v), Some("추천: 가운데(12시)에서 돌릴 양"));
        // 2: 분석이 없거나 그대로 분석이면 저장된 노브 위치(GAIN ✓). 다른 믹서에서 저장했으면 아님.
        for a in [None, Some(&done)] {
            let v = view(a, Some(&s), Some(&lf));
            let KnobView::Stored(k) = &v else { panic!("rule 2") };
            assert_eq!((act(k, "저역컷 100Hz"), act(k, "GAIN")), (Action::SwitchOn, Action::Ok));
            assert_eq!(header(&v), Some("저장된 세팅: 가운데(12시)에서 돌릴 양"));
        }
        let other_mixer = Saved { mixer: "Mackie ProFX22 (1세대, 2011)".into(), ..s.clone() };
        assert!(matches!(view(None, Some(&other_mixer), None), KnobView::None));
        assert!(matches!(view(Some(&done), Some(&other_mixer), Some(&lf)), KnobView::Estimate(_)));
        // 3: 그대로 분석 + 같은 믹서·그룹·샘플레이트의 last_flat이면 지금 노브 추정. 옛 저장(노브 위치 없음)도 여기로.
        for saved in [None, Some(&old)] {
            let v = view(Some(&done), saved, Some(&lf));
            let KnobView::Estimate(k) = &v else { panic!("rule 3") };
            assert_eq!((act(k, "저역컷 100Hz"), act(k, "GAIN")), (Action::SwitchOn, Action::Ok));
            assert_eq!(header(&v), Some("지금 노브 추정: 가운데(12시)에서 돌린 양"));
        }
        // 4: 그 밖의 그대로 분석: last_flat 없음, 다른 그룹, 다른 샘플레이트.
        let other_group = last_flat(&m, p.groups[1].label, &flat);
        let other_rate = LastFlat { sample_rate: 44_100.0, ..last_flat(&m, g.label, &flat) };
        for lf in [None, Some(&other_group), Some(&other_rate)] {
            let v = view(Some(&done), None, lf);
            assert!(matches!(&v, KnobView::Names(n) if n == names));
            assert_eq!(header(&v), None);
        }
        // 그대로 분석이 말소리 부족이면 그 노트(잘못된 추천 없음).
        let short = as_is(Analysis { summary: None, ..natural(|_| 0.0) });
        assert!(matches!(view(Some(&short), None, Some(&lf)), KnobView::Advice(a) if a.knobs.iter().all(|k| k.action == Action::Wait)));
        // 5: 분석 없음(옛 저장도).
        for saved in [None, Some(&old)] {
            let v = view(None, saved, Some(&lf));
            assert!(matches!(v, KnobView::None));
            assert_eq!(header(&v), None);
        }
    }
    #[test] fn difference_note_only_when_different() {
        let (p, m, s) = knob_setup();
        let g = &p.groups[0];
        let notes = |a: &Analysis, s: &Saved| match knob_view(Some(a), Some(s), None, &m, g, &saved_goal(s, &m, g.label, Some(48_000.0))) {
            KnobView::Advice(a) => a.notes,
            _ => panic!("rule 1"),
        };
        let note = "오늘 소리 기준 추천이 저장된 노브 위치와 다릅니다(마이크·목소리·자리가 바뀌었을 수 있음). 오늘 소리 기준인 지금 추천을 따르세요.";
        let darker = || natural(|f| -6.0 * advice::Shape::HighShelf(12_000.0).response(f)); // 오늘은 고역이 6 dB 어두움
        assert!(notes(&natural(|_| 0.0), &s).is_empty());
        assert_eq!(notes(&darker(), &s), [note]);
        // 다른 그룹에서 저장했거나 노브 위치가 없으면 비교하지 않습니다.
        let other = Saved { group: p.groups[1].label.into(), ..s.clone() };
        assert!(!notes(&darker(), &other).iter().any(|n| n == note));
        assert!(notes(&darker(), &Saved { flat: None, ..s }).is_empty());
    }
    #[test] fn flat_for_save_cases() {
        let (m, g) = ("m", "g");
        let flat = natural(|_| 0.0);
        let lf = last_flat(m, g, &natural(|_| 3.0));
        let of = |s: &Summary| Some(Flat { dynamic_range_db: s.dynamic_range_db, bands_db: s.bands_db });
        assert_eq!(flat_for_save(&flat, Some(&lf), m, g), of(flat.summary.as_ref().unwrap()));
        let done = as_is(natural(|f| hpf_db(100.0, f)));
        assert_eq!(flat_for_save(&done, Some(&lf), m, g), of(&lf.summary));
        assert_eq!(flat_for_save(&done, Some(&lf), m, "other"), None);
        assert_eq!(flat_for_save(&done, None, m, g), None);
        assert_eq!(flat_for_save(&Analysis { sample_rate: 44_100.0, ..done }, Some(&lf), m, g), None);
    }
}
