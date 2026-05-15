//! Rules screen presentation helpers.

use eframe::egui::{self, RichText};
use smartfolder_core::{model::BuiltInMode, rules::RuleProfile};

use crate::{
    plural, sample_destination_template,
    ui::{
        self,
        components::{
            note_frame as settings_note_frame, status_chip as render_status_chip, truncated_label,
        },
    },
    AiDraftProfileResult, LoadedRuleProfile, PlanningSource, ProfileEditorState, RuleEditorState,
    RuleSimulationResult, PROFILE_RULE_LIST_WIDTH, PROFILE_WORKSPACE_FIELD_HEIGHT,
};

pub(crate) fn render_profile_workspace_window(
    ctx: &egui::Context,
    contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut open = true;
    egui::Window::new("Profile workspace")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(980.0)
        .default_height(680.0)
        .min_width(760.0)
        .min_height(520.0)
        .show(ctx, contents);
    open
}

pub(crate) fn render_ai_draft_summary_strip(
    ui: &mut egui::Ui,
    result: &AiDraftProfileResult,
    show_review: &mut bool,
    show_prompt: &mut bool,
) {
    settings_note_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            if result.validation.is_usable() {
                render_status_chip(
                    ui,
                    "Draft loaded",
                    ui::theme::colors::success(),
                    ui::theme::colors::success_bg(),
                );
            } else {
                render_status_chip(
                    ui,
                    "Validation failed",
                    ui::theme::colors::error(),
                    ui::theme::colors::error_bg(),
                );
            }
            render_status_chip(
                ui,
                &format!(
                    "{} rule{}",
                    result.draft.rules.len(),
                    plural(result.draft.rules.len())
                ),
                ui::theme::colors::secondary_text(),
                ui::theme::colors::elevated_surface(),
            );
            if !result.validation.warnings.is_empty() {
                render_status_chip(
                    ui,
                    &format!(
                        "{} warning{}",
                        result.validation.warnings.len(),
                        plural(result.validation.warnings.len())
                    ),
                    ui::theme::colors::warning(),
                    ui::theme::colors::warning_bg(),
                );
            }
            if !result.validation.errors.is_empty() {
                render_status_chip(
                    ui,
                    &format!(
                        "{} error{}",
                        result.validation.errors.len(),
                        plural(result.validation.errors.len())
                    ),
                    ui::theme::colors::error(),
                    ui::theme::colors::error_bg(),
                );
            }
            if ui
                .add(ui::theme::widgets::compact_secondary_button("Review draft"))
                .clicked()
            {
                *show_review = true;
            }
            if ui
                .add(ui::theme::widgets::compact_secondary_button("Edit prompt"))
                .clicked()
            {
                *show_prompt = true;
            }
        });
        if let Some(rationale) = &result.draft.rationale {
            ui.add_space(ui::theme::spacing::XS);
            ui.add(
                egui::Label::new(
                    RichText::new(elide_text(rationale, 150))
                        .color(ui::theme::colors::secondary_text()),
                )
                .truncate(),
            )
            .on_hover_text(rationale);
        }
    });
}

pub(crate) fn render_ai_draft_review(
    ui: &mut egui::Ui,
    result: &AiDraftProfileResult,
    edit_prompt_requested: &mut bool,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("AI draft review")
                .strong()
                .size(ui::theme::typography::SECTION_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        render_status_chip(
            ui,
            &format!(
                "{} rule{}",
                result.draft.rules.len(),
                plural(result.draft.rules.len())
            ),
            ui::theme::colors::secondary_text(),
            ui::theme::colors::elevated_surface(),
        );
        if !result.validation.warnings.is_empty() {
            render_status_chip(
                ui,
                &format!(
                    "{} warning{}",
                    result.validation.warnings.len(),
                    plural(result.validation.warnings.len())
                ),
                ui::theme::colors::warning(),
                ui::theme::colors::warning_bg(),
            );
        }
        if ui
            .add(ui::theme::widgets::compact_secondary_button("Edit prompt"))
            .clicked()
        {
            *edit_prompt_requested = true;
        }
    });
    ui.add_space(ui::theme::spacing::SM);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height((ui.available_height() - 8.0).max(260.0))
        .show(ui, |ui| {
            if let Some(rationale) = &result.draft.rationale {
                ui.label(
                    RichText::new("Rationale")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(rationale).color(ui::theme::colors::secondary_text()),
                    )
                    .wrap(),
                );
                ui.add_space(ui::theme::spacing::SM);
            }

            if !result.validation.warnings.is_empty() {
                ui.label(
                    RichText::new("Warnings")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                for warning in &result.validation.warnings {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("- {warning}"))
                                .color(ui::theme::colors::warning()),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(ui::theme::spacing::SM);
            }

            if !result.validation.errors.is_empty() {
                ui.label(
                    RichText::new("Errors")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                for error in &result.validation.errors {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("- {error}")).color(ui::theme::colors::error()),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(ui::theme::spacing::SM);
            }

            egui::CollapsingHeader::new("Raw AI draft JSON")
                .default_open(false)
                .show(ui, |ui| {
                    let mut raw = result.raw_json.clone();
                    ui.add_sized(
                        [ui.available_width(), 180.0],
                        egui::TextEdit::multiline(&mut raw)
                            .font(egui::TextStyle::Monospace)
                            .interactive(false),
                    );
                });
        });
}

