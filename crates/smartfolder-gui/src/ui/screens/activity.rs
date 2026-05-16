//! Activity and restore presentation helpers.

use std::path::Path;

use eframe::egui::{self, Color32, RichText};
use smartfolder_core::model::{OperationStatus, TransactionStatus};

use crate::ui;
use crate::ui::components::{
    note_frame as settings_note_frame, status_chip as render_status_chip, truncated_label,
};
use crate::{HistoryAction, TransactionDetail, TransactionOperationRow, TransactionRow};

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

/// Render the active Activity scope bar above the timeline.
pub(crate) fn render_activity_scope_bar(
    ui: &mut egui::Ui,
    scope: &str,
    status: &str,
    action: &mut Option<HistoryAction>,
) {
    settings_note_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(scope)
                    .strong()
                    .size(ui::theme::typography::CARD_TITLE)
                    .color(ui::theme::colors::heading_text()),
            );
            render_status_chip(
                ui,
                status,
                ui::theme::colors::secondary_text(),
                ui::theme::colors::surface(),
            );
            if ui
                .add(ui::theme::widgets::tertiary_button("Change folder"))
                .clicked()
            {
                *action = Some(HistoryAction::ChangeFolder);
            }
        });
    });
}

/// Render the scoped Activity timeline for the currently selected folder.
pub(crate) fn render_current_folder_activity(
    ui: &mut egui::Ui,
    rows: &[&TransactionRow],
    hidden_count: usize,
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    if rows.is_empty() {
        ui.add(
            egui::Label::new(
                RichText::new("No activity has been recorded for this folder yet.")
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
        if hidden_count > 0 {
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{} activit{} from other folders hidden.",
                        hidden_count,
                        plural_y(hidden_count)
                    ))
                    .color(ui::theme::colors::metadata_text()),
                )
                .wrap(),
            );
        }
        return;
    }

    ui.label(
        RichText::new("Latest activity")
            .strong()
            .size(ui::theme::typography::CARD_TITLE)
            .color(ui::theme::colors::heading_text()),
    );
    render_activity_record_row(ui, rows[0], busy, true, action);

    if rows.len() > 1 {
        ui.add_space(ui::theme::spacing::MD);
        let mut current_date = "";
        for row in rows.iter().skip(1).take(8) {
            let date = activity_date_label(row);
            if date != current_date {
                current_date = date;
                ui.add_space(ui::theme::spacing::XS);
                ui.label(
                    RichText::new(date)
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
            }
            render_activity_record_row(ui, row, busy, false, action);
            ui.add_space(ui::theme::spacing::XS);
        }
    }

    if hidden_count > 0 {
        ui.add_space(ui::theme::spacing::SM);
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "{} activit{} from other folders hidden from this overview.",
                    hidden_count,
                    plural_y(hidden_count)
                ))
                .color(ui::theme::colors::metadata_text()),
            )
            .wrap(),
        );
    }
}

/// Render the advanced restore history log.
pub(crate) fn render_recovery_log(
    ui: &mut egui::Ui,
    rows: &[TransactionRow],
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Restore history")
                .strong()
                .color(ui::theme::colors::heading_text()),
        );
        ui.label(
            RichText::new("Recent recovery journals")
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
        );
    });
    if rows.is_empty() {
        ui.add(
            egui::Label::new(
                RichText::new("No restore history has been recorded yet.")
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(420.0)
        .show(ui, |ui| {
            for row in rows.iter().take(10) {
                render_recovery_log_row(ui, row, busy, action);
                ui.add_space(ui::theme::spacing::XS);
            }
        });

    if rows.len() > 10 {
        ui.label(
            RichText::new(format!("Showing 10 of {} recovery journals.", rows.len()))
                .color(ui::theme::colors::metadata_text()),
        );
    }
}

/// Render an Activity detail loading/error window.
pub(crate) fn render_activity_detail_error_window(
    ctx: &egui::Context,
    message: &str,
    action: &mut Option<HistoryAction>,
) {
    let mut open = true;
    egui::Window::new("Activity details")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.colored_label(ui::theme::colors::error(), message);
            ui.add_space(ui::theme::spacing::SM);
            if ui
                .add(ui::theme::widgets::secondary_button("Close"))
                .clicked()
            {
                *action = Some(HistoryAction::CloseDetails);
            }
        });
    if !open {
        *action = Some(HistoryAction::CloseDetails);
    }
}

