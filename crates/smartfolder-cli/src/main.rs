//! Command-line interface for smartfolder.
//!
//! Provides a user-friendly interface to the smartfolder core library.
//! The CLI supports analyze, preview, apply, resume, undo, transactions, and profiles commands.
//!
//! # Commands
//!
//! - **analyze**: Scan a directory and generate an organization plan
//! - **preview**: View a previously saved plan
//! - **apply**: Execute a plan and organize files
//! - **resume**: Continue an interrupted or failed transaction
//! - **undo**: Reverse the effects of a transaction
//! - **transactions**: List, inspect, and cleanup transaction journals
//! - **profiles**: List, import, inspect, and validate saved rule profiles
//!
//! # Workflow
//!
//! ```ignore
//! # Analyze and save a plan
//! smartfolder analyze ~/Downloads --output plan.json --mode type
//!
//! # Review the plan
//! smartfolder preview plan.json
//!
//! # Execute the plan
//! smartfolder apply plan.json
//!
//! # Later, undo if needed
//! smartfolder undo txn_20240512123456
//! ```

#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use chrono::Utc;
use serde::Serialize;
use smartfolder_core::apply::{apply_plan, ApplyCancellationToken, ApplyOptions};
use smartfolder_core::model::{BuiltInMode, PlanRecord, TransactionJournal, TransactionStatus};
use smartfolder_core::planner::{generate_plan, render_preview, render_preview_json, PlanOptions};
use smartfolder_core::recovery::{
    cleanup_transactions, inspect_transaction, list_transactions, resume_transaction,
    undo_transaction,
};
use smartfolder_core::rules::RuleProfile;
use smartfolder_core::scanner::{scan_folder, ScanOptions};
use smartfolder_core::storage::{ensure_profiles_dir, profiles_dir};
use thiserror::Error;

type Result<T> = std::result::Result<T, CliError>;

/// CLI entry point. Handles argument parsing and error display.
fn main() -> ExitCode {
    let json_errors = env::args().any(|arg| arg == "--json");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            print_error(&error, json_errors);
            ExitCode::from(error.exit_code())
        }
    }
}

/// Parse arguments and dispatch to appropriate command handler.
///
/// Matches first argument (--version, --help, analyze, preview, apply, undo, transactions)
/// and calls the corresponding command function.
fn run() -> Result<()> {
    let mut args = env::args().collect::<Vec<_>>();
    let program = args
        .first()
        .cloned()
        .unwrap_or_else(|| "smartfolder".to_string());
    args.remove(0);

    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("smartfolder {}", smartfolder_core::version());
            Ok(())
        }
        Some("--help" | "-h") | None => {
            print_help(&program);
            Ok(())
        }
        Some("analyze") => run_analyze(&args[1..]),
        Some("preview") => run_preview(&args[1..]),
        Some("apply") => run_apply(&args[1..]),
        Some("resume" | "continue") => run_resume(&args[1..]),
        Some("undo") => run_undo(&args[1..]),
        Some("transactions") => run_transactions(&args[1..]),
        Some("profiles") => run_profiles(&args[1..]),
        Some(command) => Err(CliError::UnknownCommand {
            command: (*command).to_string(),
        }),
    }
}

/// Scan a directory and generate an organization plan.
///
/// # Logical Flow
///
/// 1. Parse command-line arguments
/// 2. Scan the directory with specified options
/// 3. Generate a plan using built-in mode or rule profile
/// 4. Optionally save plan to file
/// 5. Display preview to user
fn run_analyze(args: &[String]) -> Result<()> {
    let command = AnalyzeCommand::parse(args)?;
    let scan_options = command.scan_options;
    let scan = scan_folder(&command.root, &scan_options)?;
    if !command.quiet {
        eprintln!(
            "Scanned {} entries; collected {} records; skipped {}; warnings {}.",
            scan.summary.entries_seen,
            scan.summary.records_collected,
            scan.summary.entries_skipped,
            scan.summary.warnings
        );
    }
    let now = Utc::now();
    let plan_id = format!("plan_{}", now.format("%Y%m%d%H%M%S"));
    let plan_options = match command.profile_source {
        Some(ProfileSource::Path(profile_path)) => {
            let profile = RuleProfile::from_toml(&fs::read_to_string(&profile_path)?)?;
            PlanOptions::rule_profile(profile, plan_id, now)
        }
        Some(ProfileSource::Saved(profile_id)) => {
            let profile_path = saved_profile_path(&profile_id)?;
            let profile = RuleProfile::from_toml(&fs::read_to_string(&profile_path)?)?;
            PlanOptions::rule_profile(profile, plan_id, now)
        }
        None => PlanOptions::built_in(command.mode, plan_id, now),
    };
    let plan = generate_plan(&command.root, &scan, &plan_options)?;

    if let Some(output) = command.output {
        fs::write(&output, plan.to_pretty_json()?)?;
    }

    if command.quiet {
        return Ok(());
    }

    if command.json {
        println!("{}", render_preview_json(&plan)?);
    } else {
        print!("{}", render_preview(&plan));
    }

    Ok(())
}

