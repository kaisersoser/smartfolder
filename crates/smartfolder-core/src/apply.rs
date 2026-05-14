//! Safe execution of file organization plans with transaction journaling.
//!
//! This module applies a plan to the file system while maintaining a transaction journal.
//! The journal enables rollback if needed and provides a detailed audit trail of what was done.
//!
//! # Key features
//!
//! - **Transaction journaling**: Every operation is recorded before and after execution
//! - **Cancellation support**: Can be interrupted gracefully via cancellation token
//! - **Conflict handling**: Respects "selected" flag on operations (skips conflicts)
//! - **File change detection**: Verifies files haven't changed since plan was created
//! - **Same-volume detection**: Optimizes moves on same partition
//!
//! # Workflow
//!
//! 1. Create [`ApplyOptions`] with transaction ID
//! 2. Set up cancellation handling if desired
//! 3. Call [`apply_plan`] with plan and options
//! 4. Review [`ApplySummary`] for results
//! 5. Later: inspect with [`load_journal`] or undo with [`crate::recovery::undo_transaction`]

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use chrono::{DateTime, Utc};

use crate::model::{
    ConflictState, OperationError, OperationErrorCode, OperationStatus, PlanOperation, PlanRecord,
    TransactionJournal, TransactionOperation, TransactionStatus,
};
use crate::session_store::{PlanOperationFilter, SqliteSessionStore};
use crate::storage::journal_path;
use crate::{Result, SmartfolderError};

/// Configuration options for applying a plan.
///
/// - `transaction_id`: Unique identifier for this execution (used for journal filename)
/// - `started_at`: Timestamp when execution began
/// - `journal_export`: Optional path to copy journal to (in addition to app data directory)
/// - `cancellation`: Token to signal graceful cancellation
#[derive(Debug, Clone)]
pub struct ApplyOptions {
    pub transaction_id: String,
    pub started_at: DateTime<Utc>,
    pub journal_export: Option<PathBuf>,
    pub cancellation: ApplyCancellationToken,
}

impl ApplyOptions {
    /// Create apply options with transaction ID and timestamp.
    pub fn new(transaction_id: impl Into<String>, started_at: DateTime<Utc>) -> Self {
        Self {
            transaction_id: transaction_id.into(),
            started_at,
            journal_export: None,
            cancellation: ApplyCancellationToken::default(),
        }
    }
}

