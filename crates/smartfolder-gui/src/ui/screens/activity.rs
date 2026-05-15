//! Activity and restore presentation helpers.

use std::path::Path;

use eframe::egui::{self, RichText};

use crate::ui;
use crate::ui::components::truncated_label;

/// Render the restore confirmation dialog for a completed activity.
pub(crate) fn render_restore_confirmation(
    ctx: &egui::Context,
    transaction_id: &str,
    confirmed: &mut bool,
    dismissed: &mut bool,
) {
    egui::Window::new("Restore previous layout")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new("Restore previous layout?").strong());
            ui.label("smartfolder will move completed files back to their original paths.");
            ui.label("It will refuse to overwrite anything already at an original path.");
            ui.add_space(ui::theme::spacing::XS);
            egui::CollapsingHeader::new("Technical details")
                .default_open(false)
                .show(ui, |ui| {
                    truncated_label(ui, &format!("Activity id: {transaction_id}"));
                });
            ui.add_space(ui::theme::spacing::SM);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    *dismissed = true;
                }
                if ui.button("Restore previous layout").clicked() {
                    *confirmed = true;
                }
            });
        });
}

/// Render the result summary for a completed restore operation.
pub(crate) fn render_restore_result(
    ui: &mut egui::Ui,
    rolled_back: usize,
    skipped: usize,
    failed: usize,
    transaction_id: &str,
    journal_path: &Path,
) {
    ui.label(RichText::new("Previous layout restored").strong());
    egui::Grid::new("restore-result")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Restored", rolled_back);
            summary_row(ui, "Skipped", skipped);
            summary_row(ui, "Failed", failed);
        });
    egui::CollapsingHeader::new("Technical details")
        .default_open(false)
        .show(ui, |ui| {
            ui.label(format!("Activity id: {transaction_id}"));
            truncated_label(ui, &format!("Restore history: {}", journal_path.display()));
        });
}

fn summary_row(ui: &mut egui::Ui, label: &str, value: usize) {
    ui.label(label);
    ui.label(value.to_string());
    ui.end_row();
}
