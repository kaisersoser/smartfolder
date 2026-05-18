//! Preview row data shaping and presentation helpers.

use std::{collections::BTreeMap, path::Path};

use eframe::egui::{self, Color32, RichText};
use smartfolder_core::{
    ai::{AiConfidence, AiFinding, AiFolderAnalysis},
    model::{ConflictState, PlanOperation, UntouchedReason, UntouchedRecord},
};

use crate::{
    plural,
    ui::{
        self,
        components::{status_chip as render_status_chip, truncated_label},
    },
    AnalysisOutput, PreviewAction, PreviewFilter, PreviewRow,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AiPreviewAction {
    Analyze,
    ViewNotes,
    DraftRules,
    Cancel,
}

/// Build display rows for planned operations.
pub(crate) fn preview_rows(operations: &[PlanOperation], root: &Path) -> Vec<PreviewRow> {
    operations
        .iter()
        .map(|operation| PreviewRow {
            file_name: file_name_label(&operation.source),
            original_folder: relative_folder_label(&operation.source, root),
            target_folder: relative_folder_label(&operation.destination, root),
            source_full_path: operation.source.display().to_string(),
            destination_full_path: Some(operation.destination.display().to_string()),
            reason: operation.reason.clone(),
            status: operation_status(operation).to_string(),
        })
        .collect()
}

/// Build display rows for files left untouched by the current plan.
pub(crate) fn untouched_preview_rows(records: &[UntouchedRecord], root: &Path) -> Vec<PreviewRow> {
    records
        .iter()
        .map(|record| PreviewRow {
            file_name: file_name_label(&record.path),
            original_folder: relative_folder_label(&record.path, root),
            target_folder: "Stays in place".to_string(),
            source_full_path: record.path.display().to_string(),
            destination_full_path: None,
            reason: untouched_detail(record),
            status: "Untouched".to_string(),
        })
        .collect()
}

/// Label an untouched reason for user-facing preview breakdowns.
pub(crate) fn untouched_reason_label(reason: UntouchedReason) -> &'static str {
    match reason {
        UntouchedReason::NoMatchingRule => "no matching rule",
        UntouchedReason::AlreadyOrganized => "already organized",
        UntouchedReason::UnsupportedMetadata => "unsupported metadata",
        UntouchedReason::UnsafeDestination => "unsafe destination",
        UntouchedReason::DestinationConflict => "destination conflict",
        UntouchedReason::ExcludedByPolicy => "excluded by policy",
    }
}

/// Format the highlighted destination preview path for a row.
pub(crate) fn preview_example_destination_path(row: &PreviewRow) -> String {
    if row.destination_full_path.is_none() {
        return "Stays in place".to_string();
    }
    if row.target_folder == "Selected folder" {
        "./".to_string()
    } else {
        format!("./{}/", row.target_folder.replace(" / ", "/"))
    }
}

/// Return restrained status colors for preview rows.
pub(crate) fn preview_status_colors(status: &str) -> (Color32, Color32) {
    match status {
        "Ready" => (
            ui::theme::colors::success(),
            ui::theme::colors::success_bg(),
        ),
        "Needs Review" => (
            ui::theme::colors::warning(),
            ui::theme::colors::warning_bg(),
        ),
        _ => (
            ui::theme::colors::metadata_text(),
            ui::theme::colors::subtle_surface(),
        ),
    }
}

/// Explain why a preview row is ready, needs review, or stays untouched.
pub(crate) fn preview_row_explanation(row: &PreviewRow) -> String {
    match row.status.as_str() {
        "Ready" => format!(
            "This file matched '{}'. smartfolder found a destination inside the selected folder and no conflict was detected, so it can be organized after confirmation.",
            row.reason
        ),
        "Needs Review" => format!(
            "This file matched '{}', but the destination needs attention before smartfolder will move it. It stays untouched unless the conflict is resolved.",
            row.reason
        ),
        "Untouched" => format!(
            "This file stays in place because {}. It will not be moved by the current plan.",
            row.reason
        ),
        _ => format!(
            "This row is marked '{}'. Review the source and destination before organizing.",
            row.status
        ),
    }
}

