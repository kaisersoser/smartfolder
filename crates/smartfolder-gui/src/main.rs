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
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::derivable_impls,
    clippy::useless_conversion,
    clippy::match_same_arms,
    clippy::struct_excessive_bools,
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::if_not_else,
    clippy::too_many_arguments,
    clippy::map_unwrap_or,
    clippy::redundant_closure_for_method_calls,
    clippy::if_same_then_else
)]

mod preferences;
mod ui;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use eframe::egui::{self, Color32, RichText};
use smartfolder_core::ai::{
    validate_ai_profile_draft, AiFinding, AiFolderAnalysis, AiFolderContext, AiProfileValidation,
    AiPromptRefinement, AiProviderState, AiProviderStatus, AiRuleExplanation, AiRuleProfileDraft,
    OllamaClient,
};
use smartfolder_core::apply::{
    apply_stored_plan_with_progress, ApplyOptions, ApplySummary, StoredApplyProgress,
};
use smartfolder_core::model::{
    BuiltInMode, ConflictState, FileInventoryRecord, OperationStatus, PlanMode, PlanOperation,
    PlanSummary, TransactionStatus, UntouchedReason, UntouchedRecord,
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

use preferences::{GuiPreferences, StylePreference};
use ui::components::{
    constrained_page as render_constrained_page, note_frame as settings_note_frame,
    panel_frame as settings_panel_frame, safety_line as render_safety_line,
    screen_heading as render_screen_heading, status_chip as render_status_chip, truncated_label,
};
use ui::screens::settings::{
    render_advanced_ai_preferences, render_ai_preferences, render_appearance_preferences,
    render_explorer_integration_settings, render_privacy_preferences, render_settings_heading,
    render_settings_help_window, render_settings_section_panel, render_storage_maintenance,
};

type AnalysisMessage = std::result::Result<AnalysisOutput, String>;
type ApplyMessage = std::result::Result<ApplyOutput, String>;
type UndoMessage = std::result::Result<UndoOutput, String>;
type AiStatusMessage = std::result::Result<AiProviderStatus, String>;
type AiTaskMessage = std::result::Result<AiTaskOutput, String>;

const PREVIEW_PAGE_SIZE: usize = 100;
const TRANSACTION_DETAIL_ROW_LIMIT: usize = 100;
const WINDOW_WIDTH: f32 = 1160.0;
const WINDOW_HEIGHT: f32 = 780.0;
const SHELL_NAV_WIDTH: f32 = ui::theme::spacing::SIDEBAR_WIDTH;
const SHELL_NAV_COLLAPSED_WIDTH: f32 = ui::theme::spacing::SIDEBAR_COLLAPSED_WIDTH;
const CARD_MIN_WIDTH: f32 = 188.0;
const PREVIEW_EXAMPLE_LIMIT: usize = 3;
const INSTRUCTION_PANEL_HEIGHT: f32 = 216.0;
const PROFILE_WORKSPACE_FIELD_HEIGHT: f32 = 32.0;
const PROFILE_RULE_LIST_WIDTH: f32 = 252.0;
const MAX_AI_OPERATION_RECORDS: usize = 32;

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
            Self::Activity => "Review recent changes and restore previous layouts if needed.",
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
    preview_detail_mode: PreviewDetailMode,
    analysis_receiver: Option<Receiver<AnalysisEvent>>,
    analysis_cancellation: Option<CancellationToken>,
    analysis_progress: Option<AnalysisProgress>,
    analysis_result: Option<AnalysisOutput>,
    apply_receiver: Option<Receiver<ApplyEvent>>,
    apply_progress: Option<ApplyProgress>,
    apply_result: Option<ApplyOutput>,
    show_apply_confirmation: bool,
    undo_receiver: Option<Receiver<UndoMessage>>,
    ai_status_receiver: Option<Receiver<AiStatusMessage>>,
    ai_status_check_source: Option<AiStatusCheckSource>,
    ai_status: Option<AiProviderStatus>,
    ai_task_receiver: Option<Receiver<AiTaskMessage>>,
    ai_task: Option<AiTaskKind>,
    ai_task_started_at: Option<Instant>,
    ai_task_model: Option<String>,
    ai_analysis: Option<AiFolderAnalysis>,
    show_ai_notes: bool,
    ai_draft_prompt: String,
    show_ai_draft_prompt: bool,
    ai_draft_result: Option<AiDraftProfileResult>,
    show_ai_draft_review: bool,
    ai_rule_explanation: Option<AiRuleExplanation>,
    ai_operation_records: Vec<AiOperationRecord>,
    undo_result: Option<UndoOutput>,
    transaction_rows: Vec<TransactionRow>,
    transaction_message: Option<String>,
    transaction_detail: Option<TransactionDetail>,
    transaction_detail_message: Option<String>,
    show_recovery_log: bool,
    show_undo_confirmation: Option<String>,
    show_settings_help: bool,
    show_rules_workspace: bool,
    shell_nav_expanded: bool,
    show_command_palette: bool,
    command_palette_query: String,
    rule_simulation: Option<RuleSimulationResult>,
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

        let mut app = Self {
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
            preview_detail_mode: PreviewDetailMode::List,
            analysis_receiver: None,
            analysis_cancellation: None,
            analysis_progress: None,
            analysis_result: None,
            apply_receiver: None,
            apply_progress: None,
            apply_result: None,
            show_apply_confirmation: false,
            undo_receiver: None,
            ai_status_receiver: None,
            ai_status_check_source: None,
            ai_status: None,
            ai_task_receiver: None,
            ai_task: None,
            ai_task_started_at: None,
            ai_task_model: None,
            ai_analysis: None,
            show_ai_notes: false,
            ai_draft_prompt: String::new(),
            show_ai_draft_prompt: false,
            ai_draft_result: None,
            show_ai_draft_review: false,
            ai_rule_explanation: None,
            ai_operation_records: Vec::new(),
            undo_result: None,
            transaction_rows,
            transaction_message,
            transaction_detail: None,
            transaction_detail_message: None,
            show_recovery_log: false,
            show_undo_confirmation: None,
            show_settings_help: false,
            show_rules_workspace: false,
            shell_nav_expanded: false,
            show_command_palette: false,
            command_palette_query: String::new(),
            rule_simulation: None,
            error_message: None,
            maintenance_message: preferences_message,
        };
        app.startup_ai_status_check();
        app
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

    fn is_checking_ai_status(&self) -> bool {
        self.ai_status_receiver.is_some()
    }

    fn is_running_ai_task(&self) -> bool {
        self.ai_task_receiver.is_some()
    }

    fn ai_is_available(&self) -> bool {
        self.preferences.ai.enabled
            && self
                .ai_status
                .as_ref()
                .is_some_and(|status| status.available)
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

    fn customize_builtin_style(&mut self, mode: BuiltInMode) {
        self.profile_editor = ProfileEditorState::from_builtin(mode);
        self.loaded_profile = None;
        self.select_builtin_style(mode);
        self.show_rules_workspace = true;
        self.maintenance_message = Some(format!(
            "Created an editable draft from {}. Save it as a custom profile when ready.",
            Self::mode_label(mode)
        ));
        self.error_message = None;
    }

    fn use_saved_profile(&mut self, loaded_profile: LoadedRuleProfile) {
        let profile_id = loaded_profile.profile.profile_id.clone();
        self.profile_editor = ProfileEditorState::from_profile(&loaded_profile.profile);
        self.loaded_profile = Some(loaded_profile);
        self.select_custom_rules_style();
        self.maintenance_message = Some(format!("Using custom profile '{profile_id}'."));
        self.error_message = None;
    }

    fn edit_saved_profile(&mut self, loaded_profile: LoadedRuleProfile) {
        let profile_id = loaded_profile.profile.profile_id.clone();
        self.profile_editor = ProfileEditorState::from_profile(&loaded_profile.profile);
        self.loaded_profile = Some(loaded_profile);
        self.show_rules_workspace = true;
        self.maintenance_message = Some(format!("Editing custom profile '{profile_id}'."));
        self.error_message = None;
    }

    fn invalidate_preview_state(&mut self) {
        self.analysis_result = None;
        self.apply_result = None;
        self.undo_result = None;
        self.show_apply_confirmation = false;
        self.show_detailed_preview = false;
        self.preview_detail_mode = PreviewDetailMode::List;
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

    fn complete_apply_result(&mut self) {
        self.analysis_result = None;
        self.apply_result = None;
        self.apply_progress = None;
        self.undo_result = None;
        self.show_apply_confirmation = false;
        self.show_detailed_preview = false;
        self.preview_detail_mode = PreviewDetailMode::List;
        self.preview_filter = PreviewFilter::All;
        self.preview_offset = 0;
        self.selected_preview_row = None;
        self.organize_step = OrganizeStep::Folder;
        self.furthest_organize_step = if self.has_selected_root() {
            OrganizeStep::Style
        } else {
            OrganizeStep::Folder
        };
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
        self.preview_detail_mode = PreviewDetailMode::List;
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
        self.ai_analysis = None;
        self.ai_rule_explanation = None;
        self.ai_draft_result = None;
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

    fn startup_ai_status_check(&mut self) {
        #[cfg(test)]
        {
            return;
        }

        #[cfg(not(test))]
        if self.preferences.ai.enabled {
            self.start_ai_status_check(AiStatusCheckSource::Startup);
        }
    }

    fn start_ai_status_check(&mut self, source: AiStatusCheckSource) {
        if !self.preferences.ai.enabled {
            self.ai_status = Some(AiProviderStatus {
                available: false,
                state: AiProviderState::Disabled,
                selected_model: None,
                models: Vec::new(),
                message: "AI assistance is disabled.".to_string(),
            });
            self.ai_status_check_source = None;
            return;
        }

        let settings = self.preferences.ai.clone();
        let (sender, receiver) = mpsc::channel::<AiStatusMessage>();
        self.ai_status_receiver = Some(receiver);
        self.ai_status_check_source = Some(source);
        self.error_message = None;

        std::thread::spawn(move || {
            let timeout = settings.timeout();
            let client = OllamaClient::new(settings.endpoint, timeout);
            let status = client.check_status(settings.selected_model.as_deref());
            let _ = sender.send(Ok(status));
        });
    }

    fn poll_ai_status(&mut self) {
        let Some(receiver) = &self.ai_status_receiver else {
            return;
        };

        match receiver.try_recv() {
            Ok(message) => {
                self.ai_status_receiver = None;
                let source = self.ai_status_check_source.take();
                match message {
                    Ok(status) => {
                        if status.available
                            && self.preferences.ai.selected_model.is_none()
                            && status.selected_model.is_some()
                        {
                            self.preferences
                                .ai
                                .selected_model
                                .clone_from(&status.selected_model);
                            self.save_preferences_quietly();
                        }
                        if source == Some(AiStatusCheckSource::Manual) {
                            self.maintenance_message = Some(status.message.clone());
                        }
                        self.ai_status = Some(status);
                        self.error_message = None;
                    }
                    Err(message) => {
                        if source == Some(AiStatusCheckSource::Manual) {
                            self.ai_status = Some(AiProviderStatus {
                                available: false,
                                state: AiProviderState::RequestFailed,
                                selected_model: None,
                                models: Vec::new(),
                                message: message.clone(),
                            });
                            self.error_message = Some(message);
                        }
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.ai_status_receiver = None;
                let source = self.ai_status_check_source.take();
                if source == Some(AiStatusCheckSource::Manual) {
                    self.ai_status = Some(AiProviderStatus {
                        available: false,
                        state: AiProviderState::RequestFailed,
                        selected_model: None,
                        models: Vec::new(),
                        message: "The AI status worker stopped unexpectedly.".to_string(),
                    });
                    self.error_message =
                        Some("The AI status worker stopped unexpectedly.".to_string());
                }
            }
        }
    }

    fn ai_model_for_task(&self) -> std::result::Result<String, String> {
        if !self.ai_is_available() {
            return Err(
                "AI assistance is not ready. Test the Ollama connection in Settings first."
                    .to_string(),
            );
        }
        self.ai_status
            .as_ref()
            .and_then(|status| status.selected_model.clone())
            .or_else(|| self.preferences.ai.selected_model.clone())
            .ok_or_else(|| "Select an Ollama model in Settings first.".to_string())
    }

    fn begin_ai_task(&mut self, receiver: Receiver<AiTaskMessage>, kind: AiTaskKind, model: &str) {
        self.ai_task_receiver = Some(receiver);
        self.ai_task = Some(kind);
        self.ai_task_started_at = Some(Instant::now());
        self.ai_task_model = Some(model.to_string());
        self.error_message = None;
    }

    fn cancel_ai_task(&mut self) {
        if self.ai_task_receiver.is_some() {
            if let Some(kind) = self.ai_task {
                let outcome = Err("cancelled_by_user".to_string());
                let model = self.ai_task_model.take();
                let started_at = self.ai_task_started_at.take();
                self.record_ai_operation(kind, model, started_at, &outcome);
            }
            self.ai_task_receiver = None;
            self.ai_task = None;
            self.ai_task_started_at = None;
            self.ai_task_model = None;
            self.maintenance_message =
                Some("AI task cancelled. A late provider response will be ignored.".to_string());
            self.error_message = None;
        }
    }

    fn record_ai_operation(
        &mut self,
        kind: AiTaskKind,
        model: Option<String>,
        started_at: Option<Instant>,
        outcome: &AiTaskMessage,
    ) {
        let duration_ms = started_at.map(|started| started.elapsed().as_millis() as u64);
        let (success, failure_class, validation_warnings, validation_errors) = match outcome {
            Ok(AiTaskOutput::DraftProfile(result)) => (
                result.validation.errors.is_empty(),
                if result.validation.errors.is_empty() {
                    None
                } else {
                    Some("validation".to_string())
                },
                result.validation.warnings.len(),
                result.validation.errors.len(),
            ),
            Ok(_) => (true, None, 0, 0),
            Err(message) => (false, Some(ai_failure_class(message)), 0, 0),
        };
        self.ai_operation_records.push(AiOperationRecord {
            completed_at: Utc::now().to_rfc3339(),
            request_type: kind.label().to_string(),
            provider: "ollama".to_string(),
            model: model.unwrap_or_else(|| "unknown".to_string()),
            duration_ms,
            success,
            failure_class,
            validation_warnings,
            validation_errors,
        });
        if self.ai_operation_records.len() > MAX_AI_OPERATION_RECORDS {
            let overflow = self.ai_operation_records.len() - MAX_AI_OPERATION_RECORDS;
            self.ai_operation_records.drain(0..overflow);
        }
    }

    fn start_ai_folder_analysis(&mut self) {
        let Some(result) = &self.analysis_result else {
            self.error_message = Some("Run Analyze Folder before using AI Assist.".to_string());
            return;
        };
        let model = match self.ai_model_for_task() {
            Ok(model) => model,
            Err(message) => {
                self.error_message = Some(message);
                return;
            }
        };
        let endpoint = self.preferences.ai.endpoint.clone();
        let timeout = self.preferences.ai.timeout();
        let content_inspection_enabled = self.preferences.ai.content_inspection_enabled;
        let session_id = result.session_id.clone();
        let root = result.root.clone();
        let (sender, receiver) = mpsc::channel::<AiTaskMessage>();
        self.begin_ai_task(receiver, AiTaskKind::FolderAnalysis, &model);
        self.ai_analysis = None;

        std::thread::spawn(move || {
            let message = run_ai_folder_analysis(
                &endpoint,
                timeout,
                &model,
                &session_id,
                &root,
                content_inspection_enabled,
            );
            let _ = sender.send(message);
        });
    }

    fn start_ai_draft_profile(&mut self) {
        let prompt = self.ai_draft_prompt.trim().to_string();
        if prompt.is_empty() {
            self.error_message =
                Some("Describe the rule profile you want AI to draft.".to_string());
            return;
        }
        let Some(result) = &self.analysis_result else {
            self.error_message =
                Some("Run Analyze Folder before building rules with AI.".to_string());
            return;
        };
        let model = match self.ai_model_for_task() {
            Ok(model) => model,
            Err(message) => {
                self.error_message = Some(message);
                return;
            }
        };
        let endpoint = self.preferences.ai.endpoint.clone();
        let timeout = self.preferences.ai.timeout();
        let content_inspection_enabled = self.preferences.ai.content_inspection_enabled;
        let session_id = result.session_id.clone();
        let root = result.root.clone();
        let existing_profile = self.profile_editor.to_profile().ok();
        let (sender, receiver) = mpsc::channel::<AiTaskMessage>();
        self.begin_ai_task(receiver, AiTaskKind::DraftProfile, &model);
        self.ai_draft_result = None;

        std::thread::spawn(move || {
            let message = run_ai_profile_draft(
                &endpoint,
                timeout,
                &model,
                &session_id,
                &root,
                content_inspection_enabled,
                &prompt,
                existing_profile.as_ref(),
            );
            let _ = sender.send(message);
        });
    }

    fn start_ai_prompt_refinement(&mut self) {
        let prompt = self.ai_draft_prompt.trim().to_string();
        if prompt.is_empty() {
            self.error_message = Some("Write a prompt before asking AI to refine it.".to_string());
            return;
        }
        let Some(result) = &self.analysis_result else {
            self.error_message =
                Some("Run Analyze Folder before refining the AI prompt.".to_string());
            return;
        };
        let model = match self.ai_model_for_task() {
            Ok(model) => model,
            Err(message) => {
                self.error_message = Some(message);
                return;
            }
        };
        let endpoint = self.preferences.ai.endpoint.clone();
        let timeout = self.preferences.ai.timeout();
        let content_inspection_enabled = self.preferences.ai.content_inspection_enabled;
        let session_id = result.session_id.clone();
        let root = result.root.clone();
        let (sender, receiver) = mpsc::channel::<AiTaskMessage>();
        self.begin_ai_task(receiver, AiTaskKind::PromptRefinement, &model);

        std::thread::spawn(move || {
            let message = run_ai_prompt_refinement(
                &endpoint,
                timeout,
                &model,
                &session_id,
                &root,
                content_inspection_enabled,
                &prompt,
            );
            let _ = sender.send(message);
        });
    }

    fn start_ai_rule_explanation(&mut self) {
        let profile = match self.profile_editor.to_profile() {
            Ok(profile) => profile,
            Err(message) => {
                self.error_message = Some(format!(
                    "Validate the profile before explaining it: {message}"
                ));
                return;
            }
        };
        let Some(result) = &self.analysis_result else {
            self.error_message =
                Some("Run Analyze Folder before explaining rules with AI.".to_string());
            return;
        };
        let model = match self.ai_model_for_task() {
            Ok(model) => model,
            Err(message) => {
                self.error_message = Some(message);
                return;
            }
        };
        let endpoint = self.preferences.ai.endpoint.clone();
        let timeout = self.preferences.ai.timeout();
        let content_inspection_enabled = self.preferences.ai.content_inspection_enabled;
        let session_id = result.session_id.clone();
        let root = result.root.clone();
        let (sender, receiver) = mpsc::channel::<AiTaskMessage>();
        self.begin_ai_task(receiver, AiTaskKind::RuleExplanation, &model);

        std::thread::spawn(move || {
            let message = run_ai_rule_explanation(
                &endpoint,
                timeout,
                &model,
                &session_id,
                &root,
                content_inspection_enabled,
                &profile,
            );
            let _ = sender.send(message);
        });
    }

    fn poll_ai_task(&mut self) {
        let Some(receiver) = &self.ai_task_receiver else {
            return;
        };

        match receiver.try_recv() {
            Ok(message) => {
                let completed_kind = self.ai_task;
                let completed_model = self.ai_task_model.take();
                let started_at = self.ai_task_started_at.take();
                if let Some(kind) = completed_kind {
                    self.record_ai_operation(kind, completed_model, started_at, &message);
                }
                self.ai_task_receiver = None;
                self.ai_task = None;
                match message {
                    Ok(AiTaskOutput::FolderAnalysis(analysis)) => {
                        self.ai_analysis = Some(analysis);
                        self.maintenance_message =
                            Some("AI folder analysis completed.".to_string());
                        self.error_message = None;
                    }
                    Ok(AiTaskOutput::DraftProfile(result)) => {
                        if let Some(profile) = &result.validation.profile {
                            self.profile_editor = ProfileEditorState::from_profile(profile);
                            self.loaded_profile = None;
                        }
                        let message = ai_draft_summary_message(&result.validation);
                        self.ai_draft_result = Some(result);
                        self.show_ai_draft_prompt = false;
                        self.show_ai_draft_review = true;
                        self.maintenance_message = Some(message);
                        self.error_message = None;
                    }
                    Ok(AiTaskOutput::PromptRefinement(refinement)) => {
                        let notes = if refinement.notes.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", refinement.notes.join(" "))
                        };
                        self.ai_draft_prompt = refinement.refined_prompt;
                        self.maintenance_message =
                            Some(format!("AI refined the draft prompt.{notes}"));
                        self.error_message = None;
                    }
                    Ok(AiTaskOutput::RuleExplanation(explanation)) => {
                        self.ai_rule_explanation = Some(explanation);
                        self.maintenance_message =
                            Some("AI rule explanation completed.".to_string());
                        self.error_message = None;
                    }
                    Err(message) => {
                        self.error_message = Some(message);
                        self.maintenance_message = None;
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                if let Some(kind) = self.ai_task {
                    let message = Err("worker_disconnected".to_string());
                    let model = self.ai_task_model.take();
                    let started_at = self.ai_task_started_at.take();
                    self.record_ai_operation(kind, model, started_at, &message);
                }
                self.ai_task_receiver = None;
                self.ai_task = None;
                self.ai_task_started_at = None;
                self.ai_task_model = None;
                self.error_message = Some("The AI worker stopped unexpectedly.".to_string());
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
                self.show_rules_workspace = true;
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
                self.show_rules_workspace = false;
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

    fn simulate_selected_rule(&mut self) {
        let Some(result) = &self.analysis_result else {
            self.error_message = Some("Run Preview before simulating a rule.".to_string());
            return;
        };
        let Some(rule) = self.profile_editor.selected_rule().cloned() else {
            self.error_message = Some("Select a rule before simulating it.".to_string());
            return;
        };
        let rule_name = rule.rule_name.clone();
        let session_id = result.session_id.clone();
        let root = result.root.clone();

        match rule
            .to_rule()
            .map_err(|message| format!("Fix this rule before simulating it: {message}"))
            .and_then(|rule| simulate_rule_against_session(&session_id, &root, &rule))
        {
            Ok(simulation) => {
                self.rule_simulation = Some(simulation);
                self.maintenance_message = Some(format!(
                    "Rule '{rule_name}' matched {} file{} in the current preview.",
                    self.rule_simulation
                        .as_ref()
                        .map_or(0, |simulation| simulation.matched_files),
                    plural(
                        self.rule_simulation
                            .as_ref()
                            .map_or(0, |simulation| simulation.matched_files)
                    )
                ));
                self.error_message = None;
            }
            Err(message) => {
                self.rule_simulation = None;
                self.error_message = Some(message);
                self.maintenance_message = None;
            }
        }
    }

    fn export_ai_diagnostics(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("smartfolder-ai-diagnostics.json")
            .add_filter("JSON", &["json"])
            .save_file()
        else {
            return;
        };
        let payload = serde_json::json!({
            "schema_version": 1,
            "exported_at": Utc::now().to_rfc3339(),
            "ai": {
                "enabled": self.preferences.ai.enabled,
                "provider": "ollama",
                "endpoint": sanitize_ai_endpoint(&self.preferences.ai.endpoint),
                "selected_model": self.preferences.ai.selected_model.clone(),
                "timeout_seconds": self.preferences.ai.timeout_seconds,
                "content_inspection_enabled": self.preferences.ai.content_inspection_enabled,
            },
            "status": self.ai_status.as_ref().map(|status| serde_json::json!({
                "available": status.available,
                "state": format!("{:?}", status.state),
                "selected_model": status.selected_model.clone(),
                "model_count": status.models.len(),
                "message": status.message,
            })),
            "operations": &self.ai_operation_records,
            "notes": [
                "No prompts, raw provider responses, file contents, or full file paths are included."
            ],
        });

        match serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("Failed to serialize AI diagnostics: {error}"))
            .and_then(|json| {
                fs::write(&path, json)
                    .map_err(|error| format!("Failed to write {}: {error}", path.display()))
            }) {
            Ok(()) => {
                self.maintenance_message =
                    Some(format!("Exported AI diagnostics to {}.", path.display()));
                self.error_message = None;
            }
            Err(message) => {
                self.maintenance_message = None;
                self.error_message = Some(message);
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
                        ui::theme::colors::success(),
                        ui::theme::colors::success_bg(),
                    );
                    ui.label(format!("Using {}.", profile.display_label()));
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
        let open_palette = ctx.input(|input| {
            (input.modifiers.ctrl || input.modifiers.command) && input.key_pressed(egui::Key::K)
                || input.key_pressed(egui::Key::F1)
        });
        if open_palette {
            self.show_command_palette = true;
            self.command_palette_query.clear();
        }

        if self.show_command_palette && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.show_command_palette = false;
        }

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

    fn render_shell_nav(&mut self, ui: &mut egui::Ui) {
        if self.shell_nav_expanded {
            self.render_sidebar(ui);
        } else {
            self.render_collapsed_sidebar(ui);
        }
    }

    fn render_nav_toggle(&mut self, ui: &mut egui::Ui) {
        let (icon, tooltip) = if self.shell_nav_expanded {
            (ui::icons::COLLAPSE, "Collapse sidebar")
        } else {
            (ui::icons::EXPAND, "Expand sidebar")
        };
        if ui
            .add(ui::theme::widgets::icon_button(icon))
            .on_hover_text(tooltip)
            .clicked()
        {
            self.shell_nav_expanded = !self.shell_nav_expanded;
        }
    }

    fn render_collapsed_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(ui::theme::spacing::SM);
        ui.vertical_centered(|ui| {
            self.render_nav_toggle(ui);
            ui.add_space(ui::theme::spacing::MD);

            for section in AppSection::ALL {
                let selected = self.active_section == section;
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
                    self.active_section = section;
                }
                ui.add_space(ui::theme::spacing::XS);
            }
        });
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(ui::theme::spacing::MD);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("smartfolder")
                    .size(ui::theme::typography::SECTION_TITLE)
                    .strong()
                    .color(ui::theme::colors::heading_text()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.render_nav_toggle(ui);
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
            let selected = self.active_section == section;
            let nav_width = SHELL_NAV_WIDTH - (ui::theme::spacing::LG * 2.0);
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
            "Open a folder from Explorer or choose one here, preview the safe changes, then organize with restore available afterward.",
        );
        self.render_status_messages(ui);
        ui.add_space(10.0);
        let mut start_ai_folder_analysis_requested = false;
        let mut open_ai_draft_requested = false;
        let mut cancel_ai_task_requested = false;

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
                if self.ai_is_available() {
                    match render_ai_assist_strip(
                        ui,
                        self.ai_analysis.as_ref(),
                        self.ai_task,
                        self.is_running_ai_task(),
                        self.preferences.ai.content_inspection_enabled,
                        result.preview_counts.untouched,
                    ) {
                        Some(AiPreviewAction::Analyze) => {
                            start_ai_folder_analysis_requested = true;
                        }
                        Some(AiPreviewAction::ViewNotes) => {
                            self.show_ai_notes = true;
                        }
                        Some(AiPreviewAction::DraftRules) => {
                            open_ai_draft_requested = true;
                        }
                        Some(AiPreviewAction::Cancel) => {
                            cancel_ai_task_requested = true;
                        }
                        None => {}
                    }
                    ui.add_space(10.0);
                }
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
                            "Review all changes"
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
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new("View")
                                    .strong()
                                    .color(ui::theme::colors::metadata_text()),
                            );
                            ui.selectable_value(
                                &mut self.preview_detail_mode,
                                PreviewDetailMode::List,
                                "List",
                            );
                            ui.selectable_value(
                                &mut self.preview_detail_mode,
                                PreviewDetailMode::Tree,
                                "Tree",
                            );
                        });
                        ui.add_space(8.0);
                        render_preview_controls(
                            ui,
                            result,
                            self.preview_filter,
                            self.preview_offset,
                            preview_action,
                        );
                        ui.add_space(8.0);
                        match self.preview_detail_mode {
                            PreviewDetailMode::List => {
                                render_preview_table_header(ui);
                                ui.add_space(4.0);
                                let mut selected_preview_row = self.selected_preview_row;
                                let list_height =
                                    (ui.available_height() - 190.0).clamp(120.0, 280.0);
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_height(list_height)
                                    .show(ui, |ui| {
                                        render_preview_rows(
                                            ui,
                                            result,
                                            &mut selected_preview_row,
                                        );
                                    });
                                self.selected_preview_row = selected_preview_row;

                                ui.add_space(8.0);
                                render_preview_detail(ui, result, self.selected_preview_row);
                            }
                            PreviewDetailMode::Tree => {
                                let tree_height =
                                    (ui.available_height() - 144.0).clamp(180.0, 360.0);
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_height(tree_height)
                                    .show(ui, |ui| {
                                        render_preview_tree(ui, result);
                                    });
                            }
                        }
                    });
                if close_requested {
                    show_window = false;
                }
                self.show_detailed_preview = show_window;
            }

            if self.organize_step == OrganizeStep::Preview {
                self.render_ai_notes_window(ctx);
            }
        }
        if open_ai_draft_requested {
            self.open_ai_draft_from_analysis();
        }
        if cancel_ai_task_requested {
            self.cancel_ai_task();
        }
        if start_ai_folder_analysis_requested {
            self.start_ai_folder_analysis();
        }
    }

    fn open_ai_draft_from_analysis(&mut self) {
        if let Some(analysis) = &self.ai_analysis {
            self.ai_draft_prompt = format!(
                "Create a custom rule profile for this folder using this AI strategy: {}",
                analysis.recommended_strategy
            );
        }
        self.active_section = AppSection::Rules;
        self.show_rules_workspace = true;
        self.show_ai_draft_prompt = true;
        self.show_ai_notes = false;
        self.maintenance_message =
            Some("Review the AI prompt, then generate a draft profile when ready.".to_string());
        self.error_message = None;
    }

    fn render_ai_notes_window(&mut self, ctx: &egui::Context) {
        if !self.show_ai_notes {
            return;
        }

        let Some(analysis) = self.ai_analysis.clone() else {
            self.show_ai_notes = false;
            return;
        };

        let mut open = true;
        let mut draft_rules_requested = false;
        egui::Window::new("AI notes")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(720.0)
            .default_height(560.0)
            .min_width(560.0)
            .min_height(420.0)
            .show(ctx, |ui| {
                render_ai_notes_content(ui, &analysis, &mut draft_rules_requested);
            });

        if draft_rules_requested {
            self.open_ai_draft_from_analysis();
        }
        if !open {
            self.show_ai_notes = false;
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
            "Review recent organization runs for this folder and restore a previous layout when needed.",
        );
        self.render_status_messages(ui);
        ui.add_space(ui::theme::spacing::MD);

        let activity_scope = self.active_root().map_or_else(
            || "Activity for all folders".to_string(),
            |root| format!("Activity for {}", folder_name_label(&root)),
        );
        render_activity_scope_bar(
            ui,
            &activity_scope,
            if self.active_root().is_some() {
                "Folder filtered"
            } else {
                "All folders"
            },
            history_action,
        );

        ui.add_space(ui::theme::spacing::MD);
        settings_panel_frame().show(ui, |ui| {
            ui.set_width(ui.available_width().max(CARD_MIN_WIDTH));
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
            "Choose a built-in style, adapt it into a custom profile, or manage saved rule profiles.",
        );
        self.render_status_messages(ui);
        ui.add_space(10.0);

        let saved_profiles = list_saved_rule_profiles();
        let mut profile_to_use: Option<LoadedRuleProfile> = None;
        let mut profile_to_edit: Option<LoadedRuleProfile> = None;

        render_active_profile_panel(
            ui,
            self.planning_source,
            self.mode,
            self.loaded_profile.as_ref(),
            &mut profile_to_edit,
            &mut self.maintenance_message,
        );
        ui.add_space(ui::theme::spacing::MD);

        if self.ai_is_available() {
            render_rules_ai_panel(
                ui,
                self.has_selected_root(),
                self.is_running_ai_task(),
                &mut self.show_rules_workspace,
                &mut self.show_ai_draft_prompt,
                &mut self.maintenance_message,
                &mut self.error_message,
            );
            ui.add_space(ui::theme::spacing::MD);
        }

        ui::theme::widgets::card_frame().show(ui, |ui| {
            ui.label(
                RichText::new("Rules library")
                    .strong()
                    .size(ui::theme::typography::CARD_TITLE)
                    .color(ui::theme::colors::heading_text()),
            );
            ui.label(
                RichText::new("Compare built-in styles and saved custom profiles.")
                    .color(ui::theme::colors::secondary_text()),
            );
            ui.add_space(ui::theme::spacing::SM);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(ui::theme::widgets::secondary_button("New custom profile"))
                    .clicked()
                {
                    self.profile_editor = ProfileEditorState::default();
                    self.loaded_profile = None;
                    self.planning_source = PlanningSource::BuiltIn;
                    self.show_rules_workspace = true;
                    self.maintenance_message =
                        Some("Started a new unsaved profile draft.".to_string());
                    self.error_message = None;
                }
                if ui
                    .add(ui::theme::widgets::secondary_button("Import TOML..."))
                    .clicked()
                {
                    self.import_rule_profile();
                }
            });
            ui.add_space(ui::theme::spacing::SM);

            ui.label(
                RichText::new("Built-in styles")
                    .strong()
                    .color(ui::theme::colors::heading_text()),
            );
            for (mode, title, pattern, detail) in builtin_library_items() {
                let selected =
                    self.planning_source == PlanningSource::BuiltIn && self.mode == mode;
                render_builtin_rule_row(ui, title, pattern, detail, selected, |action| {
                    match action {
                        BuiltinRuleAction::Use => {
                            self.select_builtin_style(mode);
                            self.active_section = AppSection::Organize;
                        }
                        BuiltinRuleAction::Customize => self.customize_builtin_style(mode),
                    }
                });
                ui.add_space(ui::theme::spacing::XS);
            }

            ui.add_space(ui::theme::spacing::MD);
            ui.label(
                RichText::new("Custom profiles")
                    .strong()
                    .color(ui::theme::colors::heading_text()),
            );
            match &saved_profiles {
                Ok(profiles) if profiles.is_empty() => {
                    ui.label(
                        RichText::new("No saved custom profiles yet. Customize a built-in style or save the draft below.")
                            .color(ui::theme::colors::secondary_text()),
                    );
                }
                Ok(profiles) => {
                    for profile in profiles {
                        let selected = self.loaded_profile.as_ref().is_some_and(|loaded| {
                            loaded.path == profile.path
                                && self.planning_source == PlanningSource::RuleProfile
                        });
                        match render_saved_profile_row(ui, profile, selected) {
                            Some(SavedProfileAction::Use) => profile_to_use = Some(profile.clone()),
                            Some(SavedProfileAction::Edit) => {
                                profile_to_edit = Some(profile.clone());
                            }
                            None => {}
                        }
                        ui.add_space(ui::theme::spacing::XS);
                    }
                }
                Err(message) => {
                    ui.colored_label(ui::theme::colors::error(), message);
                }
            }
        });

        if let Some(profile) = profile_to_use {
            self.use_saved_profile(profile);
        }

        if let Some(profile) = profile_to_edit {
            self.edit_saved_profile(profile);
        }
    }

    fn render_settings_screen(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .inner_margin(egui::Margin::symmetric(ui::theme::spacing::MD, 0.0))
            .show(ui, |ui| {
                render_settings_heading(ui, &mut self.show_settings_help);
                self.render_status_messages(ui);
                ui.add_space(ui::theme::spacing::MD);

                if self.show_settings_help {
                    render_settings_help_window(ui.ctx(), &mut self.show_settings_help);
                }

                let mut preferences_changed = false;
                render_settings_section_panel(
                    ui,
                    "Preferences",
                    "Local display choices for this device.",
                    |ui| {
                        preferences_changed |=
                            render_appearance_preferences(ui, &mut self.preferences);
                    },
                );

                if preferences_changed {
                    self.ai_status = None;
                    self.save_preferences_with_message();
                }

                ui.add_space(ui::theme::spacing::MD);
                let mut ai_preferences_changed = false;
                let mut test_ai_connection = false;
                let checking_ai_status = self.is_checking_ai_status();
                render_settings_section_panel(
                    ui,
                    "AI assistance",
                    "Optional Ollama-powered help for folder analysis and rule drafting.",
                    |ui| {
                        let action = render_ai_preferences(
                            ui,
                            &mut self.preferences,
                            self.ai_status.as_ref(),
                            checking_ai_status,
                        );
                        ai_preferences_changed |= action.changed;
                        test_ai_connection |= action.test_connection;
                    },
                );

                if ai_preferences_changed {
                    let should_auto_check_ai = self.preferences.ai.enabled && !checking_ai_status;
                    self.ai_status = None;
                    self.save_preferences_with_message();
                    if should_auto_check_ai {
                        self.start_ai_status_check(AiStatusCheckSource::Startup);
                    }
                }

                if test_ai_connection {
                    self.start_ai_status_check(AiStatusCheckSource::Manual);
                }

                ui.add_space(ui::theme::spacing::MD);
                let mut privacy_changed = false;
                render_settings_section_panel(
                    ui,
                    "Privacy",
                    "Control whether optional AI features can inspect sampled text contents.",
                    |ui| {
                        privacy_changed |= render_privacy_preferences(ui, &mut self.preferences);
                    },
                );

                if privacy_changed {
                    self.save_preferences_with_message();
                }

                ui.add_space(ui::theme::spacing::MD);
                render_settings_section_panel(
                    ui,
                    "Explorer integration",
                    "Read-only launch behavior for Windows Explorer.",
                    render_explorer_integration_settings,
                );

                ui.add_space(ui::theme::spacing::MD);
                let mut advanced_changed = false;
                let mut export_ai_diagnostics = false;
                let mut cleanup_requested = false;
                render_settings_section_panel(
                    ui,
                    "Advanced and maintenance",
                    "Provider details and cache cleanup. Restore history is kept separate.",
                    |ui| {
                        let action = render_advanced_ai_preferences(ui, &mut self.preferences);
                        advanced_changed |= action.changed;
                        export_ai_diagnostics |= action.export_diagnostics;

                        ui.add_space(ui::theme::spacing::MD);
                        cleanup_requested |= render_storage_maintenance(
                            ui,
                            !self.is_analyzing() && !self.is_applying() && !self.is_undoing(),
                        );
                    },
                );

                if advanced_changed {
                    let should_auto_check_ai =
                        self.preferences.ai.enabled && !self.is_checking_ai_status();
                    self.ai_status = None;
                    self.save_preferences_with_message();
                    if should_auto_check_ai {
                        self.start_ai_status_check(AiStatusCheckSource::Startup);
                    }
                }
                if export_ai_diagnostics {
                    self.export_ai_diagnostics();
                }
                if cleanup_requested {
                    self.cleanup_old_sessions();
                }
            });
    }

    fn render_profile_editor(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(PROFILE_RULE_LIST_WIDTH, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    render_rule_list_panel(ui, &mut self.profile_editor);
                },
            );

            ui.separator();

            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let rule_simulation = self.rule_simulation.clone();
                    if let Some(rule) = self.profile_editor.selected_rule_mut() {
                        render_rule_detail_editor(ui, rule, rule_simulation.as_ref());
                    }
                },
            );
        });
    }

    fn render_rules_workspace_window(&mut self, ctx: &egui::Context) {
        if !self.show_rules_workspace {
            return;
        }

        let mut open = true;
        egui::Window::new("Profile workspace")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(980.0)
            .default_height(680.0)
            .min_width(760.0)
            .min_height(520.0)
            .show(ctx, |ui| {
                self.render_profile_workspace_toolbar(ui);
                if self.show_ai_draft_prompt && self.ai_is_available() {
                    ui.add_space(ui::theme::spacing::SM);
                    self.render_ai_rule_builder_panel(ui);
                }
                if let Some(result) = self.ai_draft_result.clone() {
                    ui.add_space(ui::theme::spacing::SM);
                    render_ai_draft_summary_strip(
                        ui,
                        &result,
                        &mut self.show_ai_draft_review,
                        &mut self.show_ai_draft_prompt,
                    );
                }

                ui.add_space(ui::theme::spacing::SM);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.render_profile_editor(ui);
                    });
            });

        if !open {
            self.show_rules_workspace = false;
        }
        self.render_ai_draft_review_window(ctx);
    }

    fn render_profile_workspace_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(&self.profile_editor.profile_id)
                    .strong()
                    .size(ui::theme::typography::SECTION_TITLE)
                    .color(ui::theme::colors::heading_text()),
            );
            render_status_chip(
                ui,
                &format!(
                    "{} rule{}",
                    self.profile_editor.rules.len(),
                    plural(self.profile_editor.rules.len())
                ),
                ui::theme::colors::secondary_text(),
                ui::theme::colors::subtle_surface(),
            );
            render_profile_workspace_status(ui, self.loaded_profile.as_ref());
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(ui::theme::widgets::compact_primary_button("Save"))
                .clicked()
            {
                self.save_profile_from_editor();
            }
            if ui
                .add(ui::theme::widgets::compact_secondary_button("Use"))
                .clicked()
            {
                self.select_custom_rules_style();
                self.active_section = AppSection::Organize;
                self.show_rules_workspace = false;
            }
            if ui
                .add(ui::theme::widgets::compact_secondary_button("Validate"))
                .clicked()
            {
                self.validate_profile_editor();
            }
            if ui
                .add_enabled(
                    self.analysis_result.is_some(),
                    ui::theme::widgets::compact_secondary_button("Simulate"),
                )
                .on_disabled_hover_text("Run Preview before simulating this rule")
                .clicked()
            {
                self.simulate_selected_rule();
            }
            if self.ai_is_available() {
                if ui
                    .add(ui::theme::widgets::compact_secondary_button(
                        "Build with AI",
                    ))
                    .clicked()
                {
                    self.show_ai_draft_prompt = !self.show_ai_draft_prompt;
                }
                if ui
                    .add_enabled(
                        !self.is_running_ai_task(),
                        ui::theme::widgets::compact_secondary_button("Explain"),
                    )
                    .clicked()
                {
                    self.show_ai_draft_prompt = true;
                    self.start_ai_rule_explanation();
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
                        egui::TextEdit::singleline(&mut self.profile_editor.profile_id),
                    );
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button("New draft"))
                        .clicked()
                    {
                        self.profile_editor = ProfileEditorState::default();
                        self.loaded_profile = None;
                        self.planning_source = PlanningSource::BuiltIn;
                        self.maintenance_message =
                            Some("Started a new unsaved profile draft.".to_string());
                        self.error_message = None;
                    }
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button("Import TOML"))
                        .clicked()
                    {
                        self.import_rule_profile();
                    }
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button("Export TOML"))
                        .clicked()
                    {
                        self.export_profile_from_editor();
                    }
                });
            });
    }

    fn render_ai_rule_builder_panel(&mut self, ui: &mut egui::Ui) {
        settings_note_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Build rules with AI")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                render_status_chip(
                    ui,
                    "Requires scanned folder context",
                    ui::theme::colors::info(),
                    ui::theme::colors::info_bg(),
                );
            });
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Describe the organization you want. AI drafts deterministic rules, then smartfolder validates them before loading the editor.",
                    )
                    .color(ui::theme::colors::secondary_text()),
                )
                .wrap(),
            );
            ui.add_space(ui::theme::spacing::SM);
            ui.horizontal(|ui| {
                let refine_button_size = egui::vec2(42.0, 72.0);
                let prompt_width = (ui.available_width()
                    - refine_button_size.x
                    - ui.spacing().item_spacing.x)
                    .max(260.0);
                ui.add_sized(
                    [prompt_width, 72.0],
                    egui::TextEdit::multiline(&mut self.ai_draft_prompt)
                        .hint_text("e.g. Sort invoices into Documents/Invoices by year, and keep screenshots by month."),
                );
                let response = ui.add_enabled(
                    !self.is_running_ai_task() && !self.ai_draft_prompt.trim().is_empty(),
                    egui::Button::new(
                        RichText::new(ui::icons::AI_REFINE)
                            .size(20.0)
                            .color(ui::theme::colors::primary_text()),
                    )
                    .fill(ui::theme::colors::soft_control())
                    .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
                    .min_size(refine_button_size),
                );
                if response
                    .on_hover_text("Refine this prompt with AI")
                    .clicked()
                {
                    self.start_ai_prompt_refinement();
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.is_running_ai_task(),
                        ui::theme::widgets::primary_button("Draft profile"),
                    )
                    .clicked()
                {
                    self.start_ai_draft_profile();
                }
                if ui
                    .add(ui::theme::widgets::secondary_button("Hide"))
                    .clicked()
                {
                    self.show_ai_draft_prompt = false;
                }
                if self.is_running_ai_task()
                    && ui
                        .add(ui::theme::widgets::secondary_button("Cancel AI"))
                        .clicked()
                {
                    self.cancel_ai_task();
                }
                if self.ai_task == Some(AiTaskKind::DraftProfile) && self.is_running_ai_task() {
                    ui.label(
                        RichText::new("AI is drafting and validating rules.")
                            .color(ui::theme::colors::secondary_text()),
                    );
                }
                if self.ai_task == Some(AiTaskKind::PromptRefinement) && self.is_running_ai_task()
                {
                    ui.label(
                        RichText::new("AI is refining the prompt.")
                            .color(ui::theme::colors::secondary_text()),
                    );
                }
            });

            if let Some(explanation) = &self.ai_rule_explanation {
                ui.add_space(ui::theme::spacing::SM);
                render_ai_rule_explanation(ui, explanation);
            }
        });
    }

    fn render_ai_draft_review_window(&mut self, ctx: &egui::Context) {
        if !self.show_ai_draft_review {
            return;
        }

        let Some(result) = self.ai_draft_result.clone() else {
            self.show_ai_draft_review = false;
            return;
        };

        let mut open = true;
        let mut edit_prompt_requested = false;
        egui::Window::new("AI draft review")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(720.0)
            .default_height(520.0)
            .min_width(560.0)
            .min_height(380.0)
            .show(ctx, |ui| {
                render_ai_draft_review(ui, &result, &mut edit_prompt_requested);
            });

        if edit_prompt_requested {
            self.show_ai_draft_prompt = true;
            open = false;
        }
        if !open {
            self.show_ai_draft_review = false;
        }
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

    fn render_command_palette_window(
        &mut self,
        ctx: &egui::Context,
    ) -> Option<CommandPaletteAction> {
        if !self.show_command_palette {
            return None;
        }

        let mut action = None;
        let mut open = true;
        egui::Window::new("Command palette")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.command_palette_query)
                        .hint_text("Search commands")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(ui::theme::spacing::SM);

                let query = self.command_palette_query.trim().to_ascii_lowercase();
                for item in self.command_palette_items() {
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

        if action.is_some() || !open {
            self.show_command_palette = false;
        }

        action
    }

    fn command_palette_items(&self) -> Vec<CommandPaletteItem> {
        let busy = self.is_analyzing() || self.is_applying() || self.is_undoing();
        let ready = self
            .analysis_result
            .as_ref()
            .map_or(0, |result| result.preview_counts.ready);
        vec![
            CommandPaletteItem::new(
                "Go to Organize",
                "Open the preview-led organize workflow",
                true,
                CommandPaletteAction::OpenSection(AppSection::Organize),
            ),
            CommandPaletteItem::new(
                "Go to Activity",
                "Review recent activity and restore previous layouts",
                true,
                CommandPaletteAction::OpenSection(AppSection::Activity),
            ),
            CommandPaletteItem::new(
                "Go to Rules",
                "Open built-in and custom rule profiles",
                true,
                CommandPaletteAction::OpenSection(AppSection::Rules),
            ),
            CommandPaletteItem::new(
                "Go to Settings",
                "Open app preferences and AI readiness",
                true,
                CommandPaletteAction::OpenSection(AppSection::Settings),
            ),
            CommandPaletteItem::new(
                "Choose folder",
                "Return to folder selection",
                !busy,
                CommandPaletteAction::ChooseFolder,
            ),
            CommandPaletteItem::new(
                "Preview current folder",
                "Analyze the selected folder",
                self.can_run_analysis() && !busy,
                CommandPaletteAction::Analyze,
            ),
            CommandPaletteItem::new(
                "Review all changes",
                "Open the detailed file list",
                self.analysis_result.is_some(),
                CommandPaletteAction::ReviewAllChanges,
            ),
            CommandPaletteItem::new(
                "Show preview tree",
                "Open the detailed preview grouped by destination",
                self.analysis_result.is_some(),
                CommandPaletteAction::ShowPreviewTree,
            ),
            CommandPaletteItem::new(
                "Organize ready files",
                "Move only safe ready files after confirmation",
                ready > 0 && !busy,
                CommandPaletteAction::OrganizeReady,
            ),
            CommandPaletteItem::new(
                "Restore latest layout",
                "Restore the latest restorable activity for this folder",
                self.latest_restorable_transaction().is_some() && !busy,
                CommandPaletteAction::RestoreLatest,
            ),
            CommandPaletteItem::new(
                "Open profile workspace",
                "Edit and validate custom rules",
                true,
                CommandPaletteAction::OpenProfileWorkspace,
            ),
            CommandPaletteItem::new(
                "Test AI connection",
                "Run the Ollama readiness check",
                !self.is_checking_ai_status(),
                CommandPaletteAction::TestAiConnection,
            ),
        ]
    }

    fn apply_command_palette_action(&mut self, action: CommandPaletteAction) {
        match action {
            CommandPaletteAction::OpenSection(section) => {
                self.active_section = section;
            }
            CommandPaletteAction::ChooseFolder => {
                self.active_section = AppSection::Organize;
                self.organize_step = OrganizeStep::Folder;
            }
            CommandPaletteAction::Analyze => {
                self.active_section = AppSection::Organize;
                self.continue_from_style_step();
            }
            CommandPaletteAction::ReviewAllChanges => {
                self.active_section = AppSection::Organize;
                self.organize_step = OrganizeStep::Preview;
                self.show_detailed_preview = true;
                self.preview_detail_mode = PreviewDetailMode::List;
            }
            CommandPaletteAction::ShowPreviewTree => {
                self.active_section = AppSection::Organize;
                self.organize_step = OrganizeStep::Preview;
                self.show_detailed_preview = true;
                self.preview_detail_mode = PreviewDetailMode::Tree;
            }
            CommandPaletteAction::OrganizeReady => {
                self.active_section = AppSection::Organize;
                self.continue_from_preview_step();
            }
            CommandPaletteAction::RestoreLatest => {
                if let Some(transaction_id) = self.latest_restorable_transaction() {
                    self.show_undo_confirmation = Some(transaction_id);
                }
            }
            CommandPaletteAction::OpenProfileWorkspace => {
                self.active_section = AppSection::Rules;
                self.show_rules_workspace = true;
            }
            CommandPaletteAction::TestAiConnection => {
                self.active_section = AppSection::Settings;
                self.start_ai_status_check(AiStatusCheckSource::Manual);
            }
        }
    }

    fn latest_restorable_transaction(&self) -> Option<String> {
        let active_root = self.active_root();
        self.transaction_rows
            .iter()
            .find(|row| {
                can_undo_status(row.status)
                    && active_root
                        .as_deref()
                        .map_or(true, |root| same_folder(&row.root, root))
            })
            .map(|row| row.transaction_id.clone())
    }
}