fn elide_text(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut shortened = trimmed
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    shortened.push_str("...");
    shortened
}

pub(crate) fn render_profile_editor(
    ui: &mut egui::Ui,
    editor: &mut ProfileEditorState,
    simulation: Option<&RuleSimulationResult>,
) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(PROFILE_RULE_LIST_WIDTH, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                render_rule_list_panel(ui, editor);
            },
        );

        ui.separator();

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                if let Some(rule) = editor.selected_rule_mut() {
                    render_rule_detail_editor(ui, rule, simulation);
                }
            },
        );
    });
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum BuiltinRuleAction {
    Use,
    Customize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SavedProfileAction {
    Use,
    Edit,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ProfileWorkspaceAction {
    Save,
    Use,
    Validate,
    Simulate,
    ToggleAiDraftPrompt,
    ExplainWithAi,
    NewDraft,
    ImportToml,
    ExportToml,
}

pub(crate) fn render_profile_workspace_toolbar(
    ui: &mut egui::Ui,
    editor: &mut ProfileEditorState,
    loaded_profile: Option<&LoadedRuleProfile>,
    can_simulate: bool,
    ai_available: bool,
    running_ai_task: bool,
) -> Option<ProfileWorkspaceAction> {
    let mut action = None;
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(&editor.profile_id)
                .strong()
                .size(ui::theme::typography::SECTION_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        render_status_chip(
            ui,
            &format!("{} rule{}", editor.rules.len(), plural(editor.rules.len())),
            ui::theme::colors::secondary_text(),
            ui::theme::colors::subtle_surface(),
        );
        render_profile_workspace_status(ui, loaded_profile);
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .add(ui::theme::widgets::compact_primary_button("Save"))
            .clicked()
        {
            action = Some(ProfileWorkspaceAction::Save);
        }
        if ui
            .add(ui::theme::widgets::compact_secondary_button("Use"))
            .clicked()
        {
            action = Some(ProfileWorkspaceAction::Use);
        }
        if ui
            .add(ui::theme::widgets::compact_secondary_button("Validate"))
            .clicked()
        {
            action = Some(ProfileWorkspaceAction::Validate);
        }
        if ui
            .add_enabled(
                can_simulate,
                ui::theme::widgets::compact_secondary_button("Simulate"),
            )
            .on_disabled_hover_text("Run Preview before simulating this rule")
            .clicked()
        {
            action = Some(ProfileWorkspaceAction::Simulate);
        }
        if ai_available {
            if ui
                .add(ui::theme::widgets::compact_secondary_button(
                    "Build with AI",
                ))
                .clicked()
            {
                action = Some(ProfileWorkspaceAction::ToggleAiDraftPrompt);
            }
            if ui
                .add_enabled(
                    !running_ai_task,
                    ui::theme::widgets::compact_secondary_button("Explain"),
                )
                .clicked()
            {
                action = Some(ProfileWorkspaceAction::ExplainWithAi);
            }
        }
    });
    egui::CollapsingHeader::new("Advanced")
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Profile id")
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::metadata_text()),
                );
                ui.add_sized(
                    [220.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                    egui::TextEdit::singleline(&mut editor.profile_id),
                );
                if ui
                    .add(ui::theme::widgets::compact_secondary_button("New draft"))
                    .clicked()
                {
                    action = Some(ProfileWorkspaceAction::NewDraft);
                }
                if ui
                    .add(ui::theme::widgets::compact_secondary_button("Import TOML"))
                    .clicked()
                {
                    action = Some(ProfileWorkspaceAction::ImportToml);
                }
                if ui
                    .add(ui::theme::widgets::compact_secondary_button("Export TOML"))
                    .clicked()
                {
                    action = Some(ProfileWorkspaceAction::ExportToml);
                }
            });
        });
    action
}