/// Display a previously saved plan.
fn run_preview(args: &[String]) -> Result<()> {
    let command = PreviewCommand::parse(args)?;
    let plan: PlanRecord = serde_json::from_str(&fs::read_to_string(&command.plan_path)?)?;

    if command.json {
        println!("{}", render_preview_json(&plan)?);
    } else {
        print!("{}", render_preview(&plan));
    }

    Ok(())
}

/// Execute a plan and organize files.
///
/// # Logical Flow
///
/// 1. Load plan from file
/// 2. Warn if root is in cloud-synced folder
/// 3. Display preview and request confirmation (unless --yes)
/// 4. Set up Ctrl+C handler for graceful cancellation
/// 5. Execute plan with transaction journaling
/// 6. Display results and journal path
fn run_apply(args: &[String]) -> Result<()> {
    let command = ApplyCommand::parse(args)?;
    let plan: PlanRecord = serde_json::from_str(&fs::read_to_string(&command.plan_path)?)?;

    if is_cloud_synced_path(&plan.root) && !command.confirm_cloud_folder {
        if command.yes {
            return Err(CliError::CloudFolderRequiresConfirmation {
                root: plan.root.clone(),
            });
        }

        confirm_or_decline(&format!(
            "The root '{}' appears to be in a cloud-synced folder. Continue applying this plan?",
            plan.root.display()
        ))?;
    }

    if !command.yes {
        print!("{}", render_preview(&plan));
        confirm_or_decline("Apply the selected operations in this plan?")?;
    }

    let now = Utc::now();
    let cancellation = ApplyCancellationToken::default();
    let cancellation_handler = cancellation.clone();
    ctrlc::set_handler(move || {
        cancellation_handler.cancel();
    })
    .map_err(|error| CliError::SignalHandler(error.to_string()))?;

    let mut options = ApplyOptions::new(
        format!(
            "txn_{}_{:09}",
            now.format("%Y%m%d%H%M%S"),
            now.timestamp_subsec_nanos()
        ),
        now,
    );
    options.journal_export = command.journal_export;
    options.cancellation = cancellation;

    let summary = apply_plan(&plan, &options)?;
    println!("Transaction: {}", summary.transaction_id);
    println!("Journal: {}", summary.journal_path.display());
    println!(
        "Completed: {} | Skipped: {} | Failed: {}",
        summary.completed, summary.skipped, summary.failed
    );

    Ok(())
}

/// Undo operations from a completed transaction.
fn run_undo(args: &[String]) -> Result<()> {
    let command = UndoCommand::parse(args)?;
    let journal = inspect_transaction(&command.transaction_id)?;

    if !command.yes {
        print_journal_summary(&journal);
        confirm_or_decline("Undo completed operations from this transaction?")?;
    }

    let summary = undo_transaction(&command.transaction_id)?;
    println!("Transaction: {}", summary.transaction_id);
    println!("Journal: {}", summary.journal_path.display());
    println!(
        "Rolled back: {} | Skipped: {} | Failed: {}",
        summary.rolled_back, summary.skipped, summary.failed
    );

    Ok(())
}

/// Continue an interrupted or failed transaction from its journal.
fn run_resume(args: &[String]) -> Result<()> {
    let command = ResumeCommand::parse(args)?;
    let journal = inspect_transaction(&command.transaction_id)?;
    if !matches!(
        journal.status,
        TransactionStatus::InProgress | TransactionStatus::Interrupted | TransactionStatus::Failed
    ) {
        return Err(CliError::TransactionNotResumable {
            transaction_id: command.transaction_id,
            status: journal.status,
        });
    }

    if !command.yes {
        print_journal_summary(&journal);
        confirm_or_decline("Resume pending operations from this transaction?")?;
    }

    let summary = resume_transaction(&journal.transaction_id)?;
    println!("Transaction: {}", summary.transaction_id);
    println!("Journal: {}", summary.journal_path.display());
    println!(
        "Resumed: {} | Completed: {} | Skipped: {} | Failed: {}",
        summary.resumed, summary.completed, summary.skipped, summary.failed
    );

    Ok(())
}

