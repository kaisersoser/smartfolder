use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::model::{
    BuiltInMode, ConflictState, OperationType, PlanMode, PlanOperation, PlanRecord, PlanSummary,
    PlanWarning, PlanWarningCode, SourceSnapshot,
};
use crate::paths::safe_destination_path;
use crate::rules::{builtin_rule_match, RuleMatch, RuleProfile};
use crate::scanner::ScanResult;
use crate::Result;

#[derive(Debug, Clone)]
pub struct PlanOptions {
    pub mode: PlanningMode,
    pub plan_id: String,
    pub created_at: DateTime<Utc>,
}

impl PlanOptions {
    pub fn built_in(
        mode: BuiltInMode,
        plan_id: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            mode: PlanningMode::BuiltIn(mode),
            plan_id: plan_id.into(),
            created_at,
        }
    }

    pub fn rule_profile(
        profile: RuleProfile,
        plan_id: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            mode: PlanningMode::RuleProfile(profile),
            plan_id: plan_id.into(),
            created_at,
        }
    }
}

#[derive(Debug, Clone)]
pub enum PlanningMode {
    BuiltIn(BuiltInMode),
    RuleProfile(RuleProfile),
}

pub fn generate_plan(
    root: impl AsRef<Path>,
    scan: &ScanResult,
    options: &PlanOptions,
) -> Result<PlanRecord> {
    let root = root.as_ref();
    let mut plan = PlanRecord::new(
        options.plan_id.clone(),
        root.to_path_buf(),
        plan_mode(&options.mode),
        options.created_at,
    );
    let mut planned_destinations = HashSet::new();

    for record in &scan.records {
        let Some(rule_match) = rule_match_for_record(record, &options.mode) else {
            plan.ambiguous_files.push(record.root_relative_path.clone());
            continue;
        };

        let source = root.join(&record.root_relative_path);
        let destination = match safe_destination_path(root, &rule_match.destination) {
            Ok(destination) => destination,
            Err(error) => {
                plan.warnings.push(PlanWarning {
                    code: PlanWarningCode::SpecialFolder,
                    message: format!(
                        "skipped {} because destination is unsafe: {error}",
                        record.root_relative_path.display()
                    ),
                });
                continue;
            }
        };

        let conflict = detect_conflict(&source, &destination, &mut planned_destinations);
        let selected = matches!(conflict, ConflictState::None);

        plan.operations.push(PlanOperation {
            operation_id: format!("op_{:06}", plan.operations.len() + 1),
            operation_type: OperationType::Move,
            source,
            destination,
            reason: rule_match.reason,
            certainty: rule_match.certainty,
            conflict,
            selected,
            source_snapshot: SourceSnapshot {
                size_bytes: record.size_bytes,
                modified_at: record.modified_at,
            },
        });
    }

    plan.summary = summarize_plan(&plan, scan);
    Ok(plan)
}

pub fn render_preview(plan: &PlanRecord) -> String {
    let mut output = String::new();
    writeln!(&mut output, "Plan: {}", plan.plan_id).expect("writing to string should not fail");
    writeln!(&mut output, "Root: {}", plan.root.display())
        .expect("writing to string should not fail");
    writeln!(
        &mut output,
        "Moves: {} | Ambiguous: {} | Conflicts: {} | Skipped: {}",
        plan.summary.moves_proposed,
        plan.summary.ambiguous_files,
        plan.summary.conflicts,
        plan.summary.skipped
    )
    .expect("writing to string should not fail");
    output.push('\n');
    output.push_str("Source | Destination | Reason | Status\n");

    for operation in &plan.operations {
        writeln!(
            &mut output,
            "{} | {} | {} | {}",
            operation.source.display(),
            operation.destination.display(),
            operation.reason,
            operation_status(operation)
        )
        .expect("writing to string should not fail");
    }

    if !plan.ambiguous_files.is_empty() {
        output.push_str("\nAmbiguous files left in place:\n");
        for path in &plan.ambiguous_files {
            writeln!(&mut output, "- {}", path.display())
                .expect("writing to string should not fail");
        }
    }

    output
}

pub fn render_preview_json(plan: &PlanRecord) -> Result<String> {
    plan.to_pretty_json()
}

fn rule_match_for_record(
    record: &crate::model::FileInventoryRecord,
    mode: &PlanningMode,
) -> Option<RuleMatch> {
    match mode {
        PlanningMode::BuiltIn(mode) => builtin_rule_match(record, *mode),
        PlanningMode::RuleProfile(profile) => profile.first_match(record),
    }
}

fn plan_mode(mode: &PlanningMode) -> PlanMode {
    match mode {
        PlanningMode::BuiltIn(mode) => PlanMode::BuiltIn(*mode),
        PlanningMode::RuleProfile(profile) => PlanMode::RuleProfile {
            profile_id: profile.profile_id.clone(),
        },
    }
}

fn detect_conflict(
    source: &Path,
    destination: &Path,
    planned_destinations: &mut HashSet<PathBuf>,
) -> ConflictState {
    if is_case_only_rename(source, destination) {
        return ConflictState::CaseOnlyRename {
            path: destination.to_path_buf(),
        };
    }

    if has_legacy_windows_path_risk(destination) {
        return ConflictState::UnsafeDestination {
            reason: format!(
                "destination path is longer than the legacy Windows MAX_PATH limit: {}",
                destination.display()
            ),
        };
    }

    if destination.exists() {
        return ConflictState::DestinationExists {
            path: destination.to_path_buf(),
        };
    }

    if !planned_destinations.insert(destination.to_path_buf()) {
        return ConflictState::DestinationExists {
            path: destination.to_path_buf(),
        };
    }

    ConflictState::None
}

