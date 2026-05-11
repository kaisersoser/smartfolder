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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoSummary {
    pub transaction_id: String,
    pub journal_path: PathBuf,
    pub rolled_back: usize,
    pub skipped: usize,
    pub failed: usize,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupSummary {
    pub removed: Vec<PathBuf>,
    pub kept: Vec<PathBuf>,
}

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
    use crate::recovery::{inspect_transaction, list_transactions, undo_transaction};
    use crate::scanner::{scan_folder, ScanOptions};

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