/// List, inspect, or cleanup transactions.
fn run_transactions(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("list") => {
            for transaction in list_transactions()? {
                println!(
                    "{} | {:?} | {} | {}",
                    transaction.transaction_id,
                    transaction.status,
                    transaction.started_at,
                    transaction.root.display()
                );
            }
            Ok(())
        }
        Some("inspect") => {
            let transaction_id = args.get(1).ok_or(CliError::MissingArgument {
                name: "transaction-id",
            })?;
            let journal = inspect_transaction(transaction_id)?;
            println!("{}", journal.to_pretty_json()?);
            Ok(())
        }
        Some("cleanup") => {
            let include_incomplete = args[1..].iter().any(|arg| arg == "--include-incomplete");
            let unknown = args[1..]
                .iter()
                .find(|arg| arg.as_str() != "--include-incomplete");
            if let Some(option) = unknown {
                return Err(CliError::UnknownOption {
                    option: option.clone(),
                });
            }
            let summary = cleanup_transactions(include_incomplete)?;
            println!(
                "Removed: {} | Kept incomplete: {}",
                summary.removed.len(),
                summary.kept.len()
            );
            Ok(())
        }
        Some(command) => Err(CliError::UnknownCommand {
            command: format!("transactions {command}"),
        }),
        None => Err(CliError::MissingArgument {
            name: "transactions subcommand",
        }),
    }
}

/// List, import, inspect, or validate rule profiles.
fn run_profiles(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("list") => {
            let json = parse_flag_options(&args[1..], &["--json"])?.contains(&"--json");
            let profiles = list_saved_profiles()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&profiles)?);
            } else if profiles.is_empty() {
                println!("No saved profiles found.");
            } else {
                for profile in profiles {
                    println!(
                        "{} | {} rule{} | {}",
                        profile.profile_id,
                        profile.rule_count,
                        if profile.rule_count == 1 { "" } else { "s" },
                        profile.path.display()
                    );
                }
            }
            Ok(())
        }
        Some("inspect") => {
            let profile_id = args
                .get(1)
                .ok_or(CliError::MissingArgument { name: "profile-id" })?;
            let json = parse_flag_options(&args[2..], &["--json"])?.contains(&"--json");
            let path = saved_profile_path(profile_id)?;
            let profile = RuleProfile::from_toml(&fs::read_to_string(&path)?)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&profile)?);
            } else {
                print_profile_summary(&profile, &path);
            }
            Ok(())
        }
        Some("import") => {
            let source = args
                .get(1)
                .ok_or(CliError::MissingArgument { name: "rules.toml" })?;
            let mut profile_id_override = None;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--id" => {
                        index += 1;
                        profile_id_override = Some(value_arg(args, index, "--id")?.to_string());
                    }
                    option => {
                        return Err(CliError::UnknownOption {
                            option: option.to_string(),
                        });
                    }
                }
                index += 1;
            }

            let source_path = PathBuf::from(source);
            let mut profile = RuleProfile::from_toml(&fs::read_to_string(&source_path)?)?;
            if let Some(profile_id) = profile_id_override {
                profile.profile_id = profile_id;
                profile.validate()?;
            }
            let destination = saved_profile_path_for_write(&profile.profile_id)?;
            fs::write(&destination, profile.to_toml_string()?)?;
            println!(
                "Imported profile '{}' to {}.",
                profile.profile_id,
                destination.display()
            );
            Ok(())
        }
        Some("validate") => {
            let source = args
                .get(1)
                .ok_or(CliError::MissingArgument { name: "rules.toml" })?;
            parse_flag_options(&args[2..], &[])?;
            let path = PathBuf::from(source);
            let profile = RuleProfile::from_toml(&fs::read_to_string(&path)?)?;
            println!(
                "Profile '{}' is valid ({} rule{}).",
                profile.profile_id,
                profile.rules.len(),
                if profile.rules.len() == 1 { "" } else { "s" }
            );
            Ok(())
        }
        Some(command) => Err(CliError::UnknownProfilesCommand {
            command: (*command).to_string(),
        }),
        None => Err(CliError::MissingArgument {
            name: "profiles subcommand",
        }),
    }
}

/// Arguments for the 'analyze' command: scan and plan generation.
#[derive(Debug)]
struct AnalyzeCommand {
    root: PathBuf,
    output: Option<PathBuf>,
    profile_source: Option<ProfileSource>,
    mode: BuiltInMode,
    scan_options: ScanOptions,
    json: bool,
    quiet: bool,
}

/// Source for custom rules during analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProfileSource {
    Path(PathBuf),
    Saved(String),
}