/// Render the detailed Activity window.
pub(crate) fn render_activity_detail_window(
    ctx: &egui::Context,
    detail: &TransactionDetail,
    action: &mut Option<HistoryAction>,
) {
    let mut open = true;
    egui::Window::new("Activity details")
        .open(&mut open)
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .default_height(560.0)
        .show(ctx, |ui| {
            render_activity_detail_content(ui, detail, action);
        });
    if !open {
        *action = Some(HistoryAction::CloseDetails);
    }
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

fn render_activity_record_row(
    ui: &mut egui::Ui,
    row: &TransactionRow,
    busy: bool,
    featured: bool,
    action: &mut Option<HistoryAction>,
) {
    let can_restore = can_restore_status(row.status);
    let row_fill = ui::theme::colors::elevated_surface();
    egui::Frame::group(ui.style())
        .fill(row_fill)
        .stroke(egui::Stroke::new(
            1.0,
            if featured && can_restore {
                ui::theme::colors::success()
            } else {
                ui::theme::colors::border()
            },
        ))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(ui::theme::spacing::SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let actions_width = 220.0_f32.min(ui.available_width() * 0.34);
            let time_width = 64.0;
            let text_width = (ui.available_width()
                - actions_width
                - time_width
                - (ui::theme::spacing::MD * 2.0))
                .max(280.0);

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(time_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.label(
                            RichText::new(activity_time_label(row))
                                .monospace()
                                .color(ui::theme::colors::metadata_text()),
                        );
                        if featured && can_restore {
                            ui.label(
                                RichText::new("Restore ready")
                                    .size(ui::theme::typography::CAPTION)
                                    .color(ui::theme::colors::success()),
                            );
                        }
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(text_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let (stroke, fill) = activity_status_colors(row.status);
                            ui::theme::widgets::status_dot(ui, stroke);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(activity_event_title(row))
                                        .strong()
                                        .color(ui::theme::colors::heading_text()),
                                )
                                .wrap(),
                            );
                            render_status_chip(ui, activity_status_label(row.status), stroke, fill);
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(activity_detail(row))
                                    .color(ui::theme::colors::secondary_text()),
                            )
                            .wrap(),
                        );
                        ui.add_space(ui::theme::spacing::XS);
                        render_activity_count_chips(
                            ui,
                            row.completed,
                            row.rolled_back,
                            row.skipped + row.failed + row.pending,
                        );
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!busy, ui::theme::widgets::secondary_button("Details"))
                        .clicked()
                    {
                        *action = Some(HistoryAction::ViewDetails(row.transaction_id.clone()));
                    }
                    let can_restore_now = can_restore && !busy;
                    if ui
                        .add_enabled(
                            can_restore_now,
                            ui::theme::widgets::secondary_button("Restore"),
                        )
                        .on_disabled_hover_text(
                            "Only completed or failed activities can be restored",
                        )
                        .clicked()
                    {
                        *action = Some(HistoryAction::ConfirmUndo(row.transaction_id.clone()));
                    }
                });
            });
        });
}

fn render_recovery_log_row(
    ui: &mut egui::Ui,
    row: &TransactionRow,
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    egui::Frame::group(ui.style())
        .fill(ui::theme::colors::surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(ui::theme::spacing::SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let available = ui.available_width();
            let action_width = 132.0_f32.min(available * 0.28);
            let content_width = (available - action_width - ui::theme::spacing::MD).max(200.0);

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(content_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(activity_event_title(row))
                                        .strong()
                                        .color(ui::theme::colors::heading_text()),
                                )
                                .wrap(),
                            )
                            .on_hover_text(format!("Activity id: {}", row.transaction_id));

                            let (stroke, fill) = activity_status_colors(row.status);
                            render_status_chip(ui, activity_status_label(row.status), stroke, fill);
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(activity_count_summary(row))
                                    .color(ui::theme::colors::secondary_text()),
                            )
                            .wrap(),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("Root: {}", row.root_label))
                                    .size(ui::theme::typography::CAPTION)
                                    .color(ui::theme::colors::metadata_text()),
                            )
                            .truncate(),
                        )
                        .on_hover_text(row.root.display().to_string());
                    },
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(action_width, 0.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui
                            .add_enabled(!busy, ui::theme::widgets::secondary_button("Details"))
                            .clicked()
                        {
                            *action = Some(HistoryAction::ViewDetails(row.transaction_id.clone()));
                        }
                    },
                );
            });
        });
}

