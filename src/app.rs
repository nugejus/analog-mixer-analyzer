use eframe::egui;
use egui_plot::{GridMark, Legend, Line, Plot, PlotPoints};
use mixer_analyzer::{
    audio::{self, Input},
    feedback::FeedbackDetector,
    levels::LevelMeter,
    spectrum::{self, DisplayCurve, HOP, Spectrum},
    speech::SpeechStats,
    theme::{self, mocha},
};

const DEMO: &str = "데모 신호";
const CHANNELS: [&str; 2] = ["1(L)", "2(R)"];
const TICKS_HZ: [f64; 10] = [20.0, 50.0, 100.0, 200.0, 500.0, 1e3, 2e3, 5e3, 1e4, 2e4];

/// An open input and the analysis state built for its sample rate and channel count.
struct Live {
    input: Input,
    levels: LevelMeter,
    spectrum: Spectrum,
    speech: SpeechStats,
    feedback: FeedbackDetector,
    curve: DisplayCurve,
}

impl Live {
    fn new(input: Input) -> Self {
        let sr = input.sample_rate;
        Self {
            levels: LevelMeter::new(sr, input.channels),
            spectrum: Spectrum::new(sr),
            speech: SpeechStats::new(HOP as f32 / sr),
            feedback: FeedbackDetector::new(sr),
            curve: DisplayCurve::new(spectrum::display_freqs().len()),
            input,
        }
    }

    /// Drains whole frames from the ring and runs every analyser on them.
    fn pump(&mut self, channel: usize) {
        let (sr, ch) = (self.input.sample_rate, self.input.channels.max(1));
        let n = self.input.rx.slots() / ch * ch;
        let data: Vec<f32> = self
            .input
            .rx
            .read_chunk(n)
            .map(|c| c.into_iter().collect())
            .unwrap_or_default();
        self.levels.push_interleaved(&data);
        for f in self.spectrum.push(&audio::pick_channel(&data, ch, channel)) {
            self.speech
                .push(&f, &spectrum::third_octave_db(&f.bin_power, sr));
            self.feedback.push(&f.bin_power);
            self.curve
                .update(&spectrum::display_db(&f.bin_power, sr), HOP as f32 / sr);
        }
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
}

impl App {
    pub fn new(cc: &eframe::CreationContext, demo: bool) -> Self {
        theme::install(&cc.egui_ctx);
        let device = if demo {
            DEMO.to_owned()
        } else {
            audio::default_device_name().unwrap_or_default()
        };
        Self {
            devices: audio::device_names(),
            live: open(&device),
            device,
            channel: 0,
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
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
        egui::ComboBox::from_id_salt("channel")
            .selected_text(CHANNELS[self.channel])
            .show_ui(ui, |ui| {
                for (i, label) in CHANNELS.into_iter().enumerate() {
                    ui.selectable_value(&mut self.channel, i, label);
                }
            });
    }
}

fn spectrum_plot(ui: &mut egui::Ui, curve: &DisplayCurve) {
    let freqs = spectrum::display_freqs();
    let line = |name: &str, db: &[f32], color| {
        let pts: PlotPoints = freqs
            .iter()
            .zip(db)
            .map(|(f, d)| [f64::from(f.log10()), f64::from(*d)])
            .collect();
        Line::new(name, pts).color(color)
    };
    Plot::new("spectrum")
        .default_x_bounds(20f64.log10(), 20_000f64.log10())
        .default_y_bounds(-90.0, 0.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .grid_color(mocha::SURFACE1)
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
            p.line(line("현재 소리", &curve.smooth_db, mocha::BLUE));
            p.line(line("피크 홀드", &curve.peak_db, mocha::PEACH));
        });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Ok(live) = &mut self.live {
            live.pump(self.channel);
        }
        egui::Panel::top("top").show(ui, |ui| ui.horizontal(|ui| self.top_bar(ui)));
        egui::CentralPanel::default().show(ui, |ui| {
            ui.strong("실시간 주파수 그래프");
            match &self.live {
                Ok(live) => spectrum_plot(ui, &live.curve),
                Err(e) => {
                    ui.colored_label(mocha::RED, e);
                }
            }
        });
        ui.ctx().request_repaint();
    }
}