/// Token for cancelling an in-progress plan application.
///
/// Clone and share with execution task. Checked before each operation.
#[derive(Debug, Default, Clone)]
pub struct ApplyCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl ApplyCancellationToken {
    /// Signal that execution should stop.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Check if cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Summary of a plan execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplySummary {
    pub transaction_id: String,
    pub journal_path: PathBuf,
    pub completed: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Live progress snapshot emitted while applying operations from session storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredApplyProgress {
    pub processed: usize,
    pub total: usize,
    pub completed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub current_path: Option<PathBuf>,
}

/// Apply a plan to the file system, creating a transaction journal.
///
/// # Logical Flow
///
/// 1. Verify plan has selected operations
/// 2. Create and persist transaction journal
/// 3. For each operation:
///    - Check for cancellation
///    - Skip if not selected (has conflict)
///    - Verify source file hasn't changed since plan was created
///    - Perform the move operation
///    - Record result (success, error, rollback status)
///    - Persist journal after each operation
/// 4. Mark transaction complete or interrupted
/// 5. Return summary
///
/// # Errors
///
/// Returns error if plan has no selected operations or if IO operations fail.
pub fn apply_plan(plan: &PlanRecord, options: &ApplyOptions) -> Result<ApplySummary> {
    if plan.operations.is_empty() {
        return Err(SmartfolderError::NoSelectedOperations);
    }

    let journal_path = journal_path(&options.transaction_id)?;
    let mut journal = TransactionJournal::new(
        options.transaction_id.clone(),
        plan.plan_id.clone(),
        plan.root.clone(),
        options.started_at,
    );
    journal.operations = plan
        .operations
        .iter()
        .map(transaction_operation_from_plan)
        .collect();

    persist_journal(&journal, &journal_path, options.journal_export.as_deref())?;

    for index in 0..journal.operations.len() {
        if options.cancellation.is_cancelled() {
            journal.status = TransactionStatus::Interrupted;
            journal.completed_at = Some(Utc::now());
            persist_journal(&journal, &journal_path, options.journal_export.as_deref())?;
            break;
        }

        if !plan.operations[index].selected {
            journal.operations[index].status = OperationStatus::Skipped;
            journal.operations[index].error = Some(operation_error(
                OperationErrorCode::DestinationExists,
                "operation was not selected because the generated plan marked it unsafe or conflicted",
            ));
            persist_journal(&journal, &journal_path, options.journal_export.as_deref())?;
            continue;
        }

        let status = apply_operation(&plan.operations[index]);
        journal.operations[index].same_volume = Some(same_volume(
            &plan.operations[index].source,
            &plan.operations[index].destination,
        ));

        match status {
            Ok(()) => {
                journal.operations[index].status = OperationStatus::Completed;
                journal.operations[index].error = None;
            }
            Err(error) => {
                journal.operations[index].status = match error.code {
                    OperationErrorCode::DestinationExists => OperationStatus::Skipped,
                    _ => OperationStatus::Failed,
                };
                journal.operations[index].error = Some(error);
            }
        }

        persist_journal(&journal, &journal_path, options.journal_export.as_deref())?;
    }

    let completed = journal
        .operations
        .iter()
        .filter(|operation| operation.status == OperationStatus::Completed)
        .count();
    let skipped = journal
        .operations
        .iter()
        .filter(|operation| operation.status == OperationStatus::Skipped)
        .count();
    let failed = journal
        .operations
        .iter()
        .filter(|operation| operation.status == OperationStatus::Failed)
        .count();

    if journal.status == TransactionStatus::InProgress {
        journal.status = if failed == 0 {
            TransactionStatus::Completed
        } else {
            TransactionStatus::Failed
        };
        journal.completed_at = Some(Utc::now());
    }
    persist_journal(&journal, &journal_path, options.journal_export.as_deref())?;

    Ok(ApplySummary {
        transaction_id: journal.transaction_id,
        journal_path,
        completed,
        skipped,
        failed,
    })
}

/// Apply ready operations from a SQLite session store with bounded plan loading.
///
/// # Logical Flow
///
/// 1. Count ready operations in the session store
/// 2. Create and persist an empty transaction journal
/// 3. Read ready operations in pages
/// 4. For each operation:
///    - Check for cancellation
///    - Apply the same safety checks as [`apply_plan`]
///    - Append the result to the journal
///    - Persist the journal and emit progress
/// 5. Mark the transaction complete, interrupted, or failed
/// 6. Return aggregate execution counts
///
/// # Errors
///
/// Returns error if no ready operations exist, storage queries fail, or journal
/// persistence fails.
pub fn apply_stored_plan_with_progress(
    store: &SqliteSessionStore,
    session_id: &str,
    plan_id: impl Into<String>,
    root: impl Into<PathBuf>,
    options: &ApplyOptions,
    page_size: usize,
    progress: &mut impl FnMut(StoredApplyProgress),
) -> Result<ApplySummary> {
    let total = store.plan_operation_count(session_id, PlanOperationFilter::Ready)?;
    if total == 0 {
        return Err(SmartfolderError::NoSelectedOperations);
    }

    let journal_path = journal_path(&options.transaction_id)?;
    let mut journal = TransactionJournal::new(
        options.transaction_id.clone(),
        plan_id.into(),
        root.into(),
        options.started_at,
    );
    persist_journal(&journal, &journal_path, options.journal_export.as_deref())?;

    let page_size = page_size.max(1);
    let mut offset = 0;
    let mut processed = 0;
    let mut completed = 0;
    let mut skipped = 0;
    let mut failed = 0;

    progress(StoredApplyProgress {
        processed,
        total,
        completed,
        skipped,
        failed,
        current_path: None,
    });

    while offset < total {
        let operations = store.plan_operations_page_filtered(
            session_id,
            PlanOperationFilter::Ready,
            offset,
            page_size,
        )?;
        if operations.is_empty() {
            break;
        }

        for operation in operations {
            if options.cancellation.is_cancelled() {
                journal.status = TransactionStatus::Interrupted;
                journal.completed_at = Some(Utc::now());
                persist_journal(&journal, &journal_path, options.journal_export.as_deref())?;
                return Ok(ApplySummary {
                    transaction_id: journal.transaction_id,
                    journal_path,
                    completed,
                    skipped,
                    failed,
                });
            }

            let current_path = operation.source.clone();
            let mut transaction_operation = transaction_operation_from_plan(&operation);
            transaction_operation.same_volume =
                Some(same_volume(&operation.source, &operation.destination));

            match apply_operation(&operation) {
                Ok(()) => {
                    transaction_operation.status = OperationStatus::Completed;
                    transaction_operation.error = None;
                    completed += 1;
                }
                Err(error) => {
                    transaction_operation.status = match error.code {
                        OperationErrorCode::DestinationExists => OperationStatus::Skipped,
                        _ => OperationStatus::Failed,
                    };
                    transaction_operation.error = Some(error);
                    if transaction_operation.status == OperationStatus::Skipped {
                        skipped += 1;
                    } else {
                        failed += 1;
                    }
                }
            }

            journal.operations.push(transaction_operation);
            processed += 1;
            persist_journal(&journal, &journal_path, options.journal_export.as_deref())?;
            progress(StoredApplyProgress {
                processed,
                total,
                completed,
                skipped,
                failed,
                current_path: Some(current_path),
            });
        }

        offset += page_size;
    }

    if journal.status == TransactionStatus::InProgress {
        journal.status = if failed == 0 {
            TransactionStatus::Completed
        } else {
            TransactionStatus::Failed
        };
        journal.completed_at = Some(Utc::now());
    }
    persist_journal(&journal, &journal_path, options.journal_export.as_deref())?;

    Ok(ApplySummary {
        transaction_id: journal.transaction_id,
        journal_path,
        completed,
        skipped,
        failed,
    })
}

/// Apply ready operations from a SQLite session store without progress callbacks.
///
/// # Errors
///
/// Returns error if no ready operations exist, storage queries fail, or journal
/// persistence fails.
pub fn apply_stored_plan(
    store: &SqliteSessionStore,
    session_id: &str,
    plan_id: impl Into<String>,
    root: impl Into<PathBuf>,
    options: &ApplyOptions,
    page_size: usize,
) -> Result<ApplySummary> {
    let mut ignore_progress = |_| {};
    apply_stored_plan_with_progress(
        store,
        session_id,
        plan_id,
        root,
        options,
        page_size,
        &mut ignore_progress,
    )
}

/// Load and deserialize a transaction journal from disk.
///
/// # Errors
///
/// Returns an error if the journal file cannot be read or cannot be parsed as JSON.
pub fn load_journal(transaction_id: &str) -> Result<TransactionJournal> {
    let path = journal_path(transaction_id)?;
    let contents =
        fs::read_to_string(&path).map_err(|source| SmartfolderError::io(&path, source))?;
    serde_json::from_str(&contents).map_err(Into::into)
}

fn apply_operation(operation: &PlanOperation) -> std::result::Result<(), OperationError> {
    if !matches!(operation.conflict, ConflictState::None) {
        return Err(operation_error(
            OperationErrorCode::DestinationExists,
            "operation has a destination conflict",
        ));
    }

    let metadata = fs::metadata(&operation.source).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            operation_error(
                OperationErrorCode::SourceMissing,
                "source file no longer exists",
            )
        } else if error.kind() == std::io::ErrorKind::PermissionDenied {
            operation_error(OperationErrorCode::PermissionDenied, error.to_string())
        } else {
            operation_error(OperationErrorCode::IoError, error.to_string())
        }
    })?;

    if metadata.len() != operation.source_snapshot.size_bytes {
        return Err(operation_error(
            OperationErrorCode::SourceChanged,
            "source file size changed after the plan was generated",
        ));
    }

    if let Some(expected_modified) = operation.source_snapshot.modified_at {
        let actual_modified = metadata.modified().ok().map(DateTime::<Utc>::from);
        if actual_modified != Some(expected_modified) {
            return Err(operation_error(
                OperationErrorCode::SourceChanged,
                "source file modified timestamp changed after the plan was generated",
            ));
        }
    }

    if operation.destination.exists() {
        return Err(operation_error(
            OperationErrorCode::DestinationExists,
            "destination already exists",
        ));
    }

    if let Some(parent) = operation.destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                operation_error(OperationErrorCode::PermissionDenied, error.to_string())
            } else {
                operation_error(OperationErrorCode::IoError, error.to_string())
            }
        })?;
    }

    fs::rename(&operation.source, &operation.destination).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            operation_error(OperationErrorCode::PermissionDenied, error.to_string())
        } else {
            operation_error(OperationErrorCode::IoError, error.to_string())
        }
    })
}

