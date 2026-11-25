//! Look and feel: a dark "pro app" theme (default) and the original light theme.

use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};
use std::sync::atomic::{AtomicBool, Ordering};

static DARK: AtomicBool = AtomicBool::new(true);

pub fn dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

pub fn accent() -> Color32 {
    if dark() { hex(0x4C8DFF) } else { hex(0x1D5CD6) }
}
pub fn accent_soft() -> Color32 {
    if dark() { hex(0x2A4A80) } else { hex(0xDCE7FA) }
}
pub fn text_weak() -> Color32 {
    if dark() { hex(0x8A919C) } else { hex(0x6E7480) }
}
pub fn header() -> Color32 {
    if dark() { hex(0x9AA3B0) } else { hex(0x55606E) }
}
pub fn tile_bg() -> Color32 {
    if dark() { hex(0x2A2E35) } else { hex(0xEEEEEE) }
}
pub fn tile_hover() -> Color32 {
    if dark() { hex(0x33404F) } else { hex(0xDEE8F8) }
}
pub fn tile_text() -> Color32 {
    if dark() { hex(0xC8CDD5) } else { hex(0x3C3C3C) }
}
pub fn export_green() -> Color32 {
    if dark() { hex(0x4ADE80) } else { hex(0x147A3C) }
}
pub fn error() -> Color32 {
    if dark() { hex(0xFF6B6B) } else { hex(0xC81E1E) }
}
pub fn warn() -> Color32 {
    if dark() { hex(0xF5A524) } else { hex(0xC87800) }
}
pub fn info() -> Color32 {
    if dark() { hex(0x6CA8FF) } else { hex(0x2864B4) }
}
/// (foreground, background) for the verdict banner
pub fn ok_banner() -> (Color32, Color32) {
    if dark() { (hex(0x6EE7A0), hex(0x173524)) } else { (hex(0x146E28), hex(0xE2F5E6)) }
}
pub fn bad_banner() -> (Color32, Color32) {
    if dark() { (hex(0xFF8A8A), hex(0x3D1C1F)) } else { (hex(0xAA1414), hex(0xFDE4E4)) }
}
pub fn hint() -> Color32 {
    if dark() { hex(0x6C7480) } else { hex(0x6E6E6E) }
}

/// 3D view colours
pub fn clear_color() -> [f32; 3] {
    if dark() { [0.135, 0.150, 0.172] } else { [0.93, 0.94, 0.96] }
}
pub fn bed_color() -> [f32; 4] {
    if dark() { [0.20, 0.22, 0.25, 1.0] } else { [0.82, 0.84, 0.87, 1.0] }
}
pub fn grid_color() -> [f32; 4] {
    if dark() { [0.31, 0.34, 0.39, 0.9] } else { [0.62, 0.66, 0.72, 0.7] }
}
pub fn grid_axis_color() -> [f32; 4] {
    if dark() { [0.45, 0.50, 0.58, 1.0] } else { [0.45, 0.5, 0.6, 1.0] }
}
pub fn hole_color(selected: bool) -> [f32; 4] {
    if dark() { [0.72, 0.76, 0.82, if selected { 0.40 } else { 0.26 }] } else { [0.55, 0.58, 0.62, if selected { 0.45 } else { 0.3 }] }
}

pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title.to_uppercase()).size(11.5).strong().color(header()));
    ui.add_space(1.0);
}

pub fn load_saved() -> bool {
    !crate::settings::get("theme").iter().any(|v| v == "light")
}

pub fn save(dark: bool) {
    crate::settings::set("theme", &[if dark { "dark".into() } else { "light".into() }]);
}

pub fn apply(ctx: &egui::Context, is_dark: bool) {
    DARK.store(is_dark, Ordering::Relaxed);
    // Use our theme no matter what the operating system's light/dark setting is.
    let theme = if is_dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(theme);
    let r4 = CornerRadius::same(4);
    let mut v = if is_dark { Visuals::dark() } else { Visuals::light() };
    if is_dark {
        v.panel_fill = hex(0x1E2126);
        v.window_fill = hex(0x24282E);
        v.extreme_bg_color = hex(0x15171B);
        v.faint_bg_color = hex(0x252930);
        v.code_bg_color = hex(0x15171B);
        v.window_stroke = Stroke::new(1.0, hex(0x363B43));
        v.widgets.noninteractive.bg_fill = hex(0x1E2126);
        v.widgets.noninteractive.weak_bg_fill = hex(0x1E2126);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, hex(0x30353C));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, hex(0xC9CED6));
        v.widgets.inactive.bg_fill = hex(0x2C3139);
        v.widgets.inactive.weak_bg_fill = hex(0x2C3139);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, hex(0xD6DAE0));
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, hex(0x363B44));
        v.widgets.hovered.bg_fill = hex(0x37404C);
        v.widgets.hovered.weak_bg_fill = hex(0x37404C);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, accent());
        v.widgets.hovered.fg_stroke = Stroke::new(1.5, hex(0xFFFFFF));
        v.widgets.active.bg_fill = hex(0x2F5FB8);
        v.widgets.active.weak_bg_fill = hex(0x2F5FB8);
        v.widgets.active.fg_stroke = Stroke::new(1.5, hex(0xFFFFFF));
        v.widgets.open.weak_bg_fill = hex(0x2F3742);
        v.selection.bg_fill = hex(0x2F5FB8);
        v.selection.stroke = Stroke::new(1.0, hex(0xFFFFFF));
        v.hyperlink_color = accent();
    }
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r4;
    }
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    ctx.set_visuals_of(theme, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.spacing.interact_size.y = 22.0;
    });
}