pub(crate) fn render_active_profile_panel(
    ui: &mut egui::Ui,
    planning_source: PlanningSource,
    mode: BuiltInMode,
    loaded_profile: Option<&LoadedRuleProfile>,
    profile_to_edit: &mut Option<LoadedRuleProfile>,
    maintenance_message: &mut Option<String>,
) {
    ui::theme::widgets::card_frame().show(ui, |ui| {
        ui.label(
            RichText::new("Active profile")
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        ui.add_space(ui::theme::spacing::XS);

        let (name, rules, destination, is_custom) =
            active_profile_summary(planning_source, mode, loaded_profile);

        ui.horizontal_wrapped(|ui| {
            render_status_chip(
                ui,
                if is_custom { "Custom" } else { "Built-in" },
                ui::theme::colors::info(),
                ui::theme::colors::info_bg(),
            );
            ui.label(
                RichText::new(name)
                    .strong()
                    .color(ui::theme::colors::heading_text()),
            );
            ui.label(
                RichText::new(rules)
                    .size(ui::theme::typography::CAPTION)
                    .color(ui::theme::colors::metadata_text()),
            );
        });
        ui.label(
            RichText::new(destination)
                .monospace()
                .color(ui::theme::colors::primary_text()),
        );
        ui.add_space(ui::theme::spacing::SM);
        ui.horizontal_wrapped(|ui| {
            if let Some(profile) = loaded_profile {
                if ui
                    .add(ui::theme::widgets::secondary_button("Edit profile"))
                    .clicked()
                {
                    *profile_to_edit = Some(profile.clone());
                }
            }
            if ui
                .add(ui::theme::widgets::tertiary_button("Use another"))
                .clicked()
            {
                *maintenance_message =
                    Some("Choose a built-in style or saved custom profile below.".to_string());
            }
        });
    });
}

pub(crate) fn render_rules_ai_panel(
    ui: &mut egui::Ui,
    has_root: bool,
    running_ai_task: bool,
    show_rules_workspace: &mut bool,
    show_ai_draft_prompt: &mut bool,
    maintenance_message: &mut Option<String>,
    error_message: &mut Option<String>,
) {
    ui::theme::widgets::surface_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new("Rule suggestions")
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        ui.label(
            RichText::new("Use AI to draft deterministic custom rules, then review and save them before organizing.")
                .color(ui::theme::colors::secondary_text()),
        );
        ui.add_space(ui::theme::spacing::SM);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    has_root && !running_ai_task,
                    ui::theme::widgets::secondary_button("Suggest rules from current folder"),
                )
                .on_disabled_hover_text("Choose a folder before asking AI to suggest rules")
                .clicked()
            {
                *show_rules_workspace = true;
                *show_ai_draft_prompt = true;
                *maintenance_message =
                    Some("Describe how you want files organized, then draft rules.".to_string());
                *error_message = None;
            }
            if ui
                .add_enabled(
                    !running_ai_task,
                    ui::theme::widgets::tertiary_button("Describe how you want files organized"),
                )
                .clicked()
            {
                *show_rules_workspace = true;
                *show_ai_draft_prompt = true;
                *maintenance_message =
                    Some("Describe the rule profile you want AI to draft.".to_string());
                *error_message = None;
            }
        });
    });
}

pub(crate) fn builtin_library_items() -> [(BuiltInMode, &'static str, &'static str, &'static str); 3]
{
    [
        (
            BuiltInMode::Type,
            "By Type",
            "{type}",
            "Groups files into Documents, Images, Videos, Archives, and related folders.",
        ),
        (
            BuiltInMode::Date,
            "By Date",
            "{year}/{month}/{day}",
            "Groups files by modified date when time is the clearest way to browse them.",
        ),
        (
            BuiltInMode::TypeYear,
            "Type + Date",
            "{type}/{year}/{month}/{day}",
            "Keeps file kinds together, then adds date folders inside each type.",
        ),
    ]
}

pub(crate) fn render_builtin_rule_row(
    ui: &mut egui::Ui,
    title: &str,
    pattern: &str,
    detail: &str,
    selected: bool,
    mut on_action: impl FnMut(BuiltinRuleAction),
) {
    egui::Frame::group(ui.style())
        .fill(if selected {
            ui::theme::colors::hover_control()
        } else {
            ui::theme::colors::elevated_surface()
        })
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                ui::theme::colors::primary_blue()
            } else {
                ui::theme::colors::border()
            },
        ))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(ui::theme::spacing::SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2((ui.available_width() - 280.0).max(260.0), 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if selected {
                                ui::theme::widgets::status_dot(
                                    ui,
                                    ui::theme::colors::primary_blue(),
                                );
                            }
                            ui.label(
                                RichText::new(title)
                                    .strong()
                                    .color(ui::theme::colors::heading_text()),
                            );
                            if selected {
                                render_status_chip(
                                    ui,
                                    "Active",
                                    ui::theme::colors::info(),
                                    ui::theme::colors::info_bg(),
                                );
                            }
                        });
                        ui.label(
                            RichText::new(format!(
                                "Destination: {}",
                                sample_destination_template(pattern)
                            ))
                            .monospace()
                            .color(ui::theme::colors::primary_text()),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(detail).color(ui::theme::colors::secondary_text()),
                            )
                            .wrap(),
                        );
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button("Customize"))
                        .clicked()
                    {
                        on_action(BuiltinRuleAction::Customize);
                    }
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button("Use"))
                        .clicked()
                    {
                        on_action(BuiltinRuleAction::Use);
                    }
                });
            });
        });
}

