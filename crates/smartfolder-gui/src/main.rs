#![allow(clippy::module_name_repetitions)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use chrono::Utc;
use eframe::egui::{self, Color32, RichText};
use smartfolder_core::model::{BuiltInMode, PlanSummary};
use smartfolder_core::planner::{generate_plan, render_preview, PlanOptions};
use smartfolder_core::scanner::{scan_folder, ScanOptions};

type AnalysisMessage = std::result::Result<AnalysisOutput, String>;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions::default();
    let preloaded_root = preloaded_root_from_args(std::env::args());

    eframe::run_native(
        "smartfolder",
        native_options,
        Box::new(move |_creation_context| Ok(Box::new(SmartfolderApp::new(preloaded_root)))),
    )
}

#[derive(Debug)]
struct SmartfolderApp {
    root_input: String,
    mode: BuiltInMode,
    analysis_receiver: Option<Receiver<AnalysisMessage>>,
    analysis_result: Option<AnalysisOutput>,
    error_message: Option<String>,
}

impl SmartfolderApp {
    fn new(preloaded_root: Option<PathBuf>) -> Self {
        Self {
            root_input: preloaded_root
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            mode: BuiltInMode::TypeYear,
            analysis_receiver: None,
            analysis_result: None,
            error_message: None,
        }
    }

    fn is_analyzing(&self) -> bool {
        self.analysis_receiver.is_some()
    }

    fn start_analysis(&mut self) {
        let root_text = self.root_input.trim();
        if root_text.is_empty() {
            self.error_message = Some("Select a folder before running analysis.".to_string());
            return;
        }

        let root = PathBuf::from(root_text);
        let mode = self.mode;
        let (sender, receiver) = mpsc::channel::<AnalysisMessage>();
        self.analysis_receiver = Some(receiver);
        self.analysis_result = None;
        self.error_message = None;

        std::thread::spawn(move || {
            let result = analyze_root(&root, mode);
            let _ = sender.send(result);
        });
    }

    fn poll_analysis(&mut self) {
        let Some(receiver) = &self.analysis_receiver else {
            return;
        };

        match receiver.try_recv() {
            Ok(message) => {
                self.analysis_receiver = None;
                match message {
                    Ok(result) => {
                        self.analysis_result = Some(result);
                        self.error_message = None;
                    }
                    Err(message) => {
                        self.analysis_result = None;
                        self.error_message = Some(message);
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.analysis_receiver = None;
                self.error_message =
                    Some("The background analysis worker stopped unexpectedly.".to_string());
            }
        }
    }

    fn mode_label(mode: BuiltInMode) -> &'static str {
        match mode {
            BuiltInMode::Type => "Type",
            BuiltInMode::Date => "Date",
            BuiltInMode::Extension => "Extension",
            BuiltInMode::TypeYear => "Type / Year / Month / Day",
        }
    }
}

impl eframe::App for SmartfolderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_analysis();

        if self.is_analyzing() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("smartfolder 2.0");
            ui.label("Windows-first GUI prototype using the shared Rust core.");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label("Root folder");
                ui.text_edit_singleline(&mut self.root_input);
                if ui.button("Browse...").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.root_input = path.display().to_string();
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Built-in mode");
                egui::ComboBox::from_id_source("built-in-mode")
                    .selected_text(Self::mode_label(self.mode))
                    .show_ui(ui, |ui| {
                        for mode in [
                            BuiltInMode::Type,
                            BuiltInMode::Date,
                            BuiltInMode::Extension,
                            BuiltInMode::TypeYear,
                        ] {
                            ui.selectable_value(&mut self.mode, mode, Self::mode_label(mode));
                        }
                    });

