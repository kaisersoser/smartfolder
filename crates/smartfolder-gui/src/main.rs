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
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![allow(clippy::module_name_repetitions)]

mod preferences;
mod ui;

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

use preferences::{GuiPreferences, MotionPreference, StylePreference, ThemePreference};

type AnalysisMessage = std::result::Result<AnalysisOutput, String>;
type ApplyMessage = std::result::Result<ApplyOutput, String>;
type UndoMessage = std::result::Result<UndoOutput, String>;

const PREVIEW_PAGE_SIZE: usize = 100;
const TRANSACTION_DETAIL_ROW_LIMIT: usize = 100;
const WINDOW_WIDTH: f32 = 1160.0;
const WINDOW_HEIGHT: f32 = 780.0;
const SHELL_NAV_WIDTH: f32 = ui::theme::spacing::SIDEBAR_WIDTH;
const CARD_MIN_WIDTH: f32 = 188.0;
const PREVIEW_EXAMPLE_LIMIT: usize = 3;
const INSTRUCTION_PANEL_HEIGHT: f32 = 216.0;
const ORGANIZE_STEP_BUTTON_WIDTH: f32 = 150.0;
const ORGANIZE_STEP_FRAME_MARGIN: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppSection {
    Organize,
    Activity,
    Rules,
    Settings,
}

impl AppSection {
    const ALL: [Self; 4] = [Self::Organize, Self::Activity, Self::Rules, Self::Settings];