impl AnalyzeCommand {
    /// Parse analyze command arguments from CLI.
    fn parse(args: &[String]) -> Result<Self> {
        let root = args
            .first()
            .ok_or(CliError::MissingArgument { name: "root" })?;
        let mut command = Self {
            root: PathBuf::from(root),
            output: None,
            profile_source: None,
            mode: BuiltInMode::Type,
            scan_options: ScanOptions::default(),
            json: false,
            quiet: false,
        };
        let mut index = 1;

        while index < args.len() {
            match args[index].as_str() {
                "--output" => {
                    index += 1;
                    command.output = Some(PathBuf::from(value_arg(args, index, "--output")?));
                }
                "--profile" => {
                    index += 1;
                    command.set_profile_source(ProfileSource::Path(PathBuf::from(value_arg(
                        args,
                        index,
                        "--profile",
                    )?)))?;
                }
                "--profile-id" | "--saved-profile" => {
                    index += 1;
                    command.set_profile_source(ProfileSource::Saved(
                        value_arg(args, index, "--profile-id")?.to_string(),
                    ))?;
                }
                "--mode" => {
                    index += 1;
                    command.mode = parse_mode(value_arg(args, index, "--mode")?)?;
                }
                "--max-depth" => {
                    index += 1;
                    command.scan_options.max_depth = Some(parse_usize(
                        value_arg(args, index, "--max-depth")?,
                        "--max-depth",
                    )?);
                    command.scan_options.current_folder_only = false;
                }
                "--include-subfolders" | "--recursive" => {
                    command.scan_options.current_folder_only = false;
                }
                "--current-folder-only" => command.scan_options.current_folder_only = true,
                "--include-hidden" => command.scan_options.include_hidden = true,
                "--include-system" => command.scan_options.include_system = true,
                "--include-project-folders" => command.scan_options.include_project_folders = true,
                "--exclude" => {
                    index += 1;
                    command
                        .scan_options
                        .exclude_names
                        .push(value_arg(args, index, "--exclude")?.to_string());
                }
                "--json" => command.json = true,
                "--quiet" => command.quiet = true,
                option => {
                    return Err(CliError::UnknownOption {
                        option: option.to_string(),
                    });
                }
            }

            index += 1;
        }

        Ok(command)
    }

    fn set_profile_source(&mut self, source: ProfileSource) -> Result<()> {
        if self.profile_source.is_some() {
            return Err(CliError::ConflictingOptions {
                left: "--profile",
                right: "--profile-id",
            });
        }
        self.profile_source = Some(source);
        Ok(())
    }
}

/// Arguments for the 'preview' command: display a saved plan.
#[derive(Debug)]
struct PreviewCommand {
    plan_path: PathBuf,
    json: bool,
}

impl PreviewCommand {
    /// Parse preview command arguments from CLI.
    fn parse(args: &[String]) -> Result<Self> {
        let plan_path = args
            .first()
            .ok_or(CliError::MissingArgument { name: "plan" })?;
        let mut command = Self {
            plan_path: PathBuf::from(plan_path),
            json: false,
        };

        for option in &args[1..] {
            match option.as_str() {
                "--json" => command.json = true,
                unknown => {
                    return Err(CliError::UnknownOption {
                        option: unknown.to_string(),
                    });
                }
            }
        }

        Ok(command)
    }
}

/// Arguments for the 'apply' command: execute a plan.
#[derive(Debug)]
struct ApplyCommand {
    plan_path: PathBuf,
    yes: bool,
    confirm_cloud_folder: bool,
    journal_export: Option<PathBuf>,
}

impl ApplyCommand {
    /// Parse apply command arguments from CLI.
    fn parse(args: &[String]) -> Result<Self> {
        let plan_path = args
            .first()
            .ok_or(CliError::MissingArgument { name: "plan" })?;
        let mut command = Self {
            plan_path: PathBuf::from(plan_path),
            yes: false,
            confirm_cloud_folder: false,
            journal_export: None,
        };
        let mut index = 1;

        while index < args.len() {
            match args[index].as_str() {
                "--yes" => command.yes = true,
                "--confirm-cloud-folder" => command.confirm_cloud_folder = true,
                "--journal-export" => {
                    index += 1;
                    command.journal_export =
                        Some(PathBuf::from(value_arg(args, index, "--journal-export")?));
                }
                unknown => {
                    return Err(CliError::UnknownOption {
                        option: unknown.to_string(),
                    });
                }
            }

            index += 1;
        }

        Ok(command)
    }
}

/// Arguments for the 'undo' command: rollback a transaction.
#[derive(Debug)]
struct UndoCommand {
    transaction_id: String,
    yes: bool,
}

/// Arguments for the 'resume' command: continue a transaction.
#[derive(Debug)]
struct ResumeCommand {
    transaction_id: String,
    yes: bool,
}

