use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use chrono::{DateTime, Datelike, Utc};
use smartfolder_core::model::{
    BuiltInMode, PlanMode, PlanRecord, TransactionJournal, TransactionStatus,
};
use tempfile::TempDir;

struct TestContext {
    temp: TempDir,
    app_data_root: PathBuf,
    root: PathBuf,
}

impl TestContext {
    fn new(root_name: &str) -> Self {
        let temp = TempDir::new().expect("temp dir should be created");
        let app_data_root = temp.path().join("appdata");
        let root = temp.path().join(root_name);
        fs::create_dir_all(&app_data_root).expect("app data dir should exist");
        fs::create_dir_all(&root).expect("root dir should exist");
        Self {
            temp,
            app_data_root,
            root,
        }
    }

    fn plan_path(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }
}

fn smartfolder(app_data_root: &Path) -> Command {
    let mut command = Command::cargo_bin("smartfolder").expect("binary should build");
    command
        .env("SMARTFOLDER_DATA_DIR", app_data_root)
        .env("LOCALAPPDATA", app_data_root)
        .env("APPDATA", app_data_root)
        .env("XDG_DATA_HOME", app_data_root)
        .env("HOME", app_data_root);
    command
}

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir should exist");
    }
    fs::write(path, contents).expect("file should be written");
}

fn read_plan(path: &Path) -> PlanRecord {
    serde_json::from_str(&fs::read_to_string(path).expect("plan file should exist"))
        .expect("plan json should parse")
}

fn source_names(plan: &PlanRecord) -> Vec<String> {
    plan.operations
        .iter()
        .filter_map(|operation| operation.source.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

fn operation_for<'a>(
    plan: &'a PlanRecord,
    file_name: &str,
) -> &'a smartfolder_core::model::PlanOperation {
    plan.operations
        .iter()
        .find(|operation| {
            operation
                .source
                .file_name()
                .is_some_and(|name| name.to_string_lossy() == file_name)
        })
        .expect("operation should exist")
}

fn relative_destination(root: &Path, plan: &PlanRecord, file_name: &str) -> PathBuf {
    operation_for(plan, file_name)
        .destination
        .strip_prefix(root)
        .expect("destination should remain inside root")
        .to_path_buf()
}

fn read_journal(path: &Path) -> TransactionJournal {
    serde_json::from_str(&fs::read_to_string(path).expect("journal file should exist"))
        .expect("journal json should parse")
}

fn stdout_text(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

fn stderr_text(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
}

fn extract_line_value(output: &str, prefix: &str) -> String {
    let start = output.find(prefix).expect("prefix should exist") + prefix.len();
    output[start..]
        .lines()
        .next()
        .map(str::trim)
        .map(ToString::to_string)
        .expect("line should exist")
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "unknown-month",
    }
}

fn expected_date_parts(path: &Path) -> (String, String, String) {
    let modified: DateTime<Utc> = fs::metadata(path)
        .expect("metadata should be available")
        .modified()
        .expect("modified time should be available")
        .into();
    (
        modified.year().to_string(),
        month_name(modified.month()).to_string(),
        format!("{:02}", modified.day()),
    )
}

#[test]
fn top_level_help_and_version_are_available() {
    let context = TestContext::new("root");

    let help = smartfolder(&context.app_data_root)
        .arg("--help")
        .assert()
        .success();
    let help_stdout = stdout_text(&help);
    assert!(help_stdout.contains("ANALYZE OPTIONS"));
    assert!(help_stdout.contains("TRANSACTION SUBCOMMANDS"));
    assert!(help_stdout.contains("PROFILE SUBCOMMANDS"));

    let version = smartfolder(&context.app_data_root)
        .arg("--version")
        .assert()
        .success();
    let version_stdout = stdout_text(&version);
    assert!(version_stdout.starts_with("smartfolder "));
}

