//! The launcher's look: dark panels and the logo's gold.

use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle};

pub const GOLD: Color32 = Color32::from_rgb(0xE0, 0xB0, 0x40);
pub const GOLD_DIM: Color32 = Color32::from_rgb(0x9A, 0x77, 0x2A);
pub const BG: Color32 = Color32::from_rgb(0x12, 0x12, 0x14);
pub const PANEL: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1D);
pub const CARD: Color32 = Color32::from_rgb(0x20, 0x1F, 0x22);
pub const BUTTON: Color32 = Color32::from_rgb(0x2A, 0x28, 0x2B);
pub const TEXT: Color32 = Color32::from_rgb(0xE8, 0xE2, 0xD2);
pub const WEAK: Color32 = Color32::from_rgb(0x9C, 0x96, 0x88);
pub const LINE: Color32 = Color32::from_rgb(0x34, 0x31, 0x2C);
pub const BAD: Color32 = Color32::from_rgb(0xE0, 0x6A, 0x50);
pub const GOOD: Color32 = Color32::from_rgb(0x8C, 0xC0, 0x6A);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.global_style()).clone();
    style.text_styles = [
        (TextStyle::Heading, crate::fonts::title(26.0)),
        (TextStyle::Body, FontId::proportional(15.0)),
        (TextStyle::Button, FontId::proportional(15.0)),
        (TextStyle::Small, FontId::proportional(13.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = GOLD;
    v.selection.bg_fill = GOLD_DIM;
    v.selection.stroke = Stroke::new(1.0, GOLD);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(4);
    }
    v.widgets.inactive.weak_bg_fill = BUTTON;
    v.widgets.inactive.bg_fill = BUTTON;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x36, 0x32, 0x2C);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, GOLD_DIM);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x44, 0x3C, 0x2C);
    v.widgets.active.bg_stroke = Stroke::new(1.0, GOLD);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    ctx.set_global_style(style);
}
