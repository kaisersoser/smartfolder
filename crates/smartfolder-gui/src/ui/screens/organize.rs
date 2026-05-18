//! Organize workflow presentation helpers.

use eframe::egui::{self, Color32, RichText};

use crate::{
    is_cloud_synced_path, plural,
    ui::{
        self,
        components::{
            safety_line as render_safety_line, status_chip as render_status_chip, truncated_label,
        },
        screens::preview::{
            preview_aligned_content_width, render_preview_metric_card, render_preview_summary_line,
            untouched_reason_label,
        },
    },
    AnalysisOutput, AnalysisProgress, ApplyOutput, ApplyProgress, HistoryAction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OrganizeNavAction {
    Back,
    Continue,
    Reanalyze,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstructionDetailAction {
    ImportProfile,
    OpenRules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum OrganizeStep {
    Folder,
    Style,
    Preview,
    Organize,
}

impl OrganizeStep {
    const ALL: [Self; 4] = [Self::Folder, Self::Style, Self::Preview, Self::Organize];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Folder => "Folder",
            Self::Style => "Instructions",
            Self::Preview => "Preview",
            Self::Organize => "Organize",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Folder => "Choose the root folder",
            Self::Style => "Choose organization rules",
            Self::Preview => "Review example changes",
            Self::Organize => "Confirm and keep restore available",
        }
    }

    fn number(self) -> usize {
        match self {
            Self::Folder => 1,
            Self::Style => 2,
            Self::Preview => 3,
            Self::Organize => 4,
        }
    }

    pub(crate) fn previous(self) -> Option<Self> {
        match self {
            Self::Folder => None,
            Self::Style => Some(Self::Folder),
            Self::Preview => Some(Self::Style),
            Self::Organize => Some(Self::Preview),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstructionPreset {
    ByType,
    ByDate,
    ByExtension,
    TypeAndDate,
    CustomRules,
}

impl InstructionPreset {
    const ALL: [Self; 5] = [
        Self::ByType,
        Self::ByDate,
        Self::ByExtension,
        Self::TypeAndDate,
        Self::CustomRules,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::ByType => "By Type",
            Self::ByDate => "By Date",
            Self::ByExtension => "By Extension",
            Self::TypeAndDate => "Type + Date",
            Self::CustomRules => "Custom Rules",
        }
    }

    pub(crate) fn example_destination(self) -> &'static str {
        match self {
            Self::ByType => "Images",
            Self::ByDate => "2026/05/13",
            Self::ByExtension => "pdf",
            Self::TypeAndDate => "Images/2026/05/13",
            Self::CustomRules => "Documents/PDFs",
        }
    }

    pub(crate) fn example_file_name(self) -> &'static str {
        match self {
            Self::ByType => "beach-sunset.jpg",
            Self::ByDate => "meeting-notes.docx",
            Self::ByExtension => "project-spec.pdf",
            Self::TypeAndDate => "beach-sunset.jpg",
            Self::CustomRules => "invoice-042.pdf",
        }
    }

    pub(crate) fn secondary_example_file_name(self) -> &'static str {
        match self {
            Self::ByType => "screenshot.png",
            Self::ByDate => "budget-review.xlsx",
            Self::ByExtension => "invoice-042.pdf",
            Self::TypeAndDate => "class-photo.jpg",
            Self::CustomRules => "receipt-1042.pdf",
        }
    }

    pub(crate) fn detail(self) -> &'static str {
        match self {
            Self::ByType => {
                "Group related file types into broad folders that are easy to scan later."
            }
            Self::ByDate => {
                "Sort files by when they were last modified so recent work stays together."
            }
            Self::ByExtension => {
                "Separate files by exact extension when the file format matters more than the category."
            }
            Self::TypeAndDate => {
                "Keep similar file types together, then add date folders inside each type."
            }
            Self::CustomRules => {
                "Apply a saved rule profile when one folder needs more specific destinations than the built-in options provide."
            }
        }
    }

    pub(crate) fn note(self) -> &'static str {
        match self {
            Self::ByType => "Good default for mixed folders.",
            Self::ByDate => "Good for inboxes, downloads, and dated work.",
            Self::ByExtension => "Good when file formats need strict separation.",
            Self::TypeAndDate => "Best when you want both category and time structure.",
            Self::CustomRules => "Requires an imported or saved rule profile.",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ExampleTreeEntry {
    depth: usize,
    label: String,
    is_folder: bool,
    is_last: bool,
    ancestor_has_next: Vec<bool>,
}

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

pub(crate) fn render_instruction_picker(
    ui: &mut egui::Ui,
    selected_instruction: InstructionPreset,
    requested_instruction: &mut Option<InstructionPreset>,
) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new("Available instructions")
                .strong()
                .color(ui::theme::colors::heading_text()),
        );
        ui.add_space(6.0);
        for preset in InstructionPreset::ALL {
            let selected = selected_instruction == preset;
            let response = render_instruction_list_row(ui, preset.title(), selected);
            if response.clicked() {
                *requested_instruction = Some(preset);
            }
        }
    });
}

