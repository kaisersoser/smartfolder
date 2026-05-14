//! Plan generation for file organization.
//!
//! This module takes a scan result and generates a concrete plan of file moves.
//! It applies built-in organization modes (type, date, extension) or custom rule profiles
//! to determine destination folders for each file.
//!
//! # Key features
//!
//! - **Conflict detection**: Identifies files that would overwrite each other
//! - **Certainty levels**: Marks operations with High/Medium/Low confidence
//! - **Selection**: Marks conflicts as "not selected" so they won't be applied
//! - **Warnings**: Tracks files that couldn't be classified or have unsafe destinations
//!
//! # Workflow
//!
//! 1. Create [`PlanOptions`] with either built-in mode or rule profile
//! 2. Call [`generate_plan`] with scan result
//! 3. Review results with [`render_preview`] or [`render_preview_json`]
//! 4. Pass plan to apply module for execution

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::model::{
    BuiltInMode, ConflictState, FileInventoryRecord, OperationType, PlanMode, PlanOperation,
    PlanRecord, PlanSummary, PlanWarning, PlanWarningCode, SourceSnapshot, UntouchedReason,
    UntouchedRecord,
};
use crate::paths::safe_destination_path;
use crate::rules::{builtin_rule_match, RuleMatch, RuleProfile};
use crate::scanner::{CancellationToken, ScanResult};
use crate::session_store::SqliteSessionStore;
use crate::{Result, SmartfolderError};

const DEFAULT_STORE_PAGE_SIZE: usize = 1_000;
const PLAN_PROGRESS_INTERVAL: usize = 250;

/// Options for generating a plan.
///
/// Specifies the organization mode (built-in or rule profile) and metadata (ID, creation time).
#[derive(Debug, Clone)]
pub struct PlanOptions {
    pub mode: PlanningMode,
    pub plan_id: String,
    pub created_at: DateTime<Utc>,
}

impl PlanOptions {
    /// Create options for a built-in organization mode.
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

    /// Create options for a custom rule profile.
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

/// Organization mode used during planning (built-in or custom rules).
#[derive(Debug, Clone)]
pub enum PlanningMode {
    BuiltIn(BuiltInMode),
    RuleProfile(RuleProfile),
}

/// Result of generating a plan into persistent session storage.
///
/// Contains the metadata and aggregate counters needed by UI callers. Individual
/// operations are stored in [`SqliteSessionStore`] and should be queried by page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPlanResult {
    pub plan_id: String,
    pub root: PathBuf,
    pub mode: PlanMode,
    pub created_at: DateTime<Utc>,
    pub summary: PlanSummary,
}

/// Live progress snapshot emitted while generating a stored plan.
///
/// Unlike directory scanning, stored plan generation knows the number of scan
/// records up front, so UI callers can render a determinate progress bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanGenerationProgress {
    pub processed_records: usize,
    pub total_records: usize,
    pub operations_created: usize,
    pub ambiguous_files: usize,
    pub conflicts: usize,
    pub skipped: usize,
    pub current_path: Option<PathBuf>,
}

/// Generate a plan from a scan result and planning options.
///
/// # Logical Flow
///
/// 1. For each file in the scan:
///    - Apply rule matching (built-in or profile) to determine destination
///    - Check if destination path is safe (doesn't escape root)
/// 2. Detect conflicts:
///    - Destination already exists
///    - Case-only rename on case-insensitive filesystem
///    - Path exceeds Windows `MAX_PATH` limit
/// 3. Mark conflicting operations as "not selected"
/// 4. Generate summary statistics
/// 5. Return complete plan ready for review and execution
///
/// # Errors
///
/// Returns error for IO failures or invalid paths.
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
            plan.untouched_records.push(untouched_record(
                record,
                UntouchedReason::NoMatchingRule,
                "No organization rule matched this file.",
            ));
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
                plan.untouched_records.push(untouched_record(
                    record,
                    UntouchedReason::UnsafeDestination,
                    format!("Destination was unsafe: {error}"),
                ));
                continue;
            }
        };

        if let Some(detail) = already_organized_detail(
            record,
            &options.mode,
            &source,
            &destination,
            &rule_match.destination,
        ) {
            plan.untouched_records.push(untouched_record(
                record,
                UntouchedReason::AlreadyOrganized,
                detail,
            ));
            continue;
        }

        let conflict = detect_conflict(&source, &destination, &mut planned_destinations);
        let selected = matches!(conflict, ConflictState::None);
        if !selected {
            plan.untouched_records.push(untouched_record(
                record,
                untouched_reason_for_conflict(&conflict),
                untouched_detail_for_conflict(&conflict),
            ));
        }

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

