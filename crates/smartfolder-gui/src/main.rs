//! Desktop GUI shell for the smartfolder workflow.
//!
//! This binary wraps the shared Rust core with a Windows-first interface that can be
//! launched directly or from a folder context action in Explorer. The GUI keeps the
//! engine metadata-only and reversible while presenting a calmer, organize-first flow.
//!
//! Key features:
//! - Explorer-preloaded folder launch support
//! - Shared-core analysis, preview, apply, and undo operations
//! - Organize-first shell with Activity, Rules, and Settings sections
//! - Paged preview and transaction history backed by the on-disk session store
#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use eframe::egui::{self, Color32, RichText};
use smartfolder_core::apply::{
    apply_stored_plan_with_progress, ApplyOptions, ApplySummary, StoredApplyProgress,
};
use smartfolder_core::model::{
    BuiltInMode, ConflictState, OperationStatus, PlanMode, PlanOperation, PlanSummary,
    TransactionStatus,
};
use smartfolder_core::planner::{
    generate_plan_to_store_with_progress_and_cancellation, PlanGenerationProgress, PlanOptions,
};
use smartfolder_core::recovery::{
    inspect_transaction, list_transactions, undo_transaction, TransactionSummary, UndoSummary,
};
use smartfolder_core::rules::{CustomRule, RuleProfile};
use smartfolder_core::scanner::{
    scan_folder_to_sink_with_progress, CancellationToken, ScanOptions, StreamingScanProgress,
    StreamingScanResult,
};
use smartfolder_core::session_store::{PlanOperationFilter, SessionScanSink, SqliteSessionStore};
use smartfolder_core::storage::ensure_profiles_dir;

type AnalysisMessage = std::result::Result<AnalysisOutput, String>;
type ApplyMessage = std::result::Result<ApplyOutput, String>;
type UndoMessage = std::result::Result<UndoOutput, String>;

const PREVIEW_PAGE_SIZE: usize = 100;
const TRANSACTION_DETAIL_ROW_LIMIT: usize = 100;
const WINDOW_WIDTH: f32 = 860.0;
const WINDOW_HEIGHT: f32 = 720.0;
const SHELL_NAV_WIDTH: f32 = 196.0;
const CARD_MIN_WIDTH: f32 = 220.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppSection {
    Organize,
    Activity,
    Rules,
    Settings,
}

impl AppSection {
    fn title(self) -> &'static str {
        match self {
            Self::Organize => "Organize",
            Self::Activity => "Activity",
            Self::Rules => "Rules",
            Self::Settings => "Settings",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Self::Organize => "Preview safe changes before organizing files.",
            Self::Activity => "Review recent changes and undo them if needed.",
            Self::Rules => "Manage built-in styles and custom rule profiles.",
            Self::Settings => "Keep the app tidy and confirm launch behavior.",
        }
    }
}

#[derive(Debug, Clone)]
enum AnalysisEvent {
    Progress(AnalysisProgress),
    Finished(AnalysisMessage),
}

#[derive(Debug, Clone)]
enum ApplyEvent {
    Progress(ApplyProgress),
    Finished(ApplyMessage),
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_min_inner_size([720.0, 560.0]),
        ..eframe::NativeOptions::default()
    };
    let preloaded_root = preloaded_root_from_args(std::env::args());

    eframe::run_native(
        "smartfolder",
        native_options,
        Box::new(move |_creation_context| Ok(Box::new(SmartfolderApp::new(preloaded_root)))),
    )
}

#[derive(Debug)]
struct SmartfolderApp {
    active_section: AppSection,
    root_input: String,
    launched_with_preselected_root: bool,
    mode: BuiltInMode,
    planning_source: PlanningSource,
    loaded_profile: Option<LoadedRuleProfile>,
    profile_editor: ProfileEditorState,
    include_subfolders: bool,
    preview_filter: PreviewFilter,
    preview_offset: usize,
    selected_preview_row: Option<usize>,
    analysis_receiver: Option<Receiver<AnalysisEvent>>,
    analysis_cancellation: Option<CancellationToken>,
    analysis_progress: Option<AnalysisProgress>,
    analysis_result: Option<AnalysisOutput>,
    apply_receiver: Option<Receiver<ApplyEvent>>,
    apply_progress: Option<ApplyProgress>,
    apply_result: Option<ApplyOutput>,
    show_apply_confirmation: bool,
    undo_receiver: Option<Receiver<UndoMessage>>,
    undo_result: Option<UndoOutput>,
    transaction_rows: Vec<TransactionRow>,
    transaction_message: Option<String>,
    transaction_detail: Option<TransactionDetail>,
    transaction_detail_message: Option<String>,
    show_recovery_log: bool,
    show_undo_confirmation: Option<String>,
    error_message: Option<String>,
    maintenance_message: Option<String>,
}

impl SmartfolderApp {
    fn new(preloaded_root: Option<PathBuf>) -> Self {
        let launched_with_preselected_root = preloaded_root.is_some();
        let (transaction_rows, transaction_message) = match load_transaction_rows() {
            Ok(rows) => (rows, None),
            Err(message) => (Vec::new(), Some(message)),
        };

        Self {
            active_section: AppSection::Organize,
            root_input: preloaded_root
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            launched_with_preselected_root,
            mode: BuiltInMode::TypeYear,
            planning_source: PlanningSource::BuiltIn,
            loaded_profile: None,
            profile_editor: ProfileEditorState::default(),
            include_subfolders: false,
            preview_filter: PreviewFilter::All,
            preview_offset: 0,
            selected_preview_row: None,
            analysis_receiver: None,
            analysis_cancellation: None,
            analysis_progress: None,
            analysis_result: None,
            apply_receiver: None,
            apply_progress: None,
            apply_result: None,
            show_apply_confirmation: false,
            undo_receiver: None,
            undo_result: None,
            transaction_rows,
            transaction_message,
            transaction_detail: None,
            transaction_detail_message: None,
            show_recovery_log: false,
            show_undo_confirmation: None,
            error_message: None,
            maintenance_message: None,
        }
    }

    fn is_analyzing(&self) -> bool {
        self.analysis_receiver.is_some()
    }

    fn is_applying(&self) -> bool {
        self.apply_receiver.is_some()
    }

    fn is_undoing(&self) -> bool {
        self.undo_receiver.is_some()
    }

    fn has_selected_root(&self) -> bool {
        !self.root_input.trim().is_empty()
    }

    fn can_run_analysis(&self) -> bool {
        self.has_selected_root()
            && match self.planning_source {
                PlanningSource::BuiltIn => true,
                PlanningSource::RuleProfile => self.loaded_profile.is_some(),
            }
    }

