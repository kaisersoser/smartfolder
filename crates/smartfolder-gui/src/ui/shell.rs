//! Application shell navigation rendering.

use eframe::egui::{self, Color32, RichText};

use crate::{ui, AppSection};

/// A command palette row with app-owned action payload.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CommandPaletteItem<T> {
    label: &'static str,
    detail: &'static str,
    enabled: bool,
    action: T,
}

impl<T> CommandPaletteItem<T> {
    pub(crate) fn new(label: &'static str, detail: &'static str, enabled: bool, action: T) -> Self {
        Self {
            label,
            detail,
            enabled,
            action,
        }
    }
}

/// Return the current shell navigation width for the expanded/collapsed state.
pub(crate) fn shell_nav_width(expanded: bool) -> f32 {
    if expanded {
        ui::theme::spacing::SIDEBAR_WIDTH
    } else {
        ui::theme::spacing::SIDEBAR_COLLAPSED_WIDTH
    }
}

/// Render the command palette and return the selected action.
pub(crate) fn render_command_palette_window<T: Copy>(
    ctx: &egui::Context,
    open: &mut bool,
    query: &mut String,
    items: &[CommandPaletteItem<T>],
) -> Option<T> {
    if !*open {
        return None;
    }

    let mut action = None;
    let mut window_open = true;
    egui::Window::new("Command palette")
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .show(ctx, |ui| {
            ui.add(
                egui::TextEdit::singleline(query)
                    .hint_text("Search commands")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(ui::theme::spacing::SM);

            let query = query.trim().to_ascii_lowercase();
            for item in items {
                if !query.is_empty()
                    && !item.label.to_ascii_lowercase().contains(&query)
                    && !item.detail.to_ascii_lowercase().contains(&query)
                {
                    continue;
                }

                let response = ui
                    .add_enabled_ui(item.enabled, |ui| {
                        ui::theme::widgets::surface_frame().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(item.label)
                                        .strong()
                                        .color(ui::theme::colors::heading_text()),
                                );
                                ui.label(
                                    RichText::new(item.detail)
                                        .size(ui::theme::typography::CAPTION)
                                        .color(ui::theme::colors::metadata_text()),
                                );
                            });
                        });
                    })
                    .response
                    .interact(egui::Sense::click());

                if response.clicked() && item.enabled {
                    action = Some(item.action);
                }
                ui.add_space(ui::theme::spacing::XS);
            }
        });

    if action.is_some() || !window_open {
        *open = false;
    }

    action
}

/// Render the app's left navigation rail and return any section selection.
pub(crate) fn render_shell_nav(
    ui: &mut egui::Ui,
    expanded: &mut bool,
    active_section: AppSection,
) -> Option<AppSection> {
    if *expanded {
        render_sidebar(ui, expanded, active_section)
    } else {
        render_collapsed_sidebar(ui, expanded, active_section)
    }
}

fn render_nav_toggle(ui: &mut egui::Ui, expanded: &mut bool) {
    let (icon, tooltip) = if *expanded {
        (ui::icons::COLLAPSE, "Collapse sidebar")
    } else {
        (ui::icons::EXPAND, "Expand sidebar")
    };
    if ui
        .add(ui::theme::widgets::icon_button(icon))
        .on_hover_text(tooltip)
        .clicked()
    {
        *expanded = !*expanded;
    }
}