/// Generate a plan from records stored in SQLite and write operations back to SQLite.
///
/// # Logical Flow
///
/// 1. Clear any existing plan rows for the session
/// 2. Read scan records in bounded pages
/// 3. Apply the same rule matching and safety checks as [`generate_plan`]
/// 4. Use an indexed destination key in SQLite to detect duplicate destinations
/// 5. Persist each operation, ambiguous file, warning, and final summary
/// 6. Return aggregate plan metadata for UI display
///
/// # Errors
///
/// Returns error for storage failures, JSON decoding failures, or invalid paths.
pub fn generate_plan_to_store(
    root: impl AsRef<Path>,
    store: &mut SqliteSessionStore,
    session_id: &str,
    options: &PlanOptions,
    page_size: usize,
) -> Result<StoredPlanResult> {
    let mut ignore_progress = |_| {};
    generate_plan_to_store_with_progress(
        root,
        store,
        session_id,
        options,
        page_size,
        &mut ignore_progress,
    )
}

/// Generate a stored plan and report live progress snapshots.
///
/// Progress callbacks are emitted at the beginning, periodically as records are
/// processed, and once at completion. The callback is intended for UI updates and
/// should avoid long-running work.
///
/// # Errors
///
/// Returns error for storage failures, JSON decoding failures, or invalid paths.
pub fn generate_plan_to_store_with_progress(
    root: impl AsRef<Path>,
    store: &mut SqliteSessionStore,
    session_id: &str,
    options: &PlanOptions,
    page_size: usize,
    progress: &mut impl FnMut(PlanGenerationProgress),
) -> Result<StoredPlanResult> {
    let cancellation = CancellationToken::default();
    generate_plan_to_store_with_progress_and_cancellation(
        root,
        store,
        session_id,
        options,
        page_size,
        &cancellation,
        progress,
    )
}