    fn browse_for_root(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.root_input = path.display().to_string();
            self.launched_with_preselected_root = false;
        }
    }

    fn start_analysis(&mut self) {
        let root_text = self.root_input.trim();
        if root_text.is_empty() {
            self.error_message = Some("Select a folder before running analysis.".to_string());
            return;
        }

        let root = PathBuf::from(root_text);
        let plan_source = match self.analysis_plan_source() {
            Ok(plan_source) => plan_source,
            Err(message) => {
                self.error_message = Some(message);
                return;
            }
        };
        let include_subfolders = self.include_subfolders;
        let cancellation = CancellationToken::default();
        let worker_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel::<AnalysisEvent>();
        self.analysis_receiver = Some(receiver);
        self.analysis_cancellation = Some(cancellation);
        self.analysis_progress = Some(AnalysisProgress::preparing(root.clone()));
        self.analysis_result = None;
        self.apply_progress = None;
        self.apply_result = None;
        self.undo_result = None;
        self.show_apply_confirmation = false;
        self.error_message = None;
        self.preview_filter = PreviewFilter::All;
        self.preview_offset = 0;
        self.selected_preview_row = None;

        std::thread::spawn(move || {
            let result = analyze_root(
                &root,
                plan_source,
                include_subfolders,
                &worker_cancellation,
                &sender,
            );
            let _ = sender.send(AnalysisEvent::Finished(result));
        });
    }

    fn cancel_analysis(&mut self) {
        if let Some(cancellation) = &self.analysis_cancellation {
            cancellation.cancel();
            self.analysis_progress = Some(AnalysisProgress::cancelling());
        }
    }

    fn cleanup_old_sessions(&mut self) {
        match cleanup_old_session_data() {
            Ok(removed) => {
                self.maintenance_message = Some(format!(
                    "Removed {removed} old analysis session{}.",
                    if removed == 1 { "" } else { "s" }
                ));
                self.error_message = None;
            }
            Err(message) => {
                self.maintenance_message = None;
                self.error_message = Some(message);
            }
        }
    }

    fn import_rule_profile(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("TOML rule profile", &["toml"])
            .pick_file()
        else {
            return;
        };

        match load_rule_profile_from_path(&path) {
            Ok(profile) => {
                let profile_id = profile.profile_id.clone();
                self.profile_editor = ProfileEditorState::from_profile(&profile);
                self.loaded_profile = Some(LoadedRuleProfile { path, profile });
                self.planning_source = PlanningSource::RuleProfile;
                self.maintenance_message = Some(format!("Imported rule profile '{profile_id}'."));
                self.error_message = None;
            }
            Err(message) => {
                self.error_message = Some(message);
                self.maintenance_message = None;
            }
        }
    }

    fn save_profile_from_editor(&mut self) {
        match save_profile_from_editor(&self.profile_editor) {
            Ok(loaded_profile) => {
                let profile_id = loaded_profile.profile.profile_id.clone();
                let path = loaded_profile.path.display().to_string();
                self.loaded_profile = Some(loaded_profile);
                self.planning_source = PlanningSource::RuleProfile;
                self.maintenance_message =
                    Some(format!("Saved rule profile '{profile_id}' to {path}."));
                self.error_message = None;
            }
            Err(message) => {
                self.error_message = Some(message);
                self.maintenance_message = None;
            }
        }
    }

    fn export_profile_from_editor(&mut self) {
        match export_profile_from_editor(&self.profile_editor) {
            Ok(Some(path)) => {
                self.maintenance_message =
                    Some(format!("Exported rule profile to {}.", path.display()));
                self.error_message = None;
            }
            Ok(None) => {}
            Err(message) => {
                self.error_message = Some(message);
                self.maintenance_message = None;
            }
        }
    }

    fn validate_profile_editor(&mut self) {
        match self.profile_editor.to_profile() {
            Ok(profile) => {
                self.maintenance_message =
                    Some(format!("Rule profile '{}' is valid.", profile.profile_id));
                self.error_message = None;
            }
            Err(message) => {
                self.error_message = Some(message);
                self.maintenance_message = None;
            }
        }
    }

    fn analysis_plan_source(&self) -> std::result::Result<AnalysisPlanSource, String> {
        match self.planning_source {
            PlanningSource::BuiltIn => Ok(AnalysisPlanSource::BuiltIn(self.mode)),
            PlanningSource::RuleProfile => self
                .loaded_profile
                .as_ref()
                .map(|loaded| AnalysisPlanSource::RuleProfile(loaded.profile.clone()))
                .ok_or_else(|| {
                    "Import a rule profile before analyzing with profile rules.".to_string()
                }),
        }
    }

    fn poll_analysis(&mut self) {
        let Some(receiver) = &self.analysis_receiver else {
            return;
        };

        let mut finished = None;
        let mut disconnected = false;

        loop {
            match receiver.try_recv() {
                Ok(AnalysisEvent::Progress(progress)) => {
                    self.analysis_progress = Some(progress);
                }
                Ok(AnalysisEvent::Finished(message)) => {
                    finished = Some(message);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if let Some(message) = finished {
            self.analysis_receiver = None;
            self.analysis_cancellation = None;
            self.analysis_progress = None;
            match message {
                Ok(result) => {
                    self.analysis_result = Some(result);
                    self.error_message = None;
                    self.preview_filter = PreviewFilter::All;
                    self.preview_offset = 0;
                    self.sync_preview_selection();
                }
                Err(message) => {
                    self.analysis_result = None;
                    self.error_message = Some(message);
                }
            }
        } else if disconnected {
            self.analysis_receiver = None;
            self.analysis_cancellation = None;
            self.analysis_progress = None;
            self.error_message =
                Some("The background analysis worker stopped unexpectedly.".to_string());
        }
    }

    fn start_apply(&mut self) {
        let Some(result) = &self.analysis_result else {
            return;
        };
        if result.preview_counts.ready == 0 {
            self.error_message = Some("No safe moves are ready to organize.".to_string());
            return;
        }

        let session_id = result.session_id.clone();
        let plan_id = result.plan_id.clone();
        let root = result.root.clone();
        let ready = result.preview_counts.ready;
        let transaction_id = format!("txn_{}", Utc::now().format("%Y%m%d%H%M%S"));
        let (sender, receiver) = mpsc::channel::<ApplyEvent>();
        self.apply_receiver = Some(receiver);
        self.apply_progress = Some(ApplyProgress::preparing(ready));
        self.apply_result = None;
        self.show_apply_confirmation = false;
        self.error_message = None;

        std::thread::spawn(move || {
            let result = apply_session_plan(&session_id, &plan_id, &root, &transaction_id, &sender);
            let _ = sender.send(ApplyEvent::Finished(result));
        });
    }

    fn poll_apply(&mut self) {
        let Some(receiver) = &self.apply_receiver else {
            return;
        };

        let mut finished = None;
        let mut disconnected = false;

        loop {
            match receiver.try_recv() {
                Ok(ApplyEvent::Progress(progress)) => {
                    self.apply_progress = Some(progress);
                }
                Ok(ApplyEvent::Finished(message)) => {
                    finished = Some(message);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        if let Some(message) = finished {
            self.apply_receiver = None;
            self.apply_progress = None;
            match message {
                Ok(result) => {
                    self.apply_result = Some(result);
                    self.refresh_transaction_history();
                    self.error_message = None;
                }
                Err(message) => {
                    self.apply_result = None;
                    self.error_message = Some(message);
                }
            }
        } else if disconnected {
            self.apply_receiver = None;
            self.apply_progress = None;
            self.error_message =
                Some("The background apply worker stopped unexpectedly.".to_string());
        }
    }

    fn refresh_transaction_history(&mut self) {
        match load_transaction_rows() {
            Ok(rows) => {
                self.transaction_rows = rows;
                self.transaction_message = None;
            }
            Err(message) => {
                self.transaction_rows.clear();
                self.transaction_message = Some(message);
            }
        }
    }

    fn load_transaction_detail(&mut self, transaction_id: &str) {
        match load_transaction_detail(transaction_id) {
            Ok(detail) => {
                self.transaction_detail = Some(detail);
                self.transaction_detail_message = None;
            }
            Err(message) => {
                self.transaction_detail = None;
                self.transaction_detail_message = Some(message);
            }
        }
    }

    fn active_root(&self) -> Option<PathBuf> {
        self.analysis_result.as_ref().map_or_else(
            || {
                let root = self.root_input.trim();
                (!root.is_empty()).then(|| PathBuf::from(root))
            },
            |result| Some(result.root.clone()),
        )
    }

    fn start_undo(&mut self, transaction_id: String) {
        let (sender, receiver) = mpsc::channel::<UndoMessage>();
        self.undo_receiver = Some(receiver);
        self.undo_result = None;
        self.show_undo_confirmation = None;
        self.error_message = None;

        std::thread::spawn(move || {
            let result = undo_transaction_for_gui(&transaction_id);
            let _ = sender.send(result);
        });
    }

    fn poll_undo(&mut self) {
        let Some(receiver) = &self.undo_receiver else {
            return;
        };

        match receiver.try_recv() {
            Ok(message) => {
                self.undo_receiver = None;
                match message {
                    Ok(result) => {
                        self.undo_result = Some(result);
                        self.refresh_transaction_history();
                        self.error_message = None;
                    }
                    Err(message) => {
                        self.undo_result = None;
                        self.error_message = Some(message);
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.undo_receiver = None;
                self.error_message =
                    Some("The background undo worker stopped unexpectedly.".to_string());
            }
        }
    }

    #[cfg(test)]
    fn mode_label(mode: BuiltInMode) -> &'static str {
        match mode {
            BuiltInMode::Type => "Type",
            BuiltInMode::Date => "Date",
            BuiltInMode::Extension => "Extension",
            BuiltInMode::TypeYear => "Type / Year / Month / Day",
        }
    }

    fn selected_style_title(&self) -> &'static str {
        match self.planning_source {
            PlanningSource::BuiltIn => match self.mode {
                BuiltInMode::Type => "By Type",
                BuiltInMode::Date => "By Date",
                BuiltInMode::Extension => "By Extension",
                BuiltInMode::TypeYear => "Type + Date",
            },
            PlanningSource::RuleProfile => "Custom Rules",
        }
    }

    fn sync_preview_selection(&mut self) {
        self.selected_preview_row = self
            .analysis_result
            .as_ref()
            .and_then(|result| (!result.preview_rows.is_empty()).then_some(0));
    }

    fn root_readiness_copy(&self) -> String {
        if !self.has_selected_root() {
            return "Choose a folder to organize. If you launch from Explorer, smartfolder will preload the clicked folder here.".to_string();
        }

        if self.launched_with_preselected_root {
            "This folder was preselected at launch and is ready for Analyze Folder.".to_string()
        } else {
            "This folder is selected and ready for Analyze Folder.".to_string()
        }
    }

    fn render_status_messages(&mut self, ui: &mut egui::Ui) {
        if let Some(message) = &self.error_message {
            ui.add_space(6.0);
            ui.colored_label(Color32::from_rgb(190, 40, 40), message);
        }

        if let Some(message) = &self.maintenance_message {
            ui.add_space(6.0);
            ui.colored_label(Color32::from_rgb(70, 140, 90), message);
        }
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.label(RichText::new("smartfolder").size(26.0).strong());
        ui.label("Organize files safely, then undo changes if you need to.");
        ui.add_space(14.0);

        for section in [
            AppSection::Organize,
            AppSection::Activity,
            AppSection::Rules,
            AppSection::Settings,
        ] {
            let selected = self.active_section == section;
            let button = egui::Button::new(
                RichText::new(format!("{}\n{}", section.title(), section.subtitle())).size(14.0),
            )
            .min_size(egui::vec2(SHELL_NAV_WIDTH - 28.0, 56.0))
            .fill(if selected {
                Color32::from_rgb(230, 219, 206)
            } else {
                Color32::from_rgb(246, 240, 232)
            })
            .stroke(egui::Stroke::new(
                if selected { 2.0 } else { 1.0 },
                if selected {
                    Color32::from_rgb(166, 94, 53)
                } else {
                    Color32::from_rgb(212, 200, 186)
                },
            ));
            if ui.add(button).clicked() {
                self.active_section = section;
            }
            ui.add_space(4.0);
        }

        ui.add_space(12.0);
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(246, 240, 232))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(216, 202, 188)))
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
                ui.label(RichText::new("Launch behavior").strong());
                ui.label(
                    "Right-clicking a folder in Explorer should open smartfolder with that folder already selected.",
                );
            });
    }

    fn render_organize_screen(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        preview_action: &mut Option<PreviewAction>,
        history_action: &mut Option<HistoryAction>,
    ) {
        render_screen_heading(
            ui,
            "Organize Files",
            "Open a folder from Explorer or choose one here, preview the safe changes, then organize with undo available afterward.",
        );
        self.render_status_messages(ui);
        ui.add_space(10.0);

        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(250, 246, 240))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
            .inner_margin(egui::Margin::same(16.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Choose a folder to organize")
                                .strong()
                                .size(18.0),
                        );
                        ui.label("This folder becomes the root for preview, organizing, and undo.");
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.has_selected_root() {
                            render_status_chip(
                                ui,
                                if self.launched_with_preselected_root {
                                    "Preselected at launch"
                                } else {
                                    "Folder ready"
                                },
                                Color32::from_rgb(92, 128, 78),
                                Color32::from_rgb(231, 241, 228),
                            );
                        }
                    });
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let root_input_width = (ui.available_width() - 110.0).max(240.0);
                    ui.add_sized(
                        [root_input_width, 30.0],
                        egui::TextEdit::singleline(&mut self.root_input).hint_text("D:\\Documents"),
                    );
                    if ui.button("Browse...").clicked() {
                        self.browse_for_root();
                    }
                });
                ui.add_space(6.0);
                ui.label(self.root_readiness_copy());
            });

        ui.add_space(12.0);
        ui.label(
            RichText::new("Choose an organizing style")
                .strong()
                .size(18.0),
        );
        ui.label("Pick the layout that best matches this folder. You can change it before each analysis.");
        ui.add_space(8.0);

        ui.horizontal_wrapped(|ui| {
            if render_style_card(
                ui,
                self.planning_source == PlanningSource::BuiltIn && self.mode == BuiltInMode::Type,
                "By Type",
                "Images / PDFs / Videos",
                "Group similar file types together.",
            )
            .clicked()
            {
                self.planning_source = PlanningSource::BuiltIn;
                self.mode = BuiltInMode::Type;
            }

            if render_style_card(
                ui,
                self.planning_source == PlanningSource::BuiltIn && self.mode == BuiltInMode::Date,
                "By Date",
                "2026 / May / 13",
                "Sort files by when they were last modified.",
            )
            .clicked()
            {
                self.planning_source = PlanningSource::BuiltIn;
                self.mode = BuiltInMode::Date;
            }

            if render_style_card(
                ui,
                self.planning_source == PlanningSource::BuiltIn
                    && self.mode == BuiltInMode::TypeYear,
                "Type + Date",
                "Images / 2026 / May",
                "Keep file types together and add date folders inside them.",
            )
            .clicked()
            {
                self.planning_source = PlanningSource::BuiltIn;
                self.mode = BuiltInMode::TypeYear;
            }

            if render_style_card(
                ui,
                self.planning_source == PlanningSource::RuleProfile,
                "Custom Rules",
                "Documents / PDFs",
                "Use an imported or saved rule profile.",
            )
            .clicked()
            {
                self.planning_source = PlanningSource::RuleProfile;
            }
        });

        ui.add_space(8.0);
        ui.label(format!("Selected style: {}", self.selected_style_title()));

        if self.planning_source == PlanningSource::RuleProfile {
            ui.add_space(8.0);
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(248, 242, 233))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
                .inner_margin(egui::Margin::same(12.0))
                .show(ui, |ui| {
                    ui.label(RichText::new("Custom rule profile").strong());
                    if let Some(profile) = &self.loaded_profile {
                        ui.label(format!("Using {}.", profile.display_label()));
                    } else {
                        ui.colored_label(
                            Color32::from_rgb(170, 110, 35),
                            "Choose a profile before analyzing with Custom Rules.",
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Import profile...").clicked() {
                            self.import_rule_profile();
                        }
                        if ui.button("Open Rules").clicked() {
                            self.active_section = AppSection::Rules;
                        }
                    });
                });
        }

        ui.add_space(10.0);
        egui::CollapsingHeader::new("Advanced options")
            .default_open(false)
            .show(ui, |ui| {
                ui.checkbox(&mut self.include_subfolders, "Include subfolders");
                ui.label("By default, smartfolder analyzes only the selected folder.");
            });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.is_analyzing()
                        && !self.is_applying()
                        && !self.is_undoing()
                        && self.can_run_analysis(),
                    egui::Button::new(RichText::new("Analyze Folder").strong())
                        .min_size(egui::vec2(160.0, 34.0)),
                )
                .clicked()
            {
                self.start_analysis();
            }

            if ui
                .add_enabled(self.is_analyzing(), egui::Button::new("Cancel Analysis"))
                .clicked()
            {
                self.cancel_analysis();
            }

            if !self.can_run_analysis() && self.planning_source == PlanningSource::RuleProfile {
                ui.label("Import a rule profile to analyze with Custom Rules.");
            } else if !self.has_selected_root() {
                ui.label("Choose a folder to enable analysis.");
            } else {
                ui.label("Analysis creates a preview first. Nothing moves until you confirm.");
            }
        });

        if self.is_analyzing() {
            ui.add_space(10.0);
            if let Some(progress) = &self.analysis_progress {
                render_analysis_progress(ui, ctx, progress);
            }
        }

        if self.is_applying() {
            ui.add_space(10.0);
            if let Some(progress) = &self.apply_progress {
                render_apply_progress(ui, progress);
            }
        }

        if self.is_undoing() {
            ui.add_space(10.0);
            render_undo_progress(ui);
        }

        if let Some(result) = &self.analysis_result {
            ui.add_space(14.0);
            render_plan_summary(ui, result);

            ui.add_space(10.0);
            render_apply_entry(
                ui,
                result,
                self.is_applying(),
                self.apply_result.is_some(),
                &mut self.show_apply_confirmation,
            );

            if let Some(apply_result) = &self.apply_result {
                ui.add_space(10.0);
                render_apply_result(
                    ui,
                    apply_result,
                    self.is_analyzing() || self.is_applying() || self.is_undoing(),
                    history_action,
                );
            }

            if !result.warning_messages.is_empty() {
                ui.add_space(10.0);
                egui::CollapsingHeader::new("Warnings and exclusions")
                    .default_open(false)
                    .show(ui, |ui| {
                        for warning in &result.warning_messages {
                            truncated_label(ui, &format!("- {warning}"));
                        }
                    });
            }

            ui.add_space(10.0);
            render_preview_controls(
                ui,
                result,
                self.preview_filter,
                self.preview_offset,
                preview_action,
            );
            let mut selected_preview_row = self.selected_preview_row;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(360.0)
                .show(ui, |ui| {
                    render_preview_rows(ui, result, &mut selected_preview_row);
                });
            self.selected_preview_row = selected_preview_row;

            ui.add_space(10.0);
            render_preview_detail(ui, result, self.selected_preview_row);
        }
    }

    fn render_activity_screen(
        &mut self,
        ui: &mut egui::Ui,
        history_action: &mut Option<HistoryAction>,
    ) {
        render_screen_heading(
            ui,
            "Activity",
            "Review recent organization runs for this folder and undo changes when you need to restore the original layout.",
        );
        self.render_status_messages(ui);
        ui.add_space(10.0);
        render_transaction_history(
            ui,
            &self.transaction_rows,
            self.active_root().as_deref(),
            self.transaction_message.as_deref(),
            self.undo_result.as_ref(),
            self.transaction_detail.as_ref(),
            self.transaction_detail_message.as_deref(),
            self.show_recovery_log,
            self.is_analyzing() || self.is_applying() || self.is_undoing(),
            history_action,
        );
    }

    fn render_rules_screen(&mut self, ui: &mut egui::Ui) {
        render_screen_heading(
            ui,
            "Rules",
            "Use a built-in style or manage a simple custom rule profile for folders that need a more specific destination.",
        );
        self.render_status_messages(ui);
        ui.add_space(10.0);

        ui.label(RichText::new("Built-in styles").strong().size(18.0));
        ui.label(
            "These are the ready-made organization styles available from the Organize screen.",
        );
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if render_style_card(
                ui,
                self.planning_source == PlanningSource::BuiltIn && self.mode == BuiltInMode::Type,
                "By Type",
                "Images / PDFs / Videos",
                "Use this when a folder has mixed file kinds.",
            )
            .clicked()
            {
                self.planning_source = PlanningSource::BuiltIn;
                self.mode = BuiltInMode::Type;
                self.active_section = AppSection::Organize;
            }

            if render_style_card(
                ui,
                self.planning_source == PlanningSource::BuiltIn && self.mode == BuiltInMode::Date,
                "By Date",
                "2026 / May / 13",
                "Use this when time is the clearest grouping.",
            )
            .clicked()
            {
                self.planning_source = PlanningSource::BuiltIn;
                self.mode = BuiltInMode::Date;
                self.active_section = AppSection::Organize;
            }

            if render_style_card(
                ui,
                self.planning_source == PlanningSource::BuiltIn
                    && self.mode == BuiltInMode::TypeYear,
                "Type + Date",
                "Images / 2026 / May",
                "Use this for large folders that need both structure and time.",
            )
            .clicked()
            {
                self.planning_source = PlanningSource::BuiltIn;
                self.mode = BuiltInMode::TypeYear;
                self.active_section = AppSection::Organize;
            }
        });

        ui.add_space(12.0);
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(250, 246, 240))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                ui.label(RichText::new("Custom profile").strong().size(18.0));
                ui.label("A profile gives one folder a specific rule without editing raw TOML.");
                ui.add_space(8.0);
                if let Some(profile) = &self.loaded_profile {
                    render_status_chip(
                        ui,
                        "Profile loaded",
                        Color32::from_rgb(92, 128, 78),
                        Color32::from_rgb(231, 241, 228),
                    );
                    ui.label(format!("Current profile: {}", profile.profile.profile_id));
                    truncated_label(ui, &format!("Saved at: {}", profile.path.display()));
                } else {
                    render_status_chip(
                        ui,
                        "No profile selected",
                        Color32::from_rgb(170, 110, 35),
                        Color32::from_rgb(248, 238, 217),
                    );
                    ui.label("Create a simple profile below or import an existing TOML profile.");
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Use Custom Rules").clicked() {
                        self.planning_source = PlanningSource::RuleProfile;
                        self.active_section = AppSection::Organize;
                    }
                    if ui.button("Validate profile").clicked() {
                        self.validate_profile_editor();
                    }
                    if ui.button("Save profile").clicked() {
                        self.save_profile_from_editor();
                    }
                });
                egui::CollapsingHeader::new("Advanced TOML actions")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(
                            "Import or export TOML when you want to share or hand-edit a profile.",
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Import TOML...").clicked() {
                                self.import_rule_profile();
                            }
                            if ui.button("Export TOML...").clicked() {
                                self.export_profile_from_editor();
                            }
                        });
                    });
            });

        ui.add_space(12.0);
        self.render_profile_editor(ui);
    }

    fn render_settings_screen(&mut self, ui: &mut egui::Ui) {
        render_screen_heading(
            ui,
            "Settings",
            "Keep the app clean, confirm Explorer launch behavior, and preserve the safer default of analyzing only the selected folder.",
        );
        self.render_status_messages(ui);
        ui.add_space(10.0);

        ui.horizontal_wrapped(|ui| {
            render_info_card(
                ui,
                "Safety defaults",
                "smartfolder previews first, never overwrites existing files, and keeps subfolders opt-in.",
                "No automatic organizing",
            );
            render_info_card(
                ui,
                "History",
                "Restore history is recorded before files move so Undo Changes can restore completed moves.",
                "Undo-ready workflow",
            );
            render_info_card(
                ui,
                "Appearance",
                "The v2 interface currently uses one built-in visual style while the workflow stabilizes.",
                "No saved theme setting yet",
            );
        });

        ui.add_space(10.0);
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(250, 246, 240))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                ui.label(RichText::new("Explorer integration").strong().size(18.0));
                ui.label("The folder context menu entry should read Organize with smartfolder.");
                render_safety_line(ui, "It only opens the app with the clicked folder selected.");
                render_safety_line(ui, "It never organizes files directly from Explorer.");
                ui.add_space(8.0);
                ui.label("Register or unregister it with scripts/register-explorer-launcher.ps1 after building the release GUI.");
            });

        ui.add_space(10.0);
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(250, 246, 240))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                ui.label(RichText::new("Storage maintenance").strong().size(18.0));
                ui.label("Cleanup removes old cached analysis sessions and preview pages. It does not remove restore history for organized files.");
                if ui
                    .add_enabled(
                        !self.is_analyzing() && !self.is_applying() && !self.is_undoing(),
                        egui::Button::new("Clean old session data"),
                    )
                    .clicked()
                {
                    self.cleanup_old_sessions();
                }
            });
    }

    fn render_profile_editor(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Profile editor")
            .default_open(self.loaded_profile.is_none())
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Profile id");
                    ui.add_sized(
                        [160.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.profile_id),
                    );
                    ui.label("Rule name");
                    ui.add_sized(
                        [180.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.rule_name),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Destination");
                    ui.add_sized(
                        [360.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.destination),
                    );
                    ui.label("Priority");
                    ui.add_sized(
                        [80.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.priority),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Extensions");
                    ui.add_sized(
                        [180.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.extensions),
                    );
                    ui.label("Filename contains");
                    ui.add_sized(
                        [220.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.filename_contains),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Path contains");
                    ui.add_sized(
                        [220.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.path_contains),
                    );
                    ui.label("Min bytes");
                    ui.add_sized(
                        [90.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.min_size_bytes),
                    );
                    ui.label("Max bytes");
                    ui.add_sized(
                        [90.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.max_size_bytes),
                    );
                    ui.label("Year");
                    ui.add_sized(
                        [70.0, 22.0],
                        egui::TextEdit::singleline(&mut self.profile_editor.year),
                    );
                });

                ui.horizontal(|ui| {
                    if ui.button("Validate profile").clicked() {
                        self.validate_profile_editor();
                    }
                    if ui.button("Save profile").clicked() {
                        self.save_profile_from_editor();
                    }
                    if ui.button("Export profile...").clicked() {
                        self.export_profile_from_editor();
                    }
                    if ui.button("New profile").clicked() {
                        self.profile_editor = ProfileEditorState::default();
                        self.loaded_profile = None;
                    }
                });
            });
    }

    fn apply_preview_action(&mut self, action: PreviewAction) {
        let Some(result) = &self.analysis_result else {
            return;
        };

        let filter = match action {
            PreviewAction::Filter(filter) => filter,
            PreviewAction::Previous => self.preview_filter,
            PreviewAction::Next => self.preview_filter,
        };

        let total_rows = filter.count(&result.preview_counts);
        let offset = match action {
            PreviewAction::Filter(_) => 0,
            PreviewAction::Previous => self.preview_offset.saturating_sub(PREVIEW_PAGE_SIZE),
            PreviewAction::Next => {
                let next = self.preview_offset + PREVIEW_PAGE_SIZE;
                if next >= total_rows {
                    self.preview_offset
                } else {
                    next
                }
            }
        };

        self.reload_preview_page(filter, offset);
    }

    fn reload_preview_page(&mut self, filter: PreviewFilter, offset: usize) {
        let Some(result) = &self.analysis_result else {
            return;
        };
        let session_id = result.session_id.clone();
        let root = result.root.clone();

        match load_preview_page(&session_id, &root, filter, offset) {
            Ok(page) => {
                if let Some(result) = &mut self.analysis_result {
                    result.preview_rows = page.rows;
                    result.preview_total_rows = page.total_rows;
                }
                self.preview_filter = filter;
                self.preview_offset = offset;
                self.sync_preview_selection();
                self.error_message = None;
            }
            Err(message) => {
                self.error_message = Some(message);
            }
        }
    }
}

