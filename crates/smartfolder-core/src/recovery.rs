//! Transaction inspection, undo, and cleanup.
//!
//! Provides utilities to:
//! - List all transactions that have been executed
//! - Inspect a specific transaction's journal and operations
//! - Undo (rollback) completed operations in reverse order
//! - Clean up completed transaction journals
//!
//! # Workflow
//!
//! ```ignore
//! // View all transactions
//! let transactions = list_transactions()?;
//!
//! // Inspect one
//! let journal = inspect_transaction("txn_20240512123456")?;
//!
//! // Undo it
//! let summary = undo_transaction("txn_20240512123456")?;
//! println!("Rolled back {} operations", summary.rolled_back);
//!
//! // Clean up
//! let cleanup = cleanup_transactions(include_incomplete)?;
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use std::cmp::Reverse;

use crate::model::{
    OperationError, OperationErrorCode, OperationStatus, TransactionJournal, TransactionOperation,
    TransactionStatus,
};
use crate::storage::{journal_path, journals_dir};
use crate::{Result, SmartfolderError};

/// Summary of an undo operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoSummary {
    pub transaction_id: String,
    pub journal_path: PathBuf,
    pub rolled_back: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Summary of a transaction resume operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSummary {
    pub transaction_id: String,
    pub journal_path: PathBuf,
    pub resumed: usize,
    pub completed: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Metadata about a transaction for listing and inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionSummary {
    pub transaction_id: String,
    pub plan_id: String,
    pub root: PathBuf,
    pub status: TransactionStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub path: PathBuf,
}

/// Summary of a cleanup operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupSummary {
    pub removed: Vec<PathBuf>,
    pub kept: Vec<PathBuf>,
}

/// Undo (rollback) all completed operations in a transaction.
///
/// # Logical Flow
///
/// 1. Load journal from transaction ID
/// 2. Iterate operations in reverse order
/// 3. For each completed operation:
///    - Attempt to move file back to original location
///    - Mark as rolled back if successful, failed otherwise
///    - Persist journal after each operation
/// 4. Update transaction status (`RolledBack`, `PartiallyRolledBack`, or `Failed`)
/// 5. Set completion timestamp
/// 6. Return summary with counts
///
/// # Errors
///
/// Returns error if transaction ID is invalid or journal cannot be read.
pub fn undo_transaction(transaction_id: &str) -> Result<UndoSummary> {
    let journal_path = journal_path(transaction_id)?;
    let mut journal = load_journal_from_path(&journal_path)?;

    for index in (0..journal.operations.len()).rev() {
        if journal.operations[index].status != OperationStatus::Completed {
            continue;
        }

        match undo_operation(&journal.operations[index]) {
            Ok(()) => {
                journal.operations[index].status = OperationStatus::RolledBack;
                journal.operations[index].error = None;
            }
            Err(error) => {
                journal.operations[index].status = OperationStatus::Failed;
                journal.operations[index].error = Some(error);
            }
        }

        save_journal(&journal, &journal_path)?;
    }

    let rolled_back = journal
        .operations
        .iter()
        .filter(|operation| operation.status == OperationStatus::RolledBack)
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

    journal.status = if failed == 0 {
        TransactionStatus::RolledBack
    } else if rolled_back > 0 {
        TransactionStatus::PartiallyRolledBack
    } else {
        TransactionStatus::Failed
    };
    journal.completed_at = Some(Utc::now());
    save_journal(&journal, &journal_path)?;

    Ok(UndoSummary {
        transaction_id: journal.transaction_id,
        journal_path,
        rolled_back,
        skipped,
        failed,
    })
}

