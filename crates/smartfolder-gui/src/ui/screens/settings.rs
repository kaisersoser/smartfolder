//! Settings screen presentation helpers.

use eframe::egui::{self, RichText};
use smartfolder_core::ai::{AiProviderState, AiProviderStatus};

use crate::{
    preferences::{GuiPreferences, MotionPreference, ThemePreference},
    ui::{
        self,
        components::{
            note_frame as settings_note_frame, safety_line as render_safety_line,
            section_panel as render_component_section_panel, status_chip as render_status_chip,
        },
    },
};

const SETTINGS_CARD_MIN_WIDTH: f32 = 188.0;

const SETTINGS_HELP_ROWS: [(&str, &str, &str); 4] = [
    (
        "Safety defaults",
        "Previews first, never overwrites files, and keeps subfolders opt-in.",
        "No automatic organizing",
    ),
    (
        "History",
        "Restore history is recorded before files move so previous layouts can be restored.",
        "Restore-ready workflow",
    ),
    (
        "Appearance",
        "Theme and motion preferences are saved locally for this device.",
        "Preferences saved",
    ),
    (
        "Keyboard navigation",
        "Use Alt+1 through Alt+4 to move between sections without reaching for the sidebar.",
        "Section shortcuts",
    ),
];

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AiPreferencesAction {
    pub(crate) changed: bool,
    pub(crate) test_connection: bool,
    pub(crate) export_diagnostics: bool,
}

pub(crate) fn render_settings_heading(ui: &mut egui::Ui, show_help: &mut bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(ui::icons::SETTINGS)
                .size(22.0)
                .color(ui::theme::colors::primary_blue()),
        );
        ui.label(
            RichText::new("Settings")
                .size(30.0)
                .strong()
                .color(ui::theme::colors::heading_text()),
        );
        ui.add_space(ui::theme::spacing::SM);
        let help_response = ui
            .add(
                egui::Button::new(
                    RichText::new(ui::icons::label(ui::icons::HELP, "Defaults"))
                        .color(ui::theme::colors::primary_text()),
                )
                .fill(ui::theme::colors::soft_control())
                .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
                .min_size(egui::vec2(116.0, 32.0)),
            )
            .on_hover_text("Show safety defaults and keyboard shortcuts.");
        if help_response.clicked() {
            *show_help = true;
        }
    });
    ui.add(
        egui::Label::new(
            RichText::new(
                "Keep the app clean, confirm Explorer launch behavior, and preserve the safer default of analyzing only the selected folder.",
            )
            .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
}

pub(crate) fn render_settings_help_window(ctx: &egui::Context, show_help: &mut bool) {
    let mut close_requested = false;
    egui::Window::new(ui::icons::label(ui::icons::HELP, "Settings help"))
        .open(show_help)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .show(ctx, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new("Reference details for the safer defaults used by smartfolder.")
                        .color(ui::theme::colors::secondary_text()),
                )
                .wrap(),
            );
            ui.add_space(ui::theme::spacing::MD);

            for (index, (title, detail, status)) in SETTINGS_HELP_ROWS.iter().enumerate() {
                render_settings_help_row(ui, title, detail, status);
                if index + 1 < SETTINGS_HELP_ROWS.len() {
                    ui.add_space(ui::theme::spacing::SM);
                }
            }

            ui.add_space(ui::theme::spacing::MD);
            ui.separator();
            ui.add_space(ui::theme::spacing::SM);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(ui::theme::widgets::secondary_button("Close"))
                    .clicked()
                {
                    close_requested = true;
                }
            });
        });
    if close_requested {
        *show_help = false;
    }
}

fn render_settings_help_row(ui: &mut egui::Ui, title: &str, detail: &str, status: &str) {
    ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(title)
                    .strong()
                    .color(ui::theme::colors::primary_text()),
            );
            render_status_chip(
                ui,
                status,
                ui::theme::colors::secondary_text(),
                ui::theme::colors::subtle_surface(),
            );
        });
        ui.add(
            egui::Label::new(RichText::new(detail).color(ui::theme::colors::secondary_text()))
                .wrap(),
        );
    });
}

pub(crate) fn render_settings_section_panel(
    ui: &mut egui::Ui,
    title: &str,
    detail: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    render_component_section_panel(ui, SETTINGS_CARD_MIN_WIDTH, title, detail, contents);
}