impl eframe::App for SmartfolderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_analysis();
        self.poll_apply();
        self.poll_undo();
        apply_visual_style(ctx);

        if self.is_analyzing() || self.is_applying() || self.is_undoing() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        let mut preview_action = None;
        let mut history_action = None;

        egui::SidePanel::left("app-shell-nav")
            .resizable(false)
            .exact_width(SHELL_NAV_WIDTH)
            .show(ctx, |ui| self.render_sidebar(ui));

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match self.active_section {
                    AppSection::Organize => {
                        self.render_organize_screen(
                            ui,
                            ctx,
                            &mut preview_action,
                            &mut history_action,
                        );
                    }
                    AppSection::Activity => {
                        self.render_activity_screen(ui, &mut history_action);
                    }
                    AppSection::Rules => {
                        self.render_rules_screen(ui);
                    }
                    AppSection::Settings => {
                        self.render_settings_screen(ui);
                    }
                });
        });

        if let Some(action) = preview_action {
            self.apply_preview_action(action);
        }

        if let Some(action) = history_action {
            match action {
                HistoryAction::Refresh => self.refresh_transaction_history(),
                HistoryAction::ViewDetails(transaction_id) => {
                    self.load_transaction_detail(&transaction_id);
                }
                HistoryAction::CloseDetails => {
                    self.transaction_detail = None;
                    self.transaction_detail_message = None;
                }
                HistoryAction::ToggleRecoveryLog => {
                    self.show_recovery_log = !self.show_recovery_log;
                }
                HistoryAction::ConfirmUndo(transaction_id) => {
                    self.show_undo_confirmation = Some(transaction_id);
                }
            }
        }

        if self.show_apply_confirmation {
            if let Some(result) = &self.analysis_result {
                let mut confirmed = false;
                let mut dismissed = false;
                render_apply_confirmation(ctx, result, &mut confirmed, &mut dismissed);
                if confirmed {
                    self.start_apply();
                } else if dismissed {
                    self.show_apply_confirmation = false;
                }
            } else {
                self.show_apply_confirmation = false;
            }
        }

        if let Some(transaction_id) = self.show_undo_confirmation.clone() {
            let mut confirmed = false;
            let mut dismissed = false;
            render_undo_confirmation(ctx, &transaction_id, &mut confirmed, &mut dismissed);
            if confirmed {
                self.start_undo(transaction_id);
            } else if dismissed {
                self.show_undo_confirmation = None;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewFilter {
    All,
    Ready,
    NeedsAttention,
}

impl PreviewFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Ready => "Ready",
            Self::NeedsAttention => "Needs attention",
        }
    }

    fn core_filter(self) -> PlanOperationFilter {
        match self {
            Self::All => PlanOperationFilter::All,
            Self::Ready => PlanOperationFilter::Ready,
            Self::NeedsAttention => PlanOperationFilter::NeedsAttention,
        }
    }

    fn count(self, counts: &PreviewCounts) -> usize {
        match self {
            Self::All => counts.all,
            Self::Ready => counts.ready,
            Self::NeedsAttention => counts.needs_attention,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PreviewAction {
    Filter(PreviewFilter),
    Previous,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanningSource {
    BuiltIn,
    RuleProfile,
}

#[derive(Debug, Clone)]
enum AnalysisPlanSource {
    BuiltIn(BuiltInMode),
    RuleProfile(RuleProfile),
}

impl AnalysisPlanSource {
    fn plan_mode(&self) -> PlanMode {
        match self {
            Self::BuiltIn(mode) => PlanMode::BuiltIn(*mode),
            Self::RuleProfile(profile) => PlanMode::RuleProfile {
                profile_id: profile.profile_id.clone(),
            },
        }
    }

    fn plan_options(
        self,
        plan_id: impl Into<String>,
        created_at: chrono::DateTime<Utc>,
    ) -> PlanOptions {
        match self {
            Self::BuiltIn(mode) => PlanOptions::built_in(mode, plan_id, created_at),
            Self::RuleProfile(profile) => PlanOptions::rule_profile(profile, plan_id, created_at),
        }
    }
}

#[derive(Debug, Clone)]
struct LoadedRuleProfile {
    path: PathBuf,
    profile: RuleProfile,
}

impl LoadedRuleProfile {
    fn display_label(&self) -> String {
        format!("{} ({})", self.profile.profile_id, self.path.display())
    }
}

fn load_rule_profile_from_path(path: &Path) -> std::result::Result<RuleProfile, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read rule profile {}: {error}", path.display()))?;
    RuleProfile::from_toml(&content)
        .map_err(|error| format!("Invalid rule profile {}: {error}", path.display()))
}

fn save_profile_from_editor(
    editor: &ProfileEditorState,
) -> std::result::Result<LoadedRuleProfile, String> {
    let profile = editor.to_profile()?;
    let path = app_local_profile_path(&profile.profile_id)?;
    write_rule_profile(&path, &profile)?;
    Ok(LoadedRuleProfile { path, profile })
}

fn export_profile_from_editor(
    editor: &ProfileEditorState,
) -> std::result::Result<Option<PathBuf>, String> {
    let profile = editor.to_profile()?;
    let file_name = format!("{}.toml", profile_file_stem(&profile.profile_id));
    let Some(path) = rfd::FileDialog::new()
        .set_file_name(&file_name)
        .add_filter("TOML rule profile", &["toml"])
        .save_file()
    else {
        return Ok(None);
    };

    write_rule_profile(&path, &profile)?;
    Ok(Some(path))
}

fn write_rule_profile(path: &Path, profile: &RuleProfile) -> std::result::Result<(), String> {
    let content = profile
        .to_toml_string()
        .map_err(|error| format!("Failed to serialize rule profile: {error}"))?;
    fs::write(path, content)
        .map_err(|error| format!("Failed to write rule profile {}: {error}", path.display()))
}

fn app_local_profile_path(profile_id: &str) -> std::result::Result<PathBuf, String> {
    let directory = ensure_profiles_dir()
        .map_err(|error| format!("Failed to create profile directory: {error}"))?;
    Ok(directory.join(format!("{}.toml", profile_file_stem(profile_id))))
}

fn profile_file_stem(profile_id: &str) -> String {
    let stem = profile_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    if stem.is_empty() {
        "profile".to_string()
    } else {
        stem
    }
}

#[derive(Debug, Clone)]
struct ProfileEditorState {
    profile_id: String,
    rule_name: String,
    destination: String,
    priority: String,
    extensions: String,
    filename_contains: String,
    path_contains: String,
    min_size_bytes: String,
    max_size_bytes: String,
    year: String,
}

impl Default for ProfileEditorState {
    fn default() -> Self {
        Self {
            profile_id: "my-profile".to_string(),
            rule_name: "PDFs".to_string(),
            destination: "Documents/PDFs".to_string(),
            priority: "10".to_string(),
            extensions: "pdf".to_string(),
            filename_contains: String::new(),
            path_contains: String::new(),
            min_size_bytes: String::new(),
            max_size_bytes: String::new(),
            year: String::new(),
        }
    }
}

impl ProfileEditorState {
    fn from_profile(profile: &RuleProfile) -> Self {
        let Some(rule) = profile.rules.first() else {
            return Self {
                profile_id: profile.profile_id.clone(),
                ..Self::default()
            };
        };

        Self {
            profile_id: profile.profile_id.clone(),
            rule_name: rule.name.clone(),
            destination: rule.destination.clone(),
            priority: rule
                .priority
                .map_or_else(String::new, |value| value.to_string()),
            extensions: rule.extensions.join(", "),
            filename_contains: rule.filename_contains.join(", "),
            path_contains: rule.path_contains.join(", "),
            min_size_bytes: rule
                .min_size_bytes
                .map_or_else(String::new, |value| value.to_string()),
            max_size_bytes: rule
                .max_size_bytes
                .map_or_else(String::new, |value| value.to_string()),
            year: rule
                .year
                .map_or_else(String::new, |value| value.to_string()),
        }
    }

    fn to_profile(&self) -> std::result::Result<RuleProfile, String> {
        let profile = RuleProfile {
            profile_id: self.profile_id.trim().to_string(),
            rules: vec![CustomRule {
                name: self.rule_name.trim().to_string(),
                destination: self.destination.trim().to_string(),
                priority: parse_optional_number(&self.priority, "priority")?,
                extensions: comma_separated_values(&self.extensions),
                filename_contains: comma_separated_values(&self.filename_contains),
                path_contains: comma_separated_values(&self.path_contains),
                min_size_bytes: parse_optional_number(&self.min_size_bytes, "min_size_bytes")?,
                max_size_bytes: parse_optional_number(&self.max_size_bytes, "max_size_bytes")?,
                year: parse_optional_number(&self.year, "year")?,
            }],
        };
        profile.validate().map_err(|error| error.to_string())?;
        Ok(profile)
    }
}

fn parse_optional_number<T>(value: &str, label: &str) -> std::result::Result<Option<T>, String>
where
    T: std::str::FromStr,
{
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<T>()
        .map(Some)
        .map_err(|_| format!("{label} must be a valid number"))
}

fn comma_separated_values(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[derive(Debug, Clone)]
enum HistoryAction {
    Refresh,
    ViewDetails(String),
    CloseDetails,
    ToggleRecoveryLog,
    ConfirmUndo(String),
}

#[derive(Debug, Clone)]
struct AnalysisProgress {
    headline: String,
    detail: String,
    fraction: Option<f32>,
    entries_seen: usize,
    records_collected: usize,
    folders_scanned: usize,
    entries_skipped: usize,
    warnings: usize,
    plan_processed: usize,
    plan_total: usize,
    moves_proposed: usize,
    ambiguous_files: usize,
    conflicts: usize,
    skipped: usize,
}

impl AnalysisProgress {
    fn preparing(root: PathBuf) -> Self {
        Self {
            headline: "Getting ready to analyze".to_string(),
            detail: root.display().to_string(),
            fraction: Some(0.0),
            entries_seen: 0,
            records_collected: 0,
            folders_scanned: 0,
            entries_skipped: 0,
            warnings: 0,
            plan_processed: 0,
            plan_total: 0,
            moves_proposed: 0,
            ambiguous_files: 0,
            conflicts: 0,
            skipped: 0,
        }
    }

    fn scanning(progress: StreamingScanProgress) -> Self {
        let detail = progress.current_path.as_ref().map_or_else(
            || progress.root.display().to_string(),
            |path| path.display().to_string(),
        );
        Self {
            headline: "Scanning the selected folder".to_string(),
            detail,
            fraction: None,
            entries_seen: progress.summary.entries_seen,
            records_collected: progress.summary.records_collected,
            folders_scanned: progress.summary.folders_scanned,
            entries_skipped: progress.summary.entries_skipped,
            warnings: progress.summary.warnings,
            plan_processed: 0,
            plan_total: 0,
            moves_proposed: 0,
            ambiguous_files: 0,
            conflicts: 0,
            skipped: 0,
        }
    }

    fn planning(progress: PlanGenerationProgress) -> Self {
        let fraction = if progress.total_records == 0 {
            None
        } else {
            Some(progress.processed_records as f32 / progress.total_records as f32)
        };
        let detail = progress.current_path.as_ref().map_or_else(
            || "Checking rules and destination conflicts".to_string(),
            |path| path.display().to_string(),
        );
        Self {
            headline: "Working out safe destinations".to_string(),
            detail,
            fraction,
            entries_seen: 0,
            records_collected: progress.total_records,
            folders_scanned: 0,
            entries_skipped: 0,
            warnings: 0,
            plan_processed: progress.processed_records,
            plan_total: progress.total_records,
            moves_proposed: progress.operations_created,
            ambiguous_files: progress.ambiguous_files,
            conflicts: progress.conflicts,
            skipped: progress.skipped,
        }
    }

    fn loading_preview() -> Self {
        Self {
            headline: "Preparing your preview".to_string(),
            detail: "Loading the first page of planned changes".to_string(),
            fraction: Some(0.95),
            entries_seen: 0,
            records_collected: 0,
            folders_scanned: 0,
            entries_skipped: 0,
            warnings: 0,
            plan_processed: 0,
            plan_total: 0,
            moves_proposed: 0,
            ambiguous_files: 0,
            conflicts: 0,
            skipped: 0,
        }
    }

    fn cancelling() -> Self {
        Self {
            headline: "Cancelling analysis".to_string(),
            detail: "Stopping after the current file-system operation".to_string(),
            fraction: None,
            entries_seen: 0,
            records_collected: 0,
            folders_scanned: 0,
            entries_skipped: 0,
            warnings: 0,
            plan_processed: 0,
            plan_total: 0,
            moves_proposed: 0,
            ambiguous_files: 0,
            conflicts: 0,
            skipped: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct AnalysisOutput {
    session_id: String,
    plan_id: String,
    root: PathBuf,
    summary: PlanSummary,
    warning_messages: Vec<String>,
    preview_counts: PreviewCounts,
    preview_total_rows: usize,
    preview_rows: Vec<PreviewRow>,
}

#[derive(Debug, Clone, Copy, Default)]
struct PreviewCounts {
    all: usize,
    ready: usize,
    needs_attention: usize,
}

#[derive(Debug, Clone)]
struct PreviewPage {
    rows: Vec<PreviewRow>,
    total_rows: usize,
}

#[derive(Debug, Clone)]
struct PreviewRow {
    file_name: String,
    original_folder: String,
    target_folder: String,
    source_full_path: String,
    destination_full_path: String,
    reason: String,
    status: String,
}

#[derive(Debug, Clone)]
struct ApplyProgress {
    headline: String,
    detail: String,
    processed: usize,
    total: usize,
    completed: usize,
    skipped: usize,
    failed: usize,
}

impl ApplyProgress {
    fn preparing(total: usize) -> Self {
        Self {
            headline: "Getting ready to organize".to_string(),
            detail: "Creating the restore record before any files move".to_string(),
            processed: 0,
            total,
            completed: 0,
            skipped: 0,
            failed: 0,
        }
    }

    fn applying(progress: StoredApplyProgress) -> Self {
        let detail = progress.current_path.as_ref().map_or_else(
            || "Starting safe file moves".to_string(),
            |path| path.display().to_string(),
        );
        Self {
            headline: "Organizing files".to_string(),
            detail,
            processed: progress.processed,
            total: progress.total,
            completed: progress.completed,
            skipped: progress.skipped,
            failed: progress.failed,
        }
    }
}

#[derive(Debug, Clone)]
struct ApplyOutput {
    transaction_id: String,
    journal_path: PathBuf,
    completed: usize,
    skipped: usize,
    failed: usize,
}

#[derive(Debug, Clone)]
struct TransactionRow {
    transaction_id: String,
    root: PathBuf,
    root_label: String,
    status: TransactionStatus,
    started_at: String,
    reason_summary: String,
    completed: usize,
    skipped: usize,
    failed: usize,
    rolled_back: usize,
    pending: usize,
    total_operations: usize,
}

#[derive(Debug, Clone)]
struct TransactionDetail {
    transaction_id: String,
    plan_id: String,
    root: String,
    status: TransactionStatus,
    started_at: String,
    completed_at: String,
    reason_summary: String,
    operation_counts: TransactionOperationCounts,
    operation_rows: Vec<TransactionOperationRow>,
    total_operations: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct TransactionOperationCounts {
    pending: usize,
    completed: usize,
    skipped: usize,
    failed: usize,
    rolled_back: usize,
}

impl TransactionOperationCounts {
    fn total(self) -> usize {
        self.pending + self.completed + self.skipped + self.failed + self.rolled_back
    }
}

#[derive(Debug, Clone)]
struct TransactionOperationRow {
    operation_id: String,
    source: String,
    destination: String,
    reason: String,
    status: OperationStatus,
    error: String,
}

#[derive(Debug, Clone)]
struct UndoOutput {
    transaction_id: String,
    journal_path: PathBuf,
    rolled_back: usize,
    skipped: usize,
    failed: usize,
}

fn analyze_root(
    root: &Path,
    plan_source: AnalysisPlanSource,
    include_subfolders: bool,
    cancellation: &CancellationToken,
    sender: &Sender<AnalysisEvent>,
) -> AnalysisMessage {
    let now = Utc::now();
    let plan_id = format!("plan_{}", now.format("%Y%m%d%H%M%S"));
    let plan_mode = plan_source.plan_mode();
    let mut store = SqliteSessionStore::open_default()
        .map_err(|error| format!("Failed to open session store: {error}"))?;
    let session_id = store
        .create_session(root, &plan_mode, now)
        .map_err(|error| format!("Failed to create analysis session: {error}"))?;

    let scan = stream_scan_to_store(
        root,
        &mut store,
        &session_id,
        include_subfolders,
        cancellation,
        sender,
    )?;
    if scan.cancelled {
        let _ = store.delete_session(&session_id);
        return Err("Analysis cancelled.".to_string());
    }

    let plan_options = plan_source.plan_options(plan_id, now);
    let plan = generate_plan_to_store_with_progress_and_cancellation(
        root,
        &mut store,
        &session_id,
        &plan_options,
        1_000,
        cancellation,
        &mut |progress| send_progress(sender, AnalysisProgress::planning(progress)),
    )
    .map_err(|error| format!("Failed to generate plan for {}: {error}", root.display()))?;
    send_progress(sender, AnalysisProgress::loading_preview());
    let preview_counts = load_preview_counts_from_store(&store, &session_id)?;
    let preview_page =
        load_preview_page_from_store(&store, &session_id, root, PreviewFilter::All, 0)?;
    let warning_messages = store
        .warning_messages(&session_id)
        .map_err(|error| format!("Failed to load warning messages: {error}"))?;

    Ok(AnalysisOutput {
        session_id,
        plan_id: plan.plan_id,
        root: plan.root,
        summary: plan.summary,
        warning_messages,
        preview_counts,
        preview_total_rows: preview_page.total_rows,
        preview_rows: preview_page.rows,
    })
}

fn load_preview_page(
    session_id: &str,
    root: &Path,
    filter: PreviewFilter,
    offset: usize,
) -> std::result::Result<PreviewPage, String> {
    let store = SqliteSessionStore::open_default()
        .map_err(|error| format!("Failed to open session store: {error}"))?;
    load_preview_page_from_store(&store, session_id, root, filter, offset)
}

fn load_preview_page_from_store(
    store: &SqliteSessionStore,
    session_id: &str,
    root: &Path,
    filter: PreviewFilter,
    offset: usize,
) -> std::result::Result<PreviewPage, String> {
    let total_rows = store
        .plan_operation_count(session_id, filter.core_filter())
        .map_err(|error| format!("Failed to count preview operations: {error}"))?;
    let operations = store
        .plan_operations_page_filtered(session_id, filter.core_filter(), offset, PREVIEW_PAGE_SIZE)
        .map_err(|error| format!("Failed to load preview operations: {error}"))?;
    Ok(PreviewPage {
        rows: preview_rows(&operations, root),
        total_rows,
    })
}

fn load_preview_counts_from_store(
    store: &SqliteSessionStore,
    session_id: &str,
) -> std::result::Result<PreviewCounts, String> {
    Ok(PreviewCounts {
        all: store
            .plan_operation_count(session_id, PlanOperationFilter::All)
            .map_err(|error| format!("Failed to count preview operations: {error}"))?,
        ready: store
            .plan_operation_count(session_id, PlanOperationFilter::Ready)
            .map_err(|error| format!("Failed to count ready operations: {error}"))?,
        needs_attention: store
            .plan_operation_count(session_id, PlanOperationFilter::NeedsAttention)
            .map_err(|error| format!("Failed to count operations needing attention: {error}"))?,
    })
}

fn stream_scan_to_store(
    root: &Path,
    store: &mut SqliteSessionStore,
    session_id: &str,
    include_subfolders: bool,
    cancellation: &CancellationToken,
    sender: &Sender<AnalysisEvent>,
) -> std::result::Result<StreamingScanResult, String> {
    store
        .begin_write_batch()
        .map_err(|error| format!("Failed to start scan storage batch: {error}"))?;
    let scan_result = (|| {
        let mut sink = SessionScanSink::new(store, session_id.to_string());
        let result = scan_folder_to_sink_with_progress(
            root,
            &ScanOptions {
                current_folder_only: !include_subfolders,
                ..ScanOptions::default()
            },
            cancellation,
            &mut sink,
            &mut |progress| send_progress(sender, AnalysisProgress::scanning(progress)),
        )
        .map_err(|error| format!("Failed to scan {}: {error}", root.display()))?;
        Ok(result)
    })();

    match scan_result {
        Ok(result) => {
            store
                .save_scan_summary(session_id, &result.summary)
                .map_err(|error| format!("Failed to save scan summary: {error}"))?;
            store
                .commit_write_batch()
                .map_err(|error| format!("Failed to commit scan storage batch: {error}"))?;
            Ok(result)
        }
        Err(message) => {
            let _ = store.rollback_write_batch();
            Err(message)
        }
    }
}

fn apply_session_plan(
    session_id: &str,
    plan_id: &str,
    root: &Path,
    transaction_id: &str,
    sender: &Sender<ApplyEvent>,
) -> ApplyMessage {
    let store = SqliteSessionStore::open_default()
        .map_err(|error| format!("Failed to open session store: {error}"))?;
    let options = ApplyOptions::new(transaction_id.to_string(), Utc::now());
    let summary = apply_stored_plan_with_progress(
        &store,
        session_id,
        plan_id.to_string(),
        root.to_path_buf(),
        &options,
        PREVIEW_PAGE_SIZE,
        &mut |progress| {
            let _ = sender.send(ApplyEvent::Progress(ApplyProgress::applying(progress)));
        },
    )
    .map_err(|error| format!("Failed to organize ready files: {error}"))?;

    Ok(apply_output(summary))
}

fn apply_output(summary: ApplySummary) -> ApplyOutput {
    ApplyOutput {
        transaction_id: summary.transaction_id,
        journal_path: summary.journal_path,
        completed: summary.completed,
        skipped: summary.skipped,
        failed: summary.failed,
    }
}

fn load_transaction_rows() -> std::result::Result<Vec<TransactionRow>, String> {
    list_transactions()
        .map_err(|error| format!("Failed to load transactions: {error}"))
        .map(|transactions| transactions.into_iter().map(transaction_row).collect())
}

fn transaction_row(summary: TransactionSummary) -> TransactionRow {
    let journal = inspect_transaction(&summary.transaction_id).ok();
    let counts = journal
        .as_ref()
        .map(|journal| transaction_operation_counts(&journal.operations))
        .unwrap_or_default();
    let reason_summary = journal.as_ref().map_or_else(
        || "No rule reason recorded in this journal.".to_string(),
        |journal| transaction_reason_summary(&journal.operations),
    );
    let total_operations = counts.total();

    TransactionRow {
        transaction_id: summary.transaction_id,
        root_label: summary.root.display().to_string(),
        root: summary.root,
        status: summary.status,
        started_at: summary.started_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        reason_summary,
        completed: counts.completed,
        skipped: counts.skipped,
        failed: counts.failed,
        rolled_back: counts.rolled_back,
        pending: counts.pending,
        total_operations,
    }
}

fn load_transaction_detail(transaction_id: &str) -> std::result::Result<TransactionDetail, String> {
    inspect_transaction(transaction_id)
        .map(transaction_detail)
        .map_err(|error| format!("Failed to inspect transaction: {error}"))
}

fn transaction_detail(journal: smartfolder_core::model::TransactionJournal) -> TransactionDetail {
    let operation_counts = transaction_operation_counts(&journal.operations);
    let total_operations = journal.operations.len();
    let operation_rows = journal
        .operations
        .iter()
        .take(TRANSACTION_DETAIL_ROW_LIMIT)
        .map(transaction_operation_row)
        .collect();

    TransactionDetail {
        transaction_id: journal.transaction_id,
        plan_id: journal.plan_id,
        root: journal.root.display().to_string(),
        status: journal.status,
        started_at: journal.started_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        completed_at: journal.completed_at.map_or_else(
            || "not completed".to_string(),
            |completed| completed.format("%Y-%m-%d %H:%M:%S").to_string(),
        ),
        reason_summary: transaction_reason_summary(&journal.operations),
        operation_counts,
        operation_rows,
        total_operations,
    }
}

fn transaction_operation_counts(
    operations: &[smartfolder_core::model::TransactionOperation],
) -> TransactionOperationCounts {
    let mut counts = TransactionOperationCounts::default();
    for operation in operations {
        match operation.status {
            OperationStatus::Pending => counts.pending += 1,
            OperationStatus::Completed => counts.completed += 1,
            OperationStatus::Skipped => counts.skipped += 1,
            OperationStatus::Failed => counts.failed += 1,
            OperationStatus::RolledBack => counts.rolled_back += 1,
        }
    }
    counts
}

fn transaction_operation_row(
    operation: &smartfolder_core::model::TransactionOperation,
) -> TransactionOperationRow {
    TransactionOperationRow {
        operation_id: operation.operation_id.clone(),
        source: operation.source.display().to_string(),
        destination: operation.destination.display().to_string(),
        reason: operation
            .reason
            .clone()
            .unwrap_or_else(|| "not recorded".to_string()),
        status: operation.status,
        error: operation.error.as_ref().map_or_else(String::new, |error| {
            format!("{:?}: {}", error.code, error.message)
        }),
    }
}

fn transaction_reason_summary(
    operations: &[smartfolder_core::model::TransactionOperation],
) -> String {
    let mut reason_counts = BTreeMap::<String, usize>::new();
    for operation in operations {
        if let Some(reason) = &operation.reason {
            *reason_counts.entry(reason.clone()).or_default() += 1;
        }
    }

    let Some((reason, count)) = reason_counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
    else {
        return "No rule reason recorded in this journal.".to_string();
    };

    if count == operations.len() {
        reason
    } else {
        format!(
            "{reason} ({count} of {} recorded operations)",
            operations.len()
        )
    }
}

fn undo_transaction_for_gui(transaction_id: &str) -> UndoMessage {
    undo_transaction(transaction_id)
        .map(undo_output)
        .map_err(|error| format!("Failed to undo changes: {error}"))
}

fn undo_output(summary: UndoSummary) -> UndoOutput {
    UndoOutput {
        transaction_id: summary.transaction_id,
        journal_path: summary.journal_path,
        rolled_back: summary.rolled_back,
        skipped: summary.skipped,
        failed: summary.failed,
    }
}

fn send_progress(sender: &Sender<AnalysisEvent>, progress: AnalysisProgress) {
    let _ = sender.send(AnalysisEvent::Progress(progress));
}

fn render_analysis_progress(ui: &mut egui::Ui, ctx: &egui::Context, progress: &AnalysisProgress) {
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

fn truncated_label(ui: &mut egui::Ui, text: &str) {
    let width = ui.available_width().max(120.0);
    ui.add_sized([width, 20.0], egui::Label::new(text).truncate());
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

fn render_apply_progress(ui: &mut egui::Ui, progress: &ApplyProgress) {
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

fn cleanup_old_session_data() -> std::result::Result<usize, String> {
    let mut store = SqliteSessionStore::open_default()
        .map_err(|error| format!("Failed to open session store: {error}"))?;
    let cutoff = Utc::now() - TimeDelta::days(14);
    let removed = store
        .cleanup_sessions_before(cutoff)
        .map_err(|error| format!("Failed to clean old session data: {error}"))?;
    if removed > 0 {
        store
            .compact()
            .map_err(|error| format!("Failed to compact session database: {error}"))?;
    }
    Ok(removed)
}

fn render_plan_summary(ui: &mut egui::Ui, result: &AnalysisOutput) {
    let ready = result.preview_counts.ready;
    let needs_attention = result.preview_counts.needs_attention;
    let left_in_place = result.summary.ambiguous_files + result.summary.skipped;

    ui.label(RichText::new("Analysis summary").strong().size(18.0));
    ui.label(plan_summary_headline(result));
    ui.label(plan_summary_detail(ready, needs_attention, left_in_place));
    ui.add_space(8.0);

    ui.horizontal_wrapped(|ui| {
        render_summary_card(
            ui,
            "Ready to organize",
            ready,
            "Files that can be moved safely right now.",
            Color32::from_rgb(231, 241, 228),
            Color32::from_rgb(92, 128, 78),
        );
        render_summary_card(
            ui,
            "Needs review",
            needs_attention,
            "Planned moves with conflicts or other issues.",
            Color32::from_rgb(248, 238, 217),
            Color32::from_rgb(170, 110, 35),
        );
        render_summary_card(
            ui,
            "Left untouched",
            left_in_place,
            "Files smartfolder chose not to move automatically.",
            Color32::from_rgb(243, 238, 234),
            Color32::from_rgb(116, 103, 90),
        );
    });

    ui.add_space(8.0);
    truncated_label(
        ui,
        &format!(
            "Scanned {} files. {} warning{} recorded.",
            result.summary.files_scanned,
            result.warning_messages.len(),
            plural(result.warning_messages.len())
        ),
    );

    if needs_attention == 0
        && result.summary.ambiguous_files == 0
        && result.warning_messages.is_empty()
    {
        ui.colored_label(
            Color32::from_rgb(70, 140, 90),
            "No conflicts or warnings found.",
        );
    } else {
        ui.colored_label(
            Color32::from_rgb(170, 110, 35),
            "Review the attention and warnings views before organizing these files.",
        );
    }
}

fn render_apply_entry(
    ui: &mut egui::Ui,
    result: &AnalysisOutput,
    is_applying: bool,
    already_applied: bool,
    show_confirmation: &mut bool,
) {
    let ready = result.preview_counts.ready;
    let can_apply = ready > 0 && !is_applying && !already_applied;
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(250, 246, 240))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Ready to organize").strong().size(18.0));
                    if already_applied {
                        ui.label("This preview has already been organized. Run Analyze Folder again for a fresh preview.");
                    } else if ready == 0 {
                        ui.label("No safe moves are ready to organize.");
                    } else {
                        ui.label(format!(
                            "{} ready file{} will move. Needs-review items will stay untouched.",
                            ready,
                            plural(ready)
                        ));
                        ui.label("smartfolder records restore history before moving anything.");
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            can_apply,
                            egui::Button::new(RichText::new("Organize Files").strong())
                                .min_size(egui::vec2(160.0, 38.0)),
                        )
                        .clicked()
                    {
                        *show_confirmation = true;
                    }
                });
            });
        });
}

fn render_apply_confirmation(
    ctx: &egui::Context,
    result: &AnalysisOutput,
    confirmed: &mut bool,
    dismissed: &mut bool,
) {
    egui::Window::new("Confirm organization")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new("Ready to organize these files").strong().size(20.0));
            ui.label(format!(
                "smartfolder will move {} ready file{} into organized folders and leave review items untouched.",
                result.preview_counts.ready,
                plural(result.preview_counts.ready)
            ));
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                render_summary_card(
                    ui,
                    "Ready",
                    result.preview_counts.ready,
                    "Files that will move now.",
                    Color32::from_rgb(231, 241, 228),
                    Color32::from_rgb(92, 128, 78),
                );
                render_summary_card(
                    ui,
                    "Needs review",
                    result.preview_counts.needs_attention,
                    "Planned moves left untouched.",
                    Color32::from_rgb(248, 238, 217),
                    Color32::from_rgb(170, 110, 35),
                );
            });

            ui.add_space(8.0);
            render_safety_line(ui, "Existing files will not be overwritten.");
            render_safety_line(ui, "Restore history is recorded before files move.");
            render_safety_line(ui, "Undo Changes will be available after completion.");

            if is_cloud_synced_path(&result.root) {
                ui.add_space(6.0);
                ui.colored_label(
                    Color32::from_rgb(170, 110, 35),
                    "This folder appears to be cloud-synced. Let sync settle before organizing and review the completion summary afterward.",
                );
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    *dismissed = true;
                }
                if ui.button("Organize Files").clicked() {
                    *confirmed = true;
                }
            });
        });
}