/// Resume an interrupted or failed transaction from its journal state.
///
/// # Logical Flow
///
/// 1. Load journal from transaction ID
/// 2. Iterate operations in journal order
/// 3. For each non-completed/non-rolled-back operation:
///    - Attempt the recorded move from source to destination
///    - Mark as completed on success, skipped/failed on error
///    - Persist journal after each operation
/// 4. Recompute aggregate counts and mark transaction status
/// 5. Persist final journal and return summary
///
/// # Errors
///
/// Returns error if transaction ID is invalid or journal cannot be read.
pub fn resume_transaction(transaction_id: &str) -> Result<ResumeSummary> {
    let journal_path = journal_path(transaction_id)?;
    let mut journal = load_journal_from_path(&journal_path)?;
    let mut resumed = 0;

    for index in 0..journal.operations.len() {
        if matches!(
            journal.operations[index].status,
            OperationStatus::Completed | OperationStatus::RolledBack
        ) {
            continue;
        }

        match resume_operation(&journal.operations[index]) {
            Ok(()) => {
                journal.operations[index].status = OperationStatus::Completed;
                journal.operations[index].error = None;
                resumed += 1;
            }
            Err(error) => {
                journal.operations[index].status = match error.code {
                    OperationErrorCode::DestinationExists => OperationStatus::Skipped,
                    _ => OperationStatus::Failed,
                };
                journal.operations[index].error = Some(error);
            }
        }

        save_journal(&journal, &journal_path)?;
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

    if matches!(
        journal.status,
        TransactionStatus::InProgress | TransactionStatus::Interrupted | TransactionStatus::Failed
    ) {
        journal.status = if failed == 0 {
            TransactionStatus::Completed
        } else {
            TransactionStatus::Failed
        };
        journal.completed_at = Some(Utc::now());
        save_journal(&journal, &journal_path)?;
    }

    Ok(ResumeSummary {
        transaction_id: journal.transaction_id,
        journal_path,
        resumed,
        completed,
        skipped,
        failed,
    })
}

/// Load and inspect a transaction journal.
pub fn inspect_transaction(transaction_id: &str) -> Result<TransactionJournal> {
    load_journal_from_path(&journal_path(transaction_id)?)
}

pub fn list_transactions() -> Result<Vec<TransactionSummary>> {
    let directory = journals_dir()?;
    if !directory.exists() {
        return Ok(Vec::new());
    }

    let mut summaries = Vec::new();
    for entry in
        fs::read_dir(&directory).map_err(|source| SmartfolderError::io(&directory, source))?
    {
        let entry = entry.map_err(|source| SmartfolderError::io(&directory, source))?;
        let path = entry.path();
        if path
            .extension()
            .map_or(true, |extension| extension != "json")
        {
            continue;
        }

        if let Ok(journal) = load_journal_from_path(&path) {
            summaries.push(TransactionSummary {
                transaction_id: journal.transaction_id,
                plan_id: journal.plan_id,
                root: journal.root,
                status: journal.status,
                started_at: journal.started_at,
                completed_at: journal.completed_at,
                path,
            });
        }
    }

    summaries.sort_by_key(|summary| Reverse(summary.started_at));
    Ok(summaries)
}

pub fn cleanup_transactions(include_incomplete: bool) -> Result<CleanupSummary> {
    let mut summary = CleanupSummary {
        removed: Vec::new(),
        kept: Vec::new(),
    };

    for transaction in list_transactions()? {
        if !include_incomplete && is_incomplete(transaction.status) {
            summary.kept.push(transaction.path);
            continue;
        }

        fs::remove_file(&transaction.path)
            .map_err(|source| SmartfolderError::io(&transaction.path, source))?;
        summary.removed.push(transaction.path);
    }

    Ok(summary)
}

fn undo_operation(operation: &TransactionOperation) -> std::result::Result<(), OperationError> {
    if !operation.destination.exists() {
        return Err(operation_error(
            OperationErrorCode::SourceMissing,
            "moved file no longer exists at the recorded destination",
        ));
    }

    if operation.source.exists() {
        return Err(operation_error(
            OperationErrorCode::DestinationExists,
            "original source path already exists; refusing to overwrite during undo",
        ));
    }

    if let Some(parent) = operation.source.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                operation_error(OperationErrorCode::PermissionDenied, error.to_string())
            } else {
                operation_error(OperationErrorCode::IoError, error.to_string())
            }
        })?;
    }

    fs::rename(&operation.destination, &operation.source).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            operation_error(OperationErrorCode::PermissionDenied, error.to_string())
        } else {
            operation_error(OperationErrorCode::IoError, error.to_string())
        }
    })
}