/// Generate a stored plan, report progress, and honor cancellation requests.
///
/// # Errors
///
/// Returns error for storage failures, JSON decoding failures, invalid paths, or
/// cancellation.
pub fn generate_plan_to_store_with_progress_and_cancellation(
    root: impl AsRef<Path>,
    store: &mut SqliteSessionStore,
    session_id: &str,
    options: &PlanOptions,
    page_size: usize,
    cancellation: &CancellationToken,
    progress: &mut impl FnMut(PlanGenerationProgress),
) -> Result<StoredPlanResult> {
    let root = root.as_ref();
    let mode = plan_mode(&options.mode);
    let page_size = page_size.max(1);
    let mut offset = 0;
    let mut processed_records = 0;
    let mut operation_count = 0;
    let mut ambiguous_count = 0;
    let mut conflict_count = 0;
    let mut skipped_count = 0;
    let total_records = store
        .scan_summary(session_id)?
        .map_or(0, |summary| summary.records_collected);

    store.clear_plan(session_id)?;
    store.begin_write_batch()?;
    let generation_result = (|| {
        progress(PlanGenerationProgress {
            processed_records,
            total_records,
            operations_created: operation_count,
            ambiguous_files: ambiguous_count,
            conflicts: conflict_count,
            skipped: skipped_count,
            current_path: None,
        });

        loop {
            if cancellation.is_cancelled() {
                return Err(SmartfolderError::ScanCancelled);
            }

            let records = store.scan_records_page(session_id, offset, page_size)?;
            if records.is_empty() {
                break;
            }

            for record in &records {
                if cancellation.is_cancelled() {
                    return Err(SmartfolderError::ScanCancelled);
                }

                processed_records += 1;
                let Some(rule_match) = rule_match_for_record(record, &options.mode) else {
                    ambiguous_count += 1;
                    store.insert_ambiguous_file(session_id, &record.root_relative_path)?;
                    store.insert_untouched_record(
                        session_id,
                        &untouched_record(
                            record,
                            UntouchedReason::NoMatchingRule,
                            "No organization rule matched this file.",
                        ),
                    )?;
                    maybe_emit_plan_progress(
                        progress,
                        PlanGenerationProgress {
                            processed_records,
                            total_records,
                            operations_created: operation_count,
                            ambiguous_files: ambiguous_count,
                            conflicts: conflict_count,
                            skipped: skipped_count,
                            current_path: Some(record.root_relative_path.clone()),
                        },
                    );
                    continue;
                };

                let source = root.join(&record.root_relative_path);
                let destination = match safe_destination_path(root, &rule_match.destination) {
                    Ok(destination) => destination,
                    Err(error) => {
                        store.insert_plan_warning(
                            session_id,
                            &PlanWarning {
                                code: PlanWarningCode::SpecialFolder,
                                message: format!(
                                    "skipped {} because destination is unsafe: {error}",
                                    record.root_relative_path.display()
                                ),
                            },
                        )?;
                        store.insert_untouched_record(
                            session_id,
                            &untouched_record(
                                record,
                                UntouchedReason::UnsafeDestination,
                                format!("Destination was unsafe: {error}"),
                            ),
                        )?;
                        maybe_emit_plan_progress(
                            progress,
                            PlanGenerationProgress {
                                processed_records,
                                total_records,
                                operations_created: operation_count,
                                ambiguous_files: ambiguous_count,
                                conflicts: conflict_count,
                                skipped: skipped_count,
                                current_path: Some(record.root_relative_path.clone()),
                            },
                        );
                        continue;
                    }
                };

                if let Some(detail) = already_organized_detail(
                    record,
                    &options.mode,
                    &source,
                    &destination,
                    &rule_match.destination,
                ) {
                    store.insert_untouched_record(
                        session_id,
                        &untouched_record(record, UntouchedReason::AlreadyOrganized, detail),
                    )?;
                    maybe_emit_plan_progress(
                        progress,
                        PlanGenerationProgress {
                            processed_records,
                            total_records,
                            operations_created: operation_count,
                            ambiguous_files: ambiguous_count,
                            conflicts: conflict_count,
                            skipped: skipped_count,
                            current_path: Some(record.root_relative_path.clone()),
                        },
                    );
                    continue;
                }

                let destination_key = normalized_destination_key(&destination);
                let already_planned = store.destination_key_exists(session_id, &destination_key)?;
                let conflict = detect_conflict_from_store(&source, &destination, already_planned);
                let selected = matches!(conflict, ConflictState::None);
                if !selected {
                    conflict_count += 1;
                    skipped_count += 1;
                    store.insert_untouched_record(
                        session_id,
                        &untouched_record(
                            record,
                            untouched_reason_for_conflict(&conflict),
                            untouched_detail_for_conflict(&conflict),
                        ),
                    )?;
                }

                operation_count += 1;
                let operation = PlanOperation {
                    operation_id: format!("op_{operation_count:06}"),
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
                };
                store.insert_plan_operation(session_id, &operation, &destination_key)?;
                maybe_emit_plan_progress(
                    progress,
                    PlanGenerationProgress {
                        processed_records,
                        total_records,
                        operations_created: operation_count,
                        ambiguous_files: ambiguous_count,
                        conflicts: conflict_count,
                        skipped: skipped_count,
                        current_path: Some(record.root_relative_path.clone()),
                    },
                );
            }

            offset += records.len();
        }

        let files_scanned = if total_records == 0 {
            offset
        } else {
            total_records
        };
        let summary = PlanSummary {
            files_scanned,
            moves_proposed: operation_count,
            ambiguous_files: ambiguous_count,
            conflicts: conflict_count,
            skipped: skipped_count,
        };
        store.save_plan_summary(session_id, &summary)?;
        progress(PlanGenerationProgress {
            processed_records,
            total_records: files_scanned,
            operations_created: operation_count,
            ambiguous_files: ambiguous_count,
            conflicts: conflict_count,
            skipped: skipped_count,
            current_path: None,
        });

        Ok(StoredPlanResult {
            plan_id: options.plan_id.clone(),
            root: root.to_path_buf(),
            mode,
            created_at: options.created_at,
            summary,
        })
    })();

    match generation_result {
        Ok(result) => {
            store.commit_write_batch()?;
            Ok(result)
        }
        Err(error) => {
            let _ = store.rollback_write_batch();
            Err(error)
        }
    }
}