fn render_apply_result(
    ui: &mut egui::Ui,
    result: &ApplyOutput,
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(231, 241, 228))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(92, 128, 78)))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.label(RichText::new("Organization complete").strong().size(20.0));
            ui.label(format!(
                "{} file{} organized. Undo Changes is ready if you want to restore the original layout.",
                result.completed,
                plural(result.completed)
            ));
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                render_summary_card(
                    ui,
                    "Organized",
                    result.completed,
                    "Files moved successfully.",
                    Color32::from_rgb(231, 241, 228),
                    Color32::from_rgb(92, 128, 78),
                );
                render_summary_card(
                    ui,
                    "Skipped",
                    result.skipped,
                    "Files left untouched.",
                    Color32::from_rgb(243, 238, 234),
                    Color32::from_rgb(116, 103, 90),
                );
                render_summary_card(
                    ui,
                    "Failed",
                    result.failed,
                    "Files needing attention.",
                    Color32::from_rgb(248, 238, 217),
                    Color32::from_rgb(170, 110, 35),
                );
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(RichText::new("Undo Changes").strong())
                            .min_size(egui::vec2(150.0, 36.0)),
                    )
                    .clicked()
                {
                    *action = Some(HistoryAction::ConfirmUndo(result.transaction_id.clone()));
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("View Details"))
                    .clicked()
                {
                    *action = Some(HistoryAction::ViewDetails(result.transaction_id.clone()));
                }
            });
            ui.add_space(6.0);
            egui::CollapsingHeader::new("Restore history details")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(format!("Activity id: {}", result.transaction_id));
                    truncated_label(
                        ui,
                        &format!("Restore history: {}", result.journal_path.display()),
                    );
                });
        });
}