    fn icon(self) -> &'static str {
        match self {
            Self::Organize => ui::icons::FOLDER,
            Self::Activity => ui::icons::ACTIVITY,
            Self::Rules => ui::icons::RULES,
            Self::Settings => ui::icons::SETTINGS,
        }
    }

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

    fn shortcut(self) -> &'static str {
        match self {
            Self::Organize => "Alt+1",
            Self::Activity => "Alt+2",
            Self::Rules => "Alt+3",
            Self::Settings => "Alt+4",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OrganizeStep {
    Folder,
    Style,
    Preview,
    Organize,
}

impl OrganizeStep {
    const ALL: [Self; 4] = [Self::Folder, Self::Style, Self::Preview, Self::Organize];

    fn title(self) -> &'static str {
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
            Self::Organize => "Confirm and undo if needed",
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

    fn previous(self) -> Option<Self> {
        match self {
            Self::Folder => None,
            Self::Style => Some(Self::Folder),
            Self::Preview => Some(Self::Style),
            Self::Organize => Some(Self::Preview),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrganizeNavAction {
    Back,
    Continue,
    Reanalyze,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstructionPreset {
    ByType,
    ByDate,
    ByExtension,
    TypeAndDate,
    CustomRules,
}

#[derive(Debug, Clone)]
struct ExampleTreeEntry {
    depth: usize,
    label: String,
    is_folder: bool,
    is_last: bool,
    ancestor_has_next: Vec<bool>,
}

impl InstructionPreset {
    const ALL: [Self; 5] = [
        Self::ByType,
        Self::ByDate,
        Self::ByExtension,
        Self::TypeAndDate,
        Self::CustomRules,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::ByType => "By Type",
            Self::ByDate => "By Date",
            Self::ByExtension => "By Extension",
            Self::TypeAndDate => "Type + Date",
            Self::CustomRules => "Custom Rules",
        }
    }

    fn example_destination(self) -> &'static str {
        match self {
            Self::ByType => "Images",
            Self::ByDate => "2026/05/13",
            Self::ByExtension => "pdf",
            Self::TypeAndDate => "Images/2026/05/13",
            Self::CustomRules => "Documents/PDFs",
        }
    }

    fn example_file_name(self) -> &'static str {
        match self {
            Self::ByType => "beach-sunset.jpg",
            Self::ByDate => "meeting-notes.docx",
            Self::ByExtension => "project-spec.pdf",
            Self::TypeAndDate => "beach-sunset.jpg",
            Self::CustomRules => "invoice-042.pdf",
        }
    }

    fn secondary_example_file_name(self) -> &'static str {
        match self {
            Self::ByType => "screenshot.png",
            Self::ByDate => "budget-review.xlsx",
            Self::ByExtension => "invoice-042.pdf",
            Self::TypeAndDate => "class-photo.jpg",
            Self::CustomRules => "receipt-1042.pdf",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Self::ByType => "Group related file types into broad folders that are easy to scan later.",
            Self::ByDate => "Sort files by when they were last modified so recent work stays together.",
            Self::ByExtension => "Separate files by exact extension when the file format matters more than the category.",
            Self::TypeAndDate => "Keep similar file types together, then add date folders inside each type.",
            Self::CustomRules => "Apply a saved rule profile when one folder needs more specific destinations than the built-in options provide.",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::ByType => "Good default for mixed folders.",
            Self::ByDate => "Good for inboxes, downloads, and dated work.",
            Self::ByExtension => "Good when file formats need strict separation.",
            Self::TypeAndDate => "Best when you want both category and time structure.",
            Self::CustomRules => "Requires an imported or saved rule profile.",
        }
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_min_inner_size([1024.0, 720.0]),
        ..eframe::NativeOptions::default()
    };
    let preloaded_root = preloaded_root_from_args(std::env::args());

    eframe::run_native(
        "smartfolder",
        native_options,
        Box::new(move |creation_context| {
            ui::theme::configure_fonts(&creation_context.egui_ctx);
            Ok(Box::new(SmartfolderApp::new(preloaded_root)))
        }),
    )
}

#[derive(Debug)]
struct SmartfolderApp {
    preferences: GuiPreferences,
    active_section: AppSection,
    organize_step: OrganizeStep,
    furthest_organize_step: OrganizeStep,
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
    show_detailed_preview: bool,
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
        let (preferences, preferences_message) = match GuiPreferences::load() {
            Ok(preferences) => (preferences, None),
            Err(message) => (GuiPreferences::default(), Some(message)),
        };
        let launched_with_preselected_root = preloaded_root.is_some();
        let (transaction_rows, transaction_message) = match load_transaction_rows() {
            Ok(rows) => (rows, None),
            Err(message) => (Vec::new(), Some(message)),
        };
        let initial_mode = preferences
            .last_style
            .built_in_mode()
            .unwrap_or(BuiltInMode::TypeYear);

        Self {
            preferences,
            active_section: AppSection::Organize,
            organize_step: OrganizeStep::Folder,
            furthest_organize_step: if launched_with_preselected_root {
                OrganizeStep::Style
            } else {
                OrganizeStep::Folder
            },
            root_input: preloaded_root
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            launched_with_preselected_root,
            mode: initial_mode,
            planning_source: PlanningSource::BuiltIn,
            loaded_profile: None,
            profile_editor: ProfileEditorState::default(),
            include_subfolders: false,
            preview_filter: PreviewFilter::All,
            preview_offset: 0,
            selected_preview_row: None,
            show_detailed_preview: false,
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
            maintenance_message: preferences_message,
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
            self.set_root(path, false);
        }
    }

    fn set_root(&mut self, path: PathBuf, preselected_at_launch: bool) {
        let root_input = path.display().to_string();
        if self.root_input != root_input {
            self.invalidate_preview_state();
        }
        self.preferences.remember_folder(&path);
        self.root_input = root_input;
        self.launched_with_preselected_root = preselected_at_launch;
        self.mark_organize_step_reached(OrganizeStep::Style);
        self.save_preferences_quietly();
    }

    fn choose_recent_root(&mut self, path: PathBuf) {
        self.set_root(path, false);
    }

    fn select_builtin_style(&mut self, mode: BuiltInMode) {
        if self.planning_source != PlanningSource::BuiltIn || self.mode != mode {
            self.invalidate_preview_state();
        }
        self.planning_source = PlanningSource::BuiltIn;
        self.mode = mode;
        self.mark_organize_step_reached(OrganizeStep::Style);
        self.preferences.last_style = StylePreference::from(mode);
        self.save_preferences_quietly();
    }

    fn select_custom_rules_style(&mut self) {
        if self.loaded_profile.is_none() {
            self.planning_source = PlanningSource::BuiltIn;
            self.mode = BuiltInMode::TypeYear;
            self.preferences.last_style = StylePreference::TypeYear;
            self.maintenance_message = Some(
                "Custom Rules needs a saved profile, so Type + Date stayed selected.".to_string(),
            );
            self.save_preferences_quietly();
            return;
        }

        if self.planning_source != PlanningSource::RuleProfile {
            self.invalidate_preview_state();
        }
        self.planning_source = PlanningSource::RuleProfile;
        self.mark_organize_step_reached(OrganizeStep::Style);
        self.preferences.last_style = StylePreference::CustomRules;
        self.save_preferences_quietly();
    }

    fn invalidate_preview_state(&mut self) {
        self.analysis_result = None;
        self.apply_result = None;
        self.undo_result = None;
        self.show_apply_confirmation = false;
        self.show_detailed_preview = false;
        self.preview_filter = PreviewFilter::All;
        self.preview_offset = 0;
        self.selected_preview_row = None;
        if self.furthest_organize_step > OrganizeStep::Style {
            self.furthest_organize_step = OrganizeStep::Style;
        }
        if self.organize_step > OrganizeStep::Style {
            self.organize_step = OrganizeStep::Style;
        }
    }

    fn mark_organize_step_reached(&mut self, step: OrganizeStep) {
        if step > self.furthest_organize_step {
            self.furthest_organize_step = step;
        }
    }

    fn can_open_organize_step(&self, step: OrganizeStep) -> bool {
        step <= self.furthest_organize_step
    }

    fn go_to_organize_step(&mut self, step: OrganizeStep) {
        if self.can_open_organize_step(step) {
            self.organize_step = step;
        }
    }

    fn continue_from_folder_step(&mut self) {
        if self.has_selected_root() {
            let root = PathBuf::from(self.root_input.trim());
            self.preferences.remember_folder(root);
            self.save_preferences_quietly();
            self.mark_organize_step_reached(OrganizeStep::Style);
            self.organize_step = OrganizeStep::Style;
        } else {
            self.error_message = Some("Choose a folder before continuing.".to_string());
        }
    }

    fn continue_from_style_step(&mut self) {
        if self.can_run_analysis() {
            self.organize_step = OrganizeStep::Preview;
            self.mark_organize_step_reached(OrganizeStep::Preview);
            self.start_analysis();
        } else if self.planning_source == PlanningSource::RuleProfile {
            self.error_message = Some("Import a rule profile to use Custom Rules.".to_string());
        } else {
            self.error_message = Some("Choose a folder before previewing changes.".to_string());
        }
    }

    fn continue_from_preview_step(&mut self) {
        let ready = self
            .analysis_result
            .as_ref()
            .map_or(0, |result| result.preview_counts.ready);
        if ready == 0 {
            self.error_message = Some("No safe moves are ready to organize.".to_string());
            return;
        }
        self.mark_organize_step_reached(OrganizeStep::Organize);
        self.organize_step = OrganizeStep::Organize;
    }

    fn go_back_one_organize_step(&mut self) {
        if let Some(previous) = self.organize_step.previous() {
            self.organize_step = previous;
        }
    }

    fn save_preferences_quietly(&mut self) {
        if let Err(message) = self.preferences.save() {
            self.error_message = Some(message);
        }
    }

    fn save_preferences_with_message(&mut self) {
        match self.preferences.save() {
            Ok(()) => {
                self.maintenance_message = Some("Saved interface preferences.".to_string());
                self.error_message = None;
            }
            Err(message) => {
                self.error_message = Some(message);
                self.maintenance_message = None;
            }
        }
    }

    fn start_analysis(&mut self) {
        let root_text = self.root_input.trim();
        if root_text.is_empty() {
            self.error_message = Some("Select a folder before running analysis.".to_string());
            return;
        }

        let root = PathBuf::from(root_text);
        self.preferences.remember_folder(&root);
        self.save_preferences_quietly();
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
        self.show_detailed_preview = false;
        self.organize_step = OrganizeStep::Preview;
        self.mark_organize_step_reached(OrganizeStep::Preview);

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

    fn accept_dropped_folder(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.set_root(path, false);
            self.active_section = AppSection::Organize;
            self.organize_step = OrganizeStep::Folder;
            self.maintenance_message = Some("Folder added from drag and drop.".to_string());
            self.error_message = None;
        } else {
            self.error_message = Some("Drop a folder, not an individual file.".to_string());
        }
    }

    fn process_dropped_folders(&mut self, ctx: &egui::Context) {
        let dropped_paths = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });

        if let Some(path) = dropped_paths.into_iter().next() {
            self.accept_dropped_folder(path);
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
                self.preferences.last_style = StylePreference::CustomRules;
                self.save_preferences_quietly();
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
                self.preferences.last_style = StylePreference::CustomRules;
                self.save_preferences_quietly();
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
                    self.mark_organize_step_reached(OrganizeStep::Preview);
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
                    self.organize_step = OrganizeStep::Organize;
                    self.mark_organize_step_reached(OrganizeStep::Organize);
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

    fn selected_instruction_preset(&self) -> InstructionPreset {
        match self.planning_source {
            PlanningSource::BuiltIn => match self.mode {
                BuiltInMode::Type => InstructionPreset::ByType,
                BuiltInMode::Date => InstructionPreset::ByDate,
                BuiltInMode::Extension => InstructionPreset::ByExtension,
                BuiltInMode::TypeYear => InstructionPreset::TypeAndDate,
            },
            PlanningSource::RuleProfile => InstructionPreset::CustomRules,
        }
    }

    fn select_instruction_preset(&mut self, preset: InstructionPreset) {
        match preset {
            InstructionPreset::ByType => self.select_builtin_style(BuiltInMode::Type),
            InstructionPreset::ByDate => self.select_builtin_style(BuiltInMode::Date),
            InstructionPreset::ByExtension => self.select_builtin_style(BuiltInMode::Extension),
            InstructionPreset::TypeAndDate => self.select_builtin_style(BuiltInMode::TypeYear),
            InstructionPreset::CustomRules => self.select_custom_rules_style(),
        }
    }

    fn render_instruction_detail_panel(&mut self, ui: &mut egui::Ui, preset: InstructionPreset) {
        let example_entries = self.instruction_example_entries(preset);

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
                        render_instruction_example_tree(ui, &example_entries);
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
                if let Some(profile) = &self.loaded_profile {
                    render_status_chip(
                        ui,
                        "Profile loaded",
                        Color32::from_rgb(92, 128, 78),
                        Color32::from_rgb(231, 241, 228),
                    );
                    ui.label(format!("Using {}.", profile.display_label()));
                } else {
                    render_status_chip(
                        ui,
                        "Profile needed",
                        Color32::from_rgb(170, 110, 35),
                        Color32::from_rgb(248, 238, 217),
                    );
                    ui.label("Import a profile before previewing with Custom Rules.");
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(ui::theme::widgets::secondary_button("Import profile..."))
                        .clicked()
                    {
                        self.import_rule_profile();
                    }
                    if ui
                        .add(ui::theme::widgets::secondary_button("Open Rules"))
                        .clicked()
                    {
                        self.active_section = AppSection::Rules;
                    }
                });
            }
        });
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

    fn instruction_example_entries(&self, preset: InstructionPreset) -> Vec<ExampleTreeEntry> {
        let root_folder = self.instruction_example_root_label();
        let destination = match preset {
            InstructionPreset::CustomRules => self.custom_rule_example_destination(),
            _ => preset.example_destination().to_string(),
        };

        build_example_tree_entries(
            &root_folder,
            &destination,
            preset.example_file_name(),
            preset.secondary_example_file_name(),
        )
    }

    fn instruction_example_root_label(&self) -> String {
        let trimmed = self.root_input.trim();
        if trimmed.is_empty() {
            return "current_folder".to_string();
        }

        let label = folder_name_label(Path::new(trimmed));
        if label.trim().is_empty() {
            "current_folder".to_string()
        } else {
            label
        }
    }

    fn custom_rule_example_destination(&self) -> String {
        self.loaded_profile
            .as_ref()
            .and_then(|profile| profile.profile.rules.first())
            .map(|rule| sample_destination_template(&rule.destination))
            .filter(|destination| !destination.trim().is_empty())
            .unwrap_or_else(|| {
                InstructionPreset::CustomRules
                    .example_destination()
                    .to_string()
            })
    }

    fn render_status_messages(&mut self, ui: &mut egui::Ui) {
        if let Some(message) = &self.error_message {
            ui.add_space(6.0);
            ui.colored_label(ui::theme::colors::error(), message);
        }

        if let Some(message) = &self.maintenance_message {
            ui.add_space(6.0);
            ui.colored_label(ui::theme::colors::success(), message);
        }
    }

    fn process_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        let requested_section = ctx.input(|input| {
            if !input.modifiers.alt {
                return None;
            }

            if input.key_pressed(egui::Key::Num1) {
                Some(AppSection::Organize)
            } else if input.key_pressed(egui::Key::Num2) {
                Some(AppSection::Activity)
            } else if input.key_pressed(egui::Key::Num3) {
                Some(AppSection::Rules)
            } else if input.key_pressed(egui::Key::Num4) {
                Some(AppSection::Settings)
            } else {
                None
            }
        });

        if let Some(section) = requested_section {
            self.active_section = section;
        }
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(ui::theme::spacing::MD);
        ui.label(
            RichText::new("smartfolder")
                .size(24.0)
                .strong()
                .color(ui::theme::colors::heading_text()),
        );
        ui.label(
            RichText::new("Organize files safely, then undo changes if you need to.")
                .color(ui::theme::colors::secondary_text()),
        );
        ui.label(
            RichText::new("Use Alt+1 through Alt+4 to switch sections.")
                .small()
                .color(ui::theme::colors::metadata_text()),
        );
        ui.add_space(ui::theme::spacing::LG);

        for section in AppSection::ALL {
            let selected = self.active_section == section;
            let nav_width = SHELL_NAV_WIDTH - (ui::theme::spacing::LG * 2.0);
            let response = egui::Frame::group(ui.style())
                .fill(if selected {
                    ui::theme::colors::hover_control()
                } else {
                    ui::theme::colors::soft_control()
                })
                .stroke(egui::Stroke::new(
                    if selected { 2.0 } else { 1.0 },
                    if selected {
                        ui::theme::colors::primary_blue()
                    } else {
                        ui::theme::colors::border()
                    },
                ))
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    let inner = ui.allocate_ui_with_layout(
                        egui::vec2(nav_width, 84.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(section.icon()).size(16.0).color(
                                    if selected {
                                        ui::theme::colors::primary_blue()
                                    } else {
                                        ui::theme::colors::secondary_text()
                                    },
                                ));
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
                self.active_section = section;
            }
            ui.add_space(ui::theme::spacing::XS);
        }

        ui.add_space(ui::theme::spacing::MD);
        ui::theme::widgets::card_frame()
            .show(ui, |ui| {
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
            ui::icons::FOLDER,
            "Organize Files",
            "Open a folder from Explorer or choose one here, preview the safe changes, then organize with undo available afterward.",
        );
        self.render_status_messages(ui);
        ui.add_space(10.0);

        let mut requested_step = None;
        render_organize_step_indicator(
            ui,
            self.organize_step,
            self.furthest_organize_step,
            &mut requested_step,
        );
        if let Some(step) = requested_step {
            self.go_to_organize_step(step);
        }
        ui.add_space(10.0);

        if self.organize_step == OrganizeStep::Folder {
            egui::Frame::group(ui.style())
                .fill(ui::theme::colors::surface())
                .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("Choose a folder to organize")
                                    .strong()
                                    .size(18.0)
                                    .color(ui::theme::colors::heading_text()),
                            );
                            ui.label(
                                RichText::new("Browse or drop a folder anywhere in this section.")
                                    .color(ui::theme::colors::secondary_text()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            render_folder_status_light(
                                ui,
                                self.has_selected_root(),
                                self.launched_with_preselected_root,
                            );
                        });
                    });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let root_input_width = (ui.available_width() - 132.0).max(320.0);
                        let response = ui.add_sized(
                            [root_input_width, 36.0],
                            egui::TextEdit::singleline(&mut self.root_input)
                                .hint_text("D:\\Documents"),
                        );
                        if response.changed() {
                            self.launched_with_preselected_root = false;
                            self.invalidate_preview_state();
                            if !self.has_selected_root() {
                                self.furthest_organize_step = OrganizeStep::Folder;
                                self.organize_step = OrganizeStep::Folder;
                            }
                        }
                        if ui
                            .add(ui::theme::widgets::secondary_button("Browse..."))
                            .clicked()
                        {
                            self.browse_for_root();
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(self.root_readiness_copy())
                            .color(ui::theme::colors::secondary_text()),
                    );

                    ui.add_space(8.0);
                    render_safety_line(ui, "Nothing is moved during analysis.");
                    render_safety_line(ui, "You will preview all changes before organizing files.");

                    if !self.preferences.recent_folders.is_empty() {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Recent folders")
                                .strong()
                                .color(ui::theme::colors::heading_text()),
                        );
                        let recent_folders = self.preferences.recent_folders.clone();
                        ui.horizontal_wrapped(|ui| {
                            for folder in recent_folders.into_iter().take(4) {
                                let label = folder_name_label(&folder);
                                if ui.button(label).clicked() {
                                    self.choose_recent_root(folder);
                                }
                            }
                        });
                    }
                });
        }

        if self.organize_step == OrganizeStep::Style {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Choose instructions for this folder")
                    .strong()
                    .size(18.0)
                    .color(ui::theme::colors::heading_text()),
            );
            ui.label(
                RichText::new(
                    "Pick one rule on the left. The panel on the right shows how smartfolder will organize files with it.",
                )
                .color(ui::theme::colors::secondary_text()),
            );
            ui.add_space(8.0);
            let selected_instruction = self.selected_instruction_preset();
            let mut requested_instruction = None;

            let panel_gap = 10.0;
            let panel_margin = 10.0;
            let picker_width = 220.0;
            let minimum_detail_width = 330.0;
            let side_by_side_min_width =
                picker_width + minimum_detail_width + panel_gap + (panel_margin * 4.0);

            if ui.available_width() >= side_by_side_min_width {
                let total_width = ui.available_width();
                let detail_width = (total_width - picker_width - panel_gap - (panel_margin * 4.0))
                    .max(minimum_detail_width);
                ui.horizontal(|ui| {
                    egui::Frame::group(ui.style())
                        .fill(ui::theme::colors::surface())
                        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
                        .inner_margin(egui::Margin::same(panel_margin))
                        .show(ui, |ui| {
                            ui.set_width(picker_width);
                            ui.set_max_width(picker_width);
                            ui.set_min_height(INSTRUCTION_PANEL_HEIGHT);
                            render_instruction_picker(
                                ui,
                                selected_instruction,
                                &mut requested_instruction,
                            );
                        });
                    ui.add_space(panel_gap);
                    let detail_instruction = requested_instruction.unwrap_or(selected_instruction);
                    egui::Frame::group(ui.style())
                        .fill(ui::theme::colors::surface())
                        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
                        .inner_margin(egui::Margin::same(panel_margin))
                        .show(ui, |ui| {
                            ui.set_width(detail_width);
                            ui.set_max_width(detail_width);
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .max_height(INSTRUCTION_PANEL_HEIGHT)
                                .show(ui, |ui| {
                                    self.render_instruction_detail_panel(ui, detail_instruction);
                                });
                        });
                });
            } else {
                egui::Frame::group(ui.style())
                    .fill(ui::theme::colors::surface())
                    .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
                    .inner_margin(egui::Margin::same(panel_margin))
                    .show(ui, |ui| {
                        ui.set_max_width(picker_width);
                        ui.set_min_height(INSTRUCTION_PANEL_HEIGHT);
                        render_instruction_picker(
                            ui,
                            selected_instruction,
                            &mut requested_instruction,
                        );
                    });
                ui.add_space(10.0);
                let detail_instruction = requested_instruction.unwrap_or(selected_instruction);
                egui::Frame::group(ui.style())
                    .fill(ui::theme::colors::surface())
                    .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
                    .inner_margin(egui::Margin::same(panel_margin))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(INSTRUCTION_PANEL_HEIGHT)
                            .show(ui, |ui| {
                                self.render_instruction_detail_panel(ui, detail_instruction);
                            });
                    });
            }

            if let Some(preset) = requested_instruction {
                self.select_instruction_preset(preset);
            }

            ui.add_space(10.0);
            egui::CollapsingHeader::new("Advanced options")
                .default_open(self.preferences.advanced_options_open)
                .show(ui, |ui| {
                    ui.checkbox(&mut self.include_subfolders, "Include subfolders");
                    ui.label("By default, smartfolder analyzes only the selected folder.");
                });
        }

        ui.add_space(12.0);
        let mut nav_action = None;
        render_organize_step_controls(
            ui,
            self.organize_step,
            self.has_selected_root(),
            self.can_run_analysis(),
            self.analysis_result.as_ref(),
            self.is_analyzing() || self.is_applying() || self.is_undoing(),
            &mut nav_action,
        );
        if let Some(action) = nav_action {
            match action {
                OrganizeNavAction::Back => self.go_back_one_organize_step(),
                OrganizeNavAction::Continue => match self.organize_step {
                    OrganizeStep::Folder => self.continue_from_folder_step(),
                    OrganizeStep::Style => self.continue_from_style_step(),
                    OrganizeStep::Preview => self.continue_from_preview_step(),
                    OrganizeStep::Organize => {}
                },
                OrganizeNavAction::Reanalyze => self.start_analysis(),
            }
        }
        ui.add_space(8.0);

        if self.organize_step == OrganizeStep::Preview && self.is_analyzing() {
            ui.add_space(10.0);
            if let Some(progress) = &self.analysis_progress {
                render_analysis_progress(ui, ctx, progress);
            }
            if ui
                .add(ui::theme::widgets::secondary_button("Cancel Analysis"))
                .clicked()
            {
                self.cancel_analysis();
            }
        }

        if self.organize_step == OrganizeStep::Organize && self.is_applying() {
            ui.add_space(10.0);
            if let Some(progress) = &self.apply_progress {
                render_apply_progress(ui, progress);
            }
        }

        if self.organize_step == OrganizeStep::Organize && self.is_undoing() {
            ui.add_space(10.0);
            render_undo_progress(ui);
        }

        if let Some(result) = &self.analysis_result {
            if self.organize_step == OrganizeStep::Organize {
                ui.add_space(14.0);
                if let Some(apply_result) = &self.apply_result {
                    render_apply_result(
                        ui,
                        apply_result,
                        self.is_analyzing() || self.is_applying() || self.is_undoing(),
                        history_action,
                    );
                } else {
                    render_plan_summary(ui, result, true);
                    ui.add_space(10.0);
                    render_apply_entry(
                        ui,
                        result,
                        self.is_applying(),
                        false,
                        &mut self.show_apply_confirmation,
                    );
                }
            } else {
                ui.add_space(14.0);
                render_plan_summary(ui, result, false);
            }

            if self.organize_step == OrganizeStep::Preview {
                ui.add_space(10.0);
                render_preview_examples(ui, result);

                ui.add_space(8.0);
                render_safety_line(ui, "Preview first. Nothing moves until you confirm.");

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
                if ui
                    .add(ui::theme::widgets::secondary_button(
                        if self.show_detailed_preview {
                            "Hide Detailed File List"
                        } else {
                            "View Detailed File List"
                        },
                    ))
                    .clicked()
                {
                    self.show_detailed_preview = !self.show_detailed_preview;
                }
                ui.add_space(6.0);
                ui.label("Opens the searchable detailed file list in a separate window.");
            }

            if self.organize_step == OrganizeStep::Preview && self.show_detailed_preview {
                let mut show_window = self.show_detailed_preview;
                let mut close_requested = false;
                egui::Window::new(
                    RichText::new("Detailed File List")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                )
                    .open(&mut show_window)
                    .default_size([920.0, 560.0])
                    .min_size([760.0, 420.0])
                    .collapsible(false)
                    .resizable(true)
                    .show(ctx, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new("Detailed File List")
                                    .strong()
                                    .size(ui::theme::typography::SECTION_TITLE)
                                    .color(ui::theme::colors::heading_text()),
                            );
                            ui.add_space(12.0);
                            if ui
                                .add_sized(
                                    [128.0, 34.0],
                                    ui::theme::widgets::secondary_button("Close Window"),
                                )
                                .clicked()
                            {
                                close_requested = true;
                            }
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "Search, filter, and inspect exact destinations before organizing files.",
                                )
                                .color(ui::theme::colors::secondary_text()),
                            )
                            .wrap(),
                        );
                        ui.add_space(10.0);
                        render_preview_controls(
                            ui,
                            result,
                            self.preview_filter,
                            self.preview_offset,
                            preview_action,
                        );
                        ui.add_space(8.0);
                        render_preview_table_header(ui);
                        ui.add_space(4.0);
                        let mut selected_preview_row = self.selected_preview_row;
                        let list_height = (ui.available_height() - 170.0).clamp(120.0, 280.0);
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(list_height)
                            .show(ui, |ui| {
                                render_preview_rows(ui, result, &mut selected_preview_row);
                            });
                        self.selected_preview_row = selected_preview_row;

                        ui.add_space(8.0);
                        render_preview_detail(ui, result, self.selected_preview_row);
                    });
                if close_requested {
                    show_window = false;
                }
                self.show_detailed_preview = show_window;
            }
        }
    }

    fn render_activity_screen(
        &mut self,
        ui: &mut egui::Ui,
        history_action: &mut Option<HistoryAction>,
    ) {
        render_screen_heading(
            ui,
            ui::icons::ACTIVITY,
            "Activity",
            "Review recent organization runs for this folder and undo changes when you need to restore the original layout.",
        );
        self.render_status_messages(ui);
        ui.add_space(10.0);

        let activity_scope = self.active_root().map_or_else(
            || "Showing recorded activity from all folders on this device.".to_string(),
            |root| format!("Showing activity for {}.", folder_name_label(&root)),
        );
        ui.horizontal_wrapped(|ui| {
            render_info_card(
                ui,
                "Current scope",
                &activity_scope,
                if self.active_root().is_some() {
                    "Folder filtered"
                } else {
                    "All folders"
                },
            );
            render_info_card(
                ui,
                "Undo Changes",
                "Completed activities stay reversible. smartfolder restores from recorded history instead of guessing what changed.",
                "Restore ready",
            );
            render_info_card(
                ui,
                "History details",
                "Open details only when you need exact paths, skipped files, or recovery notes.",
                "Progressive details",
            );
        });

        ui.add_space(10.0);
        ui::theme::widgets::card_frame().show(ui, |ui| {
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
        });
    }

    fn render_rules_screen(&mut self, ui: &mut egui::Ui) {
        render_screen_heading(
            ui,
            ui::icons::RULES,
            "Rules",
            "Use a built-in style or manage a simple custom rule profile for folders that need a more specific destination.",
        );
        self.render_status_messages(ui);
        ui.add_space(10.0);

        ui.horizontal_wrapped(|ui| {
            render_info_card(
                ui,
                "Built-in styles",
                "Choose a recommended layout here, then jump back to Organize with the same selection already active.",
                "Wizard linked",
            );
            render_info_card(
                ui,
                "Custom Rules",
                "A custom profile adds one specific destination strategy without giving up preview-first safety.",
                "Preview still required",
            );
        });

        ui.add_space(10.0);
        ui::theme::widgets::card_frame().show(ui, |ui| {
            ui.label(RichText::new("Built-in styles").strong().size(18.0));
            ui.label(
                "These are the ready-made organization styles available from the Organize screen.",
            );
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if render_style_card(
                    ui,
                    self.planning_source == PlanningSource::BuiltIn
                        && self.mode == BuiltInMode::Type,
                    "By Type",
                    "Images / PDFs / Videos",
                    "Use this when a folder has mixed file kinds.",
                )
                .clicked()
                {
                    self.select_builtin_style(BuiltInMode::Type);
                    self.active_section = AppSection::Organize;
                }

                if render_style_card(
                    ui,
                    self.planning_source == PlanningSource::BuiltIn
                        && self.mode == BuiltInMode::Date,
                    "By Date",
                    "2026 / May / 13",
                    "Use this when time is the clearest grouping.",
                )
                .clicked()
                {
                    self.select_builtin_style(BuiltInMode::Date);
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
                    self.select_builtin_style(BuiltInMode::TypeYear);
                    self.active_section = AppSection::Organize;
                }
            });
        });

        ui.add_space(12.0);
        ui::theme::widgets::card_frame().show(ui, |ui| {
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
                if ui
                    .add(ui::theme::widgets::primary_button("Use Custom Rules"))
                    .clicked()
                {
                    self.select_custom_rules_style();
                    self.active_section = AppSection::Organize;
                }
                if ui
                    .add(ui::theme::widgets::secondary_button("Validate profile"))
                    .clicked()
                {
                    self.validate_profile_editor();
                }
                if ui
                    .add(ui::theme::widgets::secondary_button("Save profile"))
                    .clicked()
                {
                    self.save_profile_from_editor();
                }
            });
            ui.add_space(6.0);
            render_safety_line(
                ui,
                "Custom Rules still go through the same preview, conflict checks, and undo flow.",
            );
            egui::CollapsingHeader::new("Advanced TOML actions")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(
                        "Import or export TOML when you want to share or hand-edit a profile.",
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .add(ui::theme::widgets::secondary_button("Import TOML..."))
                            .clicked()
                        {
                            self.import_rule_profile();
                        }
                        if ui
                            .add(ui::theme::widgets::secondary_button("Export TOML..."))
                            .clicked()
                        {
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
            ui::icons::SETTINGS,
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
                "Theme and motion preferences are saved locally as the RC2 design system comes online.",
                "Preferences saved",
            );
            render_info_card(
                ui,
                "Keyboard navigation",
                "Use Alt+1 through Alt+4 to move between Organize, Activity, Rules, and Settings without reaching for the sidebar.",
                "Section shortcuts",
            );
        });

        ui.add_space(10.0);
        ui::theme::widgets::card_frame().show(ui, |ui| {
            ui.label(RichText::new("Appearance and motion").strong().size(18.0));
            ui.label("These preferences are saved locally and will carry into the RC2 wizard.");
            ui.add_space(8.0);

            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("Theme");
                egui::ComboBox::from_id_source("theme-preference")
                    .selected_text(self.preferences.theme.label())
                    .show_ui(ui, |ui| {
                        for preference in [
                            ThemePreference::System,
                            ThemePreference::Light,
                            ThemePreference::Dark,
                        ] {
                            changed |= ui
                                .selectable_value(
                                    &mut self.preferences.theme,
                                    preference,
                                    preference.label(),
                                )
                                .changed();
                        }
                    });

                ui.label("Motion");
                egui::ComboBox::from_id_source("motion-preference")
                    .selected_text(self.preferences.motion.label())
                    .show_ui(ui, |ui| {
                        for preference in [
                            MotionPreference::System,
                            MotionPreference::Reduced,
                            MotionPreference::Subtle,
                            MotionPreference::Full,
                        ] {
                            changed |= ui
                                .selectable_value(
                                    &mut self.preferences.motion,
                                    preference,
                                    preference.label(),
                                )
                                .changed();
                        }
                    });
            });

            if changed {
                self.save_preferences_with_message();
            }
        });

        ui.add_space(10.0);
        ui::theme::widgets::card_frame()
            .show(ui, |ui| {
                ui.label(RichText::new("Explorer integration").strong().size(18.0));
                ui.label("The folder context menu entry should read Organize with smartfolder.");
                render_safety_line(ui, "It only opens the app with the clicked folder selected.");
                render_safety_line(ui, "It never organizes files directly from Explorer.");
                ui.add_space(8.0);
                ui.label("Register or unregister it with scripts/register-explorer-launcher.ps1 after building the release GUI.");
            });

        ui.add_space(10.0);
        ui::theme::widgets::card_frame()
            .show(ui, |ui| {
                ui.label(RichText::new("Storage maintenance").strong().size(18.0));
                ui.label("Cleanup removes old cached analysis sessions and preview pages. It does not remove restore history for organized files.");
                if ui
                    .add_enabled(
                        !self.is_analyzing() && !self.is_applying() && !self.is_undoing(),
                        ui::theme::widgets::secondary_button("Clean old session data"),
                    )
                    .clicked()
                {
                    self.cleanup_old_sessions();
                }
            });
    }

    fn render_profile_editor(&mut self, ui: &mut egui::Ui) {
        ui::theme::widgets::card_frame().show(ui, |ui| {
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
                        if ui
                            .add(ui::theme::widgets::secondary_button("Validate profile"))
                            .clicked()
                        {
                            self.validate_profile_editor();
                        }
                        if ui
                            .add(ui::theme::widgets::primary_button("Save profile"))
                            .clicked()
                        {
                            self.save_profile_from_editor();
                        }
                        if ui
                            .add(ui::theme::widgets::secondary_button("Export profile..."))
                            .clicked()
                        {
                            self.export_profile_from_editor();
                        }
                        if ui
                            .add(ui::theme::widgets::secondary_button("New profile"))
                            .clicked()
                        {
                            self.profile_editor = ProfileEditorState::default();
                            self.loaded_profile = None;
                        }
                    });
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
        self.process_keyboard_shortcuts(ctx);
        self.process_dropped_folders(ctx);
        ui::theme::apply_visual_theme(ctx, self.preferences.visual_theme());

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
        &self,
        plan_id: impl Into<String>,
        created_at: chrono::DateTime<Utc>,
    ) -> PlanOptions {
        match self {
            Self::BuiltIn(mode) => PlanOptions::built_in(*mode, plan_id, created_at),
            Self::RuleProfile(profile) => {
                PlanOptions::rule_profile(profile.clone(), plan_id, created_at)
            }
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
    preview_examples: Vec<PreviewRow>,
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
    let preview_examples = load_preview_examples_from_store(&store, &session_id, root)?;
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
        preview_examples,
        preview_total_rows: preview_page.total_rows,
        preview_rows: preview_page.rows,
    })
}

