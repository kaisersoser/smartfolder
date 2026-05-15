//! Preview row data shaping and presentation helpers.

use std::path::Path;

use eframe::egui::Color32;
use smartfolder_core::model::{ConflictState, PlanOperation, UntouchedReason, UntouchedRecord};

use crate::ui;
use crate::PreviewRow;

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