fn render_undo_progress(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label("Undoing changes and restoring original file locations...");
    });
}

fn render_transaction_history(
    ui: &mut egui::Ui,
    rows: &[TransactionRow],
    active_root: Option<&Path>,
    message: Option<&str>,
    undo_result: Option<&UndoOutput>,
    detail: Option<&TransactionDetail>,
    detail_message: Option<&str>,
    show_recovery_log: bool,
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Current folder activity").strong());
        if ui
            .add_enabled(!busy, egui::Button::new("Refresh"))
            .clicked()
        {
            *action = Some(HistoryAction::Refresh);
        }
        let toggle_label = if show_recovery_log {
            "Hide restore history"
        } else {
            "Show restore history"
        };
        if ui.button(toggle_label).clicked() {
            *action = Some(HistoryAction::ToggleRecoveryLog);
        }
    });

    if let Some(result) = undo_result {
        render_undo_result(ui, result);
        ui.add_space(6.0);
    }

    if let Some(message) = message {
        ui.colored_label(Color32::from_rgb(190, 40, 40), message);
        return;
    }

    let Some(active_root) = active_root else {
        ui.label("Choose a folder to see activity for that folder.");
        return;
    };

    let scoped_rows: Vec<&TransactionRow> = rows
        .iter()
        .filter(|row| same_folder(&row.root, active_root))
        .collect();
    let hidden_count = rows.len().saturating_sub(scoped_rows.len());

    render_current_folder_activity(ui, &scoped_rows, hidden_count, busy, action);

    if show_recovery_log {
        ui.add_space(10.0);
        render_recovery_log(ui, rows, busy, action);
    }

    if let Some(message) = detail_message {
        ui.add_space(8.0);
        ui.colored_label(Color32::from_rgb(190, 40, 40), message);
    }

    if let Some(detail) = detail {
        ui.add_space(10.0);
        render_transaction_detail(ui, detail, action);
    }
}