impl eframe::App for SmartfolderApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_analysis();
        self.poll_apply();
        self.poll_undo();
        self.poll_ai_status();
        self.poll_ai_task();
        self.process_keyboard_shortcuts(ctx);
        self.process_dropped_folders(ctx);
        ui::theme::apply_visual_theme(
            ctx,
            self.preferences.visual_theme(frame.info().system_theme),
        );

        if self.is_analyzing()
            || self.is_applying()
            || self.is_undoing()
            || self.is_checking_ai_status()
            || self.is_running_ai_task()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        let mut preview_action = None;
        let mut history_action = None;

        let shell_nav_width = if self.shell_nav_expanded {
            SHELL_NAV_WIDTH
        } else {
            SHELL_NAV_COLLAPSED_WIDTH
        };

        egui::SidePanel::left("app-shell-nav")
            .resizable(false)
            .exact_width(shell_nav_width)
            .show(ctx, |ui| self.render_shell_nav(ui));

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let section = self.active_section;
                    render_constrained_page(ui, section_max_width(section), |ui| match section {
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
        });

        self.render_rules_workspace_window(ctx);

        if let Some(action) = self.render_command_palette_window(ctx) {
            self.apply_command_palette_action(action);
        }

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
                HistoryAction::CompleteApply => self.complete_apply_result(),
                HistoryAction::ChangeFolder => {
                    self.active_section = AppSection::Organize;
                    self.organize_step = OrganizeStep::Folder;
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
enum PreviewDetailMode {
    List,
    Tree,
}

#[cfg_attr(test, allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiStatusCheckSource {
    Startup,
    Manual,
}