pub(crate) fn render_saved_profile_row(
    ui: &mut egui::Ui,
    profile: &LoadedRuleProfile,
    selected: bool,
) -> Option<SavedProfileAction> {
    let mut action = None;
    egui::Frame::group(ui.style())
        .fill(if selected {
            ui::theme::colors::hover_control()
        } else {
            ui::theme::colors::elevated_surface()
        })
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                ui::theme::colors::primary_blue()
            } else {
                ui::theme::colors::border()
            },
        ))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(ui::theme::spacing::SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2((ui.available_width() - 280.0).max(260.0), 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if selected {
                                ui::theme::widgets::status_dot(
                                    ui,
                                    ui::theme::colors::primary_blue(),
                                );
                            }
                            ui.label(
                                RichText::new(&profile.profile.profile_id)
                                    .strong()
                                    .color(ui::theme::colors::heading_text()),
                            );
                            render_status_chip(
                                ui,
                                &format!(
                                    "{} rule{}",
                                    profile.profile.rules.len(),
                                    plural(profile.profile.rules.len())
                                ),
                                ui::theme::colors::secondary_text(),
                                ui::theme::colors::elevated_surface(),
                            );
                            if selected {
                                render_status_chip(
                                    ui,
                                    "Active",
                                    ui::theme::colors::success(),
                                    ui::theme::colors::success_bg(),
                                );
                            }
                        });
                        if let Some(rule) = profile.profile.rules.first() {
                            ui.label(
                                RichText::new(format!(
                                    "{} -> {}",
                                    rule.name,
                                    sample_destination_template(&rule.destination)
                                ))
                                .color(ui::theme::colors::secondary_text()),
                            );
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(profile.path.display().to_string())
                                    .size(ui::theme::typography::CAPTION)
                                    .color(ui::theme::colors::metadata_text()),
                            )
                            .truncate(),
                        )
                        .on_hover_text(profile.path.display().to_string());
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button("Use"))
                        .clicked()
                    {
                        action = Some(SavedProfileAction::Use);
                    }
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button("Edit"))
                        .clicked()
                    {
                        action = Some(SavedProfileAction::Edit);
                    }
                });
            });
        });
    action
}

fn active_profile_summary(
    planning_source: PlanningSource,
    mode: BuiltInMode,
    loaded_profile: Option<&LoadedRuleProfile>,
) -> (String, String, String, bool) {
    if let (PlanningSource::RuleProfile, Some(profile)) = (planning_source, loaded_profile) {
        (
            profile.profile.profile_id.clone(),
            format!(
                "{} rule{}",
                profile.profile.rules.len(),
                plural(profile.profile.rules.len())
            ),
            profile_destination_example(&profile.profile),
            true,
        )
    } else {
        let (_, title, pattern, _) = builtin_library_items()
            .into_iter()
            .find(|(candidate, _, _, _)| *candidate == mode)
            .unwrap_or((
                BuiltInMode::TypeYear,
                "Type + Date",
                "{type}/{year}/{month}/{day}",
                "",
            ));
        (
            title.to_string(),
            "Read-only built-in".to_string(),
            format!("Destination: {}", sample_destination_template(pattern)),
            false,
        )
    }
}

fn profile_destination_example(profile: &RuleProfile) -> String {
    profile
        .rules
        .first()
        .map(|rule| {
            format!(
                "Destination: {}",
                sample_destination_template(&rule.destination)
            )
        })
        .unwrap_or_else(|| "Destination: not set".to_string())
}

pub(crate) fn render_profile_workspace_status(
    ui: &mut egui::Ui,
    loaded_profile: Option<&LoadedRuleProfile>,
) {
    if let Some(profile) = loaded_profile {
        ui.horizontal_wrapped(|ui| {
            render_status_chip(
                ui,
                "Saved",
                ui::theme::colors::success(),
                ui::theme::colors::success_bg(),
            );
            ui.label(
                RichText::new(format!("Editing {}", profile.profile.profile_id))
                    .color(ui::theme::colors::primary_text()),
            );
        });
    } else {
        ui.horizontal_wrapped(|ui| {
            render_status_chip(
                ui,
                "Draft",
                ui::theme::colors::warning(),
                ui::theme::colors::warning_bg(),
            );
            ui.label(
                RichText::new("Save this draft before using it for Custom Rules.")
                    .color(ui::theme::colors::secondary_text()),
            );
        });
    }
}