impl ResumeCommand {
    /// Parse resume command arguments from CLI.
    fn parse(args: &[String]) -> Result<Self> {
        let transaction_id = args
            .first()
            .ok_or(CliError::MissingArgument {
                name: "transaction-id",
            })?
            .clone();
        let mut command = Self {
            transaction_id,
            yes: false,
        };

        for option in &args[1..] {
            match option.as_str() {
                "--yes" => command.yes = true,
                unknown => {
                    return Err(CliError::UnknownOption {
                        option: unknown.to_string(),
                    });
                }
            }
        }

        Ok(command)
    }
}

impl UndoCommand {
    /// Parse undo command arguments from CLI.
    fn parse(args: &[String]) -> Result<Self> {
        let transaction_id = args
            .first()
            .ok_or(CliError::MissingArgument {
                name: "transaction-id",
            })?
            .clone();
        let mut command = Self {
            transaction_id,
            yes: false,
        };

        for option in &args[1..] {
            match option.as_str() {
                "--yes" => command.yes = true,
                unknown => {
                    return Err(CliError::UnknownOption {
                        option: unknown.to_string(),
                    });
                }
            }
        }

        Ok(command)
    }
}

/// Get a required option value or error if not provided.
fn value_arg<'a>(args: &'a [String], index: usize, option: &'static str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or(CliError::MissingOptionValue { option })
}

/// Parse an organization mode from a string.
fn parse_mode(value: &str) -> Result<BuiltInMode> {
    match value {
        "type" => Ok(BuiltInMode::Type),
        "date" => Ok(BuiltInMode::Date),
        "extension" => Ok(BuiltInMode::Extension),
        "type-year" | "type-date" | "type-year-month-day" => Ok(BuiltInMode::TypeYear),
        _ => Err(CliError::InvalidMode {
            mode: value.to_string(),
        }),
    }
}

/// Parse an unsigned integer from a string with error context.
fn parse_usize(value: &str, option: &'static str) -> Result<usize> {
    value.parse().map_err(|_| CliError::InvalidNumber {
        option,
        value: value.to_string(),
    })
}

fn parse_flag_options<'a>(args: &[String], allowed: &[&'a str]) -> Result<Vec<&'a str>> {
    let mut flags = Vec::new();
    for arg in args {
        if let Some(&flag) = allowed.iter().find(|allowed| arg.as_str() == **allowed) {
            flags.push(flag);
        } else {
            return Err(CliError::UnknownOption {
                option: arg.clone(),
            });
        }
    }
    Ok(flags)
}

#[derive(Debug, Serialize)]
struct SavedProfileSummary {
    profile_id: String,
    rule_count: usize,
    path: PathBuf,
}

fn list_saved_profiles() -> Result<Vec<SavedProfileSummary>> {
    let directory = ensure_profiles_dir()?;
    let mut profiles = Vec::new();

    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(profile) = RuleProfile::from_toml(&content) {
                profiles.push(SavedProfileSummary {
                    profile_id: profile.profile_id,
                    rule_count: profile.rules.len(),
                    path,
                });
            }
        }
    }

    profiles.sort_by(|left, right| {
        left.profile_id
            .cmp(&right.profile_id)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(profiles)
}

fn saved_profile_path(profile_id: &str) -> Result<PathBuf> {
    Ok(profiles_dir()?.join(format!("{}.toml", profile_file_stem(profile_id)?)))
}

fn saved_profile_path_for_write(profile_id: &str) -> Result<PathBuf> {
    Ok(ensure_profiles_dir()?.join(format!("{}.toml", profile_file_stem(profile_id)?)))
}

fn profile_file_stem(profile_id: &str) -> Result<String> {
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
        Err(CliError::InvalidProfileId {
            profile_id: profile_id.to_string(),
        })
    } else {
        Ok(stem)
    }
}

fn print_profile_summary(profile: &RuleProfile, path: &std::path::Path) {
    println!("Profile: {}", profile.profile_id);
    println!("Path: {}", path.display());
    println!(
        "Rules: {} rule{}",
        profile.rules.len(),
        if profile.rules.len() == 1 { "" } else { "s" }
    );
    for (index, rule) in profile.rules.iter().enumerate() {
        println!(
            "{}. {} -> {}{}",
            index + 1,
            rule.name,
            rule.destination,
            if rule.match_all { " (match all)" } else { "" }
        );
    }
}

/// Ask user for confirmation, requiring explicit 'yes' response.
fn confirm_or_decline(message: &str) -> Result<()> {
    print!("{message} Type 'yes' to continue: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err(CliError::ConfirmationDeclined)
    }
}