fn load_preview_examples_from_store(
    store: &SqliteSessionStore,
    session_id: &str,
    root: &Path,
) -> std::result::Result<Vec<PreviewRow>, String> {
    let operations = store
        .representative_plan_examples(session_id, PREVIEW_EXAMPLE_LIMIT)
        .map_err(|error| format!("Failed to load preview examples: {error}"))?;
    Ok(preview_rows(&operations, root))
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

fn render_plan_summary(ui: &mut egui::Ui, result: &AnalysisOutput, compact: bool) {
    let ready = result.preview_counts.ready;
    let needs_attention = result.preview_counts.needs_attention;
    let left_in_place = result.summary.ambiguous_files + result.summary.skipped;
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

        let card_gap = ui.spacing().item_spacing.x;
        let card_width = ((aligned_width - (card_gap * 2.0)) / 3.0).max(160.0);
        ui.horizontal(|ui| {
            render_preview_metric_card(
                ui,
                card_width,
                "Ready",
                ready,
                "Safe to move",
                Color32::from_rgb(231, 241, 228),
                Color32::from_rgb(92, 128, 78),
            );
            render_preview_metric_card(
                ui,
                card_width,
                "Review",
                needs_attention,
                "Needs attention",
                Color32::from_rgb(248, 238, 217),
                Color32::from_rgb(170, 110, 35),
            );
            render_preview_metric_card(
                ui,
                card_width,
                "Untouched",
                left_in_place,
                "Will stay put",
                Color32::from_rgb(243, 238, 234),
                Color32::from_rgb(116, 103, 90),
            );
        });

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
    });
}