fn render_current_folder_activity(
    ui: &mut egui::Ui,
    rows: &[&TransactionRow],
    hidden_count: usize,
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    if rows.is_empty() {
        ui.label("No activity has been recorded for this folder yet.");
        if hidden_count > 0 {
            ui.label(format!(
                "{} activit{} from other folders hidden.",
                hidden_count,
                plural_y(hidden_count)
            ));
        }
        return;
    }

    let latest = rows[0];
    ui.horizontal(|ui| {
        ui.label(RichText::new("Latest activity").strong().size(18.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (stroke, fill) = activity_status_colors(latest.status);
            render_status_chip(ui, status_label(latest.status), stroke, fill);
        });
    });
    ui.label(RichText::new(activity_headline(latest)).strong());
    ui.label(activity_detail(latest));
    ui.add_space(6.0);

    ui.horizontal_wrapped(|ui| {
        render_summary_card(
            ui,
            "Organized",
            latest.completed,
            "Files moved into organized folders.",
            Color32::from_rgb(231, 241, 228),
            Color32::from_rgb(92, 128, 78),
        );
        render_summary_card(
            ui,
            "Restored",
            latest.rolled_back,
            "Files moved back during undo.",
            Color32::from_rgb(236, 236, 244),
            Color32::from_rgb(89, 102, 145),
        );
        render_summary_card(
            ui,
            "Needs review",
            latest.skipped + latest.failed + latest.pending,
            "Items skipped, failed, or still pending.",
            Color32::from_rgb(248, 238, 217),
            Color32::from_rgb(170, 110, 35),
        );
    });

    ui.add_space(6.0);
    ui.label(format!("Recorded on {}", latest.started_at));

    ui.horizontal(|ui| {
        if ui
            .add_enabled(!busy, egui::Button::new("View details"))
            .clicked()
        {
            *action = Some(HistoryAction::ViewDetails(latest.transaction_id.clone()));
        }
        let can_undo = can_undo_status(latest.status) && !busy;
        if ui
            .add_enabled(can_undo, egui::Button::new("Undo Changes"))
            .on_disabled_hover_text("Only completed or failed activities can be undone")
            .clicked()
        {
            *action = Some(HistoryAction::ConfirmUndo(latest.transaction_id.clone()));
        }
    });

    if rows.len() > 1 {
        ui.add_space(8.0);
        ui.label(RichText::new("Earlier activity").strong());
        for row in rows.iter().skip(1).take(3) {
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(250, 246, 240))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(activity_event_title(row)).strong());
                            ui.label(activity_short_label(row));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Details"))
                                .clicked()
                            {
                                *action =
                                    Some(HistoryAction::ViewDetails(row.transaction_id.clone()));
                            }
                            let can_undo = can_undo_status(row.status) && !busy;
                            if ui
                                .add_enabled(can_undo, egui::Button::new("Undo Changes"))
                                .on_disabled_hover_text(
                                    "Only completed or failed activities can be undone",
                                )
                                .clicked()
                            {
                                *action =
                                    Some(HistoryAction::ConfirmUndo(row.transaction_id.clone()));
                            }
                        });
                    });
                });
        }
    }

    if hidden_count > 0 {
        ui.add_space(6.0);
        ui.label(format!(
            "{} activit{} from other folders hidden from this overview.",
            hidden_count,
            plural_y(hidden_count)
        ));
    }
}

fn render_recovery_log(
    ui: &mut egui::Ui,
    rows: &[TransactionRow],
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    ui.label(RichText::new("Restore history").strong());
    if rows.is_empty() {
        ui.label("No restore history has been recorded yet.");
        return;
    }

    let width = ui.available_width().max(360.0);
    let transaction_width = width * 0.24;
    let status_width = width * 0.14;
    let summary_width = width * 0.24;
    let root_width = width * 0.22;
    let action_width = width * 0.16;

    egui::Grid::new("technical-recovery-log")
        .num_columns(5)
        .striped(true)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            preview_cell(ui, "Activity", transaction_width, true);
            preview_cell(ui, "Status", status_width, true);
            preview_cell(ui, "Summary", summary_width, true);
            preview_cell(ui, "Root", root_width, true);
            preview_cell(ui, "Action", action_width, true);
            ui.end_row();

            for row in rows.iter().take(8) {
                preview_cell_with_tooltip(
                    ui,
                    &activity_event_title(row),
                    transaction_width,
                    false,
                    format!("Activity id: {}", row.transaction_id),
                );
                preview_cell(ui, status_label(row.status), status_width, false);
                preview_cell(ui, &activity_count_summary(row), summary_width, false);
                preview_cell(ui, &row.root_label, root_width, false);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy, egui::Button::new("Details"))
                        .clicked()
                    {
                        *action = Some(HistoryAction::ViewDetails(row.transaction_id.clone()));
                    }
                    let can_undo = can_undo_status(row.status) && !busy;
                    if ui
                        .add_enabled(can_undo, egui::Button::new("Undo Changes"))
                        .on_disabled_hover_text("Only completed or failed activities can be undone")
                        .clicked()
                    {
                        *action = Some(HistoryAction::ConfirmUndo(row.transaction_id.clone()));
                    }
                });
                ui.end_row();
            }
        });

    if rows.len() > 8 {
        ui.label(format!("Showing 8 of {} recovery journals.", rows.len()));
    }
}

fn render_transaction_detail(
    ui: &mut egui::Ui,
    detail: &TransactionDetail,
    action: &mut Option<HistoryAction>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Activity details").strong().size(18.0));
        let (stroke, fill) = activity_status_colors(detail.status);
        render_status_chip(ui, status_label(detail.status), stroke, fill);
        if ui.button("Close").clicked() {
            *action = Some(HistoryAction::CloseDetails);
        }
    });

    ui.label(activity_detail_headline(detail));
    ui.label(format!("Why: {}", detail.reason_summary));
    truncated_label(ui, &format!("Folder: {}", detail.root));

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        render_summary_card(
            ui,
            "Organized",
            detail.operation_counts.completed,
            "Completed file moves.",
            Color32::from_rgb(231, 241, 228),
            Color32::from_rgb(92, 128, 78),
        );
        render_summary_card(
            ui,
            "Restored",
            detail.operation_counts.rolled_back,
            "Files restored by undo.",
            Color32::from_rgb(236, 236, 244),
            Color32::from_rgb(89, 102, 145),
        );
        render_summary_card(
            ui,
            "Needs review",
            detail.operation_counts.skipped
                + detail.operation_counts.failed
                + detail.operation_counts.pending,
            "Skipped, failed, or pending changes.",
            Color32::from_rgb(248, 238, 217),
            Color32::from_rgb(170, 110, 35),
        );
    });

    ui.add_space(6.0);
    egui::CollapsingHeader::new("Technical restore details")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("transaction-detail-summary")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Activity id");
                    ui.label(&detail.transaction_id);
                    ui.end_row();
                    ui.label("Plan id");
                    ui.label(&detail.plan_id);
                    ui.end_row();
                    ui.label("Started");
                    ui.label(&detail.started_at);
                    ui.end_row();
                    ui.label("Completed");
                    ui.label(&detail.completed_at);
                    ui.end_row();
                    ui.label("Recorded changes");
                    ui.label(detail.total_operations.to_string());
                    ui.end_row();
                });
        });

    ui.add_space(6.0);
    render_transaction_operation_rows(ui, detail);
}

fn render_transaction_operation_rows(ui: &mut egui::Ui, detail: &TransactionDetail) {
    if detail.operation_rows.is_empty() {
        ui.label("No operation rows recorded in this journal.");
        return;
    }

    let width = ui.available_width().max(360.0);
    let operation_width = width * 0.13;
    let status_width = width * 0.12;
    let source_width = width * 0.22;
    let destination_width = width * 0.22;
    let reason_width = width * 0.18;
    let error_width = width * 0.13;

    ui.label(RichText::new("Recorded changes").strong());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(180.0)
        .show(ui, |ui| {
            egui::Grid::new("transaction-detail-operations")
                .num_columns(6)
                .striped(true)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    preview_cell(ui, "Operation", operation_width, true);
                    preview_cell(ui, "Status", status_width, true);
                    preview_cell(ui, "Source", source_width, true);
                    preview_cell(ui, "Destination", destination_width, true);
                    preview_cell(ui, "Why", reason_width, true);
                    preview_cell(ui, "Error", error_width, true);
                    ui.end_row();

                    for row in &detail.operation_rows {
                        preview_cell(ui, &row.operation_id, operation_width, false);
                        preview_cell(ui, operation_status_label(row.status), status_width, false);
                        preview_cell(ui, &row.source, source_width, false);
                        preview_cell(ui, &row.destination, destination_width, false);
                        preview_cell(ui, &row.reason, reason_width, false);
                        preview_cell(ui, &row.error, error_width, false);
                        ui.end_row();
                    }
                });
        });

    if detail.total_operations > detail.operation_rows.len() {
        ui.label(format!(
            "Showing {} of {} recorded operations.",
            detail.operation_rows.len(),
            detail.total_operations
        ));
    }
}