#[derive(Debug, Clone, Copy)]
enum CommandPaletteAction {
    OpenSection(AppSection),
    ChooseFolder,
    Analyze,
    ReviewAllChanges,
    ShowPreviewTree,
    OrganizeReady,
    RestoreLatest,
    OpenProfileWorkspace,
    TestAiConnection,
}

#[derive(Debug, Clone, Copy)]
struct CommandPaletteItem {
    label: &'static str,
    detail: &'static str,
    enabled: bool,
    action: CommandPaletteAction,
}

impl CommandPaletteItem {
    fn new(
        label: &'static str,
        detail: &'static str,
        enabled: bool,
        action: CommandPaletteAction,
    ) -> Self {
        Self {
            label,
            detail,
            enabled,
            action,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewFilter {
    All,
    Ready,
    Untouched,
    NeedsAttention,
}

impl PreviewFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Ready => "Ready",
            Self::Untouched => "Untouched",
            Self::NeedsAttention => "Needs attention",
        }
    }

    fn operation_filter(self) -> Option<PlanOperationFilter> {
        match self {
            Self::All | Self::Untouched => None,
            Self::Ready => Some(PlanOperationFilter::Ready),
            Self::NeedsAttention => Some(PlanOperationFilter::NeedsAttention),
        }
    }