fn render_activity_detail_content(
    ui: &mut egui::Ui,
    detail: &TransactionDetail,
    action: &mut Option<HistoryAction>,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Activity details")
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        let (stroke, fill) = activity_status_colors(detail.status);
        render_status_chip(ui, activity_status_label(detail.status), stroke, fill);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(ui::theme::widgets::secondary_button("Close"))
                .clicked()
            {
                *action = Some(HistoryAction::CloseDetails);
            }
        });
    });
    ui.add_space(ui::theme::spacing::SM);

    ui.add(
        egui::Label::new(
            RichText::new(activity_detail_headline(detail))
                .color(ui::theme::colors::primary_text()),
        )
        .wrap(),
    );
    ui.add(
        egui::Label::new(
            RichText::new(format!("Why: {}", detail.reason_summary))
                .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add(
        egui::Label::new(
            RichText::new(format!("Folder: {}", detail.root))
                .color(ui::theme::colors::metadata_text()),
        )
        .truncate(),
    )
    .on_hover_text(&detail.root);

    ui.add_space(ui::theme::spacing::SM);
    render_activity_count_chips(
        ui,
        detail.operation_counts.completed,
        detail.operation_counts.rolled_back,
        detail.operation_counts.skipped
            + detail.operation_counts.failed
            + detail.operation_counts.pending,
    );

    ui.add_space(ui::theme::spacing::MD);
    egui::CollapsingHeader::new("Technical restore details")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("transaction-detail-summary")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Activity id");
                    ui.label(&detail.transaction_id);
                    ui.end_row();
                    ui.label("Plan id");
                    ui.label(&detail.plan_id);
                    ui.end_row();
                    ui.label("Started");
                    ui.label(&detail.started_at);
                    ui.end_row();
                    ui.label("Completed");
                    ui.label(&detail.completed_at);
                    ui.end_row();
                    ui.label("Recorded changes");
                    ui.label(detail.total_operations.to_string());
                    ui.end_row();
                });
            ui.add_space(ui::theme::spacing::SM);
            render_transaction_operation_rows(ui, detail);
        });
}

fn render_transaction_operation_rows(ui: &mut egui::Ui, detail: &TransactionDetail) {
    if detail.operation_rows.is_empty() {
        ui.label("No operation rows recorded in this journal.");
        return;
    }

    ui.label(RichText::new("Recorded changes").strong());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(260.0)
        .show(ui, |ui| {
            for row in &detail.operation_rows {
                render_transaction_operation_row(ui, row);
                ui.add_space(ui::theme::spacing::XS);
            }
        });

    if detail.total_operations > detail.operation_rows.len() {
        ui.label(format!(
            "Showing {} of {} recorded operations.",
            detail.operation_rows.len(),
            detail.total_operations
        ));
    }
}

fn render_transaction_operation_row(ui: &mut egui::Ui, row: &TransactionOperationRow) {
    egui::Frame::group(ui.style())
        .fill(ui::theme::colors::surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(ui::theme::spacing::SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(&row.operation_id)
                        .monospace()
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                render_status_chip(
                    ui,
                    operation_status_label(row.status),
                    ui::theme::colors::secondary_text(),
                    ui::theme::colors::elevated_surface(),
                );
                ui.label(
                    RichText::new(&row.reason)
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::metadata_text()),
                );
            });
            render_path_detail_line(ui, "From", &row.source);
            render_path_detail_line(ui, "To", &row.destination);
            if !row.error.trim().is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Error: {}", row.error))
                            .color(ui::theme::colors::error()),
                    )
                    .wrap(),
                );
            }
        });
}

fn render_path_detail_line(ui: &mut egui::Ui, label: &str, path: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [44.0, 20.0],
            egui::Label::new(
                RichText::new(label)
                    .size(ui::theme::typography::CAPTION)
                    .color(ui::theme::colors::metadata_text()),
            ),
        );
        let width = ui.available_width().max(120.0);
        ui.add_sized(
            [width, 20.0],
            egui::Label::new(
                RichText::new(path)
                    .monospace()
                    .color(ui::theme::colors::primary_text()),
            )
            .truncate(),
        )
        .on_hover_text(path);
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

fn plural_y(value: usize) -> &'static str {
    if value == 1 {
        "y"
    } else {
        "ies"
    }
}

fn activity_status_label(status: TransactionStatus) -> &'static str {
    match status {
        TransactionStatus::InProgress => "in progress",
        TransactionStatus::Completed => "completed",
        TransactionStatus::Interrupted => "interrupted",
        TransactionStatus::RolledBack => "rolled back",
        TransactionStatus::PartiallyRolledBack => "partially rolled back",
        TransactionStatus::Failed => "failed",
    }
}

fn can_restore_status(status: TransactionStatus) -> bool {
    matches!(
        status,
        TransactionStatus::Completed
            | TransactionStatus::Failed
            | TransactionStatus::PartiallyRolledBack
    )
}

fn operation_status_label(status: OperationStatus) -> &'static str {
    match status {
        OperationStatus::Pending => "pending",
        OperationStatus::Completed => "completed",
        OperationStatus::Skipped => "skipped",
        OperationStatus::Failed => "failed",
        OperationStatus::RolledBack => "rolled back",
    }
}