fn render_undo_confirmation(
    ctx: &egui::Context,
    transaction_id: &str,
    confirmed: &mut bool,
    dismissed: &mut bool,
) {
    egui::Window::new("Confirm undo")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new("Undo these changes?").strong());
            ui.label("smartfolder will move completed files back to their original paths.");
            ui.label("It will refuse to overwrite anything already at an original path.");
            ui.add_space(6.0);
            truncated_label(ui, &format!("Activity id: {transaction_id}"));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    *dismissed = true;
                }
                if ui.button("Undo Changes").clicked() {
                    *confirmed = true;
                }
            });
        });
}

fn render_undo_result(ui: &mut egui::Ui, result: &UndoOutput) {
    ui.label(RichText::new("Changes undone").strong());
    egui::Grid::new("undo-result")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            summary_row(ui, "Rolled back", result.rolled_back);
            summary_row(ui, "Skipped", result.skipped);
            summary_row(ui, "Failed", result.failed);
        });
    ui.label(format!("Activity id: {}", result.transaction_id));
    truncated_label(
        ui,
        &format!("Restore history: {}", result.journal_path.display()),
    );
}

fn plan_summary_headline(result: &AnalysisOutput) -> String {
    if result.preview_counts.ready == 0 {
        return "No safe moves are ready to organize.".to_string();
    }

    if result.preview_counts.needs_attention == 0 && result.summary.ambiguous_files == 0 {
        format!(
            "{} safe file{} ready to organize.",
            result.preview_counts.ready,
            plural(result.preview_counts.ready)
        )
    } else {
        format!(
            "{} safe file{} ready, with {} item{} needing review.",
            result.preview_counts.ready,
            plural(result.preview_counts.ready),
            result.preview_counts.needs_attention + result.summary.ambiguous_files,
            plural(result.preview_counts.needs_attention + result.summary.ambiguous_files)
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

fn render_preview_controls(
    ui: &mut egui::Ui,
    result: &AnalysisOutput,
    active_filter: PreviewFilter,
    offset: usize,
    action: &mut Option<PreviewAction>,
) {
    ui.label(RichText::new("Preview").strong().size(18.0));
    ui.label("Click a file in the preview to inspect its original folder, exact destination, and rule details below.");
    ui.horizontal(|ui| {
        for filter in [
            PreviewFilter::All,
            PreviewFilter::Ready,
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
            "No operations in this view".to_string()
        } else {
            format!("Showing {}-{} of {}", offset + 1, current_end, total_rows)
        };
        ui.label(range_text);

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

fn preview_rows(operations: &[PlanOperation], root: &Path) -> Vec<PreviewRow> {
    operations
        .iter()
        .map(|operation| PreviewRow {
            file_name: file_name_label(&operation.source),
            original_folder: relative_folder_label(&operation.source, root),
            target_folder: relative_folder_label(&operation.destination, root),
            source_full_path: operation.source.display().to_string(),
            destination_full_path: operation.destination.display().to_string(),
            reason: operation.reason.clone(),
            status: operation_status(operation).to_string(),
        })
        .collect()
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

fn render_preview_rows(
    ui: &mut egui::Ui,
    result: &AnalysisOutput,
    selected_row: &mut Option<usize>,
) {
    if result.preview_rows.is_empty() {
        *selected_row = None;
        ui.label("No operations match this view.");
        return;
    }

    if selected_row
        .map(|index| index >= result.preview_rows.len())
        .unwrap_or(true)
    {
        *selected_row = Some(0);
    }

    let width = ui.available_width().max(360.0);
    let file_width = width * 0.30;
    let target_width = width * 0.48;
    let status_width = width * 0.22;

    egui::Grid::new("preview-rows")
        .num_columns(3)
        .striped(true)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            preview_cell(ui, "File", file_width, true);
            preview_cell(ui, "Destination", target_width, true);
            preview_cell(ui, "Status", status_width, true);
            ui.end_row();

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
                        row.source_full_path, row.destination_full_path
                    ),
                );
                if file_response.clicked() {
                    *selected_row = Some(index);
                }
                preview_cell_with_tooltip(
                    ui,
                    &row.target_folder,
                    target_width,
                    false,
                    format!(
                        "Original folder: {}\nFull source: {}\nFull destination: {}\nWhy: {}",
                        row.original_folder,
                        row.source_full_path,
                        row.destination_full_path,
                        row.reason,
                    ),
                );
                preview_cell(ui, &row.status, status_width, false);
                ui.end_row();
            }
        });

    if result.preview_rows.len() < result.preview_total_rows {
        ui.add_space(8.0);
        ui.label(format!(
            "Showing {} of {} matching operations. More rows are stored on disk for paged retrieval.",
            result.preview_rows.len(), result.preview_total_rows
        ));
    }
}

fn preview_cell(ui: &mut egui::Ui, text: &str, width: f32, strong: bool) {
    let _ = preview_cell_response(ui, text, width, strong);
}

fn preview_cell_with_tooltip(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    strong: bool,
    tooltip: impl Into<egui::WidgetText>,
) {
    preview_cell_response(ui, text, width, strong).on_hover_text(tooltip);
}

fn preview_selectable_cell_with_tooltip(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    width: f32,
    selected: bool,
    tooltip: impl Into<egui::WidgetText>,
) -> egui::Response {
    ui.add_sized(
        [width.max(40.0), 20.0],
        egui::SelectableLabel::new(selected, RichText::new(text.into()).strong().monospace()),
    )
    .on_hover_text(tooltip)
}

fn preview_cell_response(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    strong: bool,
) -> egui::Response {
    let text = if strong {
        RichText::new(text).strong().monospace()
    } else {
        RichText::new(text).monospace()
    };
    ui.add_sized([width.max(40.0), 18.0], egui::Label::new(text).truncate())
}

fn operation_status(operation: &PlanOperation) -> &'static str {
    match operation.conflict {
        ConflictState::None => "Ready",
        ConflictState::DestinationExists { .. } => "Needs Review",
        ConflictState::CaseOnlyRename { .. } => "Needs Review",
        ConflictState::UnsafeDestination { .. } => "Left Untouched",
    }
}

fn render_preview_detail(ui: &mut egui::Ui, result: &AnalysisOutput, selected_row: Option<usize>) {
    let Some(index) = selected_row else {
        return;
    };
    let Some(row) = result.preview_rows.get(index) else {
        return;
    };

    ui.label(RichText::new("Selected change").strong().size(18.0));
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(250, 246, 240))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&row.file_name).strong());
                let (stroke, fill) = preview_status_colors(&row.status);
                render_status_chip(ui, &row.status, stroke, fill);
            });
            ui.add_space(8.0);
            egui::Grid::new("preview-detail-grid")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Original folder");
                    ui.label(&row.original_folder);
                    ui.end_row();
                    ui.label("Destination folder");
                    ui.label(&row.target_folder);
                    ui.end_row();
                    ui.label("Why");
                    ui.label(&row.reason);
                    ui.end_row();
                });
            ui.add_space(6.0);
            truncated_label(ui, &format!("Full source: {}", row.source_full_path));
            truncated_label(
                ui,
                &format!("Full destination: {}", row.destination_full_path),
            );
        });
}

fn preview_status_colors(status: &str) -> (Color32, Color32) {
    match status {
        "Ready" => (
            Color32::from_rgb(92, 128, 78),
            Color32::from_rgb(231, 241, 228),
        ),
        "Needs Review" => (
            Color32::from_rgb(170, 110, 35),
            Color32::from_rgb(248, 238, 217),
        ),
        _ => (
            Color32::from_rgb(116, 103, 90),
            Color32::from_rgb(243, 238, 234),
        ),
    }
}

fn activity_status_colors(status: TransactionStatus) -> (Color32, Color32) {
    match status {
        TransactionStatus::Completed => (
            Color32::from_rgb(92, 128, 78),
            Color32::from_rgb(231, 241, 228),
        ),
        TransactionStatus::RolledBack | TransactionStatus::PartiallyRolledBack => (
            Color32::from_rgb(89, 102, 145),
            Color32::from_rgb(236, 236, 244),
        ),
        TransactionStatus::Failed | TransactionStatus::Interrupted => (
            Color32::from_rgb(170, 110, 35),
            Color32::from_rgb(248, 238, 217),
        ),
        TransactionStatus::InProgress => (
            Color32::from_rgb(130, 102, 53),
            Color32::from_rgb(246, 236, 211),
        ),
    }
}

fn apply_visual_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    ctx.set_style(style);

    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = Color32::from_rgb(244, 238, 230);
    visuals.window_fill = Color32::from_rgb(252, 248, 242);
    visuals.extreme_bg_color = Color32::from_rgb(236, 229, 220);
    visuals.faint_bg_color = Color32::from_rgb(248, 243, 236);
    visuals.override_text_color = Some(Color32::from_rgb(57, 48, 39));
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(244, 238, 230);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(246, 240, 232);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(236, 227, 214);
    visuals.widgets.active.bg_fill = Color32::from_rgb(230, 219, 206);
    visuals.widgets.inactive.fg_stroke.color = Color32::from_rgb(57, 48, 39);
    visuals.widgets.hovered.fg_stroke.color = Color32::from_rgb(57, 48, 39);
    visuals.widgets.active.fg_stroke.color = Color32::from_rgb(57, 48, 39);
    ctx.set_visuals(visuals);
}

fn render_screen_heading(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.label(RichText::new(title).size(30.0).strong());
    ui.label(detail);
}

fn render_status_chip(ui: &mut egui::Ui, text: &str, stroke: Color32, fill: Color32) {
    egui::Frame::group(ui.style())
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(10.0, 4.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).strong().color(stroke));
        });
}

fn render_safety_line(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        render_status_chip(
            ui,
            "Safe",
            Color32::from_rgb(92, 128, 78),
            Color32::from_rgb(231, 241, 228),
        );
        ui.label(text);
    });
}

fn render_info_card(ui: &mut egui::Ui, title: &str, detail: &str, status: &str) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(250, 246, 240))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_min_width(CARD_MIN_WIDTH);
            ui.label(RichText::new(title).strong().size(18.0));
            ui.label(detail);
            ui.add_space(6.0);
            render_status_chip(
                ui,
                status,
                Color32::from_rgb(116, 103, 90),
                Color32::from_rgb(243, 238, 234),
            );
        });
}

fn render_style_card(
    ui: &mut egui::Ui,
    selected: bool,
    title: &str,
    example: &str,
    detail: &str,
) -> egui::Response {
    ui.add_sized(
        [CARD_MIN_WIDTH, 108.0],
        egui::Button::new(
            RichText::new(format!("{title}\n{example}\n{detail}"))
                .size(14.0)
                .color(Color32::from_rgb(57, 48, 39)),
        )
        .stroke(egui::Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                Color32::from_rgb(166, 94, 53)
            } else {
                Color32::from_rgb(212, 200, 186)
            },
        ))
        .fill(if selected {
            Color32::from_rgb(237, 226, 213)
        } else {
            Color32::from_rgb(250, 246, 240)
        }),
    )
}

fn render_summary_card(
    ui: &mut egui::Ui,
    title: &str,
    value: usize,
    detail: &str,
    fill: Color32,
    stroke: Color32,
) {
    egui::Frame::group(ui.style())
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_min_width(CARD_MIN_WIDTH);
            ui.label(RichText::new(title).strong());
            ui.label(
                RichText::new(value.to_string())
                    .size(28.0)
                    .strong()
                    .color(stroke),
            );
            ui.label(detail);
        });
}

fn activity_headline(row: &TransactionRow) -> String {
    match row.status {
        TransactionStatus::Completed => format!(
            "Moved {} file{} into organized folders.",
            row.completed,
            plural(row.completed)
        ),
        TransactionStatus::RolledBack => format!(
            "Undid {} move{} from this folder.",
            row.rolled_back,
            plural(row.rolled_back)
        ),
        TransactionStatus::PartiallyRolledBack => format!(
            "Partially undid this transaction: {} move{} restored, {} issue{} remain.",
            row.rolled_back,
            plural(row.rolled_back),
            row.failed,
            plural(row.failed)
        ),
        TransactionStatus::Interrupted => format!(
            "Transaction interrupted after {} recorded operation{}.",
            row.total_operations,
            plural(row.total_operations)
        ),
        TransactionStatus::InProgress => format!(
            "Transaction in progress with {} recorded operation{}.",
            row.total_operations,
            plural(row.total_operations)
        ),
        TransactionStatus::Failed => format!(
            "Transaction needs attention: {} failed, {} completed.",
            row.failed, row.completed
        ),
    }
}