pub(crate) fn render_appearance_preferences(
    ui: &mut egui::Ui,
    preferences: &mut GuiPreferences,
) -> bool {
    ui.add(
        egui::Label::new(
            RichText::new("These preferences are saved locally and apply across the desktop app.")
                .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add_space(ui::theme::spacing::SM);

    let mut changed = false;
    egui::Grid::new("appearance-preferences-grid")
        .num_columns(2)
        .spacing([ui::theme::spacing::LG, ui::theme::spacing::SM])
        .show(ui, |ui| {
            ui.label(
                RichText::new("Theme")
                    .strong()
                    .color(ui::theme::colors::primary_text()),
            );
            egui::ComboBox::from_id_source("theme-preference")
                .width(180.0)
                .selected_text(preferences.theme.label())
                .show_ui(ui, |ui| {
                    for preference in [
                        ThemePreference::System,
                        ThemePreference::Light,
                        ThemePreference::Dark,
                    ] {
                        changed |= ui
                            .selectable_value(
                                &mut preferences.theme,
                                preference,
                                preference.label(),
                            )
                            .changed();
                    }
                });
            ui.end_row();

            ui.label(
                RichText::new("Motion")
                    .strong()
                    .color(ui::theme::colors::primary_text()),
            );
            egui::ComboBox::from_id_source("motion-preference")
                .width(180.0)
                .selected_text(preferences.motion.label())
                .show_ui(ui, |ui| {
                    for preference in [
                        MotionPreference::System,
                        MotionPreference::Reduced,
                        MotionPreference::Subtle,
                        MotionPreference::Full,
                    ] {
                        changed |= ui
                            .selectable_value(
                                &mut preferences.motion,
                                preference,
                                preference.label(),
                            )
                            .changed();
                    }
                });
            ui.end_row();
        });

    changed
}

pub(crate) fn render_ai_preferences(
    ui: &mut egui::Ui,
    preferences: &mut GuiPreferences,
    status: Option<&AiProviderStatus>,
    checking: bool,
) -> AiPreferencesAction {
    let mut action = AiPreferencesAction::default();

    ui.add(
        egui::Label::new(
            RichText::new(
                "AI stays off until Ollama is reachable, a model is selected, and a structured readiness check succeeds.",
            )
            .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add_space(ui::theme::spacing::SM);

    ui.horizontal_wrapped(|ui| {
        action.changed |= ui
            .checkbox(&mut preferences.ai.enabled, "Enable AI assistance")
            .changed();
        render_ai_status_chip(ui, preferences.ai.enabled, status, checking);
    });

    ui.add_space(ui::theme::spacing::SM);
    egui::Grid::new("ai-preferences-grid")
        .num_columns(2)
        .spacing([ui::theme::spacing::LG, ui::theme::spacing::SM])
        .show(ui, |ui| {
            ui.label(
                RichText::new("Provider")
                    .strong()
                    .color(ui::theme::colors::primary_text()),
            );
            ui.label(RichText::new("Ollama").color(ui::theme::colors::secondary_text()));
            ui.end_row();

            ui.label(
                RichText::new("Model")
                    .strong()
                    .color(ui::theme::colors::primary_text()),
            );
            if let Some(status) = status {
                if status.models.is_empty() {
                    ui.label(
                        RichText::new("No installed models found")
                            .color(ui::theme::colors::metadata_text()),
                    );
                } else {
                    let selected = preferences
                        .ai
                        .selected_model
                        .clone()
                        .or_else(|| status.selected_model.clone())
                        .unwrap_or_else(|| "Select model".to_string());
                    egui::ComboBox::from_id_source("ai-model-preference")
                        .width(260.0)
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for model in &status.models {
                                action.changed |= ui
                                    .selectable_value(
                                        &mut preferences.ai.selected_model,
                                        Some(model.clone()),
                                        model,
                                    )
                                    .changed();
                            }
                        });
                }
            } else {
                action.changed |= ui
                    .add_enabled(
                        preferences.ai.enabled,
                        egui::TextEdit::singleline(
                            preferences
                                .ai
                                .selected_model
                                .get_or_insert_with(String::new),
                        )
                        .desired_width(260.0)
                        .hint_text("selected after Test connection"),
                    )
                    .changed();
                if preferences.ai.selected_model.as_deref() == Some("") {
                    preferences.ai.selected_model = None;
                }
            }
            ui.end_row();
        });

    ui.add_space(ui::theme::spacing::SM);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                preferences.ai.enabled && !checking,
                ui::theme::widgets::secondary_button(if checking {
                    "Testing..."
                } else {
                    "Test connection"
                }),
            )
            .clicked()
        {
            action.test_connection = true;
        }

        if let Some(status) = status {
            ui.add(
                egui::Label::new(
                    RichText::new(&status.message)
                        .size(ui::theme::typography::CAPTION)
                        .color(if status.available {
                            ui::theme::colors::success()
                        } else {
                            ui::theme::colors::metadata_text()
                        }),
                )
                .wrap(),
            );
        }
    });

    action
}