/// Render one count tile in the organization confirmation dialog.
pub(crate) fn render_preview_metric_card(
    ui: &mut egui::Ui,
    width: f32,
    title: &str,
    value: usize,
    detail: &str,
    fill: Color32,
    stroke: Color32,
) {
    ui.allocate_ui(egui::vec2(width, 76.0), |ui| {
        egui::Frame::group(ui.style())
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, stroke))
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                ui.set_width(width - 16.0);
                ui.label(
                    RichText::new(title)
                        .strong()
                        .size(ui::theme::typography::BODY)
                        .color(ui::theme::colors::heading_text()),
                );
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(value.to_string())
                            .size(18.0)
                            .strong()
                            .color(stroke),
                    );
                    ui.label(
                        RichText::new(detail)
                            .size(ui::theme::typography::CAPTION)
                            .color(ui::theme::colors::secondary_text()),
                    );
                });
            });
    });
}

/// Width used by preview-led panels so the organize screen keeps one rhythm.
pub(crate) fn preview_aligned_content_width(ui: &egui::Ui) -> f32 {
    ui.available_width().min(ui::theme::spacing::FLOW_MAX_WIDTH)
}

/// Render representative rows on the main organize screen.
pub(crate) fn render_preview_examples(ui: &mut egui::Ui, result: &AnalysisOutput) {
    let aligned_width = preview_aligned_content_width(ui);

    ui.scope(|ui| {
        ui.set_max_width(aligned_width);
        ui.label(
            RichText::new("Planned changes sample")
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        ui.add(
            egui::Label::new(
                RichText::new("Representative ready moves from this preview.")
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
        ui.add_space(6.0);

        if result.preview_examples.is_empty() {
            egui::Frame::group(ui.style())
                .fill(ui::theme::colors::surface())
                .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
                .inner_margin(egui::Margin::same(14.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("No ready examples yet")
                            .strong()
                            .color(ui::theme::colors::heading_text()),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "Files with unclear destinations were left untouched. Open the detailed list to inspect them.",
                            )
                            .color(ui::theme::colors::secondary_text()),
                        )
                        .wrap(),
                    );
                });
            return;
        }

        render_preview_sample_table(ui, &result.preview_examples);
        let hidden_examples = result.preview_examples.len().saturating_sub(PREVIEW_SAMPLE_ROWS);
        if hidden_examples > 0 {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{hidden_examples} more sample{} available in the detailed file list.",
                    plural(hidden_examples)
                ))
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
            );
        }
    });
}

/// Render a short colored summary row.
pub(crate) fn render_preview_summary_line(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui::theme::widgets::status_dot(ui, color);
        ui.label(RichText::new(text).color(ui::theme::colors::primary_text()));
    });
}