fn render_preview_metric_card(
    ui: &mut egui::Ui,
    width: f32,
    title: &str,
    value: usize,
    detail: &str,
    fill: Color32,
    stroke: Color32,
) {
    ui.allocate_ui(egui::vec2(width, 76.0), |ui| {
        egui::Frame::group(ui.style())
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, stroke))
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                ui.set_width(width - 16.0);
                ui.label(
                    RichText::new(title)
                        .strong()
                        .size(ui::theme::typography::BODY)
                        .color(ui::theme::colors::heading_text()),
                );
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(value.to_string())
                            .size(18.0)
                            .strong()
                            .color(stroke),
                    );
                    ui.label(
                        RichText::new(detail)
                            .size(ui::theme::typography::CAPTION)
                            .color(ui::theme::colors::secondary_text()),
                    );
                });
            });
    });
}

fn preview_aligned_content_width(ui: &egui::Ui) -> f32 {
    let stepper_width = (ORGANIZE_STEP_BUTTON_WIDTH * OrganizeStep::ALL.len() as f32)
        + (ui.spacing().item_spacing.x * (OrganizeStep::ALL.len().saturating_sub(1) as f32))
        + (ORGANIZE_STEP_FRAME_MARGIN * 2.0);
    ui.available_width().min(stepper_width)
}

