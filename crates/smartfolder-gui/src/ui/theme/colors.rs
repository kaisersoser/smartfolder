#![allow(dead_code)]

//! Semantic color tokens for the smartfolder GUI.
//!
//! These helpers keep color use tied to product meaning instead of local RGB
//! choices. The palette follows the v2.25 design system and resolves against the
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
    border_subtle: Color32,
    border: Color32,
    border_strong: Color32,
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
    app_background: Color32::from_rgb(245, 247, 250),
    surface: Color32::from_rgb(250, 251, 253),
    elevated_surface: Color32::from_rgb(255, 255, 255),
    subtle_surface: Color32::from_rgb(238, 242, 247),
    soft_control: Color32::from_rgb(242, 245, 249),
    hover_control: Color32::from_rgb(230, 239, 252),
    primary_blue: Color32::from_rgb(53, 126, 229),
    primary_blue_hover: Color32::from_rgb(39, 108, 203),
    on_primary: Color32::WHITE,
    primary_text: Color32::from_rgb(36, 42, 51),
    heading_text: Color32::from_rgb(22, 27, 34),
    secondary_text: Color32::from_rgb(86, 96, 111),
    metadata_text: Color32::from_rgb(119, 130, 146),
    border_subtle: Color32::from_rgb(226, 231, 238),
    border: Color32::from_rgb(209, 217, 228),
    border_strong: Color32::from_rgb(181, 193, 209),
    success: Color32::from_rgb(58, 135, 89),
    success_bg: Color32::from_rgb(230, 245, 235),
    warning: Color32::from_rgb(181, 123, 32),
    warning_bg: Color32::from_rgb(252, 242, 220),
    info: Color32::from_rgb(76, 111, 185),
    info_bg: Color32::from_rgb(231, 238, 251),
    error: Color32::from_rgb(187, 75, 81),
    error_bg: Color32::from_rgb(251, 233, 234),
};

const DARK: Palette = Palette {
    app_background: Color32::from_rgb(15, 17, 21),
    surface: Color32::from_rgb(23, 26, 32),
    elevated_surface: Color32::from_rgb(31, 35, 43),
    subtle_surface: Color32::from_rgb(37, 42, 51),
    soft_control: Color32::from_rgb(42, 48, 58),
    hover_control: Color32::from_rgb(40, 54, 76),
    primary_blue: Color32::from_rgb(106, 168, 255),
    primary_blue_hover: Color32::from_rgb(138, 188, 255),
    on_primary: Color32::from_rgb(7, 17, 31),
    primary_text: Color32::from_rgb(229, 233, 240),
    heading_text: Color32::from_rgb(243, 246, 251),
    secondary_text: Color32::from_rgb(195, 202, 214),
    metadata_text: Color32::from_rgb(143, 151, 165),
    border_subtle: Color32::from_rgba_premultiplied(255, 255, 255, 20),
    border: Color32::from_rgba_premultiplied(255, 255, 255, 34),
    border_strong: Color32::from_rgba_premultiplied(255, 255, 255, 58),
    success: Color32::from_rgb(97, 211, 148),
    success_bg: Color32::from_rgba_premultiplied(97, 211, 148, 32),
    warning: Color32::from_rgb(241, 189, 90),
    warning_bg: Color32::from_rgba_premultiplied(241, 189, 90, 34),
    info: Color32::from_rgb(122, 167, 255),
    info_bg: Color32::from_rgba_premultiplied(122, 167, 255, 34),
    error: Color32::from_rgb(255, 107, 107),
    error_bg: Color32::from_rgba_premultiplied(255, 107, 107, 34),
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

/// App background used behind the main content and shell.
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

/// Low-emphasis border color for quiet separation.
pub(crate) fn border_subtle() -> Color32 {
    palette().border_subtle
}

/// Standard border color.
pub(crate) fn border() -> Color32 {
    palette().border
}

/// High-emphasis border color for focus, selection, and critical states.
pub(crate) fn border_strong() -> Color32 {
    palette().border_strong
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