/// Render detailed-preview filter and paging controls.
pub(crate) fn render_preview_controls(
    ui: &mut egui::Ui,
    result: &AnalysisOutput,
    active_filter: PreviewFilter,
    offset: usize,
    action: &mut Option<PreviewAction>,
) {
    ui.label(
        RichText::new("File list")
            .strong()
            .size(ui::theme::typography::CARD_TITLE)
            .color(ui::theme::colors::heading_text()),
    );
    ui.add(
        egui::Label::new(
            RichText::new(
                "Click a file to inspect its original folder, exact destination, and rule details below.",
            )
            .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.horizontal(|ui| {
        for filter in [
            PreviewFilter::All,
            PreviewFilter::Ready,
            PreviewFilter::Untouched,
            PreviewFilter::NeedsAttention,
        ] {
            let label = format!(
                "{} ({})",
                filter.label(),
                filter.count(&result.preview_counts)
            );
            if ui
                .selectable_label(active_filter == filter, label)
                .clicked()
                && active_filter != filter
            {
                *action = Some(PreviewAction::Filter(filter));
            }
        }
    });

    let total_rows = active_filter.count(&result.preview_counts);
    let current_end = (offset + result.preview_rows.len()).min(total_rows);
    ui.horizontal(|ui| {
        let range_text = if total_rows == 0 {
            "No files in this view".to_string()
        } else {
            format!("Showing {}-{} of {}", offset + 1, current_end, total_rows)
        };
        ui.label(RichText::new(range_text).color(ui::theme::colors::primary_text()));

        if ui
            .add_enabled(offset > 0, egui::Button::new("Previous"))
            .clicked()
        {
            *action = Some(PreviewAction::Previous);
        }

        if ui
            .add_enabled(current_end < total_rows, egui::Button::new("Next"))
            .clicked()
        {
            *action = Some(PreviewAction::Next);
        }
    });
}

/// Render the current preview page as selectable rows.
pub(crate) fn render_preview_rows(
    ui: &mut egui::Ui,
    result: &AnalysisOutput,
    selected_row: &mut Option<usize>,
) {
    if result.preview_rows.is_empty() {
        *selected_row = None;
        ui.label("No files match this view.");
        return;
    }

    if selected_row
        .map(|index| index >= result.preview_rows.len())
        .unwrap_or(true)
    {
        *selected_row = Some(0);
    }

    let width = ui.available_width().max(360.0);
    let (file_width, target_width) = preview_table_column_widths(width);

    egui::Grid::new("preview-rows")
        .num_columns(2)
        .striped(true)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for (index, row) in result.preview_rows.iter().enumerate() {
                let is_selected = *selected_row == Some(index);
                let file_response = preview_selectable_cell_with_tooltip(
                    ui,
                    if is_selected {
                        format!("> {}", row.file_name)
                    } else {
                        row.file_name.clone()
                    },
                    file_width,
                    is_selected,
                    format!(
                        "From: {}\nTo: {}",
                        row.source_full_path,
                        row.destination_full_path
                            .as_deref()
                            .unwrap_or("Stays in place")
                    ),
                );
                if file_response.clicked() {
                    *selected_row = Some(index);
                }
                preview_destination_cell_with_tooltip(
                    ui,
                    row,
                    target_width,
                    row.destination_full_path.as_ref().map_or_else(
                        || "This file stays in its original folder".to_string(),
                        |destination| format!("Full destination: {destination}"),
                    ),
                );
                ui.end_row();
            }
        });

    if result.preview_rows.len() < result.preview_total_rows {
        ui.add_space(8.0);
        ui.label(format!(
            "Showing {} of {} matching files. More rows are stored on disk for paged retrieval.",
            result.preview_rows.len(),
            result.preview_total_rows
        ));
    }
}

/// Render the current preview page grouped by destination folder.
pub(crate) fn render_preview_tree(ui: &mut egui::Ui, result: &AnalysisOutput) {
    if result.preview_rows.is_empty() {
        ui.label("No files match this view.");
        return;
    }

    ui.add(
        egui::Label::new(
            RichText::new(
                "Tree view groups the current page by destination folder while preserving paged loading for large folders.",
            )
            .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add_space(ui::theme::spacing::SM);

    let mut folders: BTreeMap<&str, Vec<&PreviewRow>> = BTreeMap::new();
    for row in &result.preview_rows {
        folders
            .entry(row.target_folder.as_str())
            .or_default()
            .push(row);
    }

    for (folder, rows) in folders {
        let header = format!("{folder} ({})", rows.len());
        egui::CollapsingHeader::new(header)
            .default_open(true)
            .show(ui, |ui| {
                for row in rows {
                    ui.horizontal_wrapped(|ui| {
                        let (stroke, fill) = preview_status_colors(&row.status);
                        render_status_chip(ui, &row.status, stroke, fill);
                        ui.label(
                            RichText::new(&row.file_name)
                                .strong()
                                .color(ui::theme::colors::heading_text()),
                        );
                        ui.label(
                            RichText::new(format!("from {}", row.original_folder))
                                .size(ui::theme::typography::CAPTION)
                                .color(ui::theme::colors::metadata_text()),
                        );
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new(&row.reason)
                                .size(ui::theme::typography::CAPTION)
                                .color(ui::theme::colors::secondary_text()),
                        )
                        .wrap(),
                    );
                    ui.add_space(ui::theme::spacing::XS);
                }
            });
        ui.add_space(ui::theme::spacing::XS);
    }
}

/// Render the two-column preview table header.
pub(crate) fn render_preview_table_header(ui: &mut egui::Ui) {
    let width = ui.available_width().max(360.0);
    let (file_width, target_width) = preview_table_column_widths(width);

    egui::Frame::none()
        .fill(ui::theme::colors::soft_control())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(6.0, 4.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                preview_cell(ui, "File name", file_width, true);
                preview_cell(ui, "Target folder", target_width, true);
            });
        });
}