pub(crate) fn render_instruction_detail_panel(
    ui: &mut egui::Ui,
    preset: InstructionPreset,
    example_entries: &[ExampleTreeEntry],
    loaded_profile_label: Option<&str>,
    action: &mut Option<InstructionDetailAction>,
) {
    ui.vertical(|ui| {
        ui.label(
            RichText::new(preset.title())
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        ui.add(
            egui::Label::new(
                RichText::new(preset.detail())
                    .size(ui::theme::typography::BODY)
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );

        ui.add_space(8.0);
        ui.label(
            RichText::new("Example result")
                .strong()
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::heading_text()),
        );
        egui::Frame::group(ui.style())
            .fill(ui::theme::colors::soft_control())
            .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
            .inner_margin(egui::Margin::same(10.0))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    render_instruction_example_tree(ui, example_entries);
                    ui.add_space(4.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(preset.note())
                                .size(ui::theme::typography::CAPTION)
                                .color(ui::theme::colors::secondary_text()),
                        )
                        .wrap(),
                    );
                });
            });

        if preset == InstructionPreset::CustomRules {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Profile status")
                    .strong()
                    .color(ui::theme::colors::heading_text()),
            );
            if let Some(label) = loaded_profile_label {
                render_status_chip(
                    ui,
                    "Profile loaded",
                    ui::theme::colors::success(),
                    ui::theme::colors::success_bg(),
                );
                ui.label(format!("Using {label}."));
            } else {
                render_status_chip(
                    ui,
                    "Profile needed",
                    ui::theme::colors::warning(),
                    ui::theme::colors::warning_bg(),
                );
                ui.label("Import a profile before previewing with Custom Rules.");
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(ui::theme::widgets::secondary_button("Import profile..."))
                    .clicked()
                {
                    *action = Some(InstructionDetailAction::ImportProfile);
                }
                if ui
                    .add(ui::theme::widgets::secondary_button("Open Rules"))
                    .clicked()
                {
                    *action = Some(InstructionDetailAction::OpenRules);
                }
            });
        }
    });
}

fn render_instruction_list_row(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let row_size = egui::vec2(ui.available_width(), 26.0);
    let (rect, response) = ui.allocate_exact_size(row_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let fill = if selected {
            ui::theme::colors::primary_blue()
        } else if response.hovered() {
            ui::theme::colors::hover_control()
        } else {
            ui::theme::colors::surface()
        };
        let text_color = if selected {
            ui::theme::colors::on_primary()
        } else {
            ui::theme::colors::primary_text()
        };
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(2.0), fill);
        ui.painter().text(
            rect.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            text_color,
        );
    }

    response
}