/// Check if path is in a cloud-synced folder (`OneDrive`, `Dropbox`, etc.).
/// Such folders should prompt for confirmation before organizing.
fn is_cloud_synced_path(path: &std::path::Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            name.as_str(),
            "onedrive"
                | "dropbox"
                | "icloud drive"
                | "iclouddrive"
                | "google drive"
                | "googledrive"
                | "box"
        ) || name.starts_with("onedrive - ")
    })
}

/// Display a summary of a transaction for user confirmation.
fn print_journal_summary(journal: &TransactionJournal) {
    println!("Transaction: {}", journal.transaction_id);
    println!("Status: {:?}", journal.status);
    println!("Root: {}", journal.root.display());
    println!("Operations: {}", journal.operations.len());
}

/// Print an error, optionally as JSON.
fn print_error(error: &CliError, json: bool) {
    if json {
        eprintln!(
            "{}",
            serde_json::json!({
                "error": {
                    "message": error.to_string(),
                    "exit_code": error.exit_code()
                }
            })
        );
    } else {
        eprintln!("error: {error}");
    }
}

fn print_help(program: &str) {
    println!(
        "\
smartfolder {}

USAGE:
    {program} <COMMAND>

COMMANDS:
    analyze <root>              Analyze a folder and optionally write a plan
    preview <plan.json>         Preview a generated plan
    apply <plan.json>           Apply a confirmed plan
    resume <transaction-id>     Resume an interrupted or failed transaction
    undo <transaction-id>       Undo a transaction
    transactions <SUBCOMMAND>   List, inspect, or clean transaction journals
    profiles <SUBCOMMAND>       List, import, inspect, or validate rule profiles

OPTIONS:
    -h, --help                  Print help
    -V, --version               Print version

ANALYZE OPTIONS:
    --output <plan.json>        Write the generated plan as JSON
    --profile <rules.toml>      Use a TOML custom rule profile
    --profile-id <id>           Use a saved app-local rule profile
    --saved-profile <id>        Alias for --profile-id
    --mode <mode>               Built-in mode: type, date, extension, type-year, type-date
    --include-subfolders        Recurse into subfolders during analysis
    --recursive                 Alias for --include-subfolders
    --max-depth <n>             Recurse into subfolders up to this depth
    --current-folder-only       Do not recurse into subfolders (default)
    --include-hidden            Include hidden files/folders
    --include-system            Include system files/folders where detectable
    --include-project-folders   Include default project/dependency exclusions
    --exclude <name>            Exclude entries by exact name
    --json                      Print JSON instead of human-readable preview
    --quiet                     Suppress preview output after analysis

APPLY OPTIONS:
    --yes                       Apply without interactive confirmation
    --confirm-cloud-folder      Confirm applying inside a detected cloud folder
    --journal-export <path>     Write a copy of the transaction journal

UNDO OPTIONS:
    --yes                       Undo without interactive confirmation

RESUME OPTIONS:
    --yes                       Resume without interactive confirmation

TRANSACTION SUBCOMMANDS:
    transactions list
    transactions inspect <transaction-id>
    transactions cleanup [--include-incomplete]

PROFILE SUBCOMMANDS:
    profiles list [--json]
    profiles inspect <profile-id> [--json]
    profiles import <rules.toml> [--id <profile-id>]
    profiles validate <rules.toml>
",
        smartfolder_core::version()
    );
}

/// CLI error types with associated metadata for error reporting.
///
/// Maps to specific exit codes:
/// - 1: IO, core library, JSON serialization, or signal handler errors
/// - 2: User input errors (unknown command, missing args, invalid values)
#[derive(Debug, Error)]
enum CliError {
    #[error("unknown command '{command}'. Use --help to see available commands")]
    UnknownCommand { command: String },

    #[error("unknown option '{option}'")]
    UnknownOption { option: String },

    #[error("missing required argument: {name}")]
    MissingArgument { name: &'static str },

    #[error("missing value for option {option}")]
    MissingOptionValue { option: &'static str },

    #[error("conflicting options: {left} and {right}")]
    ConflictingOptions {
        left: &'static str,
        right: &'static str,
    },

    #[error(
        "invalid mode '{mode}'; expected type, date, extension, type-year, type-date, or type-year-month-day"
    )]
    InvalidMode { mode: String },

    #[error("invalid profile id '{profile_id}'")]
    InvalidProfileId { profile_id: String },

    #[error("unknown profiles subcommand '{command}'")]
    UnknownProfilesCommand { command: String },