fn render_preview_examples(ui: &mut egui::Ui, result: &AnalysisOutput) {
    let aligned_width = preview_aligned_content_width(ui);

    ui.scope(|ui| {
        ui.set_max_width(aligned_width);
        ui.label(
            RichText::new("Example changes")
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        ui.add(
            egui::Label::new(
                RichText::new("Representative moves from this preview.")
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
        ui.add_space(6.0);

        if result.preview_examples.is_empty() {
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(250, 246, 240))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
                .inner_margin(egui::Margin::same(14.0))
                .show(ui, |ui| {
                    ui.label(RichText::new("No ready examples yet").strong());
                    ui.label("Files with unclear destinations were left untouched. Open the detailed list to inspect them.");
                });
            return;
        }

        for row in &result.preview_examples {
            render_preview_example_row(ui, row);
            ui.add_space(3.0);
        }
    });
}

fn render_preview_example_row(ui: &mut egui::Ui, row: &PreviewRow) {
    egui::Frame::group(ui.style())
        .fill(ui::theme::colors::elevated_surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(6.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Before:")
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::metadata_text()),
                );
                ui.label(
                    RichText::new(format!("./{}", row.file_name))
                        .monospace()
                        .color(ui::theme::colors::primary_text()),
                );
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("After:")
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::metadata_text()),
                );
                render_preview_path_highlight(ui, &preview_example_destination_path(row));
                ui.label(
                    RichText::new(&row.file_name)
                        .monospace()
                        .color(ui::theme::colors::primary_text()),
                );
            });
        });
}

