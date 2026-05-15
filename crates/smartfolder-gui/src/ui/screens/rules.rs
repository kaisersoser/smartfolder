//! Rules screen presentation helpers.

use eframe::egui::{self, RichText};

use crate::{ui, RuleEditorState, PROFILE_WORKSPACE_FIELD_HEIGHT};

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