pub(crate) fn render_organize_step_indicator(
    ui: &mut egui::Ui,
    current: OrganizeStep,
    furthest: OrganizeStep,
    requested_step: &mut Option<OrganizeStep>,
) {
    ui.horizontal_wrapped(|ui| {
        for (index, step) in OrganizeStep::ALL.iter().copied().enumerate() {
            if index > 0 {
                ui.label(
                    RichText::new("->")
                        .size(ui::theme::typography::CONTROL)
                        .color(ui::theme::colors::metadata_text()),
                );
            }

            let is_current = step == current;
            let is_available = step <= furthest;
            let text_color = if is_current {
                ui::theme::colors::primary_blue()
            } else if is_available {
                ui::theme::colors::primary_text()
            } else {
                ui::theme::colors::metadata_text()
            };
            let fill = if is_current {
                ui::theme::colors::hover_control()
            } else {
                Color32::TRANSPARENT
            };
            let stroke = if is_current {
                egui::Stroke::new(1.0, ui::theme::colors::primary_blue())
            } else {
                egui::Stroke::NONE
            };
            let label = format!("{} {}", step.number(), step.title());
            let button = egui::Button::new(
                RichText::new(label)
                    .size(ui::theme::typography::CONTROL)
                    .strong()
                    .color(text_color),
            )
            .fill(fill)
            .stroke(stroke)
            .min_size(egui::vec2(88.0, ui::theme::spacing::COMPACT_CONTROL_HEIGHT));

            let response = ui
                .add_enabled(is_available, button)
                .on_hover_text(step.subtitle());
            if response.clicked() {
                *requested_step = Some(step);
            }
        }
    });
}

pub(crate) fn render_organize_step_controls(
    ui: &mut egui::Ui,
    current: OrganizeStep,
    has_root: bool,
    can_run_analysis: bool,
    analysis_result: Option<&AnalysisOutput>,
    busy: bool,
    nav_action: &mut Option<OrganizeNavAction>,
) {
    let mut helper_text = None;

    ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            if current.previous().is_some()
                && ui
                    .add_enabled(!busy, ui::theme::widgets::secondary_button("Back"))
                    .clicked()
            {
                *nav_action = Some(OrganizeNavAction::Back);
            }

            match current {
                OrganizeStep::Folder => {
                    if ui
                        .add_enabled(
                            has_root && !busy,
                            ui::theme::widgets::primary_button("Continue"),
                        )
                        .clicked()
                    {
                        *nav_action = Some(OrganizeNavAction::Continue);
                    }
                    helper_text = Some("Choose the folder first. Nothing moves during this step.");
                }
                OrganizeStep::Style => {
                    if ui
                        .add_enabled(
                            can_run_analysis && !busy,
                            ui::theme::widgets::primary_button("Preview Changes"),
                        )
                        .clicked()
                    {
                        *nav_action = Some(OrganizeNavAction::Continue);
                    }
                    helper_text = Some(
                        "Preview Changes analyzes this folder using the selected instructions and opens the example preview.",
                    );
                }
                OrganizeStep::Preview => {
                    let ready = analysis_result.map_or(0, |result| result.preview_counts.ready);
                    if ui
                        .add_enabled(
                            can_run_analysis && !busy,
                            ui::theme::widgets::secondary_button("Re-analyze"),
                        )
                        .clicked()
                    {
                        *nav_action = Some(OrganizeNavAction::Reanalyze);
                    }
                    if ui
                        .add_enabled(
                            ready > 0 && !busy,
                            ui::theme::widgets::primary_button(organize_files_label(ready)),
                        )
                        .clicked()
                    {
                        *nav_action = Some(OrganizeNavAction::Continue);
                    }
                    if ready == 0 {
                        helper_text = Some(
                            "No safe moves are ready yet. Review the preview details or choose another style.",
                        );
                    }
                }
                OrganizeStep::Organize => {
                    helper_text = Some(
                        "Confirm only when the preview matches what you expect. Restore remains available afterward.",
                    );
                }
            }
        });

        if let Some(helper_text) = helper_text {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(helper_text)
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::secondary_text()),
                )
                .wrap(),
            );
        }
    });
}