    fn count(self, counts: &PreviewCounts) -> usize {
        match self {
            Self::All => counts.all,
            Self::Ready => counts.ready,
            Self::Untouched => counts.untouched,
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

fn list_saved_rule_profiles() -> std::result::Result<Vec<LoadedRuleProfile>, String> {
    let directory = ensure_profiles_dir()
        .map_err(|error| format!("Failed to open profile directory: {error}"))?;
    let entries = fs::read_dir(&directory).map_err(|error| {
        format!(
            "Failed to read profile directory {}: {error}",
            directory.display()
        )
    })?;
    let mut profiles = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| format!("Failed to read profile entry: {error}"))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        if let Ok(profile) = load_rule_profile_from_path(&path) {
            profiles.push(LoadedRuleProfile { path, profile });
        }
    }

    profiles.sort_by(|left, right| {
        left.profile
            .profile_id
            .cmp(&right.profile.profile_id)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(profiles)
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
    rules: Vec<RuleEditorState>,
    selected_rule: usize,
}

#[derive(Debug, Clone)]
struct RuleEditorState {
    rule_name: String,
    destination: String,
    destination_text_mode: bool,
    new_destination_segment: String,
    priority: String,
    match_all: bool,
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
            rules: vec![RuleEditorState::default()],
            selected_rule: 0,
        }
    }
}

impl Default for RuleEditorState {
    fn default() -> Self {
        Self {
            rule_name: "PDFs".to_string(),
            destination: "Documents/PDFs".to_string(),
            destination_text_mode: false,
            new_destination_segment: String::new(),
            priority: "10".to_string(),
            match_all: false,
            extensions: String::new(),
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
        Self {
            profile_id: profile.profile_id.clone(),
            rules: if profile.rules.is_empty() {
                vec![RuleEditorState::default()]
            } else {
                profile
                    .rules
                    .iter()
                    .map(RuleEditorState::from_rule)
                    .collect()
            },
            selected_rule: 0,
        }
    }

    fn from_builtin(mode: BuiltInMode) -> Self {
        let (profile_id, rule_name, destination) = builtin_custom_profile_template(mode);
        Self {
            profile_id: profile_id.to_string(),
            rules: vec![RuleEditorState {
                rule_name: rule_name.to_string(),
                destination: destination.to_string(),
                destination_text_mode: false,
                new_destination_segment: String::new(),
                match_all: true,
                extensions: String::new(),
                filename_contains: String::new(),
                path_contains: String::new(),
                min_size_bytes: String::new(),
                max_size_bytes: String::new(),
                year: String::new(),
                ..RuleEditorState::default()
            }],
            selected_rule: 0,
        }
    }

    fn selected_rule_index(&self) -> usize {
        self.selected_rule.min(self.rules.len().saturating_sub(1))
    }

    fn selected_rule(&self) -> Option<&RuleEditorState> {
        self.rules.get(self.selected_rule_index())
    }

    fn selected_rule_mut(&mut self) -> Option<&mut RuleEditorState> {
        let index = self.selected_rule_index();
        self.rules.get_mut(index)
    }

    fn add_rule(&mut self) {
        self.rules.push(RuleEditorState {
            rule_name: format!("Rule {}", self.rules.len() + 1),
            destination: "Other".to_string(),
            destination_text_mode: false,
            new_destination_segment: String::new(),
            priority: ((self.rules.len() + 1) * 10).to_string(),
            extensions: String::new(),
            ..RuleEditorState::default()
        });
        self.selected_rule = self.rules.len().saturating_sub(1);
    }

    fn duplicate_selected_rule(&mut self) {
        let index = self.selected_rule_index();
        if let Some(rule) = self.rules.get(index).cloned() {
            let mut duplicate = rule;
            duplicate.rule_name = format!("{} copy", duplicate.rule_name);
            self.rules.insert(index + 1, duplicate);
            self.selected_rule = index + 1;
        }
    }

    fn delete_selected_rule(&mut self) {
        if self.rules.len() <= 1 {
            return;
        }
        let index = self.selected_rule_index();
        self.rules.remove(index);
        self.selected_rule = index.min(self.rules.len().saturating_sub(1));
    }

    fn move_selected_rule(&mut self, direction: isize) {
        let index = self.selected_rule_index();
        let next_index = if direction < 0 {
            index.saturating_sub(1)
        } else {
            (index + 1).min(self.rules.len().saturating_sub(1))
        };
        if index != next_index {
            self.rules.swap(index, next_index);
            self.selected_rule = next_index;
        }
    }

    fn to_profile(&self) -> std::result::Result<RuleProfile, String> {
        let profile = RuleProfile {
            profile_id: self.profile_id.trim().to_string(),
            rules: self
                .rules
                .iter()
                .map(RuleEditorState::to_rule)
                .collect::<std::result::Result<Vec<_>, _>>()?,
        };
        profile.validate().map_err(|error| error.to_string())?;
        Ok(profile)
    }
}

impl RuleEditorState {
    fn from_rule(rule: &CustomRule) -> Self {
        Self {
            rule_name: rule.name.clone(),
            destination: rule.destination.clone(),
            destination_text_mode: false,
            new_destination_segment: String::new(),
            priority: rule
                .priority
                .map_or_else(String::new, |value| value.to_string()),
            match_all: rule.match_all,
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

    fn to_rule(&self) -> std::result::Result<CustomRule, String> {
        Ok(CustomRule {
            name: self.rule_name.trim().to_string(),
            destination: self.destination.trim().to_string(),
            priority: parse_optional_number(&self.priority, "priority")?,
            match_all: self.match_all,
            extensions: comma_separated_values(&self.extensions),
            filename_contains: comma_separated_values(&self.filename_contains),
            path_contains: comma_separated_values(&self.path_contains),
            min_size_bytes: parse_optional_number(&self.min_size_bytes, "min_size_bytes")?,
            max_size_bytes: parse_optional_number(&self.max_size_bytes, "max_size_bytes")?,
            year: parse_optional_number(&self.year, "year")?,
        })
    }
}

fn builtin_custom_profile_template(
    mode: BuiltInMode,
) -> (&'static str, &'static str, &'static str) {
    match mode {
        BuiltInMode::Type => ("custom-by-type", "All files by type", "{type}"),
        BuiltInMode::Date => (
            "custom-by-date",
            "All files by date",
            "{year}/{month}/{day}",
        ),
        BuiltInMode::Extension => (
            "custom-by-extension",
            "All files by extension",
            "{extension}",
        ),
        BuiltInMode::TypeYear => (
            "custom-type-date",
            "All files by type and date",
            "{type}/{year}/{month}/{day}",
        ),
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
    CompleteApply,
    ChangeFolder,
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
    untouched_reason_counts: BTreeMap<UntouchedReason, usize>,
    preview_examples: Vec<PreviewRow>,
    preview_total_rows: usize,
    preview_rows: Vec<PreviewRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiTaskKind {
    FolderAnalysis,
    DraftProfile,
    PromptRefinement,
    RuleExplanation,
}

impl AiTaskKind {
    fn label(self) -> &'static str {
        match self {
            Self::FolderAnalysis => "folder_analysis",
            Self::DraftProfile => "draft_profile",
            Self::PromptRefinement => "prompt_refinement",
            Self::RuleExplanation => "rule_explanation",
        }
    }
}

#[derive(Debug, Clone)]
enum AiTaskOutput {
    FolderAnalysis(AiFolderAnalysis),
    DraftProfile(AiDraftProfileResult),
    PromptRefinement(AiPromptRefinement),
    RuleExplanation(AiRuleExplanation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiPreviewAction {
    Analyze,
    ViewNotes,
    DraftRules,
    Cancel,
}

#[derive(Debug, Clone)]
struct AiDraftProfileResult {
    draft: AiRuleProfileDraft,
    validation: AiProfileValidation,
    raw_json: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct AiOperationRecord {
    completed_at: String,
    request_type: String,
    provider: String,
    model: String,
    duration_ms: Option<u64>,
    success: bool,
    failure_class: Option<String>,
    validation_warnings: usize,
    validation_errors: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct PreviewCounts {
    all: usize,
    ready: usize,
    needs_attention: usize,
    untouched: usize,
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
    destination_full_path: Option<String>,
    reason: String,
    status: String,
}

#[derive(Debug, Clone)]
struct RuleSimulationResult {
    rule_name: String,
    matched_files: usize,
    total_files: usize,
    sample_matches: Vec<String>,
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
    let untouched_reason_counts = store
        .untouched_reason_counts(&session_id)
        .map_err(|error| format!("Failed to load untouched reason counts: {error}"))?;
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
        untouched_reason_counts,
        preview_examples,
        preview_total_rows: preview_page.total_rows,
        preview_rows: preview_page.rows,
    })
}

fn run_ai_folder_analysis(
    endpoint: &str,
    timeout: Duration,
    model: &str,
    session_id: &str,
    root: &Path,
    content_inspection_enabled: bool,
) -> AiTaskMessage {
    let context = load_ai_folder_context(session_id, root, content_inspection_enabled)?;
    let client = OllamaClient::new(endpoint, timeout);
    client
        .analyze_folder(model, &context)
        .map(AiTaskOutput::FolderAnalysis)
        .map_err(|error| format!("AI folder analysis failed: {error}"))
}

fn run_ai_profile_draft(
    endpoint: &str,
    timeout: Duration,
    model: &str,
    session_id: &str,
    root: &Path,
    content_inspection_enabled: bool,
    prompt: &str,
    existing_profile: Option<&RuleProfile>,
) -> AiTaskMessage {
    let context = load_ai_folder_context(session_id, root, content_inspection_enabled)?;
    let records = load_ai_context_records(session_id)?;
    let client = OllamaClient::new(endpoint, timeout);
    let draft = client
        .draft_profile(model, prompt, &context, existing_profile)
        .map_err(|error| format!("AI profile draft failed: {error}"))?;
    let raw_json = serde_json::to_string_pretty(&draft)
        .map_err(|error| format!("Failed to serialize AI draft: {error}"))?;
    let validation = validate_ai_profile_draft(draft.clone(), &records);
    Ok(AiTaskOutput::DraftProfile(AiDraftProfileResult {
        draft,
        validation,
        raw_json,
    }))
}

fn run_ai_prompt_refinement(
    endpoint: &str,
    timeout: Duration,
    model: &str,
    session_id: &str,
    root: &Path,
    content_inspection_enabled: bool,
    prompt: &str,
) -> AiTaskMessage {
    let context = load_ai_folder_context(session_id, root, content_inspection_enabled)?;
    let client = OllamaClient::new(endpoint, timeout);
    client
        .refine_prompt(model, prompt, &context)
        .map(AiTaskOutput::PromptRefinement)
        .map_err(|error| format!("AI prompt refinement failed: {error}"))
}

fn run_ai_rule_explanation(
    endpoint: &str,
    timeout: Duration,
    model: &str,
    session_id: &str,
    root: &Path,
    content_inspection_enabled: bool,
    profile: &RuleProfile,
) -> AiTaskMessage {
    let context = load_ai_folder_context(session_id, root, content_inspection_enabled)?;
    let client = OllamaClient::new(endpoint, timeout);
    client
        .explain_profile(model, profile, &context)
        .map(AiTaskOutput::RuleExplanation)
        .map_err(|error| format!("AI rule explanation failed: {error}"))
}

fn load_ai_folder_context(
    session_id: &str,
    root: &Path,
    content_inspection_enabled: bool,
) -> std::result::Result<AiFolderContext, String> {
    let records = load_ai_context_records(session_id)?;
    Ok(AiFolderContext::from_records_with_content_mode(
        root,
        &records,
        content_inspection_enabled,
    ))
}

fn load_ai_context_records(
    session_id: &str,
) -> std::result::Result<Vec<FileInventoryRecord>, String> {
    let store = SqliteSessionStore::open_default()
        .map_err(|error| format!("Failed to open session store: {error}"))?;
    store
        .scan_records_page(session_id, 0, 500)
        .map_err(|error| format!("Failed to load AI folder context: {error}"))
}

fn simulate_rule_against_session(
    session_id: &str,
    root: &Path,
    rule: &CustomRule,
) -> std::result::Result<RuleSimulationResult, String> {
    let records = load_all_scan_records(session_id)?;
    let mut sample_matches = Vec::new();
    let mut matched_files = 0;

    for record in &records {
        if rule.match_record(record).is_some() {
            matched_files += 1;
            if sample_matches.len() < 5 {
                sample_matches.push(root.join(&record.root_relative_path).display().to_string());
            }
        }
    }

    Ok(RuleSimulationResult {
        rule_name: rule.name.clone(),
        matched_files,
        total_files: records.len(),
        sample_matches,
    })
}

fn load_all_scan_records(
    session_id: &str,
) -> std::result::Result<Vec<FileInventoryRecord>, String> {
    let store = SqliteSessionStore::open_default()
        .map_err(|error| format!("Failed to open session store: {error}"))?;
    let mut records = Vec::new();
    let mut offset = 0;
    loop {
        let page = store
            .scan_records_page(session_id, offset, 500)
            .map_err(|error| format!("Failed to load rule simulation records: {error}"))?;
        if page.is_empty() {
            break;
        }
        offset += page.len();
        records.extend(page);
    }
    Ok(records)
}

fn ai_draft_summary_message(validation: &AiProfileValidation) -> String {
    if validation.is_usable() {
        if validation.warnings.is_empty() {
            "AI draft profile is valid and loaded into the editor.".to_string()
        } else {
            format!(
                "AI draft profile is loaded with {} warning{}.",
                validation.warnings.len(),
                plural(validation.warnings.len())
            )
        }
    } else {
        format!(
            "AI draft profile failed validation with {} error{}.",
            validation.errors.len(),
            plural(validation.errors.len())
        )
    }
}

fn ai_failure_class(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") {
        "timeout".to_string()
    } else if lower.contains("cancel") {
        "cancelled".to_string()
    } else if lower.contains("validation") || lower.contains("schema") {
        "validation".to_string()
    } else if lower.contains("parse") || lower.contains("json") {
        "invalid_json".to_string()
    } else if lower.contains("connect") || lower.contains("reach") || lower.contains("endpoint") {
        "provider_unavailable".to_string()
    } else {
        "provider_error".to_string()
    }
}

fn sanitize_ai_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    let without_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    if trimmed.starts_with("https://") {
        format!("https://{host_port}")
    } else if trimmed.starts_with("http://") {
        format!("http://{host_port}")
    } else {
        host_port.to_string()
    }
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
    if filter == PreviewFilter::Untouched {
        let total_rows = store
            .untouched_count(session_id)
            .map_err(|error| format!("Failed to count untouched files: {error}"))?;
        let records = store
            .untouched_records_page(session_id, offset, PREVIEW_PAGE_SIZE)
            .map_err(|error| format!("Failed to load untouched files: {error}"))?;
        return Ok(PreviewPage {
            rows: untouched_preview_rows(&records, root),
            total_rows,
        });
    }

    if let Some(operation_filter) = filter.operation_filter() {
        let total_rows = store
            .plan_operation_count(session_id, operation_filter)
            .map_err(|error| format!("Failed to count preview operations: {error}"))?;
        let operations = store
            .plan_operations_page_filtered(session_id, operation_filter, offset, PREVIEW_PAGE_SIZE)
            .map_err(|error| format!("Failed to load preview operations: {error}"))?;
        return Ok(PreviewPage {
            rows: preview_rows(&operations, root),
            total_rows,
        });
    }

    let operation_total = store
        .plan_operation_count(session_id, PlanOperationFilter::All)
        .map_err(|error| format!("Failed to count preview operations: {error}"))?;
    let untouched_total = store
        .untouched_count(session_id)
        .map_err(|error| format!("Failed to count untouched files: {error}"))?;
    let total_rows = operation_total + untouched_total;
    let mut rows = Vec::new();
    if offset < operation_total {
        let operations = store
            .plan_operations_page_filtered(
                session_id,
                PlanOperationFilter::All,
                offset,
                PREVIEW_PAGE_SIZE,
            )
            .map_err(|error| format!("Failed to load preview operations: {error}"))?;
        rows.extend(preview_rows(&operations, root));
    }
    if rows.len() < PREVIEW_PAGE_SIZE {
        let untouched_offset = offset.saturating_sub(operation_total);
        let records = store
            .untouched_records_page(session_id, untouched_offset, PREVIEW_PAGE_SIZE - rows.len())
            .map_err(|error| format!("Failed to load untouched files: {error}"))?;
        rows.extend(untouched_preview_rows(&records, root));
    }
    Ok(PreviewPage { rows, total_rows })
}

fn load_preview_counts_from_store(
    store: &SqliteSessionStore,
    session_id: &str,
) -> std::result::Result<PreviewCounts, String> {
    let operation_count = store
        .plan_operation_count(session_id, PlanOperationFilter::All)
        .map_err(|error| format!("Failed to count preview operations: {error}"))?;
    let untouched = store
        .untouched_count(session_id)
        .map_err(|error| format!("Failed to count untouched files: {error}"))?;
    Ok(PreviewCounts {
        all: operation_count + untouched,
        ready: store
            .plan_operation_count(session_id, PlanOperationFilter::Ready)
            .map_err(|error| format!("Failed to count ready operations: {error}"))?,
        needs_attention: store
            .plan_operation_count(session_id, PlanOperationFilter::NeedsAttention)
            .map_err(|error| format!("Failed to count operations needing attention: {error}"))?,
        untouched,
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
        .map_err(|error| format!("Failed to restore previous layout: {error}"))
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

fn render_preview_summary_line(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui::theme::widgets::status_dot(ui, color);
        ui.label(RichText::new(text).color(ui::theme::colors::primary_text()));
    });
}

fn render_ai_assist_strip(
    ui: &mut egui::Ui,
    analysis: Option<&AiFolderAnalysis>,
    active_task: Option<AiTaskKind>,
    busy: bool,
    content_inspection_enabled: bool,
    untouched_count: usize,
) -> Option<AiPreviewAction> {
    let mut action = None;
    let aligned_width = preview_aligned_content_width(ui);
    ui.scope(|ui| {
        ui.set_max_width(aligned_width);
        settings_note_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if active_task == Some(AiTaskKind::FolderAnalysis) && busy {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("AI Assist")
                            .strong()
                            .color(ui::theme::colors::heading_text()),
                    );
                    ui.label(
                        RichText::new("Reviewing scanned folder context...")
                            .color(ui::theme::colors::secondary_text()),
                    );
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button("Cancel"))
                        .clicked()
                    {
                        action = Some(AiPreviewAction::Cancel);
                    }
                });
                return;
            }

            ui.horizontal_wrapped(|ui| {
                if let Some(analysis) = analysis {
                    ui.label(
                        RichText::new("AI reviewed this folder")
                            .strong()
                            .color(ui::theme::colors::heading_text()),
                    );
                    render_status_chip(
                        ui,
                        ai_confidence_label(analysis.confidence),
                        ui::theme::colors::info(),
                        ui::theme::colors::info_bg(),
                    );
                    render_status_chip(
                        ui,
                        &format!(
                            "{} pattern{}",
                            analysis.patterns.len(),
                            plural(analysis.patterns.len())
                        ),
                        ui::theme::colors::success(),
                        ui::theme::colors::success_bg(),
                    );
                    render_status_chip(
                        ui,
                        &format!(
                            "{} risk{}",
                            analysis.risks.len(),
                            plural(analysis.risks.len())
                        ),
                        ui::theme::colors::warning(),
                        ui::theme::colors::warning_bg(),
                    );
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button(
                            "View AI notes",
                        ))
                        .clicked()
                    {
                        action = Some(AiPreviewAction::ViewNotes);
                    }
                    if ui
                        .add(ui::theme::widgets::compact_secondary_button(
                            "Create draft rules",
                        ))
                        .clicked()
                    {
                        action = Some(AiPreviewAction::DraftRules);
                    }
                } else {
                    ui.label(
                        RichText::new("AI Assist")
                            .strong()
                            .color(ui::theme::colors::heading_text()),
                    );
                    render_ai_mode_chip(ui, content_inspection_enabled);
                    ui.label(
                        RichText::new(
                            "Optional context review. Rules preview remains authoritative.",
                        )
                        .color(ui::theme::colors::secondary_text()),
                    );
                    if ui
                        .add_enabled(
                            !busy,
                            ui::theme::widgets::compact_secondary_button(if untouched_count > 0 {
                                "Explain untouched files"
                            } else {
                                "Analyze with AI"
                            }),
                        )
                        .clicked()
                    {
                        action = Some(AiPreviewAction::Analyze);
                    }
                }
            });
        });
    });
    action
}