/// Render details for the selected preview row.
pub(crate) fn render_preview_detail(
    ui: &mut egui::Ui,
    result: &AnalysisOutput,
    selected_row: Option<usize>,
) {
    let Some(index) = selected_row else {
        return;
    };
    let Some(row) = result.preview_rows.get(index) else {
        return;
    };

    ui.label(
        RichText::new("Selected change")
            .strong()
            .size(ui::theme::typography::CARD_TITLE)
            .color(ui::theme::colors::heading_text()),
    );
    egui::Frame::group(ui.style())
        .fill(ui::theme::colors::surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&row.file_name)
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                let (stroke, fill) = preview_status_colors(&row.status);
                render_status_chip(ui, &row.status, stroke, fill);
            });
            ui.add_space(8.0);
            egui::Grid::new("preview-detail-grid")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Original folder").color(ui::theme::colors::metadata_text()),
                    );
                    ui.label(
                        RichText::new(&row.original_folder)
                            .color(ui::theme::colors::primary_text()),
                    );
                    ui.end_row();
                    ui.label(
                        RichText::new("Destination").color(ui::theme::colors::metadata_text()),
                    );
                    render_preview_path_highlight(ui, &preview_example_destination_path(row));
                    ui.end_row();
                    ui.label(RichText::new("Why").color(ui::theme::colors::metadata_text()));
                    ui.label(RichText::new(&row.reason).color(ui::theme::colors::primary_text()));
                    ui.end_row();
                });
            ui.add_space(8.0);
            ui::theme::widgets::surface_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Explanation")
                        .strong()
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::heading_text()),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(preview_row_explanation(row))
                            .color(ui::theme::colors::secondary_text()),
                    )
                    .wrap(),
                );
            });
            ui.add_space(6.0);
            truncated_label(ui, &format!("Full source: {}", row.source_full_path));
            if let Some(destination) = &row.destination_full_path {
                truncated_label(ui, &format!("Full destination: {destination}"));
            } else {
                truncated_label(ui, "Destination: Stays in original folder");
            }
        });
}

fn untouched_detail(record: &UntouchedRecord) -> String {
    if record.detail.trim().is_empty() {
        untouched_reason_label(record.reason).to_string()
    } else {
        record.detail.clone()
    }
}