pub(crate) fn render_rule_list_panel(ui: &mut egui::Ui, editor: &mut ProfileEditorState) {
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Rules")
                .strong()
                .color(ui::theme::colors::heading_text()),
        );
        render_status_chip(
            ui,
            &format!("{} rule{}", editor.rules.len(), plural(editor.rules.len())),
            ui::theme::colors::secondary_text(),
            ui::theme::colors::elevated_surface(),
        );
    });
    ui.add_space(ui::theme::spacing::XS);

    let mut selected_rule = editor.selected_rule_index();
    for (index, rule) in editor.rules.iter().enumerate() {
        if render_rule_list_item(ui, rule, index, index == selected_rule).clicked() {
            selected_rule = index;
        }
        ui.add_space(4.0);
    }
    editor.selected_rule = selected_rule;

    ui.add_space(ui::theme::spacing::SM);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_sized(
                [80.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                ui::theme::widgets::compact_secondary_button("Add"),
            )
            .clicked()
        {
            editor.add_rule();
        }
        if ui
            .add_sized(
                [104.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                ui::theme::widgets::compact_secondary_button("Duplicate"),
            )
            .clicked()
        {
            editor.duplicate_selected_rule();
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                editor.selected_rule_index() > 0,
                ui::theme::widgets::compact_secondary_button("Up")
                    .min_size(egui::vec2(80.0, PROFILE_WORKSPACE_FIELD_HEIGHT)),
            )
            .clicked()
        {
            editor.move_selected_rule(-1);
        }
        if ui
            .add_enabled(
                editor.selected_rule_index() + 1 < editor.rules.len(),
                ui::theme::widgets::compact_secondary_button("Down")
                    .min_size(egui::vec2(80.0, PROFILE_WORKSPACE_FIELD_HEIGHT)),
            )
            .clicked()
        {
            editor.move_selected_rule(1);
        }
        if ui
            .add_enabled(
                editor.rules.len() > 1,
                ui::theme::widgets::compact_secondary_button("Delete")
                    .min_size(egui::vec2(80.0, PROFILE_WORKSPACE_FIELD_HEIGHT)),
            )
            .clicked()
        {
            editor.delete_selected_rule();
        }
    });
}

pub(crate) fn render_rule_detail_editor(
    ui: &mut egui::Ui,
    rule: &mut RuleEditorState,
    simulation: Option<&RuleSimulationResult>,
) {
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
    render_rule_editor_section(ui, "Rule", "Name this rule and set its order.", |ui| {
        ui.horizontal_wrapped(|ui| {
            labeled_text_edit(ui, "Rule name", &mut rule.rule_name, 220.0, "");
            labeled_text_edit(ui, "Priority", &mut rule.priority, 72.0, "10");
        });
    });

    ui.add_space(ui::theme::spacing::SM);
    render_rule_editor_section(
        ui,
        "Applies to",
        "Leave fields blank when they should not constrain the match.",
        |ui| render_rule_conditions_editor(ui, rule),
    );

    ui.add_space(ui::theme::spacing::SM);
    render_rule_editor_section(
        ui,
        "Destination",
        "Build the folder path for matched files.",
        |ui| {
            ui.horizontal_wrapped(|ui| {
                render_destination_mode_toggle(ui, rule);
                ui.label(
                    RichText::new(readable_destination_path(&rule.destination))
                        .monospace()
                        .color(ui::theme::colors::primary_text()),
                );
            });
            ui.add_space(ui::theme::spacing::SM);
            render_destination_builder(ui, rule);
        },
    );

    ui.add_space(ui::theme::spacing::SM);
    render_rule_editor_section(
        ui,
        "Simulation",
        "Preview how this rule behaves against the latest analyzed folder.",
        |ui| render_rule_simulation(ui, rule, simulation),
    );

    ui.add_space(ui::theme::spacing::SM);
    render_rule_editor_section(
        ui,
        "Advanced",
        "Use sparingly. These options can broaden matches.",
        |ui| {
            ui.checkbox(&mut rule.match_all, "Match all files");
            if rule.match_all {
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "This rule ignores all conditions and should usually be the final fallback rule.",
                        )
                        .color(ui::theme::colors::warning()),
                    )
                    .wrap(),
                );
            }
        },
    );
}

fn render_rule_list_item(
    ui: &mut egui::Ui,
    rule: &RuleEditorState,
    index: usize,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(180.0), 52.0),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            ui::theme::colors::hover_control()
        } else if response.hovered() {
            ui::theme::colors::soft_control()
        } else {
            ui::theme::colors::elevated_surface()
        };
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(6.0), fill);
        ui.painter().rect_stroke(
            rect,
            egui::Rounding::same(6.0),
            egui::Stroke::new(1.0, ui::theme::colors::border()),
        );
        if selected {
            let accent = egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.left() + 3.0, rect.bottom()),
            );
            ui.painter().rect_filled(
                accent,
                egui::Rounding::same(6.0),
                ui::theme::colors::primary_blue(),
            );
        }
        let text_x = rect.left() + 10.0;
        ui.painter().text(
            egui::pos2(text_x, rect.top() + 13.0),
            egui::Align2::LEFT_CENTER,
            format!("{}. {}", index + 1, rule.rule_name),
            egui::TextStyle::Button.resolve(ui.style()),
            ui::theme::colors::heading_text(),
        );
        ui.painter().text(
            egui::pos2(text_x, rect.top() + 34.0),
            egui::Align2::LEFT_CENTER,
            rule_destination_summary(rule),
            egui::TextStyle::Small.resolve(ui.style()),
            ui::theme::colors::secondary_text(),
        );
    }
    response.on_hover_text(rule_destination_summary(rule))
}