pub(crate) fn render_privacy_preferences(
    ui: &mut egui::Ui,
    preferences: &mut GuiPreferences,
) -> bool {
    let mut changed = false;
    settings_note_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        changed |= ui
            .add_enabled(
                preferences.ai.enabled,
                egui::Checkbox::new(
                    &mut preferences.ai.content_inspection_enabled,
                    "Allow AI to inspect sampled text file contents",
                ),
            )
            .changed();
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Off by default. When enabled, only sampled text-like file contents are sent to the local Ollama model. OCR, media, and broad binary extraction are not used.",
                )
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
            )
            .wrap(),
        );
    });

    if !preferences.ai.enabled {
        ui.label(
            RichText::new("Enable AI assistance before changing content inspection.")
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
        );
    }

    changed
}

pub(crate) fn render_advanced_ai_preferences(
    ui: &mut egui::Ui,
    preferences: &mut GuiPreferences,
) -> AiPreferencesAction {
    let mut action = AiPreferencesAction::default();

    ui.label(
        RichText::new("AI provider details")
            .strong()
            .color(ui::theme::colors::heading_text()),
    );
    egui::Grid::new("advanced-ai-preferences-grid")
        .num_columns(2)
        .spacing([ui::theme::spacing::LG, ui::theme::spacing::SM])
        .show(ui, |ui| {
            ui.label("Endpoint");
            action.changed |= ui
                .add_enabled(
                    preferences.ai.enabled,
                    egui::TextEdit::singleline(&mut preferences.ai.endpoint)
                        .desired_width(260.0)
                        .hint_text("http://localhost:11434"),
                )
                .changed();
            ui.end_row();

            ui.label("Timeout");
            ui.horizontal(|ui| {
                action.changed |= ui
                    .add_enabled(
                        preferences.ai.enabled,
                        egui::DragValue::new(&mut preferences.ai.timeout_seconds)
                            .range(5..=300)
                            .speed(1.0),
                    )
                    .changed();
                ui.label(
                    RichText::new("seconds")
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::metadata_text()),
                );
            });
            ui.end_row();
        });

    ui.add_space(ui::theme::spacing::SM);
    if ui
        .add(ui::theme::widgets::secondary_button(
            "Export AI diagnostics",
        ))
        .clicked()
    {
        action.export_diagnostics = true;
    }

    action
}

pub(crate) fn render_storage_maintenance(ui: &mut egui::Ui, can_cleanup: bool) -> bool {
    let mut cleanup_requested = false;
    ui.label(
        RichText::new("Storage maintenance")
            .strong()
            .color(ui::theme::colors::heading_text()),
    );
    ui.add(
        egui::Label::new(
            RichText::new(
                "Cleanup removes old cached analysis sessions and preview pages. It does not remove restore history for organized files.",
            )
            .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add_space(ui::theme::spacing::SM);
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                can_cleanup,
                ui::theme::widgets::secondary_button("Clean old session data"),
            )
            .clicked()
        {
            cleanup_requested = true;
        }
        ui.label(
            RichText::new("Available when no analysis, organize, or restore task is running.")
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
        );
    });
    cleanup_requested
}

fn render_ai_status_chip(
    ui: &mut egui::Ui,
    enabled: bool,
    status: Option<&AiProviderStatus>,
    checking: bool,
) {
    let (label, text_color, fill) = if !enabled {
        (
            "Disabled",
            ui::theme::colors::metadata_text(),
            ui::theme::colors::subtle_surface(),
        )
    } else if checking {
        (
            "Checking",
            ui::theme::colors::info(),
            ui::theme::colors::info_bg(),
        )
    } else {
        match status.map(|status| status.state) {
            Some(AiProviderState::Ready) => (
                "Ready",
                ui::theme::colors::success(),
                ui::theme::colors::success_bg(),
            ),
            Some(AiProviderState::NoModels) => (
                "No models",
                ui::theme::colors::warning(),
                ui::theme::colors::warning_bg(),
            ),
            Some(AiProviderState::ModelMissing) => (
                "Model missing",
                ui::theme::colors::warning(),
                ui::theme::colors::warning_bg(),
            ),
            Some(AiProviderState::EndpointUnavailable | AiProviderState::RequestFailed) => (
                "Unavailable",
                ui::theme::colors::error(),
                ui::theme::colors::error_bg(),
            ),
            Some(AiProviderState::Disabled) | None => (
                "Not tested",
                ui::theme::colors::metadata_text(),
                ui::theme::colors::subtle_surface(),
            ),
        }
    };

    render_status_chip(ui, label, text_color, fill);
}

pub(crate) fn render_explorer_integration_settings(ui: &mut egui::Ui) {
    ui.add(
        egui::Label::new(
            RichText::new("The folder context menu entry should read Organize with smartfolder.")
                .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add_space(ui::theme::spacing::SM);
    render_safety_line(
        ui,
        "It only opens the app with the clicked folder selected.",
    );
    render_safety_line(ui, "It never organizes files directly from Explorer.");
    ui.add_space(ui::theme::spacing::SM);
    settings_note_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Register or unregister it with scripts/register-explorer-launcher.ps1 after building the release GUI.",
                )
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
            )
            .wrap(),
        );
    });
}