fn file_name_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn relative_folder_label(path: &Path, root: &Path) -> String {
    let Some(parent) = path.parent() else {
        return "Selected folder".to_string();
    };
    let Ok(relative) = parent.strip_prefix(root) else {
        return parent.display().to_string();
    };
    if relative.as_os_str().is_empty() {
        "Selected folder".to_string()
    } else {
        relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

fn operation_status(operation: &PlanOperation) -> &'static str {
    match operation.conflict {
        ConflictState::None => "Ready",
        ConflictState::DestinationExists { .. } => "Needs Review",
        ConflictState::CaseOnlyRename { .. } => "Needs Review",
        ConflictState::UnsafeDestination { .. } => "Left Untouched",
    }
}

const PREVIEW_SAMPLE_ROWS: usize = 3;

fn render_preview_sample_table(ui: &mut egui::Ui, rows: &[PreviewRow]) {
    egui::Frame::group(ui.style())
        .fill(ui::theme::colors::elevated_surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let width = ui.available_width().max(520.0);
            let file_width = (width * 0.26).max(120.0);
            let destination_width = (width * 0.30).max(150.0);
            let rule_width = (width * 0.26).max(130.0);
            let status_width =
                (width - file_width - destination_width - rule_width - 36.0).max(90.0);

            egui::Grid::new("preview-sample-table")
                .num_columns(4)
                .spacing([12.0, 5.0])
                .show(ui, |ui| {
                    preview_sample_cell(ui, "File", file_width, true);
                    preview_sample_cell(ui, "Destination", destination_width, true);
                    preview_sample_cell(ui, "Rule", rule_width, true);
                    preview_sample_cell(ui, "Status", status_width, true);
                    ui.end_row();

                    for row in rows.iter().take(PREVIEW_SAMPLE_ROWS) {
                        preview_sample_cell(ui, &row.file_name, file_width, false);
                        preview_sample_cell(
                            ui,
                            &preview_example_destination_path(row),
                            destination_width,
                            false,
                        );
                        preview_sample_cell(ui, &row.reason, rule_width, false);
                        let (stroke, fill) = preview_status_colors(&row.status);
                        ui.allocate_ui(egui::vec2(status_width, 22.0), |ui| {
                            render_status_chip(ui, &row.status, stroke, fill);
                        });
                        ui.end_row();
                    }
                });
        });
}

fn preview_sample_cell(ui: &mut egui::Ui, text: &str, width: f32, strong: bool) {
    ui.allocate_ui(egui::vec2(width.max(40.0), 20.0), |ui| {
        let color = if strong {
            ui::theme::colors::metadata_text()
        } else {
            ui::theme::colors::primary_text()
        };
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .monospace()
                    .size(if strong {
                        ui::theme::typography::CAPTION
                    } else {
                        ui::theme::typography::BODY
                    })
                    .color(color),
            )
            .truncate(),
        )
        .on_hover_text(text);
    });
}

fn render_preview_path_highlight(ui: &mut egui::Ui, path: &str) -> egui::Response {
    let frame = egui::Frame::none()
        .fill(ui::theme::colors::hover_control())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::primary_blue()))
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(path)
                        .monospace()
                        .color(ui::theme::colors::primary_blue()),
                )
                .sense(egui::Sense::hover()),
            )
        });
    frame.response.union(frame.inner)
}

fn preview_table_column_widths(width: f32) -> (f32, f32) {
    let file_width = (width * 0.44).max(220.0);
    let target_width = (width - file_width - 18.0).max(260.0);
    (file_width, target_width)
}

fn preview_cell(ui: &mut egui::Ui, text: &str, width: f32, strong: bool) {
    let _ = preview_cell_response(ui, text, width, strong);
}

fn preview_selectable_cell_with_tooltip(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    width: f32,
    selected: bool,
    tooltip: impl Into<egui::WidgetText>,
) -> egui::Response {
    let text = text.into();
    let text_color = if selected {
        ui::theme::colors::on_primary()
    } else {
        ui::theme::colors::primary_text()
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width.max(40.0), 20.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            ui::theme::colors::primary_blue()
        } else if response.hovered() {
            ui::theme::colors::hover_control()
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(2.0), fill);
        ui.painter().text(
            rect.left_center() + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::TextStyle::Monospace.resolve(ui.style()),
            text_color,
        );
    }
    response.on_hover_text(tooltip)
}

fn preview_cell_response(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    strong: bool,
) -> egui::Response {
    let text_color = if strong {
        ui::theme::colors::heading_text()
    } else {
        ui::theme::colors::primary_text()
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width.max(40.0), 18.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().text(
            rect.left_center() + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::TextStyle::Monospace.resolve(ui.style()),
            text_color,
        );
    }
    response
}