fn preview_example_destination_path(row: &PreviewRow) -> String {
    if row.target_folder == "Selected folder" {
        "./".to_string()
    } else {
        format!("./{}/", row.target_folder.replace(" / ", "/"))
    }
}

fn render_preview_path_highlight(ui: &mut egui::Ui, path: &str) -> egui::Response {
    let frame = egui::Frame::none()
        .fill(ui::theme::colors::hover_control())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::primary_blue()))
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(path)
                        .monospace()
                        .color(ui::theme::colors::primary_blue()),
                )
                .sense(egui::Sense::hover()),
            )
        });
    frame.response.union(frame.inner)
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
    let aligned_width = preview_aligned_content_width(ui);
    ui.scope(|ui| {
        ui.set_max_width(aligned_width);
        egui::Frame::group(ui.style())
            .fill(ui::theme::colors::elevated_surface())
            .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
            .inner_margin(egui::Margin::same(14.0))
            .show(ui, |ui| {
                let button_width = 168.0;
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
                                        "Organize Files moves the ready items and leaves review or untouched items in place.",
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
                                    RichText::new("Organize Files")
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

fn render_apply_confirmation(
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
                    Color32::from_rgb(231, 241, 228),
                    Color32::from_rgb(92, 128, 78),
                );
                render_preview_metric_card(
                    ui,
                    card_width,
                    "Review",
                    result.preview_counts.needs_attention,
                    "Stays put",
                    Color32::from_rgb(248, 238, 217),
                    Color32::from_rgb(170, 110, 35),
                );
            });

            ui.add_space(10.0);
            render_safety_line(ui, "Existing files will not be overwritten.");
            render_safety_line(ui, "Restore history is recorded before files move.");
            render_safety_line(ui, "Undo Changes will be available after completion.");

            if is_cloud_synced_path(&result.root) {
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "This folder appears to be cloud-synced. Let sync settle before organizing and review the completion summary afterward.",
                        )
                        .color(Color32::from_rgb(170, 110, 35)),
                    )
                    .wrap(),
                );
            }

            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(ui::theme::widgets::primary_button("Organize Files"))
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