fn resume_operation(operation: &TransactionOperation) -> std::result::Result<(), OperationError> {
    if operation.destination.exists() {
        return Err(operation_error(
            OperationErrorCode::DestinationExists,
            "destination already exists",
        ));
    }

    if !operation.source.exists() {
        return Err(operation_error(
            OperationErrorCode::SourceMissing,
            "source file no longer exists",
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

fn load_journal_from_path(path: &Path) -> Result<TransactionJournal> {
    let contents = fs::read_to_string(path).map_err(|source| SmartfolderError::io(path, source))?;
    serde_json::from_str(&contents).map_err(Into::into)
}

fn save_journal(journal: &TransactionJournal, path: &Path) -> Result<()> {
    let contents = journal.to_pretty_json()?;
    fs::write(path, contents).map_err(|source| SmartfolderError::io(path, source))
}

fn operation_error(code: OperationErrorCode, message: impl Into<String>) -> OperationError {
    OperationError {
        code,
        message: message.into(),
    }
}

fn is_incomplete(status: TransactionStatus) -> bool {
    matches!(
        status,
        TransactionStatus::InProgress | TransactionStatus::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    use crate::apply::{apply_plan, ApplyOptions};
    use crate::model::{BuiltInMode, OperationStatus, TransactionStatus};
    use crate::planner::{generate_plan, PlanOptions};
    use crate::recovery::{
        inspect_transaction, list_transactions, resume_transaction, undo_transaction,
    };
    use crate::scanner::{scan_folder, ScanOptions};
    use crate::storage::journal_path;

    #[test]
    fn undo_restores_moved_file_to_original_location() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let transaction_id = apply_fixture(&fixture);

        let summary = undo_transaction(&transaction_id).expect("undo succeeds");

        assert_eq!(summary.rolled_back, 1);
        assert!(fixture.path().join("report.pdf").exists());
        assert!(!fixture.path().join("Documents").join("report.pdf").exists());

        let journal = inspect_transaction(&transaction_id).expect("journal loads");
        assert_eq!(journal.status, TransactionStatus::RolledBack);
        assert_eq!(journal.operations[0].status, OperationStatus::RolledBack);
    }

    #[test]
    fn undo_refuses_to_overwrite_existing_original_path() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let transaction_id = apply_fixture(&fixture);
        fs::write(fixture.path().join("report.pdf"), b"new file").expect("new source write");

        let summary = undo_transaction(&transaction_id).expect("undo completes with failure");

        assert_eq!(summary.rolled_back, 0);
        assert_eq!(summary.failed, 1);
        assert_eq!(
            fs::read_to_string(fixture.path().join("report.pdf")).expect("source remains"),
            "new file"
        );
        assert_eq!(
            fs::read_to_string(fixture.path().join("Documents").join("report.pdf"))
                .expect("destination remains"),
            "report"
        );
    }

    #[test]
    fn list_transactions_includes_new_journal() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let transaction_id = apply_fixture(&fixture);

        let transactions = list_transactions().expect("transactions list");

        assert!(transactions
            .iter()
            .any(|transaction| transaction.transaction_id == transaction_id));
    }

    #[test]
    fn resume_continues_pending_operations_in_an_interrupted_journal() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let transaction_id = apply_fixture(&fixture);
        let journal_file = journal_path(&transaction_id).expect("journal path resolves");

        let mut journal = inspect_transaction(&transaction_id).expect("journal loads");
        let operation = journal.operations[0].clone();
        fs::rename(&operation.destination, &operation.source).expect("reset source path");
        journal.status = TransactionStatus::Interrupted;
        journal.completed_at = None;
        journal.operations[0].status = OperationStatus::Pending;
        journal.operations[0].error = None;
        fs::write(
            &journal_file,
            serde_json::to_string_pretty(&journal).expect("journal serializes"),
        )
        .expect("journal writes");

        let summary = resume_transaction(&transaction_id).expect("resume succeeds");
        assert_eq!(summary.resumed, 1);
        assert_eq!(summary.completed, 1);
        assert!(operation.destination.exists());
        assert!(!operation.source.exists());

        let resumed = inspect_transaction(&transaction_id).expect("resumed journal loads");
        assert_eq!(resumed.status, TransactionStatus::Completed);
        assert_eq!(resumed.operations[0].status, OperationStatus::Completed);
    }

    fn apply_fixture(fixture: &TempDir) -> String {
        let scan = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");
        let plan = generate_plan(
            fixture.path(),
            &scan,
            &PlanOptions::built_in(BuiltInMode::Type, "plan_undo_test", test_time()),
        )
        .expect("plan succeeds");
        let transaction_id = unique_transaction_id("undo");

        apply_plan(
            &plan,
            &ApplyOptions::new(transaction_id.clone(), test_time()),
        )
        .expect("apply succeeds");

        transaction_id
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