fn render_rule_editor_section(
    ui: &mut egui::Ui,
    title: &str,
    detail: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    ui::theme::widgets::surface_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(title)
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        ui.label(
            RichText::new(detail)
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
        );
        ui.add_space(ui::theme::spacing::SM);
        contents(ui);
    });
}

fn render_rule_simulation(
    ui: &mut egui::Ui,
    rule: &RuleEditorState,
    simulation: Option<&RuleSimulationResult>,
) {
    let Some(simulation) = simulation.filter(|simulation| simulation.rule_name == rule.rule_name)
    else {
        ui.add(
            egui::Label::new(
                RichText::new("Run Preview, then use Simulate in the toolbar.")
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
        return;
    };

    ui.horizontal_wrapped(|ui| {
        render_status_chip(
            ui,
            &format!(
                "{} match{}",
                simulation.matched_files,
                plural(simulation.matched_files)
            ),
            ui::theme::colors::info(),
            ui::theme::colors::info_bg(),
        );
        ui.label(
            RichText::new(format!(
                "Checked {} file{} from the current preview.",
                simulation.total_files,
                plural(simulation.total_files)
            ))
            .color(ui::theme::colors::secondary_text()),
        );
    });

    if simulation.sample_matches.is_empty() {
        ui.label(
            RichText::new("No sampled files matched this rule.")
                .color(ui::theme::colors::metadata_text()),
        );
    } else {
        ui.add_space(ui::theme::spacing::XS);
        for sample in &simulation.sample_matches {
            truncated_label(ui, &format!("Matched: {sample}"));
        }
    }
}

fn render_rule_conditions_editor(ui: &mut egui::Ui, rule: &mut RuleEditorState) {
    if rule.match_all {
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Match all files is enabled in Advanced, so these conditions are ignored.",
                )
                .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
    }

    egui::Grid::new("rule-condition-editor")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Extensions");
            ui.add_sized(
                [220.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                egui::TextEdit::singleline(&mut rule.extensions)
                    .hint_text(example_hint("pdf, docx")),
            );
            ui.end_row();

            ui.label("Filename contains");
            ui.add_sized(
                [220.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                egui::TextEdit::singleline(&mut rule.filename_contains)
                    .hint_text(example_hint("invoice")),
            );
            ui.end_row();

            ui.label("Path contains");
            ui.add_sized(
                [220.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                egui::TextEdit::singleline(&mut rule.path_contains)
                    .hint_text(example_hint("downloads")),
            );
            ui.end_row();

            ui.label("Year");
            ui.add_sized(
                [96.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                egui::TextEdit::singleline(&mut rule.year).hint_text(example_hint("2026")),
            );
            ui.end_row();

            ui.label("Size range");
            ui.horizontal(|ui| {
                ui.add_sized(
                    [104.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                    egui::TextEdit::singleline(&mut rule.min_size_bytes)
                        .hint_text(example_hint("1000")),
                );
                ui.add_sized(
                    [104.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                    egui::TextEdit::singleline(&mut rule.max_size_bytes)
                        .hint_text(example_hint("200000")),
                );
            });
            ui.end_row();
        });
}

fn labeled_text_edit(ui: &mut egui::Ui, label: &str, value: &mut String, width: f32, hint: &str) {
    ui.label(
        RichText::new(label)
            .size(ui::theme::typography::CAPTION)
            .color(ui::theme::colors::metadata_text()),
    );
    let mut edit = egui::TextEdit::singleline(value);
    if !hint.is_empty() {
        edit = edit.hint_text(example_hint(hint));
    }
    ui.add_sized([width, PROFILE_WORKSPACE_FIELD_HEIGHT], edit);
}

#[derive(Debug, Clone)]
pub(crate) struct DestinationDragPayload {
    pub(crate) source: DestinationDragSource,
}

#[derive(Debug, Clone)]
pub(crate) enum DestinationDragSource {
    Existing(usize),
    Token(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DestinationSegment {
    Text(String),
    Token(String),
}

impl DestinationSegment {
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Text(value) | Self::Token(value) => value,
        }
    }

    fn is_token(&self) -> bool {
        matches!(self, Self::Token(_))
    }
}

pub(crate) fn render_destination_mode_toggle(ui: &mut egui::Ui, rule: &mut RuleEditorState) {
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut rule.destination_text_mode, false, "Visual");
        ui.selectable_value(&mut rule.destination_text_mode, true, "Text");
    });
}

pub(crate) fn render_destination_builder(ui: &mut egui::Ui, rule: &mut RuleEditorState) {
    if rule.destination_text_mode {
        render_destination_text_editor(ui, rule);
    } else {
        render_destination_visual_editor(ui, rule);
    }
}

pub(crate) fn readable_destination_path(destination: &str) -> String {
    let segments = parse_destination_segments(destination);
    if segments.is_empty() {
        "No destination set".to_string()
    } else {
        segments
            .iter()
            .map(DestinationSegment::label)
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

pub(crate) fn rule_destination_summary(rule: &RuleEditorState) -> String {
    if rule.destination.trim().is_empty() {
        "No destination set".to_string()
    } else {
        format!("-> {}", rule.destination.trim())
    }
}

pub(crate) fn parse_destination_segments(destination: &str) -> Vec<DestinationSegment> {
    destination
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            if is_destination_token(segment) {
                DestinationSegment::Token(segment.to_string())
            } else {
                DestinationSegment::Text(segment.to_string())
            }
        })
        .collect()
}

pub(crate) fn build_destination_from_segments(segments: &[DestinationSegment]) -> String {
    segments
        .iter()
        .map(DestinationSegment::label)
        .collect::<Vec<_>>()
        .join("/")
}

pub(crate) fn apply_destination_drop(
    segments: &mut Vec<DestinationSegment>,
    target_index: usize,
    payload: DestinationDragPayload,
) {
    match payload.source {
        DestinationDragSource::Existing(source_index) => {
            if source_index >= segments.len() {
                return;
            }
            let segment = segments.remove(source_index);
            let adjusted_target = if source_index < target_index {
                target_index.saturating_sub(1)
            } else {
                target_index
            };
            segments.insert(adjusted_target.min(segments.len()), segment);
        }
        DestinationDragSource::Token(token) => {
            segments.insert(
                target_index.min(segments.len()),
                DestinationSegment::Token(token),
            );
        }
    }
}

fn render_destination_text_editor(ui: &mut egui::Ui, rule: &mut RuleEditorState) {
    ui.add_sized(
        [
            ui.available_width().min(520.0),
            PROFILE_WORKSPACE_FIELD_HEIGHT,
        ],
        egui::TextEdit::singleline(&mut rule.destination)
            .hint_text(example_hint("Documents/PDFs/{year}/{month}")),
    );
    ui.label(
        RichText::new("Use / between folders. Supported tokens: {type}, {year}, {month}, {day}, {extension}, {filename}.")
            .size(ui::theme::typography::CAPTION)
            .color(ui::theme::colors::metadata_text()),
    );
}

fn render_destination_visual_editor(ui: &mut egui::Ui, rule: &mut RuleEditorState) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Drag chips to reorder. Drop outside to remove.")
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
        );
    });

    let mut segments = parse_destination_segments(&rule.destination);
    let mut dropped_payload: Option<(usize, DestinationDragPayload)> = None;

    let path_response = destination_path_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            if segments.is_empty() {
                ui.label(
                    RichText::new("Drop a token or add a folder segment.")
                        .color(ui::theme::colors::metadata_text()),
                );
            }

            for (index, segment) in segments.iter().enumerate() {
                let chip_response = ui.dnd_drag_source(
                    egui::Id::new(("destination-segment", index, segment.label())),
                    DestinationDragPayload {
                        source: DestinationDragSource::Existing(index),
                    },
                    |ui| {
                        render_destination_segment_chip(ui, segment);
                    },
                );
                let response = chip_response
                    .response
                    .on_hover_text("Drag to reorder. Drag outside the path to remove.");
                if let Some(payload) = response.dnd_release_payload::<DestinationDragPayload>() {
                    let target_index = ui
                        .ctx()
                        .pointer_interact_pos()
                        .map(|position| {
                            if position.x > response.rect.center().x {
                                index + 1
                            } else {
                                index
                            }
                        })
                        .unwrap_or(index);
                    dropped_payload = Some((target_index, (*payload).clone()));
                }
            }
        });
    });

    if dropped_payload.is_none() {
        if let Some(payload) = path_response
            .response
            .dnd_release_payload::<DestinationDragPayload>()
        {
            dropped_payload = Some((segments.len(), (*payload).clone()));
        }
    }

    if let Some((target_index, payload)) = dropped_payload {
        apply_destination_drop(&mut segments, target_index, payload);
        rule.destination = build_destination_from_segments(&segments);
    } else if let Some(index) =
        destination_segment_removed_outside_path(ui, path_response.response.rect, segments.len())
    {
        if index < segments.len() {
            segments.remove(index);
            rule.destination = build_destination_from_segments(&segments);
            egui::DragAndDrop::clear_payload(ui.ctx());
        }
    }

    ui.add_space(ui::theme::spacing::XS);
    render_destination_add_segment(ui, rule);
    ui.add_space(ui::theme::spacing::XS);
    render_destination_token_palette(ui, rule);
}