fn transaction_operation_from_plan(operation: &PlanOperation) -> TransactionOperation {
    TransactionOperation {
        operation_id: operation.operation_id.clone(),
        operation_type: operation.operation_type,
        source: operation.source.clone(),
        destination: operation.destination.clone(),
        status: OperationStatus::Pending,
        reason: Some(operation.reason.clone()),
        same_volume: None,
        error: None,
    }
}

fn persist_journal(journal: &TransactionJournal, path: &Path, export: Option<&Path>) -> Result<()> {
    let contents = journal.to_pretty_json()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SmartfolderError::io(parent, source))?;
    }
    fs::write(path, &contents).map_err(|source| SmartfolderError::io(path, source))?;

    if let Some(export_path) = export {
        if let Some(parent) = export_path.parent() {
            fs::create_dir_all(parent).map_err(|source| SmartfolderError::io(parent, source))?;
        }
        fs::write(export_path, contents)
            .map_err(|source| SmartfolderError::io(export_path, source))?;
    }

    Ok(())
}

fn operation_error(code: OperationErrorCode, message: impl Into<String>) -> OperationError {
    OperationError {
        code,
        message: message.into(),
    }
}

fn same_volume(source: &Path, destination: &Path) -> bool {
    volume_component(source) == volume_component(destination)
}