fn render_ai_mode_chip(ui: &mut egui::Ui, content_inspection_enabled: bool) {
    render_status_chip(
        ui,
        if content_inspection_enabled {
            "Content inspection on"
        } else {
            "Metadata only"
        },
        if content_inspection_enabled {
            ui::theme::colors::warning()
        } else {
            ui::theme::colors::info()
        },
        if content_inspection_enabled {
            ui::theme::colors::warning_bg()
        } else {
            ui::theme::colors::info_bg()
        },
    );
}

fn render_ai_notes_content(
    ui: &mut egui::Ui,
    analysis: &AiFolderAnalysis,
    draft_rules_requested: &mut bool,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("AI notes")
                .strong()
                .size(ui::theme::typography::SECTION_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        render_ai_mode_chip(ui, analysis.content_inspection_used);
        render_status_chip(
            ui,
            ai_confidence_label(analysis.confidence),
            ui::theme::colors::info(),
            ui::theme::colors::info_bg(),
        );
        if analysis.content_inspection_used {
            render_status_chip(
                ui,
                &format!(
                    "{} content sample{}",
                    analysis.content_samples_included,
                    plural(analysis.content_samples_included)
                ),
                ui::theme::colors::secondary_text(),
                ui::theme::colors::elevated_surface(),
            );
        }
    });
    ui.add_space(ui::theme::spacing::SM);
    ui.add(
        egui::Label::new(RichText::new(&analysis.summary).color(ui::theme::colors::primary_text()))
            .wrap(),
    );
    ui.add(
        egui::Label::new(
            RichText::new(&analysis.recommended_strategy)
                .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add_space(ui::theme::spacing::SM);
    if ui
        .add(ui::theme::widgets::primary_button("Create draft rules"))
        .clicked()
    {
        *draft_rules_requested = true;
    }
    ui.add_space(ui::theme::spacing::MD);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !analysis.content_sample_warnings.is_empty() {
                ui.label(
                    RichText::new("Content sampling notes")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                for warning in analysis.content_sample_warnings.iter().take(6) {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("- {warning}"))
                                .color(ui::theme::colors::secondary_text()),
                        )
                        .wrap(),
                    );
                }
                if analysis.content_sample_warnings.len() > 6 {
                    ui.label(
                        RichText::new(format!(
                            "{} more sampling note{} omitted.",
                            analysis.content_sample_warnings.len() - 6,
                            plural(analysis.content_sample_warnings.len() - 6)
                        ))
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::metadata_text()),
                    );
                }
                ui.add_space(ui::theme::spacing::SM);
            }
            if !analysis.patterns.is_empty() {
                ui.label(
                    RichText::new("Patterns")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                for finding in &analysis.patterns {
                    render_ai_finding(ui, finding);
                }
            }
            if !analysis.risks.is_empty() {
                ui.add_space(ui::theme::spacing::SM);
                ui.label(
                    RichText::new("Risks")
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                for finding in &analysis.risks {
                    render_ai_finding(ui, finding);
                }
            }
        });
}

