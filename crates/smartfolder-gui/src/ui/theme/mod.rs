//! Theme entry points for the smartfolder GUI design system.
//!
//! The theme module centralizes visual tokens that map the RC2 design system to
//! egui primitives. It applies window-level visuals and re-exports small token
//! modules so screen code can share spacing, color, typography, and widget
//! behavior.

pub(crate) mod colors;
pub(crate) mod spacing;
pub(crate) mod typography;
pub(crate) mod widgets;

use eframe::egui::{self, Style, Visuals};

const INTER_FONT_NAME: &str = "smartfolder-inter";
const INTER_VARIABLE_FONT: &[u8] = include_bytes!("../../../assets/fonts/InterVariable.ttf");

/// Resolved visual theme used after applying the user's theme preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisualTheme {
    /// Warm light theme matching the v2 mockup direction.
    Light,
    /// Dark theme using the same semantic token roles.
    Dark,
}

/// Register bundled product fonts and icon fonts with egui.
pub(crate) fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        INTER_FONT_NAME.to_owned(),
        egui::FontData::from_static(INTER_VARIABLE_FONT).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, INTER_FONT_NAME.to_owned());

    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    ctx.set_fonts(fonts);
}

/// Apply the design-system theme to the egui context.
pub(crate) fn apply_visual_theme(ctx: &egui::Context, theme: VisualTheme) {
    let mut style: Style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(spacing::SM, spacing::SM);
    style.spacing.button_padding = egui::vec2(spacing::MD, spacing::XS);
    style.spacing.menu_margin = egui::Margin::same(spacing::SM);
    style.spacing.window_margin = egui::Margin::same(spacing::LG);
    style.visuals = match theme {
        VisualTheme::Light => light_visuals(),
        VisualTheme::Dark => Visuals::dark(),
    };
    style.text_styles = typography::text_styles();
    ctx.set_style(style);
}

fn light_visuals() -> Visuals {
    let mut visuals = Visuals::light();
    visuals.window_fill = colors::surface();
    visuals.panel_fill = colors::app_background();
    visuals.faint_bg_color = colors::subtle_surface();
    visuals.extreme_bg_color = colors::elevated_surface();
    visuals.widgets.noninteractive.bg_fill = colors::surface();
    visuals.widgets.noninteractive.fg_stroke.color = colors::primary_text();
    visuals.widgets.inactive.bg_fill = colors::soft_control();
    visuals.widgets.inactive.fg_stroke.color = colors::primary_text();
    visuals.widgets.hovered.bg_fill = colors::hover_control();
    visuals.widgets.hovered.fg_stroke.color = colors::primary_text();
    visuals.widgets.hovered.weak_bg_fill = colors::hover_control();
    visuals.widgets.active.bg_fill = colors::primary_blue();
    visuals.widgets.active.fg_stroke.color = colors::on_primary();
    visuals.widgets.active.weak_bg_fill = colors::primary_blue_hover();
    visuals.override_text_color = Some(colors::primary_text());
    visuals.selection.bg_fill = colors::primary_blue();
    visuals.selection.stroke.color = colors::primary_blue();
    visuals.hyperlink_color = colors::primary_blue();
    visuals
}