fn preview_destination_cell_with_tooltip(
    ui: &mut egui::Ui,
    row: &PreviewRow,
    width: f32,
    tooltip: impl Into<egui::WidgetText>,
) {
    let path = preview_example_destination_path(row);
    let response = ui
        .allocate_ui(egui::vec2(width.max(40.0), 22.0), |ui| {
            render_preview_path_highlight(ui, &path)
        })
        .inner;
    response.on_hover_text(tooltip);
}

pub(crate) fn render_ai_assist_strip(
    ui: &mut egui::Ui,
    analysis: Option<&AiFolderAnalysis>,
    folder_analysis_running: bool,
    busy: bool,
    content_inspection_enabled: bool,
    untouched_count: usize,
) -> Option<AiPreviewAction> {
    let mut action = None;
    let aligned_width = preview_aligned_content_width(ui);
    ui.scope(|ui| {
        ui.set_max_width(aligned_width);
        egui::Frame::group(ui.style())
            .fill(ui::theme::colors::surface())
            .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
            .inner_margin(egui::Margin::symmetric(14.0, 10.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if folder_analysis_running {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("AI insights")
                                .strong()
                                .color(ui::theme::colors::heading_text()),
                        );
                        ui.label(
                            RichText::new("Reviewing the scanned folder context...")
                                .color(ui::theme::colors::secondary_text()),
                        );
                        if ui
                            .add(ui::theme::widgets::compact_secondary_button("Cancel"))
                            .clicked()
                        {
                            action = Some(AiPreviewAction::Cancel);
                        }
                    });
                    return;
                }

                if let Some(analysis) = analysis {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("AI insights ready")
                                .strong()
                                .color(ui::theme::colors::heading_text()),
                        );
                        render_status_chip(
                            ui,
                            ai_confidence_label(analysis.confidence),
                            ui::theme::colors::info(),
                            ui::theme::colors::info_bg(),
                        );
                        render_status_chip(
                            ui,
                            &format!(
                                "{} pattern{}",
                                analysis.patterns.len(),
                                plural(analysis.patterns.len())
                            ),
                            ui::theme::colors::success(),
                            ui::theme::colors::success_bg(),
                        );
                        render_status_chip(
                            ui,
                            &format!(
                                "{} risk{}",
                                analysis.risks.len(),
                                plural(analysis.risks.len())
                            ),
                            ui::theme::colors::warning(),
                            ui::theme::colors::warning_bg(),
                        );
                        if ui
                            .add(ui::theme::widgets::compact_secondary_button(
                                "View insights",
                            ))
                            .clicked()
                        {
                            action = Some(AiPreviewAction::ViewNotes);
                        }
                        if ui
                            .add(ui::theme::widgets::compact_secondary_button(
                                "Create draft rules",
                            ))
                            .clicked()
                        {
                            action = Some(AiPreviewAction::DraftRules);
                        }
                    });
                } else {
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new("AI insights")
                                    .strong()
                                    .color(ui::theme::colors::heading_text()),
                            );
                            render_ai_mode_chip(ui, content_inspection_enabled);
                            if ui
                                .add_enabled(
                                    !busy,
                                    ui::theme::widgets::compact_secondary_button(
                                        "Analyze folder with AI",
                                    ),
                                )
                                .clicked()
                            {
                                action = Some(AiPreviewAction::Analyze);
                            }
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(ai_preview_context_copy(untouched_count))
                                    .color(ui::theme::colors::secondary_text()),
                            )
                            .wrap(),
                        );
                    });
                }
            });
    });
    action
}