pub(crate) fn render_folder_status_light(ui: &mut egui::Ui, has_root: bool, preselected: bool) {
    let (color, label, detail) = if has_root {
        (
            ui::theme::colors::success(),
            "Folder ready",
            if preselected {
                "This folder was preselected from launch. Nothing has been analyzed or moved yet."
            } else {
                "This folder is selected. Nothing has been analyzed or moved yet."
            },
        )
    } else {
        (
            ui::theme::colors::warning(),
            "Folder needed",
            "Choose or drop a folder before continuing.",
        )
    };

    let response = ui
        .add(
            egui::Label::new(RichText::new("●").size(18.0).color(color))
                .sense(egui::Sense::click()),
        )
        .on_hover_ui(|ui| {
            ui.label(
                RichText::new(label)
                    .strong()
                    .color(ui::theme::colors::heading_text()),
            );
            ui.label(RichText::new(detail).color(ui::theme::colors::secondary_text()));
        });

    if response.clicked() {
        response.request_focus();
    }
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

pub(crate) fn build_example_tree_entries(
    root_folder: &str,
    destination: &str,
    file_name: &str,
    secondary_file_name: &str,
) -> Vec<ExampleTreeEntry> {
    let normalized_destination = destination.replace('\\', "/");
    let segments: Vec<&str> = normalized_destination
        .split('/')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect();

    let mut entries = vec![ExampleTreeEntry {
        depth: 0,
        label: format!("./{root_folder}"),
        is_folder: true,
        is_last: false,
        ancestor_has_next: Vec::new(),
    }];

    for (index, segment) in segments.iter().enumerate() {
        entries.push(ExampleTreeEntry {
            depth: index + 1,
            label: (*segment).to_string(),
            is_folder: true,
            is_last: false,
            ancestor_has_next: vec![true; index + 1],
        });
    }

    let file_depth = segments.len() + 1;
    entries.push(ExampleTreeEntry {
        depth: file_depth,
        label: file_name.to_string(),
        is_folder: false,
        is_last: false,
        ancestor_has_next: vec![true; file_depth],
    });
    entries.push(ExampleTreeEntry {
        depth: file_depth,
        label: secondary_file_name.to_string(),
        is_folder: false,
        is_last: true,
        ancestor_has_next: vec![true; file_depth.saturating_sub(1)],
    });

    entries
}

pub(crate) fn render_instruction_example_tree(ui: &mut egui::Ui, entries: &[ExampleTreeEntry]) {
    let text_color = ui::theme::colors::primary_text();
    let branch_color = ui::theme::colors::metadata_text();
    let tree_text_size = ui::theme::typography::CAPTION;

    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(2.0, 1.0);
        for entry in entries {
            ui.horizontal(|ui| {
                let connector = example_tree_connector(entry);
                if !connector.is_empty() {
                    ui.label(
                        RichText::new(connector)
                            .monospace()
                            .size(tree_text_size)
                            .color(branch_color),
                    );
                }
                if entry.is_folder {
                    paint_example_folder_icon(ui);
                    ui.add_space(2.0);
                }
                ui.label(
                    RichText::new(&entry.label)
                        .size(tree_text_size)
                        .strong()
                        .color(text_color),
                );
            });
        }
    });
}

fn paint_example_folder_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(15.0, 12.0), egui::Sense::hover());
    let painter = ui.painter();
    let tab = egui::Rect::from_min_size(rect.min + egui::vec2(1.0, 1.0), egui::vec2(6.0, 3.5));
    let body = egui::Rect::from_min_size(rect.min + egui::vec2(1.0, 4.0), egui::vec2(13.0, 7.0));

    painter.rect_filled(
        tab,
        egui::Rounding::same(1.0),
        Color32::from_rgb(251, 209, 86),
    );
    painter.rect_filled(
        body,
        egui::Rounding::same(1.0),
        Color32::from_rgb(244, 178, 53),
    );
    painter.line_segment(
        [body.left_top(), body.right_top()],
        egui::Stroke::new(1.0, Color32::from_rgb(255, 224, 123)),
    );
}

fn example_tree_connector(entry: &ExampleTreeEntry) -> String {
    if entry.depth == 0 {
        return String::new();
    }

    let mut connector = String::new();
    for has_next in entry
        .ancestor_has_next
        .iter()
        .take(entry.depth.saturating_sub(1))
    {
        connector.push_str(if *has_next { "│ " } else { "  " });
    }
    connector.push_str(if entry.is_last { "└─" } else { "├─" });
    connector
}
