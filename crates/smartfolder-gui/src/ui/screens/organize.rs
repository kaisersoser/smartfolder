//! Organize workflow presentation helpers.

use eframe::egui::{self, RichText};

use crate::{
    is_cloud_synced_path, plural,
    ui::{
        self,
        components::{safety_line as render_safety_line, truncated_label},
        screens::preview::{
            preview_aligned_content_width, render_preview_metric_card, render_preview_summary_line,
            untouched_reason_label,
        },
    },
    AnalysisOutput, AnalysisProgress, ApplyOutput, ApplyProgress, HistoryAction,
};

pub(crate) fn render_analysis_progress(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    progress: &AnalysisProgress,
) {
    ui.label(RichText::new(&progress.headline).strong());
    let fraction = progress
        .fraction
        .unwrap_or_else(|| ((ctx.input(|input| input.time) * 0.35) as f32).fract());
    let progress_width = ui.available_width().max(120.0);
    ui.add(
        egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
            .desired_width(progress_width)
            .text(progress_text(progress)),
    );
    truncated_label(ui, &progress.detail);
    ui.add_space(6.0);
    egui::CollapsingHeader::new("Analysis details")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("analysis-progress-grid")
                .num_columns(4)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    progress_stat(ui, "Entries", progress.entries_seen);
                    progress_stat(ui, "Records", progress.records_collected);
                    ui.end_row();
                    progress_stat(ui, "Folders", progress.folders_scanned);
                    progress_stat(ui, "Warnings", progress.warnings);
                    ui.end_row();
                    progress_stat(ui, "Skipped entries", progress.entries_skipped);
                    progress_stat(ui, "Planned", progress.moves_proposed);
                    ui.end_row();
                    progress_stat(ui, "Needs review", progress.ambiguous_files);
                    progress_stat(ui, "Conflicts", progress.conflicts);
                    ui.end_row();
                    progress_stat(ui, "Skipped plan items", progress.skipped);
                    ui.end_row();
                });
        });
}

fn progress_text(progress: &AnalysisProgress) -> String {
    match progress.fraction {
        Some(fraction) if progress.plan_total > 0 => format!(
            "{} of {} records ({:.0}%)",
            progress.plan_processed,
            progress.plan_total,
            fraction * 100.0
        ),
        Some(fraction) => format!("{:.0}%", fraction * 100.0),
        None => "Scanning, total unknown".to_string(),
    }
}

fn progress_stat(ui: &mut egui::Ui, label: &str, value: usize) {
    ui.label(label);
    ui.label(value.to_string());
}

pub(crate) fn render_apply_progress(ui: &mut egui::Ui, progress: &ApplyProgress) {
    ui.label(RichText::new(&progress.headline).strong());
    let fraction = if progress.total == 0 {
        0.0
    } else {
        progress.processed as f32 / progress.total as f32
    };
    let progress_width = ui.available_width().max(120.0);
    ui.add(
        egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
            .desired_width(progress_width)
            .text(format!(
                "{} of {} moves ({:.0}%)",
                progress.processed,
                progress.total,
                fraction * 100.0
            )),
    );
    truncated_label(ui, &progress.detail);
    ui.add_space(6.0);
    egui::CollapsingHeader::new("Progress details")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("apply-progress-grid")
                .num_columns(4)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    progress_stat(ui, "Completed", progress.completed);
                    progress_stat(ui, "Skipped", progress.skipped);
                    ui.end_row();
                    progress_stat(ui, "Failed", progress.failed);
                    progress_stat(
                        ui,
                        "Remaining",
                        progress.total.saturating_sub(progress.processed),
                    );
                    ui.end_row();
                });
        });
}

