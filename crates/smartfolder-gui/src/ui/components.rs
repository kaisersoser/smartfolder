//! Shared egui components used across app screens.

use eframe::egui::{self, Color32, RichText};

use crate::ui::theme;

pub(crate) fn status_chip(ui: &mut egui::Ui, text: &str, stroke: Color32, fill: Color32) {
    egui::Frame::group(ui.style())
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(10.0, 4.0))
        .show(ui, |ui| {
            ui.set_min_width(0.0);
            ui.label(RichText::new(text).strong().color(stroke));
        });
}

pub(crate) fn muted_status_chip(ui: &mut egui::Ui, text: &str) {
    status_chip(
        ui,
        text,
        theme::colors::secondary_text(),
        theme::colors::subtle_surface(),
    );
}
