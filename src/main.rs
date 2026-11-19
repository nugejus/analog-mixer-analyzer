#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use eframe::egui;

fn main() -> eframe::Result {
    let demo = std::env::args().any(|a| a == "--demo");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mixer Analyzer")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([1060.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Mixer Analyzer",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, demo)))),
    )
}