pub(crate) fn render_plan_summary(ui: &mut egui::Ui, result: &AnalysisOutput, compact: bool) {
    let ready = result.preview_counts.ready;
    let needs_attention = result.preview_counts.needs_attention;
    let left_in_place = if result.preview_counts.untouched > 0 {
        result.preview_counts.untouched
    } else {
        result.summary.ambiguous_files + result.summary.skipped
    };
    let aligned_width = preview_aligned_content_width(ui);

    ui.scope(|ui| {
        ui.set_max_width(aligned_width);
        ui.label(
            RichText::new("Analysis summary")
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        if !compact {
            ui.add(
                egui::Label::new(
                    RichText::new(plan_summary_headline(result))
                        .color(ui::theme::colors::primary_text()),
                )
                .wrap(),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(plan_summary_detail(ready, needs_attention, left_in_place))
                        .color(ui::theme::colors::primary_text()),
                )
                .wrap(),
            );
            ui.add_space(8.0);
        } else {
            ui.add_space(6.0);
        }

        render_preview_summary_panel(ui, result, ready, needs_attention, left_in_place);

        if !compact {
            ui.add_space(8.0);
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "Scanned {} files. {} warning{} recorded.",
                        result.summary.files_scanned,
                        result.warning_messages.len(),
                        plural(result.warning_messages.len())
                    ))
                    .color(ui::theme::colors::primary_text()),
                )
                .wrap(),
            );

            if needs_attention == 0 && result.warning_messages.is_empty() {
                ui.colored_label(
                    ui::theme::colors::success(),
                    "No conflicts or warnings found.",
                );
                if left_in_place > 0 {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "{left_in_place} untouched item{} will stay put unless reviewed in the detailed file list.",
                                plural(left_in_place)
                            ))
                            .color(ui::theme::colors::secondary_text()),
                        )
                        .wrap(),
                    );
                }
            } else if needs_attention > 0 || !result.warning_messages.is_empty() {
                ui.colored_label(
                    ui::theme::colors::warning(),
                    "Review the attention and warnings views before organizing these files.",
                );
            }
        }
    });
}

fn render_preview_summary_panel(
    ui: &mut egui::Ui,
    result: &AnalysisOutput,
    ready: usize,
    needs_attention: usize,
    left_in_place: usize,
) {
    ui::theme::widgets::surface_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        render_preview_summary_line(
            ui,
            ui::theme::colors::success(),
            &format!("{ready} file{} ready to move", plural(ready)),
        );
        let review_line = if needs_attention == 0 {
            "No planned moves need review".to_string()
        } else {
            format!(
                "{needs_attention} planned move{} need review",
                plural(needs_attention)
            )
        };
        render_preview_summary_line(
            ui,
            if needs_attention == 0 {
                ui::theme::colors::success()
            } else {
                ui::theme::colors::warning()
            },
            &review_line,
        );
        render_preview_summary_line(
            ui,
            ui::theme::colors::metadata_text(),
            &format!(
                "{left_in_place} item{} will stay untouched",
                plural(left_in_place)
            ),
        );

        if !result.untouched_reason_counts.is_empty() {
            ui.add_space(ui::theme::spacing::SM);
            ui.label(
                RichText::new("Untouched reasons")
                    .size(ui::theme::typography::CAPTION)
                    .strong()
                    .color(ui::theme::colors::metadata_text()),
            );
            for (reason, count) in &result.untouched_reason_counts {
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(ui::theme::spacing::MD);
                    ui::theme::widgets::status_dot(ui, ui::theme::colors::metadata_text());
                    ui.label(
                        RichText::new(format!("{} {}", count, untouched_reason_label(*reason)))
                            .size(ui::theme::typography::CAPTION)
                            .color(ui::theme::colors::secondary_text()),
                    );
                });
            }
        }
    });
}

