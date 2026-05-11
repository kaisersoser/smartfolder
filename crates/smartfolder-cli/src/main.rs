#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use chrono::Utc;
use smartfolder_core::apply::{apply_plan, ApplyCancellationToken, ApplyOptions};
use smartfolder_core::model::{BuiltInMode, PlanRecord, TransactionJournal};
use smartfolder_core::planner::{generate_plan, render_preview, render_preview_json, PlanOptions};
use smartfolder_core::recovery::{
    cleanup_transactions, inspect_transaction, list_transactions, undo_transaction,
};
use smartfolder_core::rules::RuleProfile;
use smartfolder_core::scanner::{scan_folder, ScanOptions};
use thiserror::Error;

type Result<T> = std::result::Result<T, CliError>;

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
        Some("undo") => run_undo(&args[1..]),
        Some("transactions") => run_transactions(&args[1..]),
        Some(command) => Err(CliError::UnknownCommand {
            command: (*command).to_string(),
        }),
    }
}

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
    let plan_options = match command.profile {
        Some(profile_path) => {
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

#[derive(Debug)]
struct AnalyzeCommand {
    root: PathBuf,
    output: Option<PathBuf>,
    profile: Option<PathBuf>,
    mode: BuiltInMode,
    scan_options: ScanOptions,
    json: bool,
    quiet: bool,
}

impl AnalyzeCommand {
    fn parse(args: &[String]) -> Result<Self> {
        let root = args
            .first()
            .ok_or(CliError::MissingArgument { name: "root" })?;
        let mut command = Self {
            root: PathBuf::from(root),
            output: None,
            profile: None,
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
                    command.profile = Some(PathBuf::from(value_arg(args, index, "--profile")?));
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
}

#[derive(Debug)]
struct PreviewCommand {
    plan_path: PathBuf,
    json: bool,
}

impl PreviewCommand {
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

#[derive(Debug)]
struct ApplyCommand {
    plan_path: PathBuf,
    yes: bool,
    confirm_cloud_folder: bool,
    journal_export: Option<PathBuf>,
}

impl ApplyCommand {
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

#[derive(Debug)]
struct UndoCommand {
    transaction_id: String,
    yes: bool,
}

impl UndoCommand {
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

fn value_arg<'a>(args: &'a [String], index: usize, option: &'static str) -> Result<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or(CliError::MissingOptionValue { option })
}

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

fn parse_usize(value: &str, option: &'static str) -> Result<usize> {
    value.parse().map_err(|_| CliError::InvalidNumber {
        option,
        value: value.to_string(),
    })
}

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

fn print_journal_summary(journal: &TransactionJournal) {
    println!("Transaction: {}", journal.transaction_id);
    println!("Status: {:?}", journal.status);
    println!("Root: {}", journal.root.display());
    println!("Operations: {}", journal.operations.len());
}

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

PLANNED COMMANDS:
    analyze <root>              Analyze a folder and optionally write a plan
    preview <plan.json>         Preview a generated plan
    apply <plan.json>           Apply a confirmed plan
    undo <transaction-id>       Undo a transaction
    transactions <SUBCOMMAND>   List, inspect, or clean transaction journals

OPTIONS:
    -h, --help                  Print help
    -V, --version               Print version

ANALYZE OPTIONS:
    --output <plan.json>        Write the generated plan as JSON
    --profile <rules.toml>      Use a TOML custom rule profile
    --mode <mode>               Built-in mode: type, date, extension, type-year, type-date
    --max-depth <n>             Limit recursive scan depth
    --current-folder-only       Do not recurse into subfolders
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

TRANSACTION SUBCOMMANDS:
    transactions list
    transactions inspect <transaction-id>
    transactions cleanup [--include-incomplete]
",
        smartfolder_core::version()
    );
}

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

    #[error(
        "invalid mode '{mode}'; expected type, date, extension, type-year, type-date, or type-year-month-day"
    )]
    InvalidMode { mode: String },

    #[error("invalid number for {option}: {value}")]
    InvalidNumber { option: &'static str, value: String },

    #[error("confirmation declined")]
    ConfirmationDeclined,

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
    const fn exit_code(&self) -> u8 {
        match self {
            Self::UnknownCommand { .. }
            | Self::UnknownOption { .. }
            | Self::MissingArgument { .. }
            | Self::MissingOptionValue { .. }
            | Self::InvalidMode { .. }
            | Self::InvalidNumber { .. }
            | Self::ConfirmationDeclined
            | Self::CloudFolderRequiresConfirmation { .. } => 2,
            Self::Io(_) | Self::Core(_) | Self::Json(_) | Self::SignalHandler(_) => 1,
        }
    }
}
