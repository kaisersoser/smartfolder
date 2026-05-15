//! Activity and restore presentation helpers.

use std::path::Path;

use eframe::egui::{self, Color32, RichText};
use smartfolder_core::model::TransactionStatus;

use crate::ui;
use crate::ui::components::{status_chip as render_status_chip, truncated_label};
use crate::{TransactionDetail, TransactionRow};

/// Return restrained status colors for Activity chips.
pub(crate) fn activity_status_colors(status: TransactionStatus) -> (Color32, Color32) {
    match status {
        TransactionStatus::Completed => (
            ui::theme::colors::success(),
            ui::theme::colors::success_bg(),
        ),
        TransactionStatus::RolledBack | TransactionStatus::PartiallyRolledBack => {
            (ui::theme::colors::info(), ui::theme::colors::info_bg())
        }
        TransactionStatus::Failed | TransactionStatus::Interrupted => (
            ui::theme::colors::warning(),
            ui::theme::colors::warning_bg(),
        ),
        TransactionStatus::InProgress => (
            ui::theme::colors::metadata_text(),
            ui::theme::colors::subtle_surface(),
        ),
    }
}

/// Build the primary Activity row title.
pub(crate) fn activity_event_title(row: &TransactionRow) -> String {
    let folder = folder_name_label(&row.root);
    match row.status {
        TransactionStatus::Completed => format!(
            "Organized {} file{} in {folder}",
            row.completed,
            plural(row.completed)
        ),
        TransactionStatus::RolledBack => format!(
            "Restored previous layout in {folder}: {} file{} restored",
            row.rolled_back,
            plural(row.rolled_back)
        ),
        TransactionStatus::PartiallyRolledBack => format!(
            "Partially restored previous layout in {folder}: {} file{} restored",
            row.rolled_back,
            plural(row.rolled_back)
        ),
        TransactionStatus::Interrupted => format!(
            "Organization interrupted in {folder} after {} recorded change{}",
            row.total_operations,
            plural(row.total_operations)
        ),
        TransactionStatus::InProgress => format!(
            "Organization running in {folder}: {} recorded change{}",
            row.total_operations,
            plural(row.total_operations)
        ),
        TransactionStatus::Failed => format!(
            "Organization needs review in {folder}: {} failed, {} organized",
            row.failed, row.completed
        ),
    }
}

/// Extract the date portion from an Activity row timestamp.
pub(crate) fn activity_date_label(row: &TransactionRow) -> &str {
    row.started_at
        .split_once(' ')
        .map_or(row.started_at.as_str(), |(date, _)| date)
}

/// Extract an HH:MM time label from an Activity row timestamp.
pub(crate) fn activity_time_label(row: &TransactionRow) -> &str {
    row.started_at
        .split_once(' ')
        .map_or("", |(_, time)| time)
        .get(0..5)
        .unwrap_or("")
}

/// Build the leading sentence for the Activity detail window.
pub(crate) fn activity_detail_headline(detail: &TransactionDetail) -> String {
    match detail.status {
        TransactionStatus::Completed => format!(
            "Organized {} file{} on {}.",
            detail.operation_counts.completed,
            plural(detail.operation_counts.completed),
            detail.started_at
        ),
        TransactionStatus::RolledBack => format!(
            "Restored {} file{} from this activity.",
            detail.operation_counts.rolled_back,
            plural(detail.operation_counts.rolled_back)
        ),
        TransactionStatus::PartiallyRolledBack => format!(
            "Restored {} file{} with some changes still needing review.",
            detail.operation_counts.rolled_back,
            plural(detail.operation_counts.rolled_back)
        ),
        TransactionStatus::Interrupted => format!(
            "Organization was interrupted after {} recorded change{}.",
            detail.total_operations,
            plural(detail.total_operations)
        ),
        TransactionStatus::InProgress => format!(
            "Organization is still in progress with {} recorded change{}.",
            detail.total_operations,
            plural(detail.total_operations)
        ),
        TransactionStatus::Failed => format!(
            "Organization needs review: {} failed, {} completed.",
            detail.operation_counts.failed, detail.operation_counts.completed
        ),
    }
}

/// Build secondary Activity row copy.
pub(crate) fn activity_detail(row: &TransactionRow) -> String {
    match row.status {
        TransactionStatus::Completed => format!(
            "Why: {}. Completed {}. {} item{} skipped or failed.",
            row.reason_summary,
            row.started_at,
            row.skipped + row.failed,
            plural(row.skipped + row.failed)
        ),
        TransactionStatus::RolledBack => format!(
            "Restore completed {}. Original file locations were restored where possible.",
            row.started_at
        ),
        TransactionStatus::PartiallyRolledBack => {
            "Restore completed with remaining issues. Review details before making more changes."
                .to_string()
        }
        TransactionStatus::Interrupted | TransactionStatus::InProgress => format!(
            "Why: {}. {} completed, {} pending. Resume or restore from the recovery controls.",
            row.reason_summary, row.completed, row.pending
        ),
        TransactionStatus::Failed => {
            "Some file moves failed. Review details before retrying or restoring.".to_string()
        }
    }
}

/// Summarize Activity operation counts for dense rows.
pub(crate) fn activity_count_summary(row: &TransactionRow) -> String {
    format!(
        "{} moved / {} restored / {} needs review",
        row.completed,
        row.rolled_back,
        row.skipped + row.failed + row.pending
    )
}

/// Render compact moved/restored/review count chips.
pub(crate) fn render_activity_count_chips(
    ui: &mut egui::Ui,
    completed: usize,
    rolled_back: usize,
    needs_review: usize,
) {
    ui.horizontal_wrapped(|ui| {
        render_status_chip(
            ui,
            &format!("Moved {completed}"),
            ui::theme::colors::success(),
            ui::theme::colors::success_bg(),
        );
        render_status_chip(
            ui,
            &format!("Restored {rolled_back}"),
            ui::theme::colors::info(),
            ui::theme::colors::info_bg(),
        );
        render_status_chip(
            ui,
            &format!("Review {needs_review}"),
            ui::theme::colors::warning(),
            ui::theme::colors::warning_bg(),
        );
    });
}

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

fn folder_name_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn plural(value: usize) -> &'static str {
    if value == 1 {
        ""
    } else {
        "s"
    }
}