fn render_collapsed_sidebar(
    ui: &mut egui::Ui,
    expanded: &mut bool,
    active_section: AppSection,
) -> Option<AppSection> {
    let mut selected_section = None;
    ui.add_space(ui::theme::spacing::SM);
    ui.vertical_centered(|ui| {
        render_nav_toggle(ui, expanded);
        ui.add_space(ui::theme::spacing::MD);

        for section in AppSection::ALL {
            let selected = active_section == section;
            let response = ui
                .add_sized(
                    [40.0, 36.0],
                    egui::Button::new(RichText::new(section.icon()).size(18.0).color(
                        if selected {
                            ui::theme::colors::primary_blue()
                        } else {
                            ui::theme::colors::secondary_text()
                        },
                    ))
                    .fill(if selected {
                        ui::theme::colors::hover_control()
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(egui::Stroke::new(
                        1.0,
                        if selected {
                            ui::theme::colors::border()
                        } else {
                            Color32::TRANSPARENT
                        },
                    )),
                )
                .on_hover_text(format!("{} ({})", section.title(), section.shortcut()));
            if response.clicked() {
                selected_section = Some(section);
            }
            ui.add_space(ui::theme::spacing::XS);
        }
    });
    selected_section
}

fn render_sidebar(
    ui: &mut egui::Ui,
    expanded: &mut bool,
    active_section: AppSection,
) -> Option<AppSection> {
    let mut selected_section = None;
    ui.add_space(ui::theme::spacing::MD);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("smartfolder")
                .size(ui::theme::typography::SECTION_TITLE)
                .strong()
                .color(ui::theme::colors::heading_text()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            render_nav_toggle(ui, expanded);
        });
    });
    ui.label(
        RichText::new("Organize files safely, then restore the previous layout if needed.")
            .color(ui::theme::colors::secondary_text()),
    );
    ui.label(
        RichText::new("Use Alt+1 through Alt+4 to switch sections.")
            .small()
            .color(ui::theme::colors::metadata_text()),
    );
    ui.add_space(ui::theme::spacing::LG);

    for section in AppSection::ALL {
        let selected = active_section == section;
        let nav_width = ui::theme::spacing::SIDEBAR_WIDTH - (ui::theme::spacing::LG * 2.0);
        let response = egui::Frame::none()
            .fill(if selected {
                ui::theme::colors::hover_control()
            } else {
                Color32::TRANSPARENT
            })
            .stroke(egui::Stroke::new(
                1.0,
                if selected {
                    ui::theme::colors::border()
                } else {
                    Color32::TRANSPARENT
                },
            ))
            .rounding(egui::Rounding::same(ui::theme::spacing::RADIUS_MD))
            .inner_margin(egui::Margin::symmetric(
                ui::theme::spacing::MD,
                ui::theme::spacing::SM,
            ))
            .show(ui, |ui| {
                let inner = ui.allocate_ui_with_layout(
                    egui::vec2(nav_width, 56.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(section.icon()).size(16.0).color(if selected {
                                ui::theme::colors::primary_blue()
                            } else {
                                ui::theme::colors::secondary_text()
                            }));
                            ui.label(
                                RichText::new(section.title())
                                    .strong()
                                    .color(ui::theme::colors::heading_text()),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(section.shortcut())
                                            .small()
                                            .color(ui::theme::colors::metadata_text()),
                                    );
                                },
                            );
                        });
                        ui.add_space(ui::theme::spacing::XS);
                        ui.label(
                            RichText::new(section.subtitle())
                                .color(ui::theme::colors::secondary_text()),
                        );
                    },
                );
                ui.interact(
                    inner.response.rect,
                    ui.id().with("app-section-nav").with(section.title()),
                    egui::Sense::click(),
                )
            })
            .inner;
        if response.clicked() {
            selected_section = Some(section);
        }
        ui.add_space(ui::theme::spacing::XS);
    }

    ui.add_space(ui::theme::spacing::MD);
    ui::theme::widgets::card_frame().show(ui, |ui| {
        ui.label(
            RichText::new("Launch behavior")
                .strong()
                .color(ui::theme::colors::heading_text()),
        );
        ui.label(
            RichText::new(
                "Right-clicking a folder in Explorer should open smartfolder with that folder already selected.",
            )
            .color(ui::theme::colors::secondary_text()),
        );
        ui.add_space(ui::theme::spacing::XS);
        ui.label(
            RichText::new("Keyboard section shortcuts never organize files by themselves.")
                .small()
                .color(ui::theme::colors::metadata_text()),
        );
    });

    selected_section
}
