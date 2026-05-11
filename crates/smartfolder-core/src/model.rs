use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const PLAN_SCHEMA_VERSION: u16 = 1;
pub const JOURNAL_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInventoryRecord {
    pub file_id: String,
    pub root_relative_path: PathBuf,
    pub name: String,
    pub extension: Option<String>,
    pub detected_type: FileTypeBucket,
    pub size_bytes: u64,
    pub created_at: Option<DateTime<Utc>>,
    pub modified_at: Option<DateTime<Utc>>,
    pub accessed_at: Option<DateTime<Utc>>,
    pub depth: usize,
    pub entry_kind: FileEntryKind,
    pub scan_warnings: Vec<ScanWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTypeBucket {
    Document,
    Image,
    Video,
    Audio,
    Archive,
    Spreadsheet,
    Presentation,
    Code,
    Directory,
    Link,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileEntryKind {
    File,
    Directory,
    Symlink,
    Junction,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanWarning {
    pub code: ScanWarningCode,
    pub path: Option<PathBuf>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanWarningCode {
    UnreadableEntry,
    UnsupportedMetadata,
    SkippedByPolicy,
    SpecialFolder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRecord {
    pub schema_version: u16,
    pub plan_id: String,
    pub root: PathBuf,
    pub mode: PlanMode,
    pub created_at: DateTime<Utc>,
    pub operations: Vec<PlanOperation>,
    pub ambiguous_files: Vec<PathBuf>,
    pub warnings: Vec<PlanWarning>,
    pub summary: PlanSummary,
}

impl PlanRecord {
    pub fn new(
        plan_id: impl Into<String>,
        root: impl Into<PathBuf>,
        mode: PlanMode,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            schema_version: PLAN_SCHEMA_VERSION,
            plan_id: plan_id.into(),
            root: root.into(),
            mode,
            created_at,
            operations: Vec::new(),
            ambiguous_files: Vec::new(),
            warnings: Vec::new(),
            summary: PlanSummary::default(),
        }
    }

    pub fn to_pretty_json(&self) -> crate::Result<String> {
        serde_json::to_string_pretty(self).map_err(Into::into)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    BuiltIn(BuiltInMode),
    RuleProfile { profile_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltInMode {
    Type,
    Date,
    Extension,
    TypeYear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanOperation {
    pub operation_id: String,
    pub operation_type: OperationType,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub reason: String,
    pub certainty: Certainty,
    pub conflict: ConflictState,
    pub selected: bool,
    pub source_snapshot: SourceSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationType {
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ConflictState {
    None,
    DestinationExists { path: PathBuf },
    CaseOnlyRename { path: PathBuf },
    UnsafeDestination { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSnapshot {
    pub size_bytes: u64,
    pub modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSummary {
    pub files_scanned: usize,
    pub moves_proposed: usize,
    pub ambiguous_files: usize,
    pub conflicts: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanWarning {
    pub code: PlanWarningCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanWarningCode {
    CloudFolder,
    SpecialFolder,
    ExclusionsApplied,
    AmbiguousFiles,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionJournal {
    pub schema_version: u16,
    pub transaction_id: String,
    pub plan_id: String,
    pub root: PathBuf,
    pub status: TransactionStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub operations: Vec<TransactionOperation>,
}

impl TransactionJournal {
    pub fn new(
        transaction_id: impl Into<String>,
        plan_id: impl Into<String>,
        root: impl Into<PathBuf>,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            transaction_id: transaction_id.into(),
            plan_id: plan_id.into(),
            root: root.into(),
            status: TransactionStatus::InProgress,
            started_at,
            completed_at: None,
            operations: Vec::new(),
        }
    }

    pub fn to_pretty_json(&self) -> crate::Result<String> {
        serde_json::to_string_pretty(self).map_err(Into::into)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    InProgress,
    Completed,
    Interrupted,
    RolledBack,
    PartiallyRolledBack,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionOperation {
    pub operation_id: String,
    pub operation_type: OperationType,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub status: OperationStatus,
    pub same_volume: Option<bool>,
    pub error: Option<OperationError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Pending,
    Completed,
    Skipped,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationError {
    pub code: OperationErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationErrorCode {
    SourceMissing,
    SourceChanged,
    DestinationExists,
    PermissionDenied,
    IoError,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;

    use super::{
        BuiltInMode, PlanMode, PlanRecord, TransactionJournal, JOURNAL_SCHEMA_VERSION,
        PLAN_SCHEMA_VERSION,
    };

    #[test]
    fn plan_record_round_trips_with_schema_version() {
        let plan = PlanRecord::new(
            "plan_test",
            PathBuf::from("C:\\data"),
            PlanMode::BuiltIn(BuiltInMode::Type),
            Utc::now(),
        );

        let json = serde_json::to_string(&plan).expect("plan should serialize");
        let restored: PlanRecord = serde_json::from_str(&json).expect("plan should deserialize");

        assert_eq!(restored.schema_version, PLAN_SCHEMA_VERSION);
        assert_eq!(restored.plan_id, "plan_test");
        assert_eq!(restored.mode, PlanMode::BuiltIn(BuiltInMode::Type));
    }

    #[test]
    fn journal_round_trips_with_schema_version() {
        let journal = TransactionJournal::new(
            "txn_test",
            "plan_test",
            PathBuf::from("C:\\data"),
            Utc::now(),
        );

        let json = serde_json::to_string(&journal).expect("journal should serialize");
        let restored: TransactionJournal =
            serde_json::from_str(&json).expect("journal should deserialize");

        assert_eq!(restored.schema_version, JOURNAL_SCHEMA_VERSION);
        assert_eq!(restored.transaction_id, "txn_test");
        assert_eq!(restored.plan_id, "plan_test");
    }
}