fn render_ai_finding(ui: &mut egui::Ui, finding: &AiFinding) {
    ui.add_space(ui::theme::spacing::XS);
    ui.label(
        RichText::new(&finding.title)
            .strong()
            .color(ui::theme::colors::primary_text()),
    );
    ui.add(
        egui::Label::new(RichText::new(&finding.detail).color(ui::theme::colors::secondary_text()))
            .wrap(),
    );
    if !finding.examples.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Examples")
                    .size(ui::theme::typography::CAPTION)
                    .color(ui::theme::colors::metadata_text()),
            );
            for item in finding.examples.iter().take(4) {
                render_status_chip(
                    ui,
                    item,
                    ui::theme::colors::metadata_text(),
                    ui::theme::colors::subtle_surface(),
                );
            }
        });
    }
}

fn render_ai_draft_summary_strip(
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

fn render_ai_draft_review(
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

fn render_ai_rule_explanation(ui: &mut egui::Ui, explanation: &AiRuleExplanation) {
    ui.label(
        RichText::new("AI rule explanation")
            .strong()
            .color(ui::theme::colors::heading_text()),
    );
    ui.add(
        egui::Label::new(
            RichText::new(&explanation.summary).color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    if !explanation.rule_order.is_empty() {
        egui::CollapsingHeader::new("Rule order")
            .default_open(false)
            .show(ui, |ui| {
                for rule in &explanation.rule_order {
                    ui.label(
                        RichText::new(&rule.rule_name)
                            .strong()
                            .color(ui::theme::colors::primary_text()),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(&rule.explanation)
                                .color(ui::theme::colors::secondary_text()),
                        )
                        .wrap(),
                    );
                }
            });
    }
    for warning in &explanation.warnings {
        ui.label(RichText::new(format!("Warning: {warning}")).color(ui::theme::colors::warning()));
    }
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
    ui.available_width().min(ui::theme::spacing::FLOW_MAX_WIDTH)
}

fn render_preview_examples(ui: &mut egui::Ui, result: &AnalysisOutput) {
    let aligned_width = preview_aligned_content_width(ui);

    ui.scope(|ui| {
        ui.set_max_width(aligned_width);
        ui.label(
            RichText::new("Planned changes sample")
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        ui.add(
            egui::Label::new(
                RichText::new("Representative ready moves from this preview.")
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
        ui.add_space(6.0);

        if result.preview_examples.is_empty() {
            egui::Frame::group(ui.style())
                .fill(ui::theme::colors::surface())
                .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
                .inner_margin(egui::Margin::same(14.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("No ready examples yet")
                            .strong()
                            .color(ui::theme::colors::heading_text()),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "Files with unclear destinations were left untouched. Open the detailed list to inspect them.",
                            )
                            .color(ui::theme::colors::secondary_text()),
                        )
                        .wrap(),
                    );
                });
            return;
        }

        render_preview_sample_table(ui, &result.preview_examples);
        let hidden_examples = result.preview_examples.len().saturating_sub(PREVIEW_SAMPLE_ROWS);
        if hidden_examples > 0 {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{hidden_examples} more sample{} available in the detailed file list.",
                    plural(hidden_examples)
                ))
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
            );
        }
    });
}

const PREVIEW_SAMPLE_ROWS: usize = 3;

fn render_preview_sample_table(ui: &mut egui::Ui, rows: &[PreviewRow]) {
    egui::Frame::group(ui.style())
        .fill(ui::theme::colors::elevated_surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let width = ui.available_width().max(520.0);
            let file_width = (width * 0.26).max(120.0);
            let destination_width = (width * 0.30).max(150.0);
            let rule_width = (width * 0.26).max(130.0);
            let status_width =
                (width - file_width - destination_width - rule_width - 36.0).max(90.0);

            egui::Grid::new("preview-sample-table")
                .num_columns(4)
                .spacing([12.0, 5.0])
                .show(ui, |ui| {
                    preview_sample_cell(ui, "File", file_width, true);
                    preview_sample_cell(ui, "Destination", destination_width, true);
                    preview_sample_cell(ui, "Rule", rule_width, true);
                    preview_sample_cell(ui, "Status", status_width, true);
                    ui.end_row();

                    for row in rows.iter().take(PREVIEW_SAMPLE_ROWS) {
                        preview_sample_cell(ui, &row.file_name, file_width, false);
                        preview_sample_cell(
                            ui,
                            &preview_example_destination_path(row),
                            destination_width,
                            false,
                        );
                        preview_sample_cell(ui, &row.reason, rule_width, false);
                        let (stroke, fill) = preview_status_colors(&row.status);
                        ui.allocate_ui(egui::vec2(status_width, 22.0), |ui| {
                            render_status_chip(ui, &row.status, stroke, fill);
                        });
                        ui.end_row();
                    }
                });
        });
}

fn preview_sample_cell(ui: &mut egui::Ui, text: &str, width: f32, strong: bool) {
    ui.allocate_ui(egui::vec2(width.max(40.0), 20.0), |ui| {
        let color = if strong {
            ui::theme::colors::metadata_text()
        } else {
            ui::theme::colors::primary_text()
        };
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .monospace()
                    .size(if strong {
                        ui::theme::typography::CAPTION
                    } else {
                        ui::theme::typography::BODY
                    })
                    .color(color),
            )
            .truncate(),
        )
        .on_hover_text(text);
    });
}

fn preview_example_destination_path(row: &PreviewRow) -> String {
    if row.destination_full_path.is_none() {
        return "Stays in place".to_string();
    }
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

fn organize_files_label(ready: usize) -> String {
    format!("Organize {ready} file{}", plural(ready))
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

fn render_undo_progress(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label("Restoring files to their previous locations...");
    });
}