#[test]
fn analyze_respects_hidden_project_and_custom_exclusions() {
    let context = TestContext::new("root");
    write_file(&context.root.join("report.pdf"), b"report");
    write_file(&context.root.join(".hidden.txt"), b"hidden");
    write_file(
        &context.root.join("node_modules").join("lib.js"),
        b"console.log('hi');",
    );
    write_file(&context.root.join("skipme").join("skip.pdf"), b"skip");

    let default_plan_path = context.plan_path("default-plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--include-subfolders",
            "--output",
            default_plan_path
                .to_str()
                .expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();
    let default_plan = read_plan(&default_plan_path);
    let default_names = source_names(&default_plan);
    assert!(default_names.contains(&"report.pdf".to_string()));
    assert!(!default_names.contains(&".hidden.txt".to_string()));
    assert!(!default_names.contains(&"lib.js".to_string()));
    assert!(default_names.contains(&"skip.pdf".to_string()));

    let include_plan_path = context.plan_path("include-plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--include-subfolders",
            "--output",
            include_plan_path
                .to_str()
                .expect("plan path should be utf-8"),
            "--include-hidden",
            "--include-system",
            "--include-project-folders",
            "--exclude",
            "skipme",
            "--quiet",
        ])
        .assert()
        .success();
    let include_plan = read_plan(&include_plan_path);
    let include_names = source_names(&include_plan);
    assert!(include_names.contains(&"report.pdf".to_string()));
    assert!(include_names.contains(&".hidden.txt".to_string()));
    assert!(include_names.contains(&"lib.js".to_string()));
    assert!(!include_names.contains(&"skip.pdf".to_string()));
}

