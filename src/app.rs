use eframe::egui::{
    self, Align, Color32, Layout, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, vec2,
};
use egui_plot::{GridMark, Legend, Line, LineStyle, Plot, PlotPoints, Points};
use mixer_analyzer::{
    advice::{self, Action, Advice, AdviceEngine, AdviceSet, Inputs, fmt_hz},
    audio::{self, Input},
    feedback::FeedbackDetector,
    levels::{HistPoint, LevelMeter},
    mixers::{ChannelGroup, Control, KNOB_TRAVEL_DEG, MixerFactory, MixerModel, MixerProfile},
    spectrum::{self, DisplayCurve, Frame, HOP, Spectrum},
    speech::{SpeechStats, Summary},
    target::{BANDS_HZ, Voice, ltass_db},
    theme::{self, mocha},
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEMO: &str = "데모 신호";
const CHANNELS: [&str; 2] = ["1(L)", "2(R)"];
const TICKS_HZ: [f64; 10] = [20.0, 50.0, 100.0, 200.0, 500.0, 1e3, 2e3, 5e3, 1e4, 2e4];
const SMOOTHING: [(&str, f32); 3] = [("빠름 0.3초", 0.3), ("보통 1초", 1.0), ("느림 3초", 3.0)];
const KNOB_W: f32 = 96.0;
const KNOB_R: f32 = 20.0;

/// One press of `분석`: collects audio frames until `needed` have arrived.
struct Recording {
    stats: SpeechStats,
    needed: usize,
    /// Last howl frequency seen during the recording.
    howl: Option<f32>,
}

impl Recording {
    fn new(seconds: u32, hop_s: f32) -> Self {
        Self {
            stats: SpeechStats::new(hop_s),
            needed: (seconds as f32 / hop_s).round() as usize,
            howl: None,
        }
    }

    fn push(&mut self, f: &Frame, bands: &[f32; 25], disp: &[f32], howl: Option<f32>) {
        self.stats.push(f, bands, disp);
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
    /// Recording length; `clipped` and `silent` cover this window.
    #[expect(dead_code, reason = "brief interface; no spec UI shows it")]
    seconds: u32,
    clipped: bool,
    howl: Option<f32>,
    silent: bool,
}

/// A fresh engine each time, so every knob starts from ✓ (no deadband state carried over).
fn advise(a: &Analysis, group: &ChannelGroup, voice: Voice) -> AdviceSet {
    AdviceEngine::new().advise(
        group,
        &Inputs {
            summary: a.summary.as_ref(),
            target: &ltass_db(voice),
            clipped_recently: a.clipped,
            feedback_hz: a.howl,
            silent: a.silent,
        },
    )
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
    fn new(input: Input) -> Self {
        let sr = input.sample_rate;
        Self {
            levels: LevelMeter::new(sr, input.channels),
            spectrum: Spectrum::new(sr),
            feedback: FeedbackDetector::new(sr),
            curve: DisplayCurve::new(spectrum::display_freqs().len()),
            howl: None,
            rec: None,
            last_data: Instant::now(),
            input,
        }
    }

    /// Drains whole frames from the ring and runs every analyser on them.
    fn pump(&mut self, channel: usize, tau_s: f32) {
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
            let display = spectrum::display_db(&f.bin_power, sr);
            self.howl = self.feedback.push(&f.bin_power);
            self.curve.update(&display, HOP as f32 / sr, tau_s);
            if let Some(r) = self.rec.as_mut().filter(|r| !r.done()) {
                let bands = spectrum::third_octave_db(&f.bin_power, sr);
                r.push(&f, &bands, &display, self.howl);
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

fn open(device: &str) -> Result<Live, String> {
    if device == DEMO {
        Ok(audio::open_demo())
    } else {
        audio::open(Some(device))
    }
    .map(Live::new)
    .map_err(|e| format!("입력 장치를 열 수 없습니다: {e}"))
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
    rec_seconds: u32,
    /// Index into `SMOOTHING`.
    smooth: usize,
    analysis: Option<Analysis>,
    advice: Option<AdviceSet>,
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
        Self {
            devices: audio::device_names(),
            live: open(&device),
            device,
            channel: 0,
            model,
            profile: MixerFactory::create(model),
            group: 0,
            voice: Voice::Male,
            rec_seconds: 20,
            smooth: 1,
            analysis: None,
            advice: None,
        }
    }

    fn readvise(&mut self) {
        let group = &self.profile.groups[self.group];
        self.advice = self.analysis.as_ref().map(|a| advise(a, group, self.voice));
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let name = |p: &MixerProfile| format!("{} {}", p.brand, p.name);
        ui.label("믹서");
        let mut model = self.model;
        egui::ComboBox::from_id_salt("mixer")
            .selected_text(name(&self.profile))
            .show_ui(ui, |ui| {
                for m in MixerModel::ALL {
                    ui.selectable_value(&mut model, m, name(&MixerFactory::create(m)));
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
            self.live = open(&pick);
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
        ui.label("목표");
        let voice = self.voice;
        ui.selectable_value(&mut self.voice, Voice::Male, "남성");
        ui.selectable_value(&mut self.voice, Voice::Female, "여성");
        if self.voice != voice {
            self.readvise();
        }
        let recording = self.live.as_ref().is_ok_and(|l| l.rec.is_some());
        ui.label("녹음");
        ui.add_enabled(
            !recording,
            egui::Slider::new(&mut self.rec_seconds, 15..=30).suffix("초"),
        );
        let label = if recording { "취소" } else { "분석" };
        if ui
            .add_enabled(self.live.is_ok(), egui::Button::new(label))
            .clicked()
            && let Ok(live) = &mut self.live
        {
            live.rec = if recording {
                None
            } else {
                live.curve.reset_peak();
                Some(Recording::new(
                    self.rec_seconds,
                    HOP as f32 / live.input.sample_rate,
                ))
            };
        }
        ui.label("실시간 선");
        egui::ComboBox::from_id_salt("smooth")
            .selected_text(SMOOTHING[self.smooth].0)
            .show_ui(ui, |ui| {
                for (i, (label, _)) in SMOOTHING.into_iter().enumerate() {
                    ui.selectable_value(&mut self.smooth, i, label);
                }
            });
        if ui.button("클리핑 리셋").clicked()
            && let Ok(live) = &mut self.live
        {
            live.levels.reset_clips();
        }
        if ui.button("피크 리셋").clicked()
            && let Ok(live) = &mut self.live
        {
            live.curve.reset_peak();
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

/// `result_db` is the last analysis on the display points; `target_db` is the 1/3-octave
/// target already shifted onto the display scale.
fn spectrum_plot(
    ui: &mut egui::Ui,
    curve: &DisplayCurve,
    result_db: Option<&[f32]>,
    target_db: Option<[f32; 25]>,
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
    let freqs = spectrum::display_freqs();
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
    .legend(Legend::default())
    .show(ui, |p| {
        if let Some(db) = result_db {
            p.line(line("분석 결과", &freqs, db, mocha::GREEN).width(2.5));
        }
        p.line(line("현재 소리", &freqs, &curve.smooth_db, mocha::BLUE));
        p.line(line("피크", &freqs, &curve.peak_db, mocha::PEACH));
        if let Some(t) = target_db {
            p.line(line("목표", &BANDS_HZ, &t, mocha::SUBTEXT0).style(LineStyle::dotted_dense()));
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
            let mut v = one(format!("{dir} {:.0}°", degrees.abs()), mocha::YELLOW);
            // COMP의 db는 의미 없는 0입니다.
            if k.control != Control::Comp {
                v.push((format!("(≈{db:+.0}dB)"), mocha::YELLOW));
            }
            v
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
    match (k.action.clone(), k.control) {
        (Action::Rotate { degrees, .. }, _) => {
            pointer(0.0, mocha::TEXT);
            let ra = KNOB_R + 7.0;
            let arc: Vec<Pos2> = (0..=24)
                .map(|i| at(c, ra, degrees * i as f32 / 24.0))
                .collect();
            p.add(Shape::line(arc, Stroke::new(2.5, mocha::YELLOW)));
            let a = degrees.to_radians();
            let (e, t, n) = (
                at(c, ra, degrees),
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
            pointer(0.0, mocha::SUBTEXT0);
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let finished = self.live.as_mut().ok().and_then(|live| {
            live.pump(self.channel, SMOOTHING[self.smooth].1);
            let rec = live.rec.take_if(|r| r.done())?;
            let n = self.rec_seconds as f32;
            Some(Analysis {
                summary: rec.stats.summary(),
                seconds: self.rec_seconds,
                clipped: live.levels.clipped_within(n),
                howl: rec.howl,
                silent: live.silent_for(n),
            })
        });
        if finished.is_some() {
            self.analysis = finished;
            self.readvise();
        }
        egui::Panel::top("top").show(ui, |ui| ui.horizontal_wrapped(|ui| self.top_bar(ui)));
        let (advice, live) = (self.advice.as_ref(), self.live.as_ref().ok());
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
                if live.is_some_and(Live::silent) {
                    ui.colored_label(mocha::RED, advice::NO_INPUT);
                }
            }
            if let Some(a) = advice {
                ui.strong(summary.map_or("노브 추천".into(), |s| {
                    format!("노브 추천 — 말소리 {:.1}초 분석", s.active_seconds)
                }));
                knob_row(ui, &a.knobs);
            } else if rec.is_none() {
                ui.label(format!(
                    "[분석]을 누르고 {}초 동안 평소처럼 말씀하세요.",
                    self.rec_seconds
                ));
            }
            ui.separator();
            ui.strong("안내");
            for n in advice.iter().flat_map(|a| &a.notes) {
                ui.label(n);
            }
            ui.colored_label(mocha::SUBTEXT0, self.profile.pc_tip);
        });
        egui::CentralPanel::default().show(ui, |ui| match &self.live {
            Ok(live) => {
                let offset =
                    advice.map_or(0.0, |a| a.level_offset_db) - spectrum::DISPLAY_TO_BAND_DB;
                let target = summary.map(|_| ltass_db(self.voice).map(|t| t + offset));
                let h = (ui.available_height() - 90.0).max(120.0);
                ui.strong("실시간 주파수 그래프");
                let result = summary.map(|s| s.display_db.as_slice());
                spectrum_plot(ui, &live.curve, result, target, h * 0.6);
                ui.strong("레벨/디스토션 기록 (최근 30초)");
                level_plot(ui, live.levels.history(), h * 0.4);
                status_line(
                    ui,
                    live,
                    advice.is_some_and(|a| a.notes.iter().any(|n| n == advice::OVERLOAD)),
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
            ["반시계 50°", "(≈-10dB)"]
        );
        assert_eq!(
            text(
                "COMP",
                Control::Comp,
                Action::Rotate {
                    degrees: -30.0,
                    db: 0.0
                }
            ),
            ["반시계 30°"]
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
        let mut live = Live::new(audio::open_demo());
        live.last_data = Instant::now() - Duration::from_secs(4);
        assert!(live.silent());
        std::thread::sleep(Duration::from_millis(100));
        live.pump(0, 1.0);
        assert!(!live.silent());
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
            r.push(&f, &[-30.0; 25], &[], None);
        }
        assert!(!r.done());
        r.push(&f, &[-30.0; 25], &[], None);
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
        r.push(&f, &[-30.0; 25], &[], Some(2500.0));
        r.push(&f, &[-30.0; 25], &[], None);
        assert_eq!(r.howl, Some(2500.0));
    }

    #[test]
    fn analysis_without_speech_says_too_little() {
        let a = Analysis {
            summary: None,
            seconds: 20,
            clipped: false,
            howl: None,
            silent: false,
        };
        let g = &MixerFactory::create(MixerModel::MackieProFx22v3).groups[0];
        let s = advise(&a, g, Voice::Male);
        assert!(s.knobs.iter().all(|k| k.action == Action::Wait));
        assert!(s.notes.iter().any(|n| n.contains("말소리가 부족합니다")));
    }

    #[test]
    fn analysis_advice_follows_group() {
        let s = Summary {
            bands_db: ltass_db(Voice::Male).map(|t| t - 40.0),
            display_db: vec![],
            peak_db: -12.0,
            rms_db: -30.0,
            noise_floor_db: -80.0,
            snr_db: 50.0,
            dynamic_range_db: 10.0,
            crest_db: 18.0,
            active_seconds: 10.0,
        };
        let a = Analysis {
            summary: Some(s),
            seconds: 20,
            clipped: false,
            howl: None,
            silent: false,
        };
        let knobs = |m| {
            advise(&a, &MixerFactory::create(m).groups[0], Voice::Male)
                .knobs
                .len()
        };
        assert_eq!(knobs(MixerModel::MackieProFx22v3), 7);
        assert_eq!(knobs(MixerModel::YamahaMg06x), 4);
    }
}