fn render_activity_scope_bar(
    ui: &mut egui::Ui,
    scope: &str,
    status: &str,
    action: &mut Option<HistoryAction>,
) {
    settings_note_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(scope)
                    .strong()
                    .size(ui::theme::typography::CARD_TITLE)
                    .color(ui::theme::colors::heading_text()),
            );
            render_status_chip(
                ui,
                status,
                ui::theme::colors::secondary_text(),
                ui::theme::colors::surface(),
            );
            if ui
                .add(ui::theme::widgets::tertiary_button("Change folder"))
                .clicked()
            {
                *action = Some(HistoryAction::ChangeFolder);
            }
        });
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
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Current folder activity")
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        if ui
            .add_enabled(!busy, ui::theme::widgets::secondary_button("Refresh"))
            .clicked()
        {
            *action = Some(HistoryAction::Refresh);
        }
        let toggle_label = if show_recovery_log {
            "Hide restore history"
        } else {
            "Show restore history"
        };
        if ui
            .add(ui::theme::widgets::secondary_button(toggle_label))
            .clicked()
        {
            *action = Some(HistoryAction::ToggleRecoveryLog);
        }
    });
    ui.add_space(ui::theme::spacing::SM);

    if let Some(result) = undo_result {
        render_undo_result(ui, result);
        ui.add_space(6.0);
    }

    if let Some(message) = message {
        ui.colored_label(ui::theme::colors::error(), message);
        return;
    }

    let Some(active_root) = active_root else {
        ui.add(
            egui::Label::new(
                RichText::new("Choose a folder to see activity for that folder.")
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
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

    if let Some(detail) = detail {
        render_transaction_detail_window(ui.ctx(), detail, action);
    } else if let Some(message) = detail_message {
        render_transaction_detail_error_window(ui.ctx(), message, action);
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
        ui.add(
            egui::Label::new(
                RichText::new("No activity has been recorded for this folder yet.")
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
        if hidden_count > 0 {
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{} activit{} from other folders hidden.",
                        hidden_count,
                        plural_y(hidden_count)
                    ))
                    .color(ui::theme::colors::metadata_text()),
                )
                .wrap(),
            );
        }
        return;
    }

    ui.label(
        RichText::new("Latest activity")
            .strong()
            .size(ui::theme::typography::CARD_TITLE)
            .color(ui::theme::colors::heading_text()),
    );
    render_activity_record_row(ui, rows[0], busy, true, action);

    if rows.len() > 1 {
        ui.add_space(ui::theme::spacing::MD);
        let mut current_date = "";
        for row in rows.iter().skip(1).take(8) {
            let date = activity_date_label(row);
            if date != current_date {
                current_date = date;
                ui.add_space(ui::theme::spacing::XS);
                ui.label(
                    RichText::new(date)
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
            }
            render_activity_record_row(ui, row, busy, false, action);
            ui.add_space(ui::theme::spacing::XS);
        }
    }

    if hidden_count > 0 {
        ui.add_space(ui::theme::spacing::SM);
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "{} activit{} from other folders hidden from this overview.",
                    hidden_count,
                    plural_y(hidden_count)
                ))
                .color(ui::theme::colors::metadata_text()),
            )
            .wrap(),
        );
    }
}

fn render_activity_record_row(
    ui: &mut egui::Ui,
    row: &TransactionRow,
    busy: bool,
    featured: bool,
    action: &mut Option<HistoryAction>,
) {
    let can_restore = can_undo_status(row.status);
    let row_fill = ui::theme::colors::elevated_surface();
    egui::Frame::group(ui.style())
        .fill(row_fill)
        .stroke(egui::Stroke::new(
            1.0,
            if featured && can_restore {
                ui::theme::colors::success()
            } else {
                ui::theme::colors::border()
            },
        ))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(ui::theme::spacing::SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let actions_width = 220.0_f32.min(ui.available_width() * 0.34);
            let time_width = 64.0;
            let text_width = (ui.available_width()
                - actions_width
                - time_width
                - (ui::theme::spacing::MD * 2.0))
                .max(280.0);

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(time_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.label(
                            RichText::new(activity_time_label(row))
                                .monospace()
                                .color(ui::theme::colors::metadata_text()),
                        );
                        if featured && can_restore {
                            ui.label(
                                RichText::new("Restore ready")
                                    .size(ui::theme::typography::CAPTION)
                                    .color(ui::theme::colors::success()),
                            );
                        }
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(text_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let (stroke, fill) = activity_status_colors(row.status);
                            ui::theme::widgets::status_dot(ui, stroke);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(activity_event_title(row))
                                        .strong()
                                        .color(ui::theme::colors::heading_text()),
                                )
                                .wrap(),
                            );
                            render_status_chip(ui, status_label(row.status), stroke, fill);
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(activity_detail(row))
                                    .color(ui::theme::colors::secondary_text()),
                            )
                            .wrap(),
                        );
                        ui.add_space(ui::theme::spacing::XS);
                        render_activity_count_chips(
                            ui,
                            row.completed,
                            row.rolled_back,
                            row.skipped + row.failed + row.pending,
                        );
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!busy, ui::theme::widgets::secondary_button("Details"))
                        .clicked()
                    {
                        *action = Some(HistoryAction::ViewDetails(row.transaction_id.clone()));
                    }
                    let can_undo = can_restore && !busy;
                    if ui
                        .add_enabled(can_undo, ui::theme::widgets::secondary_button("Restore"))
                        .on_disabled_hover_text(
                            "Only completed or failed activities can be restored",
                        )
                        .clicked()
                    {
                        *action = Some(HistoryAction::ConfirmUndo(row.transaction_id.clone()));
                    }
                });
            });
        });
}

fn render_activity_count_chips(
    ui: &mut egui::Ui,
    completed: usize,
    rolled_back: usize,
    needs_review: usize,
) {
    ui.horizontal_wrapped(|ui| {
        render_status_chip(
            ui,
            &format!("Moved {completed}"),
            ui::theme::colors::success(),
            ui::theme::colors::success_bg(),
        );
        render_status_chip(
            ui,
            &format!("Restored {rolled_back}"),
            ui::theme::colors::info(),
            ui::theme::colors::info_bg(),
        );
        render_status_chip(
            ui,
            &format!("Review {needs_review}"),
            ui::theme::colors::warning(),
            ui::theme::colors::warning_bg(),
        );
    });
}

fn render_recovery_log(
    ui: &mut egui::Ui,
    rows: &[TransactionRow],
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new("Restore history")
                .strong()
                .color(ui::theme::colors::heading_text()),
        );
        ui.label(
            RichText::new("Recent recovery journals")
                .size(ui::theme::typography::CAPTION)
                .color(ui::theme::colors::metadata_text()),
        );
    });
    if rows.is_empty() {
        ui.add(
            egui::Label::new(
                RichText::new("No restore history has been recorded yet.")
                    .color(ui::theme::colors::secondary_text()),
            )
            .wrap(),
        );
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(420.0)
        .show(ui, |ui| {
            for row in rows.iter().take(10) {
                render_recovery_log_row(ui, row, busy, action);
                ui.add_space(ui::theme::spacing::XS);
            }
        });

    if rows.len() > 10 {
        ui.label(
            RichText::new(format!("Showing 10 of {} recovery journals.", rows.len()))
                .color(ui::theme::colors::metadata_text()),
        );
    }
}

fn render_recovery_log_row(
    ui: &mut egui::Ui,
    row: &TransactionRow,
    busy: bool,
    action: &mut Option<HistoryAction>,
) {
    egui::Frame::group(ui.style())
        .fill(ui::theme::colors::surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(ui::theme::spacing::SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let available = ui.available_width();
            let action_width = 132.0_f32.min(available * 0.28);
            let content_width = (available - action_width - ui::theme::spacing::MD).max(200.0);

            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(content_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(activity_event_title(row))
                                        .strong()
                                        .color(ui::theme::colors::heading_text()),
                                )
                                .wrap(),
                            )
                            .on_hover_text(format!("Activity id: {}", row.transaction_id));

                            let (stroke, fill) = activity_status_colors(row.status);
                            render_status_chip(ui, status_label(row.status), stroke, fill);
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(activity_count_summary(row))
                                    .color(ui::theme::colors::secondary_text()),
                            )
                            .wrap(),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("Root: {}", row.root_label))
                                    .size(ui::theme::typography::CAPTION)
                                    .color(ui::theme::colors::metadata_text()),
                            )
                            .truncate(),
                        )
                        .on_hover_text(row.root.display().to_string());
                    },
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(action_width, 0.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui
                            .add_enabled(!busy, ui::theme::widgets::secondary_button("Details"))
                            .clicked()
                        {
                            *action = Some(HistoryAction::ViewDetails(row.transaction_id.clone()));
                        }
                    },
                );
            });
        });
}

fn render_transaction_detail_error_window(
    ctx: &egui::Context,
    message: &str,
    action: &mut Option<HistoryAction>,
) {
    let mut open = true;
    egui::Window::new("Activity details")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.colored_label(ui::theme::colors::error(), message);
            ui.add_space(ui::theme::spacing::SM);
            if ui
                .add(ui::theme::widgets::secondary_button("Close"))
                .clicked()
            {
                *action = Some(HistoryAction::CloseDetails);
            }
        });
    if !open {
        *action = Some(HistoryAction::CloseDetails);
    }
}

fn render_transaction_detail_window(
    ctx: &egui::Context,
    detail: &TransactionDetail,
    action: &mut Option<HistoryAction>,
) {
    let mut open = true;
    egui::Window::new("Activity details")
        .open(&mut open)
        .title_bar(false)
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .default_height(560.0)
        .show(ctx, |ui| {
            render_transaction_detail(ui, detail, action);
        });
    if !open {
        *action = Some(HistoryAction::CloseDetails);
    }
}

fn render_transaction_detail(
    ui: &mut egui::Ui,
    detail: &TransactionDetail,
    action: &mut Option<HistoryAction>,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Activity details")
                .strong()
                .size(ui::theme::typography::CARD_TITLE)
                .color(ui::theme::colors::heading_text()),
        );
        let (stroke, fill) = activity_status_colors(detail.status);
        render_status_chip(ui, status_label(detail.status), stroke, fill);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(ui::theme::widgets::secondary_button("Close"))
                .clicked()
            {
                *action = Some(HistoryAction::CloseDetails);
            }
        });
    });
    ui.add_space(ui::theme::spacing::SM);

    ui.add(
        egui::Label::new(
            RichText::new(activity_detail_headline(detail))
                .color(ui::theme::colors::primary_text()),
        )
        .wrap(),
    );
    ui.add(
        egui::Label::new(
            RichText::new(format!("Why: {}", detail.reason_summary))
                .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add(
        egui::Label::new(
            RichText::new(format!("Folder: {}", detail.root))
                .color(ui::theme::colors::metadata_text()),
        )
        .truncate(),
    )
    .on_hover_text(&detail.root);

    ui.add_space(ui::theme::spacing::SM);
    render_activity_count_chips(
        ui,
        detail.operation_counts.completed,
        detail.operation_counts.rolled_back,
        detail.operation_counts.skipped
            + detail.operation_counts.failed
            + detail.operation_counts.pending,
    );

    ui.add_space(ui::theme::spacing::MD);
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
            ui.add_space(ui::theme::spacing::SM);
            render_transaction_operation_rows(ui, detail);
        });
}

fn render_transaction_operation_rows(ui: &mut egui::Ui, detail: &TransactionDetail) {
    if detail.operation_rows.is_empty() {
        ui.label("No operation rows recorded in this journal.");
        return;
    }

    ui.label(RichText::new("Recorded changes").strong());
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(260.0)
        .show(ui, |ui| {
            for row in &detail.operation_rows {
                render_transaction_operation_row(ui, row);
                ui.add_space(ui::theme::spacing::XS);
            }
        });

    if detail.total_operations > detail.operation_rows.len() {
        ui.label(format!(
            "Showing {} of {} recorded operations.",
            detail.operation_rows.len(),
            detail.total_operations
        ));
    }
}

fn render_transaction_operation_row(ui: &mut egui::Ui, row: &TransactionOperationRow) {
    egui::Frame::group(ui.style())
        .fill(ui::theme::colors::surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(ui::theme::spacing::SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(&row.operation_id)
                        .monospace()
                        .strong()
                        .color(ui::theme::colors::heading_text()),
                );
                render_status_chip(
                    ui,
                    operation_status_label(row.status),
                    ui::theme::colors::secondary_text(),
                    ui::theme::colors::elevated_surface(),
                );
                ui.label(
                    RichText::new(&row.reason)
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::metadata_text()),
                );
            });
            render_path_detail_line(ui, "From", &row.source);
            render_path_detail_line(ui, "To", &row.destination);
            if !row.error.trim().is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Error: {}", row.error))
                            .color(ui::theme::colors::error()),
                    )
                    .wrap(),
                );
            }
        });
}

fn render_path_detail_line(ui: &mut egui::Ui, label: &str, path: &str) {
    ui.horizontal(|ui| {
        ui.add_sized(
            [44.0, 20.0],
            egui::Label::new(
                RichText::new(label)
                    .size(ui::theme::typography::CAPTION)
                    .color(ui::theme::colors::metadata_text()),
            ),
        );
        let width = ui.available_width().max(120.0);
        ui.add_sized(
            [width, 20.0],
            egui::Label::new(
                RichText::new(path)
                    .monospace()
                    .color(ui::theme::colors::primary_text()),
            )
            .truncate(),
        )
        .on_hover_text(path);
    });
}

fn render_undo_confirmation(
    ctx: &egui::Context,
    transaction_id: &str,
    confirmed: &mut bool,
    dismissed: &mut bool,
) {
    egui::Window::new("Restore previous layout")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new("Restore previous layout?").strong());
            ui.label("smartfolder will move completed files back to their original paths.");
            ui.label("It will refuse to overwrite anything already at an original path.");
            ui.add_space(6.0);
            truncated_label(ui, &format!("Activity id: {transaction_id}"));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    *dismissed = true;
                }
                if ui.button("Restore previous layout").clicked() {
                    *confirmed = true;
                }
            });
        });
}

fn render_undo_result(ui: &mut egui::Ui, result: &UndoOutput) {
    ui.label(RichText::new("Previous layout restored").strong());
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

fn untouched_reason_label(reason: UntouchedReason) -> &'static str {
    match reason {
        UntouchedReason::NoMatchingRule => "no matching rule",
        UntouchedReason::AlreadyOrganized => "already organized",
        UntouchedReason::UnsupportedMetadata => "unsupported metadata",
        UntouchedReason::UnsafeDestination => "unsafe destination",
        UntouchedReason::DestinationConflict => "destination conflict",
        UntouchedReason::ExcludedByPolicy => "excluded by policy",
    }
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
            PreviewFilter::Untouched,
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
            "No files in this view".to_string()
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
            destination_full_path: Some(operation.destination.display().to_string()),
            reason: operation.reason.clone(),
            status: operation_status(operation).to_string(),
        })
        .collect()
}

fn untouched_preview_rows(records: &[UntouchedRecord], root: &Path) -> Vec<PreviewRow> {
    records
        .iter()
        .map(|record| PreviewRow {
            file_name: file_name_label(&record.path),
            original_folder: relative_folder_label(&record.path, root),
            target_folder: "Stays in place".to_string(),
            source_full_path: record.path.display().to_string(),
            destination_full_path: None,
            reason: untouched_detail(record),
            status: "Untouched".to_string(),
        })
        .collect()
}