pub(crate) fn render_ai_notes_content(
    ui: &mut egui::Ui,
    analysis: &AiFolderAnalysis,
    draft_rules_requested: &mut bool,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("AI insights")
                .strong()
                .size(ui::theme::typography::SECTION_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        render_ai_mode_chip(ui, analysis.content_inspection_used);
        render_status_chip(
            ui,
            ai_confidence_label(analysis.confidence),
            ui::theme::colors::info(),
            ui::theme::colors::info_bg(),
        );
        if analysis.content_inspection_used {
            render_status_chip(
                ui,
                &format!(
                    "{} content sample{}",
                    analysis.content_samples_included,
                    plural(analysis.content_samples_included)
                ),
                ui::theme::colors::secondary_text(),
                ui::theme::colors::elevated_surface(),
            );
        }
    });
    ui.add_space(ui::theme::spacing::SM);
    ui.add(
        egui::Label::new(RichText::new(&analysis.summary).color(ui::theme::colors::primary_text()))
            .wrap(),
    );
    ui.add(
        egui::Label::new(
            RichText::new(&analysis.recommended_strategy)
                .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add_space(ui::theme::spacing::SM);
    if ui
        .add(ui::theme::widgets::primary_button("Create draft rules"))
        .clicked()
    {
        *draft_rules_requested = true;
    }
    ui.add_space(ui::theme::spacing::MD);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !analysis.content_sample_warnings.is_empty() {
                ui.label(
                    RichText::new("Content sampling notes")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                for warning in analysis.content_sample_warnings.iter().take(6) {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("- {warning}"))
                                .color(ui::theme::colors::secondary_text()),
                        )
                        .wrap(),
                    );
                }
                if analysis.content_sample_warnings.len() > 6 {
                    ui.label(
                        RichText::new(format!(
                            "{} more sampling note{} omitted.",
                            analysis.content_sample_warnings.len() - 6,
                            plural(analysis.content_sample_warnings.len() - 6)
                        ))
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::metadata_text()),
                    );
                }
                ui.add_space(ui::theme::spacing::SM);
            }
            if !analysis.patterns.is_empty() {
                ui.label(
                    RichText::new("Patterns")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                for finding in &analysis.patterns {
                    render_ai_finding(ui, finding);
                }
            }
            if !analysis.risks.is_empty() {
                ui.add_space(ui::theme::spacing::SM);
                ui.label(
                    RichText::new("Risks")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                for finding in &analysis.risks {
                    render_ai_finding(ui, finding);
                }
            }
        });
}

fn render_ai_mode_chip(ui: &mut egui::Ui, content_inspection_enabled: bool) {
    render_status_chip(
        ui,
        if content_inspection_enabled {
            "Content-aware"
        } else {
            "Metadata only"
        },
        ui::theme::colors::info(),
        ui::theme::colors::info_bg(),
    );
}

fn ai_preview_context_copy(untouched_count: usize) -> String {
    let base =
        "Optional folder-wide review of scanned items. The deterministic preview remains authoritative.";
    if untouched_count == 0 {
        base.to_string()
    } else {
        format!(
            "{base} It can also explain why {untouched_count} item{} did not match a rule.",
            plural(untouched_count)
        )
    }
}

fn render_ai_finding(ui: &mut egui::Ui, finding: &AiFinding) {
    ui.add_space(ui::theme::spacing::XS);
    ui.label(
        RichText::new(&finding.title)
            .strong()
            .color(ui::theme::colors::primary_text()),
    );
    ui.add(
        egui::Label::new(RichText::new(&finding.detail).color(ui::theme::colors::secondary_text()))
            .wrap(),
    );
    if !finding.examples.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Examples")
                    .size(ui::theme::typography::CAPTION)
                    .color(ui::theme::colors::metadata_text()),
            );
            for item in finding.examples.iter().take(4) {
                render_status_chip(
                    ui,
                    item,
                    ui::theme::colors::metadata_text(),
                    ui::theme::colors::subtle_surface(),
                );
            }
        });
    }
}

fn ai_confidence_label(confidence: AiConfidence) -> &'static str {
    match confidence {
        AiConfidence::Low => "Low confidence",
        AiConfidence::Medium => "Medium confidence",
        AiConfidence::High => "High confidence",
    }
}
