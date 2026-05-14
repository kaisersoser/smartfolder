#![allow(dead_code)]

//! Shared widget styling helpers for the smartfolder GUI.
//!
//! These helpers provide consistent button, status, and surface primitives while
//! the existing screens are migrated from local styling into reusable components.

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
    .min_size(egui::vec2(144.0, spacing::CONTROL_HEIGHT))
}

/// Build a secondary action button with the design-system minimum target size.
pub(crate) fn secondary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.into()).color(colors::primary_text()))
        .fill(colors::soft_control())
        .stroke(egui::Stroke::new(1.0, colors::border_subtle()))
        .min_size(egui::vec2(112.0, spacing::CONTROL_HEIGHT))
}

/// Build a tertiary text-like action button for low-priority actions.
pub(crate) fn tertiary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.into()).color(colors::secondary_text()))
        .fill(egui::Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE)
        .min_size(egui::vec2(88.0, spacing::COMPACT_CONTROL_HEIGHT))
}

/// Build a danger action button for destructive or irreversible actions.
pub(crate) fn danger_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.into()).color(colors::error()).strong())
        .fill(colors::error_bg())
        .stroke(egui::Stroke::new(1.0, colors::error()))
        .min_size(egui::vec2(128.0, spacing::CONTROL_HEIGHT))
}

/// Build a compact primary action button for dense toolbars.
pub(crate) fn compact_primary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(label.into())
            .color(colors::on_primary())
            .strong(),
    )
    .fill(colors::primary_blue())
    .min_size(egui::vec2(88.0, spacing::COMPACT_CONTROL_HEIGHT))
}

/// Build a compact secondary action button for dense toolbars.
pub(crate) fn compact_secondary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.into()).color(colors::primary_text()))
        .fill(colors::soft_control())
        .stroke(egui::Stroke::new(1.0, colors::border_subtle()))
        .min_size(egui::vec2(72.0, spacing::COMPACT_CONTROL_HEIGHT))
}

/// Build a compact tertiary action button for dense toolbars.
pub(crate) fn compact_tertiary_button(label: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.into()).color(colors::secondary_text()))
        .fill(egui::Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE)
        .min_size(egui::vec2(64.0, spacing::COMPACT_CONTROL_HEIGHT))
}

/// Build a square icon button.
pub(crate) fn icon_button(icon: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(icon.into())
            .size(16.0)
            .color(colors::primary_text()),
    )
    .fill(colors::soft_control())
    .stroke(egui::Stroke::new(1.0, colors::border_subtle()))
    .min_size(egui::vec2(
        spacing::COMPACT_CONTROL_HEIGHT,
        spacing::COMPACT_CONTROL_HEIGHT,
    ))
}

/// Return a standard card frame for elevated content.
pub(crate) fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(colors::elevated_surface())
        .stroke(egui::Stroke::new(1.0, colors::border_subtle()))
        .rounding(egui::Rounding::same(spacing::RADIUS_LG))
        .inner_margin(egui::Margin::same(spacing::LG))
}

/// Return a quiet surface frame for grouped content with minimal elevation.
pub(crate) fn surface_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(colors::surface())
        .stroke(egui::Stroke::new(1.0, colors::border_subtle()))
        .rounding(egui::Rounding::same(spacing::RADIUS_MD))
        .inner_margin(egui::Margin::same(spacing::LG))
}

/// Return a subtle frame for callouts, hints, and secondary groups.
pub(crate) fn subtle_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(colors::subtle_surface())
        .stroke(egui::Stroke::new(1.0, colors::border_subtle()))
        .rounding(egui::Rounding::same(spacing::RADIUS_MD))
        .inner_margin(egui::Margin::same(spacing::MD))
}

/// Render a compact status pill with a small semantic dot.
pub(crate) fn status_pill(ui: &mut egui::Ui, label: &str, color: egui::Color32) {
    egui::Frame::none()
        .fill(colors::soft_control())
        .stroke(egui::Stroke::new(1.0, colors::border_subtle()))
        .rounding(egui::Rounding::same(spacing::RADIUS_PILL))
        .inner_margin(egui::Margin::symmetric(spacing::SM, spacing::XS))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, color);
                ui.label(
                    RichText::new(label)
                        .size(super::typography::CAPTION)
                        .color(colors::secondary_text()),
                );
            });
        });
}

/// Render a small semantic status dot.
pub(crate) fn status_dot(ui: &mut egui::Ui, color: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().circle_filled(rect.center(), 3.5, color);
    }
    response
}