fn volume_component(path: &Path) -> Option<String> {
    path.components()
        .next()
        .and_then(|component| match component {
            Component::Prefix(prefix) => {
                Some(prefix.as_os_str().to_string_lossy().to_ascii_lowercase())
            }
            Component::RootDir => Some(std::path::MAIN_SEPARATOR.to_string()),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use std::fs;
    use tempfile::TempDir;

    use crate::apply::{apply_plan, apply_stored_plan_with_progress, load_journal, ApplyOptions};
    use crate::model::{BuiltInMode, OperationStatus, PlanMode};
    use crate::planner::{generate_plan, generate_plan_to_store, PlanOptions};
    use crate::scanner::{scan_folder, scan_folder_to_sink, CancellationToken, ScanOptions};
    use crate::session_store::{SessionScanSink, SqliteSessionStore};

    #[test]
    fn apply_moves_selected_files_and_writes_journal() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let plan = fixture_plan(&fixture, BuiltInMode::Type);
        let transaction_id = unique_transaction_id("apply_success");

        let summary = apply_plan(
            &plan,
            &ApplyOptions::new(transaction_id.clone(), test_time()),
        )
        .expect("apply succeeds");

        assert_eq!(summary.completed, 1);
        assert!(fixture.path().join("Documents").join("report.pdf").exists());
        assert!(!fixture.path().join("report.pdf").exists());

        let journal = load_journal(&transaction_id).expect("journal loads");
        assert_eq!(journal.operations[0].status, OperationStatus::Completed);
        assert_eq!(
            journal.operations[0].reason.as_deref(),
            Some("Built-in rule: Type")
        );
    }

    #[test]
    fn apply_skips_existing_destination_without_overwrite() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        fs::create_dir(fixture.path().join("Documents")).expect("fixture dir");
        fs::write(
            fixture.path().join("Documents").join("report.pdf"),
            b"existing",
        )
        .expect("fixture write");
        let plan = fixture_plan(&fixture, BuiltInMode::Type);
        let transaction_id = unique_transaction_id("apply_conflict");

        let summary = apply_plan(
            &plan,
            &ApplyOptions::new(transaction_id.clone(), test_time()),
        )
        .expect("apply succeeds with skipped conflict");

        assert_eq!(summary.completed, 0);
        assert!(summary.skipped >= 1);
        assert_eq!(
            fs::read_to_string(fixture.path().join("Documents").join("report.pdf"))
                .expect("existing file remains"),
            "existing"
        );
        assert!(fixture.path().join("report.pdf").exists());
    }

    #[test]
    fn apply_fails_operation_when_source_changes() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let plan = fixture_plan(&fixture, BuiltInMode::Type);
        fs::write(fixture.path().join("report.pdf"), b"changed").expect("fixture rewrite");
        let transaction_id = unique_transaction_id("apply_changed");

        let summary = apply_plan(&plan, &ApplyOptions::new(transaction_id, test_time()))
            .expect("apply completes with failed operation");

        assert_eq!(summary.completed, 0);
        assert_eq!(summary.failed, 1);
        assert!(fixture.path().join("report.pdf").exists());
    }

    #[test]
    fn apply_stored_plan_moves_ready_operations_and_reports_progress() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let mut store = SqliteSessionStore::in_memory().expect("store opens");
        store
            .create_session_with_id(
                "session_apply_store",
                fixture.path(),
                &PlanMode::BuiltIn(BuiltInMode::Type),
                test_time(),
            )
            .expect("session creates");

        let mut sink = SessionScanSink::new(&mut store, "session_apply_store");
        let scan = scan_folder_to_sink(
            fixture.path(),
            &ScanOptions::default(),
            &CancellationToken::default(),
            &mut sink,
        )
        .expect("scan streams");
        drop(sink);
        store
            .save_scan_summary("session_apply_store", &scan.summary)
            .expect("scan summary saves");
        generate_plan_to_store(
            fixture.path(),
            &mut store,
            "session_apply_store",
            &PlanOptions::built_in(BuiltInMode::Type, "plan_apply_store", test_time()),
            10,
        )
        .expect("stored plan generates");

        let transaction_id = unique_transaction_id("apply_store");
        let mut progress = Vec::new();
        let summary = apply_stored_plan_with_progress(
            &store,
            "session_apply_store",
            "plan_apply_store",
            fixture.path(),
            &ApplyOptions::new(transaction_id.clone(), test_time()),
            1,
            &mut |snapshot| progress.push(snapshot),
        )
        .expect("stored apply succeeds");

        assert_eq!(summary.completed, 1);
        assert_eq!(summary.failed, 0);
        assert!(fixture.path().join("Documents").join("report.pdf").exists());
        assert!(!fixture.path().join("report.pdf").exists());
        assert!(progress.iter().any(|snapshot| snapshot.processed == 1));

        let journal = load_journal(&transaction_id).expect("journal loads");
        assert_eq!(journal.operations.len(), 1);
        assert_eq!(journal.operations[0].status, OperationStatus::Completed);
        assert_eq!(
            journal.operations[0].reason.as_deref(),
            Some("Built-in rule: Type")
        );
    }

    fn fixture_plan(fixture: &TempDir, mode: BuiltInMode) -> crate::model::PlanRecord {
        let scan = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");
        generate_plan(
            fixture.path(),
            &scan,
            &PlanOptions::built_in(mode, "plan_apply_test", test_time()),
        )
        .expect("plan succeeds")
    }

    fn fixture_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn test_time() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap()
    }

    fn unique_transaction_id(prefix: &str) -> String {
        format!(
            "{prefix}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        )
    }
}
