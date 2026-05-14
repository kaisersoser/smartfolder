#![allow(dead_code)]

//! Semantic color tokens for the smartfolder GUI.
//!
//! These helpers keep color use tied to product meaning instead of local RGB
//! choices. The palette follows the RC2 design system and resolves against the
//! currently applied visual theme.

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::Color32;

use super::VisualTheme;

static DARK_MODE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy)]
struct Palette {
    app_background: Color32,
    surface: Color32,
    elevated_surface: Color32,
    subtle_surface: Color32,
    soft_control: Color32,
    hover_control: Color32,
    primary_blue: Color32,
    primary_blue_hover: Color32,
    on_primary: Color32,
    primary_text: Color32,
    heading_text: Color32,
    secondary_text: Color32,
    metadata_text: Color32,
    border: Color32,
    success: Color32,
    success_bg: Color32,
    warning: Color32,
    warning_bg: Color32,
    info: Color32,
    info_bg: Color32,
    error: Color32,
    error_bg: Color32,
}

const LIGHT: Palette = Palette {
    app_background: Color32::from_rgb(245, 242, 236),
    surface: Color32::from_rgb(251, 250, 247),
    elevated_surface: Color32::WHITE,
    subtle_surface: Color32::from_rgb(240, 235, 227),
    soft_control: Color32::from_rgb(248, 246, 242),
    hover_control: Color32::from_rgb(239, 244, 252),
    primary_blue: Color32::from_rgb(47, 128, 237),
    primary_blue_hover: Color32::from_rgb(37, 111, 209),
    on_primary: Color32::WHITE,
    primary_text: Color32::from_rgb(45, 45, 45),
    heading_text: Color32::from_rgb(34, 34, 34),
    secondary_text: Color32::from_rgb(90, 90, 90),
    metadata_text: Color32::from_rgb(123, 123, 123),
    border: Color32::from_rgb(216, 210, 200),
    success: Color32::from_rgb(79, 157, 105),
    success_bg: Color32::from_rgb(237, 247, 240),
    warning: Color32::from_rgb(216, 154, 43),
    warning_bg: Color32::from_rgb(255, 246, 231),
    info: Color32::from_rgb(89, 102, 145),
    info_bg: Color32::from_rgb(236, 236, 244),
    error: Color32::from_rgb(198, 90, 90),
    error_bg: Color32::from_rgb(252, 238, 238),
};

const DARK: Palette = Palette {
    app_background: Color32::from_rgb(18, 20, 22),
    surface: Color32::from_rgb(25, 27, 30),
    elevated_surface: Color32::from_rgb(34, 37, 41),
    subtle_surface: Color32::from_rgb(43, 45, 49),
    soft_control: Color32::from_rgb(49, 52, 57),
    hover_control: Color32::from_rgb(43, 57, 78),
    primary_blue: Color32::from_rgb(102, 168, 255),
    primary_blue_hover: Color32::from_rgb(77, 144, 235),
    on_primary: Color32::from_rgb(12, 17, 24),
    primary_text: Color32::from_rgb(229, 232, 236),
    heading_text: Color32::from_rgb(247, 248, 249),
    secondary_text: Color32::from_rgb(187, 193, 201),
    metadata_text: Color32::from_rgb(144, 151, 161),
    border: Color32::from_rgb(75, 79, 87),
    success: Color32::from_rgb(118, 204, 148),
    success_bg: Color32::from_rgb(29, 58, 40),
    warning: Color32::from_rgb(235, 184, 91),
    warning_bg: Color32::from_rgb(70, 50, 24),
    info: Color32::from_rgb(142, 170, 236),
    info_bg: Color32::from_rgb(34, 45, 74),
    error: Color32::from_rgb(234, 126, 126),
    error_bg: Color32::from_rgb(75, 36, 39),
};

/// Update the active palette used by semantic color tokens.
pub(crate) fn set_visual_theme(theme: VisualTheme) {
    DARK_MODE.store(matches!(theme, VisualTheme::Dark), Ordering::Relaxed);
}

fn palette() -> Palette {
    if DARK_MODE.load(Ordering::Relaxed) {
        DARK
    } else {
        LIGHT
    }
}

/// Warm app background used behind the main content and shell.
pub(crate) fn app_background() -> Color32 {
    palette().app_background
}

/// Soft content surface used for central panels.
pub(crate) fn surface() -> Color32 {
    palette().surface
}

/// Elevated card surface.
pub(crate) fn elevated_surface() -> Color32 {
    palette().elevated_surface
}

/// Subtle tinted section surface.
pub(crate) fn subtle_surface() -> Color32 {
    palette().subtle_surface
}

/// Soft neutral control fill.
pub(crate) fn soft_control() -> Color32 {
    palette().soft_control
}

/// Hovered neutral control fill.
pub(crate) fn hover_control() -> Color32 {
    palette().hover_control
}

/// Primary action and focus blue.
pub(crate) fn primary_blue() -> Color32 {
    palette().primary_blue
}

/// Primary action hover blue.
pub(crate) fn primary_blue_hover() -> Color32 {
    palette().primary_blue_hover
}

/// Foreground on primary action surfaces.
pub(crate) fn on_primary() -> Color32 {
    palette().on_primary
}

/// Primary text color.
pub(crate) fn primary_text() -> Color32 {
    palette().primary_text
}

/// Higher-emphasis text for page titles and key headings.
pub(crate) fn heading_text() -> Color32 {
    palette().heading_text
}

/// Secondary explanatory text color.
pub(crate) fn secondary_text() -> Color32 {
    palette().secondary_text
}

/// Metadata and caption text color.
pub(crate) fn metadata_text() -> Color32 {
    palette().metadata_text
}

/// Standard border color.
pub(crate) fn border() -> Color32 {
    palette().border
}

/// Success semantic color.
pub(crate) fn success() -> Color32 {
    palette().success
}

/// Success semantic background.
pub(crate) fn success_bg() -> Color32 {
    palette().success_bg
}

/// Warning semantic color.
pub(crate) fn warning() -> Color32 {
    palette().warning
}

/// Warning semantic background.
pub(crate) fn warning_bg() -> Color32 {
    palette().warning_bg
}

/// Informational semantic color.
pub(crate) fn info() -> Color32 {
    palette().info
}

/// Informational semantic background.
pub(crate) fn info_bg() -> Color32 {
    palette().info_bg
}

/// Error semantic color.
pub(crate) fn error() -> Color32 {
    palette().error
}

/// Error semantic background.
pub(crate) fn error_bg() -> Color32 {
    palette().error_bg
}