#[test]
fn analyze_defaults_to_current_folder_and_can_include_subfolders_with_depth() {
    let context = TestContext::new("root");
    write_file(&context.root.join("top.txt"), b"top");
    write_file(&context.root.join("level1").join("one.txt"), b"one");
    write_file(
        &context.root.join("level1").join("level2").join("two.txt"),
        b"two",
    );

    let current_plan_path = context.plan_path("current-plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--output",
            current_plan_path
                .to_str()
                .expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();
    let current_plan = read_plan(&current_plan_path);
    assert_eq!(source_names(&current_plan), vec!["top.txt".to_string()]);

    let depth_plan_path = context.plan_path("depth-plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--include-subfolders",
            "--max-depth",
            "2",
            "--output",
            depth_plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();
    let depth_plan = read_plan(&depth_plan_path);
    let depth_names = source_names(&depth_plan);
    assert!(depth_names.contains(&"top.txt".to_string()));
    assert!(depth_names.contains(&"one.txt".to_string()));
    assert!(!depth_names.contains(&"two.txt".to_string()));
}

#[test]
fn analyze_covers_builtin_modes_json_and_quiet_output() {
    let context = TestContext::new("root");
    let report = context.root.join("report.pdf");
    write_file(&report, b"report");
    let (year, month, day) = expected_date_parts(&report);

    let extension_plan_path = context.plan_path("extension-plan.json");
    let extension = smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--mode",
            "extension",
            "--output",
            extension_plan_path
                .to_str()
                .expect("plan path should be utf-8"),
            "--json",
        ])
        .assert()
        .success();
    let extension_stdout = stdout_text(&extension);
    let extension_stderr = stderr_text(&extension);
    assert!(extension_stdout.contains("\"schema_version\""));
    assert!(extension_stderr.contains("Scanned"));
    let extension_plan: PlanRecord =
        serde_json::from_str(&extension_stdout).expect("stdout should contain plan json");
    assert_eq!(
        extension_plan.mode,
        PlanMode::BuiltIn(BuiltInMode::Extension)
    );
    assert_eq!(
        relative_destination(&context.root, &extension_plan, "report.pdf"),
        PathBuf::from("pdf").join("report.pdf")
    );

    let date_plan_path = context.plan_path("date-plan.json");
    let date = smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--mode",
            "date",
            "--output",
            date_plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();
    assert!(stdout_text(&date).is_empty());
    assert!(stderr_text(&date).is_empty());
    let date_plan = read_plan(&date_plan_path);
    assert_eq!(
        relative_destination(&context.root, &date_plan, "report.pdf"),
        PathBuf::from(&year)
            .join(&month)
            .join(&day)
            .join("report.pdf")
    );

    let type_year_plan_path = context.plan_path("type-year-plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--mode",
            "type-year-month-day",
            "--output",
            type_year_plan_path
                .to_str()
                .expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();
    let type_year_plan = read_plan(&type_year_plan_path);
    assert_eq!(
        relative_destination(&context.root, &type_year_plan, "report.pdf"),
        PathBuf::from("Documents")
            .join(&year)
            .join(&month)
            .join(&day)
            .join("report.pdf")
    );
}

#[test]
fn analyze_supports_custom_rule_profiles_with_placeholders() {
    let context = TestContext::new("root");
    let report = context.root.join("report.pdf");
    write_file(&report, b"report");
    let rules_path = context.plan_path("rules.toml");
    fs::write(
        &rules_path,
        r#"
profile_id = "docs"

[[rules]]
name = "pdf reports"
destination = "ByRule/{type}/{year}/{month}/{day}/{extension}"
extensions = ["pdf"]
"#,
    )
    .expect("rules file should be written");
    let (year, month, day) = expected_date_parts(&report);

    let plan_path = context.plan_path("profile-plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--profile",
            rules_path.to_str().expect("rules path should be utf-8"),
            "--output",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();

    let plan = read_plan(&plan_path);
    assert_eq!(
        plan.mode,
        PlanMode::RuleProfile {
            profile_id: "docs".to_string()
        }
    );
    assert_eq!(
        relative_destination(&context.root, &plan, "report.pdf"),
        PathBuf::from("ByRule")
            .join("Documents")
            .join(&year)
            .join(&month)
            .join(&day)
            .join("pdf")
            .join("report.pdf")
    );
}

#[test]
fn cli_profiles_manage_saved_gui_profiles() {
    let context = TestContext::new("root");
    let report = context.root.join("invoice.pdf");
    write_file(&report, b"invoice");
    let rules_path = context.plan_path("saved-rules.toml");
    fs::write(
        &rules_path,
        r#"
profile_id = "invoices"

[[rules]]
name = "invoice pdfs"
destination = "Invoices/{year}/{month}"
extensions = ["pdf"]
filename_contains = ["invoice"]
"#,
    )
    .expect("rules file should be written");

    smartfolder(&context.app_data_root)
        .args([
            "profiles",
            "validate",
            rules_path.to_str().expect("utf-8 path"),
        ])
        .assert()
        .success();

    smartfolder(&context.app_data_root)
        .args([
            "profiles",
            "import",
            rules_path.to_str().expect("utf-8 path"),
        ])
        .assert()
        .success();

    let list = smartfolder(&context.app_data_root)
        .args(["profiles", "list"])
        .assert()
        .success();
    assert!(stdout_text(&list).contains("invoices"));

    let inspect = smartfolder(&context.app_data_root)
        .args(["profiles", "inspect", "invoices"])
        .assert()
        .success();
    assert!(stdout_text(&inspect).contains("invoice pdfs"));

    let plan_path = context.plan_path("saved-profile-plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--profile-id",
            "invoices",
            "--output",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();

    let (year, month, _) = expected_date_parts(&report);
    let plan = read_plan(&plan_path);
    assert_eq!(
        relative_destination(&context.root, &plan, "invoice.pdf"),
        PathBuf::from("Invoices")
            .join(&year)
            .join(&month)
            .join("invoice.pdf")
    );
}

#[test]
fn preview_supports_human_and_json_output() {
    let context = TestContext::new("root");
    write_file(&context.root.join("report.pdf"), b"report");
    let plan_path = context.plan_path("plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--output",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();

    let preview = smartfolder(&context.app_data_root)
        .args([
            "preview",
            plan_path.to_str().expect("plan path should be utf-8"),
        ])
        .assert()
        .success();
    let preview_stdout = stdout_text(&preview);
    assert!(preview_stdout.contains("Source | Destination | Reason | Status"));
    assert!(preview_stdout.contains("report.pdf"));

    let preview_json = smartfolder(&context.app_data_root)
        .args([
            "preview",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--json",
        ])
        .assert()
        .success();
    let preview_json_stdout = stdout_text(&preview_json);
    let parsed_plan: PlanRecord =
        serde_json::from_str(&preview_json_stdout).expect("preview json should parse");
    assert_eq!(parsed_plan.operations.len(), 1);
}

#[test]
fn apply_transactions_and_undo_cover_end_to_end_mvp_flow() {
    let context = TestContext::new("root");
    write_file(&context.root.join("report.pdf"), b"report");
    let plan_path = context.plan_path("plan.json");
    let journal_export = context.plan_path("journal-export.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--mode",
            "type-year",
            "--output",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();
    let plan = read_plan(&plan_path);
    let destination = operation_for(&plan, "report.pdf").destination.clone();

    let apply = smartfolder(&context.app_data_root)
        .args([
            "apply",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--yes",
            "--journal-export",
            journal_export
                .to_str()
                .expect("journal export path should be utf-8"),
        ])
        .assert()
        .success();
    let apply_stdout = stdout_text(&apply);
    let transaction_id = extract_line_value(&apply_stdout, "Transaction: ");
    let journal_path = PathBuf::from(extract_line_value(&apply_stdout, "Journal: "));

    assert!(!context.root.join("report.pdf").exists());
    assert!(destination.exists());
    assert!(journal_export.exists());
    let exported_journal = read_journal(&journal_export);
    assert_eq!(exported_journal.status, TransactionStatus::Completed);

    let transactions = smartfolder(&context.app_data_root)
        .args(["transactions", "list"])
        .assert()
        .success();
    assert!(stdout_text(&transactions).contains(&transaction_id));

    let inspect = smartfolder(&context.app_data_root)
        .args(["transactions", "inspect", &transaction_id])
        .assert()
        .success();
    let inspected: TransactionJournal =
        serde_json::from_str(&stdout_text(&inspect)).expect("transaction json should parse");
    assert_eq!(inspected.status, TransactionStatus::Completed);

    let undo = smartfolder(&context.app_data_root)
        .args(["undo", &transaction_id, "--yes"])
        .assert()
        .success();
    let undo_stdout = stdout_text(&undo);
    assert!(undo_stdout.contains("Rolled back: 1 | Skipped: 0 | Failed: 0"));
    assert!(context.root.join("report.pdf").exists());
    assert!(!destination.exists());

    let cleanup = smartfolder(&context.app_data_root)
        .args(["transactions", "cleanup"])
        .assert()
        .success();
    assert!(stdout_text(&cleanup).contains("Removed: 1 | Kept incomplete: 0"));
    assert!(!journal_path.exists());
}

#[test]
fn apply_requires_explicit_cloud_confirmation_in_noninteractive_mode() {
    let context = TestContext::new("OneDrive\\Documents");
    write_file(&context.root.join("report.pdf"), b"report");
    let plan_path = context.plan_path("plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--output",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();

    let apply = smartfolder(&context.app_data_root)
        .args([
            "apply",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--yes",
        ])
        .assert()
        .failure()
        .code(2);
    let apply_stderr = stderr_text(&apply);
    assert!(apply_stderr.contains("cloud-synced folder requires explicit confirmation"));
    assert!(context.root.join("report.pdf").exists());
}

#[test]
fn interactive_apply_and_undo_work_for_cloud_folders() {
    let context = TestContext::new("OneDrive\\Documents");
    write_file(&context.root.join("report.pdf"), b"report");
    let plan_path = context.plan_path("plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--output",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();
    let plan = read_plan(&plan_path);
    let destination = operation_for(&plan, "report.pdf").destination.clone();

    let apply = smartfolder(&context.app_data_root)
        .args([
            "apply",
            plan_path.to_str().expect("plan path should be utf-8"),
        ])
        .write_stdin("yes\nyes\n")
        .assert()
        .success();
    let transaction_id = extract_line_value(&stdout_text(&apply), "Transaction: ");
    assert!(destination.exists());

    let undo = smartfolder(&context.app_data_root)
        .args(["undo", &transaction_id])
        .write_stdin("yes\n")
        .assert()
        .success();
    assert!(stdout_text(&undo).contains("Rolled back: 1 | Skipped: 0 | Failed: 0"));
    assert!(context.root.join("report.pdf").exists());
    assert!(!destination.exists());
}

#[test]
fn cleanup_can_optionally_remove_incomplete_transactions() {
    let context = TestContext::new("root");
    write_file(&context.root.join("report.pdf"), b"report");
    let plan_path = context.plan_path("plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--output",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();

    let apply = smartfolder(&context.app_data_root)
        .args([
            "apply",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--yes",
        ])
        .assert()
        .success();
    let apply_stdout = stdout_text(&apply);
    let journal_path = PathBuf::from(extract_line_value(&apply_stdout, "Journal: "));

    let mut incomplete = read_journal(&journal_path);
    incomplete.transaction_id = "txn_manual_incomplete".to_string();
    incomplete.status = TransactionStatus::InProgress;
    incomplete.completed_at = None;
    let incomplete_path = journal_path
        .parent()
        .expect("journal should have parent")
        .join("txn_manual_incomplete.json");
    fs::write(
        &incomplete_path,
        serde_json::to_string_pretty(&incomplete).expect("journal json should serialize"),
    )
    .expect("incomplete journal should be written");

    let cleanup = smartfolder(&context.app_data_root)
        .args(["transactions", "cleanup"])
        .assert()
        .success();
    assert!(stdout_text(&cleanup).contains("Removed: 1 | Kept incomplete: 1"));
    assert!(!journal_path.exists());
    assert!(incomplete_path.exists());

    let cleanup_all = smartfolder(&context.app_data_root)
        .args(["transactions", "cleanup", "--include-incomplete"])
        .assert()
        .success();
    assert!(stdout_text(&cleanup_all).contains("Removed: 1 | Kept incomplete: 0"));
    assert!(!incomplete_path.exists());
}

#[test]
fn resume_continues_interrupted_transactions_from_cli() {
    let context = TestContext::new("root");
    write_file(&context.root.join("report.pdf"), b"report");
    let plan_path = context.plan_path("plan.json");
    smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--output",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--quiet",
        ])
        .assert()
        .success();

    let apply = smartfolder(&context.app_data_root)
        .args([
            "apply",
            plan_path.to_str().expect("plan path should be utf-8"),
            "--yes",
        ])
        .assert()
        .success();
    let apply_stdout = stdout_text(&apply);
    let transaction_id = extract_line_value(&apply_stdout, "Transaction: ");
    let journal_path = PathBuf::from(extract_line_value(&apply_stdout, "Journal: "));

    let mut interrupted = read_journal(&journal_path);
    let operation = interrupted.operations[0].clone();
    fs::rename(&operation.destination, &operation.source).expect("reset source path");
    interrupted.status = TransactionStatus::Interrupted;
    interrupted.completed_at = None;
    interrupted.operations[0].status = smartfolder_core::model::OperationStatus::Pending;
    interrupted.operations[0].error = None;
    fs::write(
        &journal_path,
        serde_json::to_string_pretty(&interrupted).expect("journal should serialize"),
    )
    .expect("interrupted journal should be written");

    let resume = smartfolder(&context.app_data_root)
        .args(["resume", &transaction_id, "--yes"])
        .assert()
        .success();
    let resume_stdout = stdout_text(&resume);
    assert!(resume_stdout.contains("Resumed: 1 | Completed: 1 | Skipped: 0 | Failed: 0"));
    assert!(operation.destination.exists());
    assert!(!operation.source.exists());

    let resumed = read_journal(&journal_path);
    assert_eq!(resumed.status, TransactionStatus::Completed);
}

#[test]
fn invalid_cli_inputs_return_documented_error_shapes() {
    let context = TestContext::new("root");
    write_file(&context.root.join("report.pdf"), b"report");

    let invalid_mode = smartfolder(&context.app_data_root)
        .args([
            "analyze",
            context.root.to_str().expect("root path should be utf-8"),
            "--mode",
            "bogus",
            "--json",
        ])
        .assert()
        .failure()
        .code(2);
    let invalid_mode_stderr = stderr_text(&invalid_mode);
    let invalid_mode_json: serde_json::Value =
        serde_json::from_str(&invalid_mode_stderr).expect("json error should parse");
    assert_eq!(invalid_mode_json["error"]["exit_code"], 2);
    assert!(invalid_mode_json["error"]["message"]
        .as_str()
        .expect("message should be present")
        .contains("invalid mode"));

    smartfolder(&context.app_data_root)
        .args(["preview"])
        .assert()
        .failure()
        .code(2);

    smartfolder(&context.app_data_root)
        .args(["transactions", "cleanup", "--bogus"])
        .assert()
        .failure()
        .code(2);
}