fn destination_path_frame() -> egui::Frame {
    egui::Frame::group(&egui::Style::default())
        .fill(ui::theme::colors::surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(4.0))
}

fn render_destination_segment_chip(ui: &mut egui::Ui, segment: &DestinationSegment) {
    let stroke = if segment.is_token() {
        ui::theme::colors::info()
    } else {
        ui::theme::colors::secondary_text()
    };
    let fill = if segment.is_token() {
        ui::theme::colors::info_bg()
    } else {
        ui::theme::colors::elevated_surface()
    };

    egui::Frame::group(ui.style())
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .rounding(egui::Rounding::same(999.0))
        .inner_margin(egui::Margin::symmetric(9.0, 4.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(segment.label())
                    .monospace()
                    .strong()
                    .size(ui::theme::typography::CAPTION)
                    .color(stroke),
            );
        });
}

fn destination_segment_removed_outside_path(
    ui: &mut egui::Ui,
    path_rect: egui::Rect,
    segment_count: usize,
) -> Option<usize> {
    if segment_count == 0 || !ui.ctx().input(|input| input.pointer.any_released()) {
        return None;
    }

    let pointer = ui.ctx().pointer_interact_pos()?;
    if path_rect.contains(pointer) {
        return None;
    }

    let payload = egui::DragAndDrop::payload::<DestinationDragPayload>(ui.ctx())?;
    match payload.source {
        DestinationDragSource::Existing(index) => Some(index),
        DestinationDragSource::Token(_) => None,
    }
}

