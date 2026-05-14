//! Shared egui components used across app screens.

use eframe::egui::{self, Color32, RichText};

use crate::ui::theme;

pub(crate) fn status_chip(ui: &mut egui::Ui, text: &str, stroke: Color32, fill: Color32) {
    egui::Frame::group(ui.style())
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(10.0, 4.0))
        .show(ui, |ui| {
            ui.set_min_width(0.0);
            ui.label(RichText::new(text).strong().color(stroke));
        });
}

pub(crate) fn muted_status_chip(ui: &mut egui::Ui, text: &str) {
    status_chip(
        ui,
        text,
        theme::colors::secondary_text(),
        theme::colors::subtle_surface(),
    );
}

pub(crate) fn safety_line(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_wrapped(|ui| {
        status_chip(
            ui,
            "Safe",
            theme::colors::success(),
            theme::colors::success_bg(),
        );
        ui.add(egui::Label::new(text).wrap());
    });
}

pub(crate) fn truncated_label(ui: &mut egui::Ui, text: &str) {
    let width = ui.available_width().max(120.0);
    ui.add_sized([width, 20.0], egui::Label::new(text).truncate());
}

pub(crate) fn screen_heading(ui: &mut egui::Ui, icon: &str, title: &str, detail: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(icon)
                .size(18.0)
                .color(theme::colors::primary_blue()),
        );
        ui.label(
            RichText::new(title)
                .size(theme::typography::PAGE_TITLE)
                .strong()
                .color(theme::colors::heading_text()),
        );
    });
    ui.add_space(theme::spacing::XS);
    ui.add(
        egui::Label::new(
            RichText::new(detail)
                .size(theme::typography::BODY)
                .color(theme::colors::secondary_text()),
        )
        .wrap(),
    );
}

pub(crate) fn note_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(theme::colors::subtle_surface())
        .stroke(egui::Stroke::new(1.0, theme::colors::border()))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(theme::spacing::LG))
}

pub(crate) fn panel_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(theme::colors::elevated_surface())
        .stroke(egui::Stroke::new(1.0, theme::colors::border()))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(theme::spacing::LG))
}

pub(crate) fn constrained_page(
    ui: &mut egui::Ui,
    max_width: f32,
    contents: impl FnOnce(&mut egui::Ui),
) {
    let available_width = ui.available_width();
    let content_width = available_width.min(max_width).max(320.0);
    let side_margin = ((available_width - content_width) / 2.0).max(0.0);

    ui.add_space(theme::spacing::PAGE);
    ui.horizontal(|ui| {
        ui.add_space(side_margin);
        ui.allocate_ui_with_layout(
            egui::vec2(content_width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            contents,
        );
    });
}

pub(crate) fn section_panel(
    ui: &mut egui::Ui,
    min_width: f32,
    title: &str,
    detail: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    panel_frame().show(ui, |ui| {
        ui.set_width(ui.available_width().max(min_width));
        ui.label(
            RichText::new(title)
                .strong()
                .size(theme::typography::CARD_TITLE)
                .color(theme::colors::heading_text()),
        );
        ui.add(
            egui::Label::new(
                RichText::new(detail)
                    .size(theme::typography::CAPTION)
                    .color(theme::colors::metadata_text()),
            )
            .wrap(),
        );
        ui.add_space(theme::spacing::MD);
        contents(ui);
    });
}