fn maybe_emit_plan_progress(
    progress: &mut impl FnMut(PlanGenerationProgress),
    snapshot: PlanGenerationProgress,
) {
    if snapshot.processed_records % PLAN_PROGRESS_INTERVAL == 0 {
        progress(snapshot);
    }
}

/// Generate a plan into SQLite using the default page size.
///
/// # Errors
///
/// Returns error for storage failures, JSON decoding failures, or invalid paths.
pub fn generate_plan_to_store_default(
    root: impl AsRef<Path>,
    store: &mut SqliteSessionStore,
    session_id: &str,
    options: &PlanOptions,
) -> Result<StoredPlanResult> {
    generate_plan_to_store(root, store, session_id, options, DEFAULT_STORE_PAGE_SIZE)
}

/// Render a plan as human-readable text for terminal display.
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

/// Render a plan as formatted JSON.
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

fn already_organized_detail(
    record: &FileInventoryRecord,
    mode: &PlanningMode,
    source: &Path,
    destination: &Path,
    rule_destination: &Path,
) -> Option<String> {
    if normalized_destination_key(source) == normalized_destination_key(destination) {
        return Some("File is already in the planned destination.".to_string());
    }

    let PlanningMode::BuiltIn(mode) = mode else {
        return None;
    };

    if is_inside_organized_subtree(record, *mode, rule_destination) {
        return Some(format!(
            "File is inside an existing {} organization folder.",
            built_in_mode_guard_label(*mode)
        ));
    }

    None
}

fn is_inside_organized_subtree(
    record: &FileInventoryRecord,
    mode: BuiltInMode,
    rule_destination: &Path,
) -> bool {
    if record.depth <= 1 {
        return false;
    }

    let Some(source_parent) = record.root_relative_path.parent() else {
        return false;
    };
    let Some(destination_parent) = rule_destination.parent() else {
        return false;
    };

    let guard_len = organized_guard_component_count(mode);
    let guard_prefix = path_component_keys(destination_parent)
        .into_iter()
        .take(guard_len)
        .collect::<Vec<_>>();
    if guard_prefix.is_empty() {
        return false;
    }

    path_component_keys(source_parent).starts_with(&guard_prefix)
}

fn organized_guard_component_count(mode: BuiltInMode) -> usize {
    match mode {
        BuiltInMode::Type | BuiltInMode::Date | BuiltInMode::Extension | BuiltInMode::TypeYear => 1,
    }
}

fn built_in_mode_guard_label(mode: BuiltInMode) -> &'static str {
    match mode {
        BuiltInMode::Type => "type",
        BuiltInMode::Date => "date",
        BuiltInMode::Extension => "extension",
        BuiltInMode::TypeYear => "type/date",
    }
}

