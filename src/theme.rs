//! Colours: Catppuccin Mocha, palette v1.8.0 (https://github.com/catppuccin/palette).
//! Copyright (c) 2021 Catppuccin. MIT License.
//! Font: Pretendard Regular v1.3.9, SIL Open Font License 1.1 (assets/Pretendard-LICENSE.txt).

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, Theme};
use std::sync::Arc;

pub mod mocha {
    use eframe::egui::Color32;

    pub const ROSEWATER: Color32 = Color32::from_rgb(0xf5, 0xe0, 0xdc);
    pub const RED: Color32 = Color32::from_rgb(0xf3, 0x8b, 0xa8);
    pub const MAROON: Color32 = Color32::from_rgb(0xeb, 0xa0, 0xac);
    pub const PEACH: Color32 = Color32::from_rgb(0xfa, 0xb3, 0x87);
    pub const YELLOW: Color32 = Color32::from_rgb(0xf9, 0xe2, 0xaf);
    pub const GREEN: Color32 = Color32::from_rgb(0xa6, 0xe3, 0xa1);
    pub const SAPPHIRE: Color32 = Color32::from_rgb(0x74, 0xc7, 0xec);
    pub const BLUE: Color32 = Color32::from_rgb(0x89, 0xb4, 0xfa);
    pub const TEXT: Color32 = Color32::from_rgb(0xcd, 0xd6, 0xf4);
    pub const SUBTEXT0: Color32 = Color32::from_rgb(0xa6, 0xad, 0xc8);
    pub const OVERLAY1: Color32 = Color32::from_rgb(0x7f, 0x84, 0x9c);
    pub const SURFACE2: Color32 = Color32::from_rgb(0x58, 0x5b, 0x70);
    pub const SURFACE1: Color32 = Color32::from_rgb(0x45, 0x47, 0x5a);
    pub const SURFACE0: Color32 = Color32::from_rgb(0x31, 0x32, 0x44);
    pub const BASE: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2e);
    pub const MANTLE: Color32 = Color32::from_rgb(0x18, 0x18, 0x25);
    pub const CRUST: Color32 = Color32::from_rgb(0x11, 0x11, 0x1b);
}

/// Pretendard for Korean text and dark visuals mapped onto Mocha.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "pretendard".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/Pretendard-Regular.otf"
        ))),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "pretendard".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("pretendard".to_owned());
    ctx.set_fonts(fonts);

    use mocha::*;
    let mut v = egui::Visuals::dark();
    v.panel_fill = BASE;
    v.window_fill = MANTLE;
    v.extreme_bg_color = CRUST;
    v.window_stroke.color = SURFACE0;
    v.selection.bg_fill = BLUE;
    v.selection.stroke.color = CRUST;
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke.color = SURFACE0;
    for (wv, bg) in [
        (&mut w.noninteractive, BASE),
        (&mut w.inactive, SURFACE0),
        (&mut w.hovered, SURFACE1),
        (&mut w.active, SURFACE2),
        (&mut w.open, SURFACE1),
    ] {
        (wv.bg_fill, wv.weak_bg_fill, wv.fg_stroke.color) = (bg, bg, TEXT);
    }
    ctx.set_visuals_of(Theme::Dark, v);
    ctx.set_theme(Theme::Dark);
}
