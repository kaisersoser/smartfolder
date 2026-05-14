#![allow(dead_code)]

//! Shared widget styling helpers for the smartfolder GUI.
//!
//! These helpers provide
//! consistent button/card primitives while the existing screens are migrated
//! from local styling into reusable components.

use eframe::egui::{self, RichText};

use super::{colors, spacing};

/// Build a primary action button with the design-system minimum target size.
pub(crate) fn primary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(label.into())
            .color(colors::on_primary())
            .strong(),
    )
    .fill(colors::primary_blue())
    .min_size(egui::vec2(160.0, spacing::MIN_TARGET))
}

/// Build a secondary action button with the design-system minimum target size.
pub(crate) fn secondary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.into()).color(colors::primary_text()))
        .fill(colors::soft_control())
        .stroke(egui::Stroke::new(1.0, colors::border()))
        .min_size(egui::vec2(120.0, spacing::MIN_TARGET))
}

/// Build a compact primary action button for dense toolbars.
pub(crate) fn compact_primary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(label.into())
            .color(colors::on_primary())
            .strong(),
    )
    .fill(colors::primary_blue())
    .min_size(egui::vec2(88.0, 32.0))
}

/// Build a compact secondary action button for dense toolbars.
pub(crate) fn compact_secondary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.into()).color(colors::primary_text()))
        .fill(colors::soft_control())
        .stroke(egui::Stroke::new(1.0, colors::border()))
        .min_size(egui::vec2(72.0, 32.0))
}

/// Return a standard card frame for elevated content.
pub(crate) fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(colors::elevated_surface())
        .stroke(egui::Stroke::new(1.0, colors::border()))
        .rounding(egui::Rounding::same(14.0))
        .inner_margin(egui::Margin::same(spacing::XL))
}