    #[error("invalid number for {option}: {value}")]
    InvalidNumber { option: &'static str, value: String },

    #[error("confirmation declined")]
    ConfirmationDeclined,

    #[error("transaction '{transaction_id}' is not resumable because it is '{status:?}'")]
    TransactionNotResumable {
        transaction_id: String,
        status: TransactionStatus,
    },

    #[error(
        "cloud-synced folder requires explicit confirmation; rerun with --confirm-cloud-folder: {root}"
    )]
    CloudFolderRequiresConfirmation { root: PathBuf },

    #[error("failed to install Ctrl+C handler: {0}")]
    SignalHandler(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Core(#[from] smartfolder_core::SmartfolderError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl CliError {
    /// Get the exit code for this error.
    const fn exit_code(&self) -> u8 {
        match self {
            Self::UnknownCommand { .. }
            | Self::UnknownOption { .. }
            | Self::MissingArgument { .. }
            | Self::MissingOptionValue { .. }
            | Self::ConflictingOptions { .. }
            | Self::InvalidMode { .. }
            | Self::InvalidProfileId { .. }
            | Self::UnknownProfilesCommand { .. }
            | Self::InvalidNumber { .. }
            | Self::ConfirmationDeclined
            | Self::TransactionNotResumable { .. }
            | Self::CloudFolderRequiresConfirmation { .. } => 2,
            Self::Io(_) | Self::Core(_) | Self::Json(_) | Self::SignalHandler(_) => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn analyze_parser_supports_all_mvp_flags() {
        let command = AnalyzeCommand::parse(&strings(&[
            r"D:\Root",
            "--output",
            "plan.json",
            "--profile",
            "rules.toml",
            "--mode",
            "type-year-month-day",
            "--max-depth",
            "2",
            "--current-folder-only",
            "--include-hidden",
            "--include-system",
            "--include-project-folders",
            "--exclude",
            "node_modules",
            "--exclude",
            "skipme",
            "--json",
            "--quiet",
        ]))
        .expect("all options should parse");

        assert_eq!(command.root, PathBuf::from(r"D:\Root"));
        assert_eq!(command.output, Some(PathBuf::from("plan.json")));
        assert_eq!(
            command.profile_source,
            Some(ProfileSource::Path(PathBuf::from("rules.toml")))
        );
        assert_eq!(command.mode, BuiltInMode::TypeYear);
        assert_eq!(command.scan_options.max_depth, Some(2));
        assert!(command.scan_options.current_folder_only);
        assert!(command.scan_options.include_hidden);
        assert!(command.scan_options.include_system);
        assert!(command.scan_options.include_project_folders);
        assert_eq!(
            command.scan_options.exclude_names,
            vec!["node_modules".to_string(), "skipme".to_string()]
        );
        assert!(command.json);
        assert!(command.quiet);
    }

    #[test]
    fn analyze_parser_defaults_to_current_folder_and_supports_recursive_aliases() {
        let default_command =
            AnalyzeCommand::parse(&strings(&[r"D:\Root"])).expect("default analyze parses");
        assert!(default_command.scan_options.current_folder_only);

        let include_subfolders =
            AnalyzeCommand::parse(&strings(&[r"D:\Root", "--include-subfolders"]))
                .expect("include subfolders parses");
        assert!(!include_subfolders.scan_options.current_folder_only);

        let recursive = AnalyzeCommand::parse(&strings(&[r"D:\Root", "--recursive"]))
            .expect("recursive alias parses");
        assert!(!recursive.scan_options.current_folder_only);

        let max_depth = AnalyzeCommand::parse(&strings(&[r"D:\Root", "--max-depth", "2"]))
            .expect("max depth parses");
        assert!(!max_depth.scan_options.current_folder_only);
        assert_eq!(max_depth.scan_options.max_depth, Some(2));
    }

    #[test]
    fn analyze_parser_supports_saved_profile_ids() {
        let command = AnalyzeCommand::parse(&strings(&[
            r"D:\Root",
            "--profile-id",
            "my-profile",
            "--quiet",
        ]))
        .expect("profile id should parse");

        assert_eq!(
            command.profile_source,
            Some(ProfileSource::Saved("my-profile".to_string()))
        );

        let conflict = AnalyzeCommand::parse(&strings(&[
            r"D:\Root",
            "--profile",
            "rules.toml",
            "--profile-id",
            "my-profile",
        ]))
        .expect_err("profile sources should be mutually exclusive");
        assert!(matches!(conflict, CliError::ConflictingOptions { .. }));
    }

    #[test]
    fn analyze_parser_rejects_missing_values_and_invalid_numbers() {
        let missing_output = AnalyzeCommand::parse(&strings(&[r"D:\Root", "--output"]))
            .expect_err("missing output path should fail");
        assert!(matches!(
            missing_output,
            CliError::MissingOptionValue { option: "--output" }
        ));

        let missing_profile = AnalyzeCommand::parse(&strings(&[r"D:\Root", "--profile"]))
            .expect_err("missing profile path should fail");
        assert!(matches!(
            missing_profile,
            CliError::MissingOptionValue {
                option: "--profile"
            }
        ));

        let invalid_number = AnalyzeCommand::parse(&strings(&[r"D:\Root", "--max-depth", "nope"]))
            .expect_err("invalid max depth should fail");
        assert!(matches!(
            invalid_number,
            CliError::InvalidNumber {
                option: "--max-depth",
                ..
            }
        ));
    }

    #[test]
    fn preview_apply_resume_and_undo_parsers_support_mvp_options() {
        let preview =
            PreviewCommand::parse(&strings(&["plan.json", "--json"])).expect("preview parses");
        assert_eq!(preview.plan_path, PathBuf::from("plan.json"));
        assert!(preview.json);

        let apply = ApplyCommand::parse(&strings(&[
            "plan.json",
            "--yes",
            "--confirm-cloud-folder",
            "--journal-export",
            "journal.json",
        ]))
        .expect("apply parses");
        assert_eq!(apply.plan_path, PathBuf::from("plan.json"));
        assert!(apply.yes);
        assert!(apply.confirm_cloud_folder);
        assert_eq!(apply.journal_export, Some(PathBuf::from("journal.json")));

        let resume = ResumeCommand::parse(&strings(&["txn_123", "--yes"])).expect("resume parses");
        assert_eq!(resume.transaction_id, "txn_123");
        assert!(resume.yes);

        let undo = UndoCommand::parse(&strings(&["txn_123", "--yes"])).expect("undo parses");
        assert_eq!(undo.transaction_id, "txn_123");
        assert!(undo.yes);
    }

    #[test]
    fn parse_mode_supports_aliases_and_rejects_unknown_values() {
        assert_eq!(parse_mode("type").expect("type mode"), BuiltInMode::Type);
        assert_eq!(parse_mode("date").expect("date mode"), BuiltInMode::Date);
        assert_eq!(
            parse_mode("extension").expect("extension mode"),
            BuiltInMode::Extension
        );
        assert_eq!(
            parse_mode("type-year").expect("type-year mode"),
            BuiltInMode::TypeYear
        );
        assert_eq!(
            parse_mode("type-date").expect("type-date alias"),
            BuiltInMode::TypeYear
        );
        assert_eq!(
            parse_mode("type-year-month-day").expect("type-year-month-day alias"),
            BuiltInMode::TypeYear
        );

        let error = parse_mode("invalid").expect_err("invalid mode should fail");
        assert!(matches!(error, CliError::InvalidMode { .. }));
    }

    #[test]
    fn cloud_sync_detection_matches_supported_provider_names() {
        assert!(is_cloud_synced_path(Path::new(
            r"C:\Users\User\OneDrive\Documents"
        )));
        assert!(is_cloud_synced_path(Path::new(
            r"C:\Users\User\OneDrive - Personal\Documents"
        )));
        assert!(is_cloud_synced_path(Path::new(
            r"C:\Users\User\Google Drive\Documents"
        )));
        assert!(is_cloud_synced_path(Path::new(
            r"C:\Users\User\Dropbox\Documents"
        )));
        assert!(!is_cloud_synced_path(Path::new(r"C:\Users\User\Documents")));
    }

    #[test]
    fn transaction_subcommands_validate_arguments() {
        let missing = run_transactions(&[]).expect_err("missing subcommand should be rejected");
        assert!(matches!(
            missing,
            CliError::MissingArgument {
                name: "transactions subcommand"
            }
        ));

        let unknown_cleanup = run_transactions(&strings(&["cleanup", "--bogus"]))
            .expect_err("cleanup should reject unknown options");
        assert!(matches!(unknown_cleanup, CliError::UnknownOption { .. }));

        let unknown_subcommand = run_transactions(&strings(&["bogus"]))
            .expect_err("unknown transaction subcommand should fail");
        assert!(matches!(
            unknown_subcommand,
            CliError::UnknownCommand { .. }
        ));
    }

    #[test]
    fn cli_error_exit_codes_match_documented_contract() {
        assert_eq!(
            CliError::UnknownCommand {
                command: "bogus".to_string()
            }
            .exit_code(),
            2
        );
        assert_eq!(CliError::ConfirmationDeclined.exit_code(), 2);
        assert_eq!(
            CliError::CloudFolderRequiresConfirmation {
                root: PathBuf::from(r"D:\OneDrive")
            }
            .exit_code(),
            2
        );
        assert_eq!(
            CliError::Io(std::io::Error::other("io failure")).exit_code(),
            1
        );
    }
}