pub(crate) fn render_apply_entry(
    ui: &mut egui::Ui,
    result: &AnalysisOutput,
    is_applying: bool,
    already_applied: bool,
    show_confirmation: &mut bool,
) {
    let ready = result.preview_counts.ready;
    let can_apply = ready > 0 && !is_applying && !already_applied;
    let aligned_width = preview_aligned_content_width(ui);
    ui.scope(|ui| {
        ui.set_max_width(aligned_width);
        egui::Frame::group(ui.style())
            .fill(ui::theme::colors::elevated_surface())
            .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                let button_label = organize_files_label(ready);
                let button_width = 180.0;
                let column_gap = 12.0;
                let text_width = (ui.available_width() - button_width - column_gap).max(220.0);

                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.label(
                                RichText::new("Ready to organize")
                                    .strong()
                                    .size(ui::theme::typography::CARD_TITLE)
                                    .color(ui::theme::colors::heading_text()),
                            );
                            if already_applied {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            "This preview has already been organized. Run Analyze Folder again for a fresh preview.",
                                        )
                                        .color(ui::theme::colors::secondary_text()),
                                    )
                                    .wrap(),
                                );
                            } else if ready == 0 {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new("No safe moves are ready to organize.")
                                            .color(ui::theme::colors::secondary_text()),
                                    )
                                    .wrap(),
                                );
                            } else {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            "smartfolder moves the ready items and leaves review or untouched items in place.",
                                        )
                                        .color(ui::theme::colors::primary_text()),
                                    )
                                    .wrap(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(
                                            "Restore history is recorded before any files move.",
                                        )
                                        .color(ui::theme::colors::secondary_text()),
                                    )
                                    .wrap(),
                                );
                            }
                        },
                    );
                    ui.add_space(column_gap);
                    ui.allocate_ui_with_layout(
                        egui::vec2(button_width, 0.0),
                        egui::Layout::top_down(egui::Align::Max),
                        |ui| {
                            let response = ui.add_enabled(
                                can_apply,
                                egui::Button::new(
                                    RichText::new(button_label)
                                        .strong()
                                        .color(ui::theme::colors::on_primary()),
                                )
                                .fill(ui::theme::colors::primary_blue())
                                .min_size(egui::vec2(
                                    button_width,
                                    ui::theme::spacing::MIN_TARGET,
                                )),
                            );
                            if response.clicked() {
                                *show_confirmation = true;
                            }
                        },
                    );
                });
            });
    });
}

pub(crate) fn organize_files_label(ready: usize) -> String {
    format!("Organize {ready} file{}", plural(ready))
}

pub(crate) fn render_apply_confirmation(
    ctx: &egui::Context,
    result: &AnalysisOutput,
    confirmed: &mut bool,
    dismissed: &mut bool,
) {
    egui::Window::new(
        RichText::new("Confirm organization")
            .strong()
            .color(ui::theme::colors::heading_text()),
    )
    .collapsible(false)
    .resizable(false)
    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
    .default_width(560.0)
    .show(ctx, |ui| {
        let content_width = 520.0;
        ui.set_min_width(content_width);
        ui.set_max_width(content_width);

        ui.label(
            RichText::new("Ready to organize these files")
                .strong()
                .size(ui::theme::typography::SECTION_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "smartfolder will move {} ready file{} into organized folders and leave review items untouched.",
                    result.preview_counts.ready,
                    plural(result.preview_counts.ready)
                ))
                .color(ui::theme::colors::primary_text()),
            )
            .wrap(),
        );
        ui.add_space(10.0);

        let card_gap = ui.spacing().item_spacing.x;
        let card_width = ((content_width - card_gap) / 2.0).max(220.0);
        ui.horizontal(|ui| {
            render_preview_metric_card(
                ui,
                card_width,
                "Ready",
                result.preview_counts.ready,
                "Moves now",
                ui::theme::colors::success_bg(),
                ui::theme::colors::success(),
            );
            render_preview_metric_card(
                ui,
                card_width,
                "Review",
                result.preview_counts.needs_attention,
                "Stays put",
                ui::theme::colors::warning_bg(),
                ui::theme::colors::warning(),
            );
        });

        ui.add_space(10.0);
        render_safety_line(ui, "Existing files will not be overwritten.");
        render_safety_line(ui, "Restore history is recorded before files move.");
        render_safety_line(
            ui,
            "Restore previous layout will be available after completion.",
        );

        if is_cloud_synced_path(&result.root) {
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "This folder appears to be cloud-synced. Let sync settle before organizing and review the completion summary afterward.",
                    )
                    .color(ui::theme::colors::warning()),
                )
                .wrap(),
            );
        }

        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(ui::theme::widgets::primary_button(organize_files_label(
                    result.preview_counts.ready,
                )))
                .clicked()
            {
                *confirmed = true;
            }
            if ui
                .add(ui::theme::widgets::secondary_button("Cancel"))
                .clicked()
            {
                *dismissed = true;
            }
        });
    });
}