fn render_apply_result(
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
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(92, 128, 78)))
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
                            "{} file{} organized. Undo Changes is ready if you want to restore the original layout.",
                            result.completed,
                            plural(result.completed)
                        ))
                        .color(ui::theme::colors::primary_text()),
                    )
                    .wrap(),
                );
                ui.add_space(10.0);

                let card_gap = ui.spacing().item_spacing.x;
                let card_width = ((aligned_width - (card_gap * 2.0)) / 3.0).max(160.0);
                ui.horizontal(|ui| {
                    render_preview_metric_card(
                        ui,
                        card_width,
                        "Organized",
                        result.completed,
                        "Moved successfully",
                        Color32::from_rgb(231, 241, 228),
                        Color32::from_rgb(92, 128, 78),
                    );
                    render_preview_metric_card(
                        ui,
                        card_width,
                        "Skipped",
                        result.skipped,
                        "Left untouched",
                        Color32::from_rgb(243, 238, 234),
                        Color32::from_rgb(116, 103, 90),
                    );
                    render_preview_metric_card(
                        ui,
                        card_width,
                        "Failed",
                        result.failed,
                        "Need attention",
                        Color32::from_rgb(248, 238, 217),
                        Color32::from_rgb(170, 110, 35),
                    );
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy, ui::theme::widgets::secondary_button("Undo Changes"))
                        .clicked()
                    {
                        *action = Some(HistoryAction::ConfirmUndo(result.transaction_id.clone()));
                    }
                    if ui
                        .add_enabled(!busy, ui::theme::widgets::secondary_button("View Details"))
                        .clicked()
                    {
                        *action = Some(HistoryAction::ViewDetails(result.transaction_id.clone()));
                    }
                });
                ui.add_space(8.0);
                egui::CollapsingHeader::new("Restore history details")
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
    } else if result.preview_counts.needs_attention == 0 {
        format!(
            "{} safe file{} ready, with {} item{} left untouched.",
            result.preview_counts.ready,
            plural(result.preview_counts.ready),
            result.summary.ambiguous_files,
            plural(result.summary.ambiguous_files)
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

fn render_preview_controls(
    ui: &mut egui::Ui,
    result: &AnalysisOutput,
    active_filter: PreviewFilter,
    offset: usize,
    action: &mut Option<PreviewAction>,
) {
    ui.label(
        RichText::new("File list")
            .strong()
            .size(ui::theme::typography::CARD_TITLE)
            .color(ui::theme::colors::heading_text()),
    );
    ui.add(
        egui::Label::new(
            RichText::new(
                "Click a file to inspect its original folder, exact destination, and rule details below.",
            )
            .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
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
        ui.label(RichText::new(range_text).color(ui::theme::colors::primary_text()));

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
    let (file_width, target_width) = preview_table_column_widths(width);

    egui::Grid::new("preview-rows")
        .num_columns(2)
        .striped(true)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
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
                preview_destination_cell_with_tooltip(
                    ui,
                    row,
                    target_width,
                    format!("Full destination: {}", row.destination_full_path),
                );
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

fn render_preview_table_header(ui: &mut egui::Ui) {
    let width = ui.available_width().max(360.0);
    let (file_width, target_width) = preview_table_column_widths(width);

    egui::Frame::none()
        .fill(ui::theme::colors::soft_control())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::symmetric(6.0, 4.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                preview_cell(ui, "File name", file_width, true);
                preview_cell(ui, "Target folder", target_width, true);
            });
        });
}

fn preview_table_column_widths(width: f32) -> (f32, f32) {
    let file_width = (width * 0.44).max(220.0);
    let target_width = (width - file_width - 18.0).max(260.0);
    (file_width, target_width)
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
    let text = text.into();
    let text_color = if selected {
        ui::theme::colors::on_primary()
    } else {
        ui::theme::colors::primary_text()
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width.max(40.0), 20.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            ui::theme::colors::primary_blue()
        } else if response.hovered() {
            ui::theme::colors::hover_control()
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(2.0), fill);
        ui.painter().text(
            rect.left_center() + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::TextStyle::Monospace.resolve(ui.style()),
            text_color,
        );
    }
    response.on_hover_text(tooltip)
}

fn preview_cell_response(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    strong: bool,
) -> egui::Response {
    let text_color = if strong {
        ui::theme::colors::heading_text()
    } else {
        ui::theme::colors::primary_text()
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width.max(40.0), 18.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().text(
            rect.left_center() + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::TextStyle::Monospace.resolve(ui.style()),
            text_color,
        );
    }
    response
}

fn preview_destination_cell_with_tooltip(
    ui: &mut egui::Ui,
    row: &PreviewRow,
    width: f32,
    tooltip: impl Into<egui::WidgetText>,
) {
    let path = preview_example_destination_path(row);
    let response = ui
        .allocate_ui(egui::vec2(width.max(40.0), 22.0), |ui| {
            render_preview_path_highlight(ui, &path)
        })
        .inner;
    response.on_hover_text(tooltip);
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

    ui.label(
        RichText::new("Selected change")
            .strong()
            .size(ui::theme::typography::CARD_TITLE)
            .color(ui::theme::colors::heading_text()),
    );
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(250, 246, 240))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&row.file_name)
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                let (stroke, fill) = preview_status_colors(&row.status);
                render_status_chip(ui, &row.status, stroke, fill);
            });
            ui.add_space(8.0);
            egui::Grid::new("preview-detail-grid")
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Original folder").color(ui::theme::colors::metadata_text()),
                    );
                    ui.label(
                        RichText::new(&row.original_folder)
                            .color(ui::theme::colors::primary_text()),
                    );
                    ui.end_row();
                    ui.label(
                        RichText::new("Destination").color(ui::theme::colors::metadata_text()),
                    );
                    render_preview_path_highlight(ui, &preview_example_destination_path(row));
                    ui.end_row();
                    ui.label(RichText::new("Why").color(ui::theme::colors::metadata_text()));
                    ui.label(RichText::new(&row.reason).color(ui::theme::colors::primary_text()));
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