fn render_destination_add_segment(ui: &mut egui::Ui, rule: &mut RuleEditorState) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Folder segment")
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
        );
        ui.add_sized(
            [180.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
            egui::TextEdit::singleline(&mut rule.new_destination_segment)
                .hint_text(example_hint("Invoices")),
        );
        if ui
            .add_sized(
                [72.0, PROFILE_WORKSPACE_FIELD_HEIGHT],
                ui::theme::widgets::compact_secondary_button("Add"),
            )
            .clicked()
        {
            append_text_destination_segment(rule);
        }
    });
}

fn render_destination_token_palette(ui: &mut egui::Ui, rule: &mut RuleEditorState) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Tokens")
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
        );
        for token in ["{type}", "{year}", "{month}", "{day}", "{extension}"] {
            render_token_palette_chip(ui, rule, token);
        }
    });
    egui::CollapsingHeader::new("Advanced tokens")
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                render_token_palette_chip(ui, rule, "{filename}");
            });
        });
}

fn render_token_palette_chip(ui: &mut egui::Ui, rule: &mut RuleEditorState, token: &'static str) {
    let response = ui.dnd_drag_source(
        egui::Id::new(("destination-token-palette", token)),
        DestinationDragPayload {
            source: DestinationDragSource::Token(token.to_string()),
        },
        |ui| {
            let button = egui::Button::new(
                RichText::new(token)
                    .monospace()
                    .strong()
                    .color(ui::theme::colors::info()),
            )
            .fill(ui::theme::colors::surface())
            .stroke(egui::Stroke::new(1.0, ui::theme::colors::info()))
            .min_size(egui::vec2(72.0, PROFILE_WORKSPACE_FIELD_HEIGHT));
            if ui.add(button).clicked() {
                append_destination_token(&mut rule.destination, token);
            }
        },
    );
    response
        .response
        .on_hover_text("Drag into destination or click to append");
}

fn append_text_destination_segment(rule: &mut RuleEditorState) {
    let trimmed = rule
        .new_destination_segment
        .trim()
        .trim_matches(['/', '\\'])
        .to_string();
    if trimmed.is_empty() {
        return;
    }
    let mut segments = parse_destination_segments(&rule.destination);
    segments.push(if is_destination_token(&trimmed) {
        DestinationSegment::Token(trimmed)
    } else {
        DestinationSegment::Text(trimmed)
    });
    rule.destination = build_destination_from_segments(&segments);
    rule.new_destination_segment.clear();
}

fn is_destination_token(segment: &str) -> bool {
    matches!(
        segment,
        "{type}" | "{year}" | "{month}" | "{day}" | "{extension}" | "{filename}"
    )
}

fn append_destination_token(destination: &mut String, token: &str) {
    if destination.trim().is_empty() {
        destination.push_str(token);
    } else if destination.ends_with('/') || destination.ends_with('\\') {
        destination.push_str(token);
    } else {
        destination.push('/');
        destination.push_str(token);
    }
}

fn example_hint(value: &str) -> RichText {
    RichText::new(format!("e.g. {value}"))
        .italics()
        .color(ui::theme::colors::metadata_text())
}