fn path_component_keys(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect()
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

fn detect_conflict_from_store(
    source: &Path,
    destination: &Path,
    already_planned: bool,
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

    if destination.exists() || already_planned {
        return ConflictState::DestinationExists {
            path: destination.to_path_buf(),
        };
    }

    ConflictState::None
}

fn untouched_record(
    record: &FileInventoryRecord,
    reason: UntouchedReason,
    detail: impl Into<String>,
) -> UntouchedRecord {
    UntouchedRecord {
        path: record.root_relative_path.clone(),
        reason,
        detail: detail.into(),
    }
}

fn untouched_reason_for_conflict(conflict: &ConflictState) -> UntouchedReason {
    match conflict {
        ConflictState::None => UntouchedReason::AlreadyOrganized,
        ConflictState::DestinationExists { .. } | ConflictState::CaseOnlyRename { .. } => {
            UntouchedReason::DestinationConflict
        }
        ConflictState::UnsafeDestination { .. } => UntouchedReason::UnsafeDestination,
    }
}

fn untouched_detail_for_conflict(conflict: &ConflictState) -> String {
    match conflict {
        ConflictState::None => "File is already in the planned destination.".to_string(),
        ConflictState::DestinationExists { path } => {
            format!("Destination already exists: {}", path.display())
        }
        ConflictState::CaseOnlyRename { path } => {
            format!(
                "Destination differs only by letter case: {}",
                path.display()
            )
        }
        ConflictState::UnsafeDestination { reason } => {
            format!("Destination was unsafe: {reason}")
        }
    }
}

fn normalized_destination_key(destination: &Path) -> String {
    destination
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
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

    use crate::model::{BuiltInMode, ConflictState, UntouchedReason};
    use crate::planner::{
        generate_plan, generate_plan_to_store_with_progress, render_preview, render_preview_json,
        PlanOptions,
    };
    use crate::rules::RuleProfile;
    use crate::scanner::{scan_folder, scan_folder_to_sink, CancellationToken, ScanOptions};
    use crate::session_store::{SessionScanSink, SqliteSessionStore};

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
        assert_eq!(plan.untouched_records.len(), 1);
        assert_eq!(
            plan.untouched_records[0].reason,
            UntouchedReason::NoMatchingRule
        );
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
        assert!(plan.untouched_records.iter().any(|record| {
            record.path == PathBuf::from("report.pdf")
                && record.reason == UntouchedReason::DestinationConflict
        }));
    }

    #[test]
    fn already_organized_files_are_left_untouched() {
        let fixture = fixture_dir();
        fs::create_dir_all(fixture.path().join("Documents")).expect("fixture dir");
        fs::write(
            fixture.path().join("Documents").join("report.pdf"),
            b"report",
        )
        .expect("fixture write");

        let scan = scan_folder(
            fixture.path(),
            &ScanOptions {
                current_folder_only: false,
                ..ScanOptions::default()
            },
        )
        .expect("scan succeeds");
        let plan = generate_plan(
            fixture.path(),
            &scan,
            &PlanOptions::built_in(BuiltInMode::Type, "plan_test", test_time()),
        )
        .expect("plan succeeds");

        assert!(plan.operations.is_empty());
        assert!(plan.untouched_records.iter().any(|record| {
            record.path == path(&["Documents", "report.pdf"])
                && record.reason == UntouchedReason::AlreadyOrganized
        }));
    }

    #[test]
    fn organized_subtrees_are_left_untouched_when_scanning_subfolders() {
        let fixture = fixture_dir();
        fs::create_dir_all(fixture.path().join(path(&["Documents", "Invoices"])))
            .expect("fixture dir");
        fs::write(
            fixture
                .path()
                .join(path(&["Documents", "Invoices", "report.pdf"])),
            b"report",
        )
        .expect("fixture write");

        let scan = scan_folder(
            fixture.path(),
            &ScanOptions {
                current_folder_only: false,
                ..ScanOptions::default()
            },
        )
        .expect("scan succeeds");
        let plan = generate_plan(
            fixture.path(),
            &scan,
            &PlanOptions::built_in(BuiltInMode::Type, "plan_test", test_time()),
        )
        .expect("plan succeeds");

        assert!(plan.operations.is_empty());
        assert!(plan.untouched_records.iter().any(|record| {
            record.path == path(&["Documents", "Invoices", "report.pdf"])
                && record.reason == UntouchedReason::AlreadyOrganized
        }));
    }

    #[test]
    fn stored_plan_persists_already_organized_reasons() {
        let fixture = fixture_dir();
        fs::create_dir_all(fixture.path().join("Documents")).expect("fixture dir");
        fs::write(
            fixture.path().join("Documents").join("report.pdf"),
            b"report",
        )
        .expect("fixture write");

        let mut store = SqliteSessionStore::in_memory().expect("store opens");
        let options = PlanOptions::built_in(BuiltInMode::Type, "plan_store_test", test_time());
        store
            .create_session_with_id(
                "session_already_organized",
                fixture.path(),
                &crate::model::PlanMode::BuiltIn(BuiltInMode::Type),
                test_time(),
            )
            .expect("session creates");
        let mut sink = SessionScanSink::new(&mut store, "session_already_organized");
        let scan = scan_folder_to_sink(
            fixture.path(),
            &ScanOptions {
                current_folder_only: false,
                ..ScanOptions::default()
            },
            &CancellationToken::default(),
            &mut sink,
        )
        .expect("scan streams");
        drop(sink);
        store
            .save_scan_summary("session_already_organized", &scan.summary)
            .expect("scan summary saves");

        generate_plan_to_store_with_progress(
            fixture.path(),
            &mut store,
            "session_already_organized",
            &options,
            1,
            &mut |_| {},
        )
        .expect("plan persists");

        assert!(store
            .plan_operations_page("session_already_organized", 0, 10)
            .expect("operations load")
            .is_empty());
        let counts = store
            .untouched_reason_counts("session_already_organized")
            .expect("untouched counts load");
        assert_eq!(counts[&UntouchedReason::AlreadyOrganized], 1);
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
    fn generated_plan_can_be_persisted_to_sqlite_store() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let mut store = SqliteSessionStore::in_memory().expect("store opens");
        let options = PlanOptions::built_in(BuiltInMode::Type, "plan_store_test", test_time());
        store
            .create_session_with_id(
                "session_plan_test",
                fixture.path(),
                &crate::model::PlanMode::BuiltIn(BuiltInMode::Type),
                test_time(),
            )
            .expect("session creates");
        let mut sink = SessionScanSink::new(&mut store, "session_plan_test");
        let scan = scan_folder_to_sink(
            fixture.path(),
            &ScanOptions::default(),
            &CancellationToken::default(),
            &mut sink,
        )
        .expect("scan streams");
        drop(sink);
        store
            .save_scan_summary("session_plan_test", &scan.summary)
            .expect("scan summary saves");

        let mut progress_snapshots = Vec::new();
        let result = generate_plan_to_store_with_progress(
            fixture.path(),
            &mut store,
            "session_plan_test",
            &options,
            1,
            &mut |progress| progress_snapshots.push(progress),
        )
        .expect("plan persists");

        assert_eq!(result.summary.moves_proposed, 1);
        assert!(progress_snapshots
            .iter()
            .any(|progress| progress.processed_records == 1));
        let operations = store
            .plan_operations_page("session_plan_test", 0, 10)
            .expect("operations load");
        assert_eq!(operations.len(), 1);
        assert!(operations[0].destination.ends_with("report.pdf"));
        assert_eq!(
            store
                .untouched_count("session_plan_test")
                .expect("untouched count loads"),
            0
        );
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
        assert_eq!(
            plan.untouched_records[0].reason,
            UntouchedReason::UnsafeDestination
        );
    }

    fn fixture_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn test_time() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap()
    }
}