fn activity_event_title(row: &TransactionRow) -> String {
    let folder = folder_name_label(&row.root);
    match row.status {
        TransactionStatus::Completed => format!(
            "Organized {} file{} in {folder}",
            row.completed,
            plural(row.completed)
        ),
        TransactionStatus::RolledBack => format!(
            "Undid organization in {folder}: {} file{} restored",
            row.rolled_back,
            plural(row.rolled_back)
        ),
        TransactionStatus::PartiallyRolledBack => format!(
            "Partially undid organization in {folder}: {} file{} restored",
            row.rolled_back,
            plural(row.rolled_back)
        ),
        TransactionStatus::Interrupted => format!(
            "Organization interrupted in {folder} after {} recorded change{}",
            row.total_operations,
            plural(row.total_operations)
        ),
        TransactionStatus::InProgress => format!(
            "Organization running in {folder}: {} recorded change{}",
            row.total_operations,
            plural(row.total_operations)
        ),
        TransactionStatus::Failed => format!(
            "Organization needs review in {folder}: {} failed, {} organized",
            row.failed, row.completed
        ),
    }
}

fn activity_detail_headline(detail: &TransactionDetail) -> String {
    match detail.status {
        TransactionStatus::Completed => format!(
            "Organized {} file{} on {}.",
            detail.operation_counts.completed,
            plural(detail.operation_counts.completed),
            detail.started_at
        ),
        TransactionStatus::RolledBack => format!(
            "Restored {} file{} from this activity.",
            detail.operation_counts.rolled_back,
            plural(detail.operation_counts.rolled_back)
        ),
        TransactionStatus::PartiallyRolledBack => format!(
            "Restored {} file{} with some changes still needing review.",
            detail.operation_counts.rolled_back,
            plural(detail.operation_counts.rolled_back)
        ),
        TransactionStatus::Interrupted => format!(
            "Organization was interrupted after {} recorded change{}.",
            detail.total_operations,
            plural(detail.total_operations)
        ),
        TransactionStatus::InProgress => format!(
            "Organization is still in progress with {} recorded change{}.",
            detail.total_operations,
            plural(detail.total_operations)
        ),
        TransactionStatus::Failed => format!(
            "Organization needs review: {} failed, {} completed.",
            detail.operation_counts.failed, detail.operation_counts.completed
        ),
    }
}

fn activity_detail(row: &TransactionRow) -> String {
    match row.status {
        TransactionStatus::Completed => format!(
            "Why: {}. Completed {}. {} item{} skipped or failed.",
            row.reason_summary,
            row.started_at,
            row.skipped + row.failed,
            plural(row.skipped + row.failed)
        ),
        TransactionStatus::RolledBack => format!(
            "Rollback completed {}. Original file locations were restored where possible.",
            row.started_at
        ),
        TransactionStatus::PartiallyRolledBack => {
            "Rollback completed with remaining issues. Review details before making more changes."
                .to_string()
        }
        TransactionStatus::Interrupted | TransactionStatus::InProgress => format!(
            "Why: {}. {} completed, {} pending. Resume or undo from the recovery controls.",
            row.reason_summary, row.completed, row.pending
        ),
        TransactionStatus::Failed => {
            "Some file moves failed. Review details before retrying or undoing.".to_string()
        }
    }
}

fn activity_short_label(row: &TransactionRow) -> String {
    format!("{} - {}", row.started_at, activity_count_summary(row))
}

fn activity_count_summary(row: &TransactionRow) -> String {
    format!(
        "{} moved, {} undone, {} attention",
        row.completed,
        row.rolled_back,
        row.skipped + row.failed + row.pending
    )
}

fn same_folder(left: &Path, right: &Path) -> bool {
    normalized_path_key(left) == normalized_path_key(right)
}

fn normalized_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn folder_name_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn plural_y(value: usize) -> &'static str {
    if value == 1 {
        "y"
    } else {
        "ies"
    }
}

fn summary_row(ui: &mut egui::Ui, label: &str, value: usize) {
    ui.label(label);
    ui.label(value.to_string());
    ui.end_row();
}

fn status_label(status: TransactionStatus) -> &'static str {
    match status {
        TransactionStatus::InProgress => "in progress",
        TransactionStatus::Completed => "completed",
        TransactionStatus::Interrupted => "interrupted",
        TransactionStatus::RolledBack => "rolled back",
        TransactionStatus::PartiallyRolledBack => "partially rolled back",
        TransactionStatus::Failed => "failed",
    }
}

fn operation_status_label(status: OperationStatus) -> &'static str {
    match status {
        OperationStatus::Pending => "pending",
        OperationStatus::Completed => "completed",
        OperationStatus::Skipped => "skipped",
        OperationStatus::Failed => "failed",
        OperationStatus::RolledBack => "rolled back",
    }
}

fn can_undo_status(status: TransactionStatus) -> bool {
    matches!(
        status,
        TransactionStatus::Completed
            | TransactionStatus::Failed
            | TransactionStatus::PartiallyRolledBack
    )
}

fn is_cloud_synced_path(path: &Path) -> bool {
    path.components().any(|component| {
        let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        value.contains("onedrive")
            || value.contains("dropbox")
            || value.contains("google drive")
            || value.contains("icloud")
    })
}

fn plural(value: usize) -> &'static str {
    if value == 1 {
        ""
    } else {
        "s"
    }
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

    use smartfolder_core::model::{
        BuiltInMode, Certainty, ConflictState, OperationStatus, OperationType, PlanOperation,
        SourceSnapshot, TransactionStatus,
    };
    use smartfolder_core::rules::{CustomRule, RuleProfile};

    use super::{
        activity_count_summary, activity_headline, can_undo_status, is_cloud_synced_path,
        operation_status_label, preloaded_root_from_args, preview_rows, profile_file_stem,
        same_folder, status_label, transaction_operation_counts, AnalysisPlanSource,
        LoadedRuleProfile, PlanningSource, ProfileEditorState, SmartfolderApp, TransactionRow,
    };

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

    #[test]
    fn cloud_synced_paths_are_detected_for_apply_warning() {
        assert!(is_cloud_synced_path(&PathBuf::from(
            r"D:\OneDrive\Documents"
        )));
        assert!(is_cloud_synced_path(&PathBuf::from(
            r"C:\Users\User\Dropbox\Work"
        )));
        assert!(!is_cloud_synced_path(&PathBuf::from(r"D:\Local\Documents")));
    }

    #[test]
    fn transaction_status_copy_matches_undo_rules() {
        assert_eq!(status_label(TransactionStatus::Completed), "completed");
        assert!(can_undo_status(TransactionStatus::Completed));
        assert!(can_undo_status(TransactionStatus::Failed));
        assert!(!can_undo_status(TransactionStatus::Interrupted));
        assert!(!can_undo_status(TransactionStatus::RolledBack));
    }

    #[test]
    fn operation_status_copy_matches_detail_counts() {
        assert_eq!(operation_status_label(OperationStatus::Pending), "pending");
        assert_eq!(
            operation_status_label(OperationStatus::RolledBack),
            "rolled back"
        );

        let operations = [
            test_transaction_operation("op_pending", OperationStatus::Pending),
            test_transaction_operation("op_done", OperationStatus::Completed),
            test_transaction_operation("op_failed", OperationStatus::Failed),
            test_transaction_operation("op_rollback", OperationStatus::RolledBack),
        ];
        let counts = transaction_operation_counts(&operations);

        assert_eq!(counts.pending, 1);
        assert_eq!(counts.completed, 1);
        assert_eq!(counts.failed, 1);
        assert_eq!(counts.rolled_back, 1);
    }

    #[test]
    fn activity_overview_scopes_to_selected_folder() {
        assert!(same_folder(
            &PathBuf::from(r"D:\OneDrive\Documents\"),
            &PathBuf::from(r"d:/onedrive/documents")
        ));
        assert!(!same_folder(
            &PathBuf::from(r"C:\Users\User\AppData\Local\Temp\root"),
            &PathBuf::from(r"D:\OneDrive\Documents")
        ));

        let row = test_transaction_row(TransactionStatus::Completed);
        assert_eq!(
            activity_headline(&row),
            "Moved 3 files into organized folders."
        );
        assert_eq!(
            activity_count_summary(&row),
            "3 moved, 0 undone, 1 attention"
        );
    }

    #[test]
    fn preview_rows_show_file_and_relative_folders() {
        let root = PathBuf::from(r"D:\OneDrive\Documents");
        let operations = [PlanOperation {
            operation_id: "op_1".to_string(),
            operation_type: OperationType::Move,
            source: root.join("loose.jpg"),
            destination: root
                .join("Images")
                .join("2013")
                .join("January")
                .join("loose.jpg"),
            reason: "Built-in rule: TypeYear".to_string(),
            certainty: Certainty::High,
            conflict: ConflictState::None,
            selected: true,
            source_snapshot: SourceSnapshot {
                size_bytes: 10,
                modified_at: None,
            },
        }];

        let rows = preview_rows(&operations, &root);

        assert_eq!(rows[0].file_name, "loose.jpg");
        assert_eq!(rows[0].original_folder, "Selected folder");
        assert_eq!(rows[0].target_folder, "Images / 2013 / January");
        assert!(rows[0].source_full_path.ends_with("loose.jpg"));
    }

    #[test]
    fn gui_planning_source_requires_imported_profile() {
        let mut app = SmartfolderApp::new(None);
        app.planning_source = PlanningSource::RuleProfile;

        let message = app
            .analysis_plan_source()
            .expect_err("missing profile should block profile analysis");
        assert_eq!(
            message,
            "Import a rule profile before analyzing with profile rules."
        );

        app.loaded_profile = Some(LoadedRuleProfile {
            path: PathBuf::from("rules.toml"),
            profile: test_rule_profile(),
        });
        let source = app
            .analysis_plan_source()
            .expect("loaded profile should be accepted");
        assert!(matches!(source, AnalysisPlanSource::RuleProfile(_)));
    }

    #[test]
    fn profile_editor_builds_valid_core_profile() {
        let editor = ProfileEditorState {
            profile_id: "downloads".to_string(),
            rule_name: "Invoices".to_string(),
            destination: "Documents/Invoices/{year}".to_string(),
            priority: "5".to_string(),
            extensions: "pdf, docx".to_string(),
            filename_contains: "invoice".to_string(),
            path_contains: "downloads".to_string(),
            min_size_bytes: "100".to_string(),
            max_size_bytes: "200000".to_string(),
            year: "2026".to_string(),
        };

        let profile = editor.to_profile().expect("editor profile is valid");

        assert_eq!(profile.profile_id, "downloads");
        assert_eq!(profile.rules[0].name, "Invoices");
        assert_eq!(profile.rules[0].extensions, vec!["pdf", "docx"]);
        assert_eq!(profile.rules[0].priority, Some(5));
        assert_eq!(profile.rules[0].min_size_bytes, Some(100));
        assert_eq!(profile.rules[0].year, Some(2026));

        let restored = ProfileEditorState::from_profile(&profile);
        assert_eq!(restored.profile_id, "downloads");
        assert_eq!(restored.extensions, "pdf, docx");
    }

    #[test]
    fn profile_editor_rejects_invalid_numbers_and_sanitizes_file_names() {
        let editor = ProfileEditorState {
            priority: "soon".to_string(),
            ..ProfileEditorState::default()
        };

        let message = editor
            .to_profile()
            .expect_err("invalid priority should fail");

        assert_eq!(message, "priority must be a valid number");
        assert_eq!(
            profile_file_stem("Family Photos 2026!"),
            "Family_Photos_2026"
        );
        assert_eq!(profile_file_stem("***"), "profile");
    }

    fn test_transaction_operation(
        operation_id: &str,
        status: OperationStatus,
    ) -> smartfolder_core::model::TransactionOperation {
        smartfolder_core::model::TransactionOperation {
            operation_id: operation_id.to_string(),
            operation_type: smartfolder_core::model::OperationType::Move,
            source: PathBuf::from(format!(r"D:\Source\{operation_id}.txt")),
            destination: PathBuf::from(format!(r"D:\Destination\{operation_id}.txt")),
            status,
            reason: Some("Built-in rule: Type".to_string()),
            same_volume: Some(true),
            error: None,
        }
    }

    fn test_transaction_row(status: TransactionStatus) -> TransactionRow {
        TransactionRow {
            transaction_id: "txn_test".to_string(),
            root: PathBuf::from(r"D:\OneDrive\Documents"),
            root_label: r"D:\OneDrive\Documents".to_string(),
            status,
            started_at: "2026-05-12 13:00:00".to_string(),
            reason_summary: "Built-in rule: Type".to_string(),
            completed: 3,
            skipped: 1,
            failed: 0,
            rolled_back: 0,
            pending: 0,
            total_operations: 4,
        }
    }

    fn test_rule_profile() -> RuleProfile {
        RuleProfile {
            profile_id: "documents".to_string(),
            rules: vec![CustomRule {
                name: "PDFs".to_string(),
                destination: "Documents/PDFs".to_string(),
                priority: Some(10),
                extensions: vec!["pdf".to_string()],
                filename_contains: Vec::new(),
                path_contains: Vec::new(),
                min_size_bytes: None,
                max_size_bytes: None,
                year: None,
            }],
        }
    }
}