fn render_screen_heading(ui: &mut egui::Ui, icon: &str, title: &str, detail: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(icon)
                .size(22.0)
                .color(ui::theme::colors::primary_blue()),
        );
        ui.label(
            RichText::new(title)
                .size(30.0)
                .strong()
                .color(ui::theme::colors::heading_text()),
        );
    });
    ui.label(RichText::new(detail).color(ui::theme::colors::secondary_text()));
}

fn render_instruction_picker(
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
            Color32::WHITE
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

fn render_organize_step_indicator(
    ui: &mut egui::Ui,
    current: OrganizeStep,
    furthest: OrganizeStep,
    requested_step: &mut Option<OrganizeStep>,
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(250, 246, 240))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(220, 206, 190)))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for step in OrganizeStep::ALL {
                    let is_current = step == current;
                    let is_available = step <= furthest;
                    let label = format!("{}. {}", step.number(), step.title());
                    let button =
                        egui::Button::new(RichText::new(label).strong().color(if is_current {
                            Color32::from_rgb(47, 128, 237)
                        } else if is_available {
                            Color32::from_rgb(45, 45, 45)
                        } else {
                            Color32::from_rgb(123, 123, 123)
                        }))
                        .fill(if is_current {
                            Color32::from_rgb(239, 244, 252)
                        } else if is_available {
                            Color32::from_rgb(255, 255, 255)
                        } else {
                            Color32::from_rgb(243, 238, 234)
                        })
                        .stroke(egui::Stroke::new(
                            if is_current { 2.0 } else { 1.0 },
                            if is_current {
                                Color32::from_rgb(47, 128, 237)
                            } else {
                                Color32::from_rgb(216, 210, 200)
                            },
                        ))
                        .min_size(egui::vec2(150.0, 40.0));

                    let response = ui
                        .add_enabled(is_available, button)
                        .on_hover_text(step.subtitle());
                    if response.clicked() {
                        *requested_step = Some(step);
                    }
                }
            });
        });
}

fn render_organize_step_controls(
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
                            ui::theme::widgets::primary_button("Continue to Organize"),
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
                        "Confirm only when the preview matches what you expect. Undo remains available afterward.",
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

fn render_status_chip(ui: &mut egui::Ui, text: &str, stroke: Color32, fill: Color32) {
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

fn render_folder_status_light(ui: &mut egui::Ui, has_root: bool, preselected: bool) {
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
        .fill(ui::theme::colors::surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            let card_width = ui.available_width().clamp(CARD_MIN_WIDTH, 320.0);
            ui.set_width(card_width);
            ui.label(
                RichText::new(title)
                    .strong()
                    .size(18.0)
                    .color(ui::theme::colors::heading_text()),
            );
            ui.label(RichText::new(detail).color(ui::theme::colors::secondary_text()));
            ui.add_space(6.0);
            render_status_chip(
                ui,
                status,
                ui::theme::colors::secondary_text(),
                ui::theme::colors::subtle_surface(),
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
    let card_width = ui.available_width().clamp(CARD_MIN_WIDTH, 320.0);
    ui.add_sized(
        [card_width, 120.0],
        egui::Button::new(
            RichText::new(format!("{title}\n{example}\n{detail}"))
                .size(14.0)
                .color(ui::theme::colors::primary_text()),
        )
        .stroke(egui::Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                ui::theme::colors::primary_blue()
            } else {
                ui::theme::colors::border()
            },
        ))
        .fill(if selected {
            ui::theme::colors::hover_control()
        } else {
            ui::theme::colors::surface()
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
            let card_width = ui.available_width().clamp(CARD_MIN_WIDTH, 320.0);
            ui.set_width(card_width);
            ui.label(
                RichText::new(title)
                    .strong()
                    .color(ui::theme::colors::heading_text()),
            );
            ui.label(
                RichText::new(value.to_string())
                    .size(28.0)
                    .strong()
                    .color(stroke),
            );
            ui.label(RichText::new(detail).color(ui::theme::colors::secondary_text()));
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

fn build_example_tree_entries(
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

fn render_instruction_example_tree(ui: &mut egui::Ui, entries: &[ExampleTreeEntry]) {
    let text_color = ui::theme::colors::primary_text();
    let branch_color = Color32::from_rgb(117, 117, 117);
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

fn sample_destination_template(template: &str) -> String {
    let mut rendered = template.trim().replace('\\', "/");
    for (token, replacement) in [
        ("{year}", "2026"),
        ("{month}", "05"),
        ("{day}", "13"),
        ("{ext}", "pdf"),
        ("{extension}", "pdf"),
        ("{stem}", "invoice-042"),
        ("{filename}", "invoice-042.pdf"),
        ("{name}", "invoice-042.pdf"),
        ("{type}", "Documents"),
        ("{category}", "Documents"),
    ] {
        rendered = rendered.replace(token, replacement);
    }
    rendered
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