                if ui
                    .add_enabled(!self.is_analyzing(), egui::Button::new("Analyze"))
                    .clicked()
                {
                    self.start_analysis();
                }
            });

            if self.is_analyzing() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Analyzing folder with the shared core...");
                });
            }

            if let Some(message) = &self.error_message {
                ui.add_space(8.0);
                ui.colored_label(Color32::from_rgb(190, 40, 40), message);
            }

            if let Some(result) = &self.analysis_result {
                ui.add_space(12.0);
                ui.separator();
                ui.label(RichText::new(format!("Plan {}", result.plan_id)).strong());
                ui.label(format!("Root: {}", result.root.display()));

                ui.add_space(8.0);
                egui::Grid::new("analysis-summary")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        summary_row(ui, "Files scanned", result.summary.files_scanned);
                        summary_row(ui, "Moves proposed", result.summary.moves_proposed);
                        summary_row(ui, "Ambiguous files", result.summary.ambiguous_files);
                        summary_row(ui, "Conflicts", result.summary.conflicts);
                        summary_row(ui, "Skipped", result.summary.skipped);
                    });

                if !result.warning_messages.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Warnings").strong());
                    for warning in &result.warning_messages {
                        ui.label(format!("- {warning}"));
                    }
                }

                ui.add_space(8.0);
                ui.label(RichText::new("Preview").strong());
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(360.0)
                    .show(ui, |ui| {
                        ui.code(&result.preview);
                    });
            }

            ui.add_space(12.0);
            ui.separator();
            ui.label(
                "This first GUI milestone covers app launch, folder preloading, built-in mode selection, and shared-core analysis. Apply, undo, and rule editing remain upcoming v2 milestones.",
            );
        });
    }
}

#[derive(Debug, Clone)]
struct AnalysisOutput {
    plan_id: String,
    root: PathBuf,
    summary: PlanSummary,
    warning_messages: Vec<String>,
    preview: String,
}

fn analyze_root(root: &Path, mode: BuiltInMode) -> AnalysisMessage {
    let scan = scan_folder(root, &ScanOptions::default())
        .map_err(|error| format!("Failed to scan {}: {error}", root.display()))?;
    let now = Utc::now();
    let plan_id = format!("plan_{}", now.format("%Y%m%d%H%M%S"));
    let plan_options = PlanOptions::built_in(mode, plan_id, now);
    let plan = generate_plan(root, &scan, &plan_options)
        .map_err(|error| format!("Failed to generate plan for {}: {error}", root.display()))?;

    Ok(AnalysisOutput {
        plan_id: plan.plan_id.clone(),
        root: plan.root.clone(),
        summary: plan.summary.clone(),
        warning_messages: plan
            .warnings
            .iter()
            .map(|warning| warning.message.clone())
            .collect(),
        preview: render_preview(&plan),
    })
}

fn summary_row(ui: &mut egui::Ui, label: &str, value: usize) {
    ui.label(label);
    ui.label(value.to_string());
    ui.end_row();
}

fn preloaded_root_from_args(args: impl IntoIterator<Item = String>) -> Option<PathBuf> {
    args.into_iter()
        .skip(1)
        .find(|arg| !arg.starts_with('-'))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use smartfolder_core::model::BuiltInMode;

    use super::{preloaded_root_from_args, SmartfolderApp};

    #[test]
    fn first_non_option_argument_is_used_as_preloaded_root() {
        let root = preloaded_root_from_args([
            "smartfolder-gui".to_string(),
            "--ignored".to_string(),
            r"D:\Users\User\Documents".to_string(),
        ]);

        assert_eq!(root, Some(PathBuf::from(r"D:\Users\User\Documents")));
    }

    #[test]
    fn mode_labels_match_planned_gui_copy() {
        assert_eq!(SmartfolderApp::mode_label(BuiltInMode::Type), "Type");
        assert_eq!(SmartfolderApp::mode_label(BuiltInMode::Date), "Date");
        assert_eq!(
            SmartfolderApp::mode_label(BuiltInMode::Extension),
            "Extension"
        );
        assert_eq!(
            SmartfolderApp::mode_label(BuiltInMode::TypeYear),
            "Type / Year / Month / Day"
        );
    }
}