pub(crate) fn render_apply_result(
    ui: &mut egui::Ui,
    result: &ApplyOutput,
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    let aligned_width = preview_aligned_content_width(ui);
    ui.scope(|ui| {
        ui.set_max_width(aligned_width);
        egui::Frame::group(ui.style())
            .fill(ui::theme::colors::elevated_surface())
            .stroke(egui::Stroke::new(1.0, ui::theme::colors::success()))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Organization complete")
                        .strong()
                        .size(ui::theme::typography::SECTION_TITLE)
                        .color(ui::theme::colors::heading_text()),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{} file{} moved successfully.",
                            result.completed,
                            plural(result.completed)
                        ))
                        .color(ui::theme::colors::primary_text()),
                    )
                    .wrap(),
                );
                ui.add_space(10.0);

                ui::theme::widgets::surface_frame().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    render_preview_summary_line(
                        ui,
                        ui::theme::colors::success(),
                        &format!(
                            "{} file{} moved successfully",
                            result.completed,
                            plural(result.completed)
                        ),
                    );
                    render_preview_summary_line(
                        ui,
                        if result.skipped == 0 && result.failed == 0 {
                            ui::theme::colors::success()
                        } else {
                            ui::theme::colors::warning()
                        },
                        &format!("{} skipped · {} failed", result.skipped, result.failed),
                    );
                    render_preview_summary_line(
                        ui,
                        ui::theme::colors::success(),
                        "Restore point saved",
                    );
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy, ui::theme::widgets::primary_button("Done"))
                        .clicked()
                    {
                        *action = Some(HistoryAction::CompleteApply);
                    }
                    if ui
                        .add_enabled(
                            !busy,
                            ui::theme::widgets::secondary_button("Restore previous layout"),
                        )
                        .clicked()
                    {
                        *action = Some(HistoryAction::ConfirmUndo(result.transaction_id.clone()));
                    }
                    if ui
                        .add_enabled(!busy, ui::theme::widgets::tertiary_button("View details"))
                        .clicked()
                    {
                        *action = Some(HistoryAction::ViewDetails(result.transaction_id.clone()));
                    }
                });
                ui.add_space(8.0);
                egui::CollapsingHeader::new("Technical details")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("Activity id: {}", result.transaction_id))
                                .color(ui::theme::colors::primary_text()),
                        );
                        truncated_label(
                            ui,
                            &format!("Restore history: {}", result.journal_path.display()),
                        );
                    });
            });
    });
}

fn plan_summary_headline(result: &AnalysisOutput) -> String {
    if result.preview_counts.ready == 0 {
        return "No safe moves are ready to organize.".to_string();
    }

    if result.preview_counts.needs_attention == 0 && result.preview_counts.untouched == 0 {
        format!(
            "{} safe file{} ready to organize.",
            result.preview_counts.ready,
            plural(result.preview_counts.ready)
        )
    } else if result.preview_counts.needs_attention == 0 {
        format!(
            "{} safe file{} ready, with {} item{} left untouched.",
            result.preview_counts.ready,
            plural(result.preview_counts.ready),
            result.preview_counts.untouched,
            plural(result.preview_counts.untouched)
        )
    } else {
        format!(
            "{} safe file{} ready, with {} planned move{} needing review.",
            result.preview_counts.ready,
            plural(result.preview_counts.ready),
            result.preview_counts.needs_attention,
            plural(result.preview_counts.needs_attention)
        )
    }
}

fn plan_summary_detail(ready: usize, needs_attention: usize, left_in_place: usize) -> String {
    format!(
        "smartfolder can organize {ready} item{} automatically. {needs_attention} planned move{} and {left_in_place} unplanned item{} will stay put unless reviewed.",
        plural(ready),
        plural(needs_attention),
        plural(left_in_place)
    )
}