fn untouched_detail(record: &UntouchedRecord) -> String {
    if record.detail.trim().is_empty() {
        untouched_reason_label(record.reason).to_string()
    } else {
        record.detail.clone()
    }
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
        ui.label("No files match this view.");
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
                        row.source_full_path,
                        row.destination_full_path
                            .as_deref()
                            .unwrap_or("Stays in place")
                    ),
                );
                if file_response.clicked() {
                    *selected_row = Some(index);
                }
                preview_destination_cell_with_tooltip(
                    ui,
                    row,
                    target_width,
                    row.destination_full_path.as_ref().map_or_else(
                        || "This file stays in its original folder".to_string(),
                        |destination| format!("Full destination: {destination}"),
                    ),
                );
                ui.end_row();
            }
        });

    if result.preview_rows.len() < result.preview_total_rows {
        ui.add_space(8.0);
        ui.label(format!(
            "Showing {} of {} matching files. More rows are stored on disk for paged retrieval.",
            result.preview_rows.len(),
            result.preview_total_rows
        ));
    }
}

fn render_preview_tree(ui: &mut egui::Ui, result: &AnalysisOutput) {
    if result.preview_rows.is_empty() {
        ui.label("No files match this view.");
        return;
    }

    ui.add(
        egui::Label::new(
            RichText::new(
                "Tree view groups the current page by destination folder while preserving paged loading for large folders.",
            )
            .color(ui::theme::colors::secondary_text()),
        )
        .wrap(),
    );
    ui.add_space(ui::theme::spacing::SM);

    let mut folders: BTreeMap<&str, Vec<&PreviewRow>> = BTreeMap::new();
    for row in &result.preview_rows {
        folders
            .entry(row.target_folder.as_str())
            .or_default()
            .push(row);
    }

    for (folder, rows) in folders {
        let header = format!("{folder} ({})", rows.len());
        egui::CollapsingHeader::new(header)
            .default_open(true)
            .show(ui, |ui| {
                for row in rows {
                    ui.horizontal_wrapped(|ui| {
                        let (stroke, fill) = preview_status_colors(&row.status);
                        render_status_chip(ui, &row.status, stroke, fill);
                        ui.label(
                            RichText::new(&row.file_name)
                                .strong()
                                .color(ui::theme::colors::heading_text()),
                        );
                        ui.label(
                            RichText::new(format!("from {}", row.original_folder))
                                .size(ui::theme::typography::CAPTION)
                                .color(ui::theme::colors::metadata_text()),
                        );
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new(&row.reason)
                                .size(ui::theme::typography::CAPTION)
                                .color(ui::theme::colors::secondary_text()),
                        )
                        .wrap(),
                    );
                    ui.add_space(ui::theme::spacing::XS);
                }
            });
        ui.add_space(ui::theme::spacing::XS);
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
        .fill(ui::theme::colors::surface())
        .stroke(egui::Stroke::new(1.0, ui::theme::colors::border()))
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
            ui.add_space(8.0);
            ui::theme::widgets::surface_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Explanation")
                        .strong()
                        .size(ui::theme::typography::CAPTION)
                        .color(ui::theme::colors::heading_text()),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(preview_row_explanation(row))
                            .color(ui::theme::colors::secondary_text()),
                    )
                    .wrap(),
                );
            });
            ui.add_space(6.0);
            truncated_label(ui, &format!("Full source: {}", row.source_full_path));
            if let Some(destination) = &row.destination_full_path {
                truncated_label(ui, &format!("Full destination: {destination}"));
            } else {
                truncated_label(ui, "Destination: Stays in original folder");
            }
        });
}

fn preview_status_colors(status: &str) -> (Color32, Color32) {
    match status {
        "Ready" => (
            ui::theme::colors::success(),
            ui::theme::colors::success_bg(),
        ),
        "Needs Review" => (
            ui::theme::colors::warning(),
            ui::theme::colors::warning_bg(),
        ),
        _ => (
            ui::theme::colors::metadata_text(),
            ui::theme::colors::subtle_surface(),
        ),
    }
}

fn preview_row_explanation(row: &PreviewRow) -> String {
    match row.status.as_str() {
        "Ready" => format!(
            "This file matched '{}'. smartfolder found a destination inside the selected folder and no conflict was detected, so it can be organized after confirmation.",
            row.reason
        ),
        "Needs Review" => format!(
            "This file matched '{}', but the destination needs attention before smartfolder will move it. It stays untouched unless the conflict is resolved.",
            row.reason
        ),
        "Untouched" => format!(
            "This file stays in place because {}. It will not be moved by the current plan.",
            row.reason
        ),
        _ => format!(
            "This row is marked '{}'. Review the source and destination before organizing.",
            row.status
        ),
    }
}

fn activity_status_colors(status: TransactionStatus) -> (Color32, Color32) {
    match status {
        TransactionStatus::Completed => (
            ui::theme::colors::success(),
            ui::theme::colors::success_bg(),
        ),
        TransactionStatus::RolledBack | TransactionStatus::PartiallyRolledBack => {
            (ui::theme::colors::info(), ui::theme::colors::info_bg())
        }
        TransactionStatus::Failed | TransactionStatus::Interrupted => (
            ui::theme::colors::warning(),
            ui::theme::colors::warning_bg(),
        ),
        TransactionStatus::InProgress => (
            ui::theme::colors::warning(),
            ui::theme::colors::warning_bg(),
        ),
    }
}

fn section_max_width(section: AppSection) -> f32 {
    match section {
        AppSection::Organize | AppSection::Settings => ui::theme::spacing::FLOW_MAX_WIDTH,
        AppSection::Activity | AppSection::Rules => ui::theme::spacing::CONTENT_MAX_WIDTH,
    }
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

fn render_organize_step_indicator(
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

fn ai_confidence_label(confidence: smartfolder_core::ai::AiConfidence) -> &'static str {
    match confidence {
        smartfolder_core::ai::AiConfidence::Low => "Low confidence",
        smartfolder_core::ai::AiConfidence::Medium => "Medium confidence",
        smartfolder_core::ai::AiConfidence::High => "High confidence",
    }
}

#[derive(Debug, Clone, Copy)]
enum BuiltinRuleAction {
    Use,
    Customize,
}

#[derive(Debug, Clone, Copy)]
enum SavedProfileAction {
    Use,
    Edit,
}

fn render_active_profile_panel(
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

fn active_profile_summary(
    planning_source: PlanningSource,
    mode: BuiltInMode,
    loaded_profile: Option<&LoadedRuleProfile>,
) -> (String, String, String, bool) {
    match (planning_source, loaded_profile) {
        (PlanningSource::RuleProfile, Some(profile)) => (
            profile.profile.profile_id.clone(),
            format!(
                "{} rule{}",
                profile.profile.rules.len(),
                plural(profile.profile.rules.len())
            ),
            profile_destination_example(&profile.profile),
            true,
        ),
        _ => {
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

fn render_rules_ai_panel(
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

fn builtin_library_items() -> [(BuiltInMode, &'static str, &'static str, &'static str); 3] {
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

fn render_builtin_rule_row(
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

fn render_saved_profile_row(
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

fn render_profile_workspace_status(ui: &mut egui::Ui, loaded_profile: Option<&LoadedRuleProfile>) {
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

fn render_rule_list_panel(ui: &mut egui::Ui, editor: &mut ProfileEditorState) {
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

fn render_rule_detail_editor(
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

fn example_hint(value: &str) -> RichText {
    RichText::new(format!("e.g. {value}"))
        .italics()
        .color(ui::theme::colors::metadata_text())
}

#[derive(Debug, Clone)]
struct DestinationDragPayload {
    source: DestinationDragSource,
}

#[derive(Debug, Clone)]
enum DestinationDragSource {
    Existing(usize),
    Token(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DestinationSegment {
    Text(String),
    Token(String),
}

impl DestinationSegment {
    fn label(&self) -> &str {
        match self {
            Self::Text(value) | Self::Token(value) => value,
        }
    }

    fn is_token(&self) -> bool {
        matches!(self, Self::Token(_))
    }
}

fn render_destination_mode_toggle(ui: &mut egui::Ui, rule: &mut RuleEditorState) {
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut rule.destination_text_mode, false, "Visual");
        ui.selectable_value(&mut rule.destination_text_mode, true, "Text");
    });
}

fn render_destination_builder(ui: &mut egui::Ui, rule: &mut RuleEditorState) {
    if rule.destination_text_mode {
        render_destination_text_editor(ui, rule);
    } else {
        render_destination_visual_editor(ui, rule);
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

fn parse_destination_segments(destination: &str) -> Vec<DestinationSegment> {
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

fn build_destination_from_segments(segments: &[DestinationSegment]) -> String {
    segments
        .iter()
        .map(|segment| segment.label())
        .collect::<Vec<_>>()
        .join("/")
}

fn readable_destination_path(destination: &str) -> String {
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

fn apply_destination_drop(
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

fn rule_destination_summary(rule: &RuleEditorState) -> String {
    if rule.destination.trim().is_empty() {
        "No destination set".to_string()
    } else {
        format!("-> {}", rule.destination.trim())
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
            "Restored previous layout in {folder}: {} file{} restored",
            row.rolled_back,
            plural(row.rolled_back)
        ),
        TransactionStatus::PartiallyRolledBack => format!(
            "Partially restored previous layout in {folder}: {} file{} restored",
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

fn activity_date_label(row: &TransactionRow) -> &str {
    row.started_at
        .split_once(' ')
        .map_or(row.started_at.as_str(), |(date, _)| date)
}

fn activity_time_label(row: &TransactionRow) -> &str {
    row.started_at
        .split_once(' ')
        .map_or("", |(_, time)| time)
        .get(0..5)
        .unwrap_or("")
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
            "Restore completed {}. Original file locations were restored where possible.",
            row.started_at
        ),
        TransactionStatus::PartiallyRolledBack => {
            "Restore completed with remaining issues. Review details before making more changes."
                .to_string()
        }
        TransactionStatus::Interrupted | TransactionStatus::InProgress => format!(
            "Why: {}. {} completed, {} pending. Resume or restore from the recovery controls.",
            row.reason_summary, row.completed, row.pending
        ),
        TransactionStatus::Failed => {
            "Some file moves failed. Review details before retrying or restoring.".to_string()
        }
    }
}

fn activity_count_summary(row: &TransactionRow) -> String {
    format!(
        "{} moved / {} restored / {} needs review",
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
        SourceSnapshot, TransactionStatus, UntouchedReason, UntouchedRecord,
    };
    use smartfolder_core::rules::{CustomRule, RuleProfile};

    use super::{
        activity_count_summary, activity_date_label, activity_event_title, activity_time_label,
        apply_destination_drop, build_destination_from_segments, can_undo_status,
        is_cloud_synced_path, operation_status_label, parse_destination_segments,
        preloaded_root_from_args, preview_rows, profile_file_stem, same_folder, status_label,
        transaction_operation_counts, untouched_preview_rows, AnalysisPlanSource,
        DestinationDragPayload, DestinationDragSource, DestinationSegment, LoadedRuleProfile,
        PlanningSource, ProfileEditorState, RuleEditorState, SmartfolderApp, TransactionRow,
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
        assert_eq!(activity_event_title(&row), "Organized 3 files in Documents");
        assert_eq!(
            activity_count_summary(&row),
            "3 moved / 0 restored / 1 needs review"
        );

        assert_eq!(activity_date_label(&row), "2026-05-12");
        assert_eq!(activity_time_label(&row), "13:00");

        let restored_row = test_transaction_row(TransactionStatus::RolledBack);
        assert_eq!(
            activity_event_title(&restored_row),
            "Restored previous layout in Documents: 0 files restored"
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
        assert!(rows[0]
            .destination_full_path
            .as_ref()
            .is_some_and(|path| path.ends_with("loose.jpg")));
    }

    #[test]
    fn untouched_preview_rows_stay_inspectable_without_destination() {
        let root = PathBuf::from(r"D:\OneDrive\Documents");
        let records = [UntouchedRecord {
            path: root.join("notes.tmp"),
            reason: UntouchedReason::UnsupportedMetadata,
            detail: "Unsupported file metadata".to_string(),
        }];

        let rows = untouched_preview_rows(&records, &root);

        assert_eq!(rows[0].file_name, "notes.tmp");
        assert_eq!(rows[0].target_folder, "Stays in place");
        assert_eq!(rows[0].status, "Untouched");
        assert_eq!(rows[0].destination_full_path, None);
        assert_eq!(rows[0].reason, "Unsupported file metadata");
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
            rules: vec![RuleEditorState {
                rule_name: "Invoices".to_string(),
                destination: "Documents/Invoices/{year}".to_string(),
                destination_text_mode: false,
                new_destination_segment: String::new(),
                priority: "5".to_string(),
                match_all: false,
                extensions: "pdf, docx".to_string(),
                filename_contains: "invoice".to_string(),
                path_contains: "downloads".to_string(),
                min_size_bytes: "100".to_string(),
                max_size_bytes: "200000".to_string(),
                year: "2026".to_string(),
            }],
            selected_rule: 0,
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
        assert_eq!(restored.rules[0].extensions, "pdf, docx");
    }

    #[test]
    fn profile_editor_rejects_invalid_numbers_and_sanitizes_file_names() {
        let mut editor = ProfileEditorState::default();
        editor.rules[0].priority = "soon".to_string();

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

    #[test]
    fn destination_visual_builder_round_trips_segments() {
        let segments = parse_destination_segments(r"Documents/PDFs/{year}\{month}");
        assert_eq!(
            segments,
            vec![
                DestinationSegment::Text("Documents".to_string()),
                DestinationSegment::Text("PDFs".to_string()),
                DestinationSegment::Token("{year}".to_string()),
                DestinationSegment::Token("{month}".to_string()),
            ]
        );
        assert_eq!(
            build_destination_from_segments(&segments),
            "Documents/PDFs/{year}/{month}"
        );
    }

    #[test]
    fn destination_drop_reorders_and_inserts_tokens() {
        let mut segments = parse_destination_segments("Documents/PDFs/{year}");
        apply_destination_drop(
            &mut segments,
            1,
            DestinationDragPayload {
                source: DestinationDragSource::Existing(2),
            },
        );
        assert_eq!(
            build_destination_from_segments(&segments),
            "Documents/{year}/PDFs"
        );

        apply_destination_drop(
            &mut segments,
            3,
            DestinationDragPayload {
                source: DestinationDragSource::Token("{month}".to_string()),
            },
        );
        assert_eq!(
            build_destination_from_segments(&segments),
            "Documents/{year}/PDFs/{month}"
        );
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
                match_all: false,
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
