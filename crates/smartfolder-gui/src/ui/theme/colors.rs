#![allow(dead_code)]

//! Semantic color tokens for the smartfolder GUI.
//!
//! These helpers keep color use tied to product meaning instead of local RGB
//! choices. The palette follows the RC2 design system and can later gain dark
//! theme equivalents without changing screen code.

use eframe::egui::Color32;

/// Warm app background used behind the main content and shell.
pub(crate) fn app_background() -> Color32 {
    Color32::from_rgb(245, 242, 236)
}

/// Soft content surface used for central panels.
pub(crate) fn surface() -> Color32 {
    Color32::from_rgb(251, 250, 247)
}

/// Elevated card surface.
pub(crate) fn elevated_surface() -> Color32 {
    Color32::WHITE
}

/// Subtle tinted section surface.
pub(crate) fn subtle_surface() -> Color32 {
    Color32::from_rgb(240, 235, 227)
}

/// Soft neutral control fill.
pub(crate) fn soft_control() -> Color32 {
    Color32::from_rgb(248, 246, 242)
}

/// Hovered neutral control fill.
pub(crate) fn hover_control() -> Color32 {
    Color32::from_rgb(239, 244, 252)
}

/// Primary action and focus blue.
pub(crate) fn primary_blue() -> Color32 {
    Color32::from_rgb(47, 128, 237)
}

/// Primary action hover blue.
pub(crate) fn primary_blue_hover() -> Color32 {
    Color32::from_rgb(37, 111, 209)
}

/// Foreground on primary action surfaces.
pub(crate) fn on_primary() -> Color32 {
    Color32::WHITE
}

/// Primary text color.
pub(crate) fn primary_text() -> Color32 {
    Color32::from_rgb(45, 45, 45)
}

/// Secondary explanatory text color.
pub(crate) fn secondary_text() -> Color32 {
    Color32::from_rgb(90, 90, 90)
}

/// Metadata and caption text color.
pub(crate) fn metadata_text() -> Color32 {
    Color32::from_rgb(123, 123, 123)
}

/// Standard border color.
pub(crate) fn border() -> Color32 {
    Color32::from_rgb(216, 210, 200)
}

/// Success semantic color.
pub(crate) fn success() -> Color32 {
    Color32::from_rgb(79, 157, 105)
}

/// Success semantic background.
pub(crate) fn success_bg() -> Color32 {
    Color32::from_rgb(237, 247, 240)
}

/// Warning semantic color.
pub(crate) fn warning() -> Color32 {
    Color32::from_rgb(216, 154, 43)
}

/// Warning semantic background.
pub(crate) fn warning_bg() -> Color32 {
    Color32::from_rgb(255, 246, 231)
}

/// Error semantic color.
pub(crate) fn error() -> Color32 {
    Color32::from_rgb(198, 90, 90)
}

/// Error semantic background.
pub(crate) fn error_bg() -> Color32 {
    Color32::from_rgb(252, 238, 238)
}