fn is_case_only_rename(source: &Path, destination: &Path) -> bool {
    source != destination
        && source
            .to_string_lossy()
            .eq_ignore_ascii_case(&destination.to_string_lossy())
}

#[cfg(windows)]
fn has_legacy_windows_path_risk(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().chars().count() >= 260
}

#[cfg(not(windows))]
fn has_legacy_windows_path_risk(_path: &Path) -> bool {
    false
}

fn summarize_plan(plan: &PlanRecord, scan: &ScanResult) -> PlanSummary {
    let conflicts = plan
        .operations
        .iter()
        .filter(|operation| !matches!(operation.conflict, ConflictState::None))
        .count();
    let skipped = plan
        .operations
        .iter()
        .filter(|operation| !operation.selected)
        .count();

    PlanSummary {
        files_scanned: scan.summary.records_collected,
        moves_proposed: plan.operations.len(),
        ambiguous_files: plan.ambiguous_files.len(),
        conflicts,
        skipped,
    }
}

fn operation_status(operation: &PlanOperation) -> &'static str {
    match operation.conflict {
        ConflictState::None => "selected",
        ConflictState::DestinationExists { .. } => "conflict: destination exists",
        ConflictState::CaseOnlyRename { .. } => "conflict: case-only rename",
        ConflictState::UnsafeDestination { .. } => "skipped: unsafe destination",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    use crate::model::{BuiltInMode, ConflictState};
    use crate::planner::{generate_plan, render_preview, render_preview_json, PlanOptions};
    use crate::rules::RuleProfile;
    use crate::scanner::{scan_folder, ScanOptions};

    fn path(parts: &[&str]) -> PathBuf {
        let mut path = PathBuf::new();
        for part in parts {
            path.push(part);
        }
        path
    }

    #[test]
    fn generated_builtin_plan_keeps_destinations_inside_root() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");

        let scan = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");
        let plan = generate_plan(
            fixture.path(),
            &scan,
            &PlanOptions::built_in(BuiltInMode::Type, "plan_test", test_time()),
        )
        .expect("plan succeeds");

        assert_eq!(plan.operations.len(), 1);
        assert!(plan.operations[0].destination.starts_with(fixture.path()));
        assert_eq!(
            plan.operations[0].destination,
            fixture.path().join(path(&["Documents", "report.pdf"]))
        );
    }

    #[test]
    fn generated_profile_plan_reports_ambiguous_files() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        fs::write(fixture.path().join("photo.jpg"), b"image").expect("fixture write");
        let profile = RuleProfile::from_toml(
            r#"
profile_id = "documents"

[[rules]]
name = "PDFs"
destination = "Documents"
extensions = ["pdf"]
"#,
        )
        .expect("profile parses");

        let scan = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");
        let plan = generate_plan(
            fixture.path(),
            &scan,
            &PlanOptions::rule_profile(profile, "plan_test", test_time()),
        )
        .expect("plan succeeds");

        assert_eq!(plan.operations.len(), 1);
        assert_eq!(plan.ambiguous_files, vec![PathBuf::from("photo.jpg")]);
    }

    #[test]
    fn destination_conflicts_are_marked_unselected() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        fs::create_dir(fixture.path().join("Documents")).expect("fixture dir");
        fs::write(
            fixture.path().join("Documents").join("report.pdf"),
            b"existing",
        )
        .expect("fixture write");

        let scan = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");
        let plan = generate_plan(
            fixture.path(),
            &scan,
            &PlanOptions::built_in(BuiltInMode::Type, "plan_test", test_time()),
        )
        .expect("plan succeeds");

        let operation = plan
            .operations
            .iter()
            .find(|operation| operation.source.ends_with("report.pdf"))
            .expect("operation exists");
        assert!(matches!(
            operation.conflict,
            ConflictState::DestinationExists { .. }
        ));
        assert!(!operation.selected);
    }

    #[test]
    fn preview_renders_human_and_json_output() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let scan = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");
        let plan = generate_plan(
            fixture.path(),
            &scan,
            &PlanOptions::built_in(BuiltInMode::Type, "plan_test", test_time()),
        )
        .expect("plan succeeds");

        assert!(render_preview(&plan).contains("report.pdf"));
        assert!(render_preview_json(&plan)
            .expect("json preview")
            .contains("\"schema_version\""));
    }

    #[test]
    fn case_only_renames_are_marked_as_conflicts() {
        let mut planned_destinations = std::collections::HashSet::new();
        let conflict = super::detect_conflict(
            &PathBuf::from("C:\\root\\Report.pdf"),
            &PathBuf::from("C:\\root\\report.pdf"),
            &mut planned_destinations,
        );

        assert!(matches!(conflict, ConflictState::CaseOnlyRename { .. }));
    }

    #[cfg(windows)]
    #[test]
    fn legacy_windows_long_paths_are_marked_unsafe() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let long_folder = "a".repeat(260);
        let profile = RuleProfile::from_toml(&format!(
            r#"
profile_id = "long-path"

[[rules]]
name = "Long"
destination = "{long_folder}"
extensions = ["pdf"]
"#
        ))
        .expect("profile parses");
        let scan = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");
        let plan = generate_plan(
            fixture.path(),
            &scan,
            &PlanOptions::rule_profile(profile, "plan_test", test_time()),
        )
        .expect("plan succeeds");

        assert!(matches!(
            plan.operations[0].conflict,
            ConflictState::UnsafeDestination { .. }
        ));
        assert!(!plan.operations[0].selected);
    }

    fn fixture_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn test_time() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap()
    }
}
