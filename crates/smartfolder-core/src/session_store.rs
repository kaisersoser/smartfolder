//! SQLite-backed working storage for large scan and plan sessions.
//!
//! The original v1 flow stores scan records and plan operations in memory. That
//! is simple for CLI-sized runs, but it is a poor fit for long-lived GUI sessions
//! over large folders. This module stores the large row-oriented working sets in
//! SQLite while keeping the public domain models unchanged.
//!
//! # Workflow
//!
//! 1. Open a [`SqliteSessionStore`] from the app-local database path.
//! 2. Create a session for a root folder and planning mode.
//! 3. Stream scan records into the store.
//! 4. Stream plan operations into the store.
//! 5. Query summaries or pages of operations for GUI display.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::model::{
    FileInventoryRecord, PlanMode, PlanOperation, PlanSummary, PlanWarning, ScanWarning,
};
use crate::scanner::{ScanRecordSink, ScanSummary};
use crate::storage::session_db_path;
use crate::{Result, SmartfolderError};

const SCHEMA_VERSION: u16 = 1;

/// SQLite store for GUI and large-folder working sessions.
///
/// The store is intentionally row-oriented. It keeps large collections, such as
/// scan records and plan operations, on disk and exposes page-oriented queries so
/// callers do not need to materialize every row in memory.
pub struct SqliteSessionStore {
    connection: Connection,
}

/// Filter used when loading stored plan operations for paged UI previews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanOperationFilter {
    All,
    Ready,
    NeedsAttention,
}

impl SqliteSessionStore {
    /// Open the default app-local session database.
    ///
    /// # Errors
    ///
    /// Returns an error if the app data directory cannot be resolved, the parent
    /// directory cannot be created, or SQLite cannot open or migrate the file.
    pub fn open_default() -> Result<Self> {
        let path = session_db_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| SmartfolderError::io(parent, source))?;
        }
        Self::open(path)
    }

    /// Open a session database at a specific path.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot open the database or initialize schema.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        let store = Self { connection };
        store.initialize()?;
        Ok(store)
    }

    /// Open an in-memory session database for tests.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot create or initialize the database.
    pub fn in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        let store = Self { connection };
        store.initialize()?;
        Ok(store)
    }

    /// Create a new working session and return its identifier.
    ///
    /// # Errors
    ///
    /// Returns an error if the mode cannot be serialized or the row cannot be inserted.
    pub fn create_session(
        &mut self,
        root: &Path,
        mode: &PlanMode,
        created_at: DateTime<Utc>,
    ) -> Result<String> {
        let session_id = format!(
            "session_{}_{:09}",
            created_at.format("%Y%m%d%H%M%S"),
            created_at.timestamp_subsec_nanos()
        );
        self.create_session_with_id(&session_id, root, mode, created_at)?;
        Ok(session_id)
    }

    /// Create a working session with a caller-provided identifier.
    ///
    /// # Errors
    ///
    /// Returns an error if the mode cannot be serialized or the row cannot be inserted.
    pub fn create_session_with_id(
        &mut self,
        session_id: &str,
        root: &Path,
        mode: &PlanMode,
        created_at: DateTime<Utc>,
    ) -> Result<()> {
        let mode_json = serde_json::to_string(mode)?;
        self.connection.execute(
            "INSERT OR REPLACE INTO sessions \
             (session_id, root, mode_json, created_at, schema_version) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                path_to_string(root),
                mode_json,
                created_at.to_rfc3339(),
                SCHEMA_VERSION
            ],
        )?;
        Ok(())
    }

    /// Begin a batched write transaction for large scan or plan inserts.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot start the transaction.
    pub fn begin_write_batch(&mut self) -> Result<()> {
        if self.connection.is_autocommit() {
            self.connection.execute_batch("BEGIN IMMEDIATE")?;
        }
        Ok(())
    }

    /// Commit a batched write transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot commit the transaction.
    pub fn commit_write_batch(&mut self) -> Result<()> {
        if !self.connection.is_autocommit() {
            self.connection.execute_batch("COMMIT")?;
        }
        Ok(())
    }

    /// Roll back a batched write transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot roll back the transaction.
    pub fn rollback_write_batch(&mut self) -> Result<()> {
        if !self.connection.is_autocommit() {
            self.connection.execute_batch("ROLLBACK")?;
        }
        Ok(())
    }

    /// Insert one scan record into the session.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or SQLite insertion fails.
    pub fn insert_scan_record(
        &mut self,
        session_id: &str,
        record: &FileInventoryRecord,
    ) -> Result<()> {
        let record_json = serde_json::to_string(record)?;
        self.connection.execute(
            "INSERT OR REPLACE INTO scan_records \
             (session_id, file_id, root_relative_path, name, extension, detected_type, \
              size_bytes, modified_at, depth, entry_kind, record_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                session_id,
                record.file_id,
                path_to_string(&record.root_relative_path),
                record.name,
                record.extension,
                serde_json::to_string(&record.detected_type)?,
                record.size_bytes,
                record.modified_at.map(|value| value.to_rfc3339()),
                record.depth as i64,
                serde_json::to_string(&record.entry_kind)?,
                record_json
            ],
        )?;
        Ok(())
    }

    /// Insert one scan warning into the session.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or SQLite insertion fails.
    pub fn insert_scan_warning(&mut self, session_id: &str, warning: &ScanWarning) -> Result<()> {
        let warning_json = serde_json::to_string(warning)?;
        self.connection.execute(
            "INSERT INTO scan_warnings (session_id, warning_json) VALUES (?1, ?2)",
            params![session_id, warning_json],
        )?;
        Ok(())
    }

    /// Persist aggregate scan counters for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite insertion fails.
    pub fn save_scan_summary(&mut self, session_id: &str, summary: &ScanSummary) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO scan_summaries \
             (session_id, entries_seen, records_collected, entries_skipped, folders_scanned, warnings) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                summary.entries_seen as i64,
                summary.records_collected as i64,
                summary.entries_skipped as i64,
                summary.folders_scanned as i64,
                summary.warnings as i64
            ],
        )?;
        Ok(())
    }

    /// Load aggregate scan counters for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite query fails.
    pub fn scan_summary(&self, session_id: &str) -> Result<Option<ScanSummary>> {
        self.connection
            .query_row(
                "SELECT entries_seen, records_collected, entries_skipped, folders_scanned, warnings \
                 FROM scan_summaries WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok(ScanSummary {
                        entries_seen: row.get::<_, i64>(0)? as usize,
                        records_collected: row.get::<_, i64>(1)? as usize,
                        entries_skipped: row.get::<_, i64>(2)? as usize,
                        folders_scanned: row.get::<_, i64>(3)? as usize,
                        warnings: row.get::<_, i64>(4)? as usize,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Load a page of scan records for planning or inspection.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite query or JSON decoding fails.
    pub fn scan_records_page(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<FileInventoryRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT record_json FROM scan_records \
             WHERE session_id = ?1 ORDER BY rowid LIMIT ?2 OFFSET ?3",
        )?;
        let rows = statement
            .query_map(params![session_id, limit as i64, offset as i64], |row| {
                row.get::<_, String>(0)
            })?;

        let mut records = Vec::new();
        for row in rows {
            records.push(serde_json::from_str(&row?)?);
        }
        Ok(records)
    }

    /// Remove all plan rows for a session before regenerating a plan.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite deletion fails.
    pub fn clear_plan(&mut self, session_id: &str) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM plan_operations WHERE session_id = ?1",
            params![session_id],
        )?;
        transaction.execute(
            "DELETE FROM ambiguous_files WHERE session_id = ?1",
            params![session_id],
        )?;
        transaction.execute(
            "DELETE FROM plan_warnings WHERE session_id = ?1",
            params![session_id],
        )?;
        transaction.execute(
            "DELETE FROM plan_summaries WHERE session_id = ?1",
            params![session_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Check whether a destination has already been planned for the session.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite query fails.
    pub fn destination_key_exists(&self, session_id: &str, destination_key: &str) -> Result<bool> {
        let exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM plan_operations WHERE session_id = ?1 AND destination_key = ?2)",
            params![session_id, destination_key],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(exists != 0)
    }

    /// Insert one plan operation into the session.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or SQLite insertion fails.
    pub fn insert_plan_operation(
        &mut self,
        session_id: &str,
        operation: &PlanOperation,
        destination_key: &str,
    ) -> Result<()> {
        let operation_json = serde_json::to_string(operation)?;
        self.connection.execute(
            "INSERT OR REPLACE INTO plan_operations \
             (session_id, operation_id, source, destination, destination_key, selected, conflict_state, operation_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id,
                operation.operation_id,
                path_to_string(&operation.source),
                path_to_string(&operation.destination),
                destination_key,
                i64::from(operation.selected),
                serde_json::to_string(&operation.conflict)?,
                operation_json
            ],
        )?;
        Ok(())
    }

    /// Insert one ambiguous file path into the session.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite insertion fails.
    pub fn insert_ambiguous_file(&mut self, session_id: &str, path: &Path) -> Result<()> {
        self.connection.execute(
            "INSERT INTO ambiguous_files (session_id, path) VALUES (?1, ?2)",
            params![session_id, path_to_string(path)],
        )?;
        Ok(())
    }

    /// Insert one plan warning into the session.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or SQLite insertion fails.
    pub fn insert_plan_warning(&mut self, session_id: &str, warning: &PlanWarning) -> Result<()> {
        let warning_json = serde_json::to_string(warning)?;
        self.connection.execute(
            "INSERT INTO plan_warnings (session_id, warning_json) VALUES (?1, ?2)",
            params![session_id, warning_json],
        )?;
        Ok(())
    }

    /// Persist aggregate plan counters for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite insertion fails.
    pub fn save_plan_summary(&mut self, session_id: &str, summary: &PlanSummary) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO plan_summaries \
             (session_id, files_scanned, moves_proposed, ambiguous_files, conflicts, skipped) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id,
                summary.files_scanned as i64,
                summary.moves_proposed as i64,
                summary.ambiguous_files as i64,
                summary.conflicts as i64,
                summary.skipped as i64
            ],
        )?;
        Ok(())
    }

    /// Load aggregate plan counters for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite query fails.
    pub fn plan_summary(&self, session_id: &str) -> Result<Option<PlanSummary>> {
        self.connection
            .query_row(
                "SELECT files_scanned, moves_proposed, ambiguous_files, conflicts, skipped \
                 FROM plan_summaries WHERE session_id = ?1",
                params![session_id],
                |row| {
                    Ok(PlanSummary {
                        files_scanned: row.get::<_, i64>(0)? as usize,
                        moves_proposed: row.get::<_, i64>(1)? as usize,
                        ambiguous_files: row.get::<_, i64>(2)? as usize,
                        conflicts: row.get::<_, i64>(3)? as usize,
                        skipped: row.get::<_, i64>(4)? as usize,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// Load a page of plan operations for GUI preview.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite query or JSON decoding fails.
    pub fn plan_operations_page(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<PlanOperation>> {
        self.plan_operations_page_filtered(session_id, PlanOperationFilter::All, offset, limit)
    }

    /// Count stored plan operations for a GUI preview filter.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite query fails.
    pub fn plan_operation_count(
        &self,
        session_id: &str,
        filter: PlanOperationFilter,
    ) -> Result<usize> {
        let count = match filter {
            PlanOperationFilter::All => self.connection.query_row(
                "SELECT COUNT(*) FROM plan_operations WHERE session_id = ?1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )?,
            PlanOperationFilter::Ready => self.connection.query_row(
                "SELECT COUNT(*) FROM plan_operations WHERE session_id = ?1 AND selected = 1",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )?,
            PlanOperationFilter::NeedsAttention => self.connection.query_row(
                "SELECT COUNT(*) FROM plan_operations WHERE session_id = ?1 AND selected = 0",
                params![session_id],
                |row| row.get::<_, i64>(0),
            )?,
        };
        Ok(count as usize)
    }

    /// Load a filtered page of plan operations for GUI preview.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite query or JSON decoding fails.
    pub fn plan_operations_page_filtered(
        &self,
        session_id: &str,
        filter: PlanOperationFilter,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<PlanOperation>> {
        let mut statement = self.connection.prepare(match filter {
            PlanOperationFilter::All => {
                "SELECT operation_json FROM plan_operations \
                     WHERE session_id = ?1 ORDER BY rowid LIMIT ?2 OFFSET ?3"
            }
            PlanOperationFilter::Ready => {
                "SELECT operation_json FROM plan_operations \
                     WHERE session_id = ?1 AND selected = 1 ORDER BY rowid LIMIT ?2 OFFSET ?3"
            }
            PlanOperationFilter::NeedsAttention => {
                "SELECT operation_json FROM plan_operations \
                     WHERE session_id = ?1 AND selected = 0 ORDER BY rowid LIMIT ?2 OFFSET ?3"
            }
        })?;
        let rows = statement
            .query_map(params![session_id, limit as i64, offset as i64], |row| {
                row.get::<_, String>(0)
            })?;

        let mut operations = Vec::new();
        for row in rows {
            operations.push(serde_json::from_str(&row?)?);
        }
        Ok(operations)
    }

    /// Load representative ready operations for a simplified GUI preview.
    ///
    /// The returned rows favor diverse file extensions and destination folders
    /// before falling back to insertion order. This gives the GUI a small,
    /// metadata-only example set without materializing the full preview table.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite query or JSON decoding fails.
    pub fn representative_plan_examples(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<PlanOperation>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let candidate_limit = (limit * 64).clamp(32, 512);
        let candidates = self.plan_operations_page_filtered(
            session_id,
            PlanOperationFilter::Ready,
            0,
            candidate_limit,
        )?;
        Ok(select_representative_operations(candidates, limit))
    }

    /// Load user-facing warning messages for a session.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite query or JSON decoding fails.
    pub fn warning_messages(&self, session_id: &str) -> Result<Vec<String>> {
        let mut messages = Vec::new();
        let mut scan_statement = self.connection.prepare(
            "SELECT warning_json FROM scan_warnings WHERE session_id = ?1 ORDER BY rowid",
        )?;
        let scan_rows =
            scan_statement.query_map(params![session_id], |row| row.get::<_, String>(0))?;
        for row in scan_rows {
            let warning: ScanWarning = serde_json::from_str(&row?)?;
            messages.push(warning.message);
        }

        let mut plan_statement = self.connection.prepare(
            "SELECT warning_json FROM plan_warnings WHERE session_id = ?1 ORDER BY rowid",
        )?;
        let plan_rows =
            plan_statement.query_map(params![session_id], |row| row.get::<_, String>(0))?;
        for row in plan_rows {
            let warning: PlanWarning = serde_json::from_str(&row?)?;
            messages.push(warning.message);
        }

        Ok(messages)
    }

    /// Delete a working session and all scan/plan rows attached to it.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite deletion fails.
    pub fn delete_session(&mut self, session_id: &str) -> Result<usize> {
        let removed = self.connection.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(removed)
    }

    /// Delete working sessions created before a cutoff timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite deletion fails.
    pub fn cleanup_sessions_before(&mut self, cutoff: DateTime<Utc>) -> Result<usize> {
        let removed = self.connection.execute(
            "DELETE FROM sessions WHERE created_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        Ok(removed)
    }

    /// Compact the database file after deleting old sessions.
    ///
    /// # Errors
    ///
    /// Returns an error if SQLite cannot vacuum the database.
    pub fn compact(&mut self) -> Result<()> {
        self.connection.execute_batch("VACUUM")?;
        Ok(())
    }

    fn initialize(&self) -> Result<()> {
        self.connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             CREATE TABLE IF NOT EXISTS sessions (
                 session_id TEXT PRIMARY KEY,
                 root TEXT NOT NULL,
                 mode_json TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 schema_version INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS scan_records (
                 session_id TEXT NOT NULL,
                 file_id TEXT NOT NULL,
                 root_relative_path TEXT NOT NULL,
                 name TEXT NOT NULL,
                 extension TEXT,
                 detected_type TEXT NOT NULL,
                 size_bytes INTEGER NOT NULL,
                 modified_at TEXT,
                 depth INTEGER NOT NULL,
                 entry_kind TEXT NOT NULL,
                 record_json TEXT NOT NULL,
                 PRIMARY KEY (session_id, file_id),
                 FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_scan_records_type ON scan_records(session_id, detected_type);
             CREATE TABLE IF NOT EXISTS scan_warnings (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL,
                 warning_json TEXT NOT NULL,
                 FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS scan_summaries (
                 session_id TEXT PRIMARY KEY,
                 entries_seen INTEGER NOT NULL,
                 records_collected INTEGER NOT NULL,
                 entries_skipped INTEGER NOT NULL,
                 folders_scanned INTEGER NOT NULL,
                 warnings INTEGER NOT NULL,
                 FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS plan_operations (
                 session_id TEXT NOT NULL,
                 operation_id TEXT NOT NULL,
                 source TEXT NOT NULL,
                 destination TEXT NOT NULL,
                 destination_key TEXT NOT NULL,
                 selected INTEGER NOT NULL,
                 conflict_state TEXT NOT NULL,
                 operation_json TEXT NOT NULL,
                 PRIMARY KEY (session_id, operation_id),
                 FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_plan_operations_destination ON plan_operations(session_id, destination_key);
             CREATE INDEX IF NOT EXISTS idx_plan_operations_selected ON plan_operations(session_id, selected);
             CREATE TABLE IF NOT EXISTS ambiguous_files (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL,
                 path TEXT NOT NULL,
                 FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS plan_warnings (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL,
                 warning_json TEXT NOT NULL,
                 FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
             );
             CREATE TABLE IF NOT EXISTS plan_summaries (
                 session_id TEXT PRIMARY KEY,
                 files_scanned INTEGER NOT NULL,
                 moves_proposed INTEGER NOT NULL,
                 ambiguous_files INTEGER NOT NULL,
                 conflicts INTEGER NOT NULL,
                 skipped INTEGER NOT NULL,
                 FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
             );",
        )?;
        Ok(())
    }
}

/// Streaming sink that writes scanner output into a SQLite session.
pub struct SessionScanSink<'a> {
    store: &'a mut SqliteSessionStore,
    session_id: String,
}

impl<'a> SessionScanSink<'a> {
    /// Create a scan sink for a session.
    pub fn new(store: &'a mut SqliteSessionStore, session_id: impl Into<String>) -> Self {
        Self {
            store,
            session_id: session_id.into(),
        }
    }
}

impl ScanRecordSink for SessionScanSink<'_> {
    fn push_record(&mut self, record: FileInventoryRecord) -> Result<()> {
        self.store.insert_scan_record(&self.session_id, &record)
    }

    fn push_warning(&mut self, warning: ScanWarning) -> Result<()> {
        self.store.insert_scan_warning(&self.session_id, &warning)
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn select_representative_operations(
    candidates: Vec<PlanOperation>,
    limit: usize,
) -> Vec<PlanOperation> {
    let mut examples = Vec::new();
    let mut seen_keys = BTreeSet::new();
    let mut remaining = Vec::new();

    for operation in candidates {
        let key = representative_operation_key(&operation);
        if examples.len() < limit && seen_keys.insert(key) {
            examples.push(operation);
        } else {
            remaining.push(operation);
        }
    }

    for operation in remaining {
        if examples.len() >= limit {
            break;
        }
        examples.push(operation);
    }

    examples
}

fn representative_operation_key(operation: &PlanOperation) -> String {
    let extension = operation
        .source
        .extension()
        .and_then(|value| value.to_str())
        .map_or_else(
            || "no-extension".to_string(),
            |value| value.to_ascii_lowercase(),
        );
    let destination_group = operation
        .destination
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .map_or_else(String::new, ToString::to_string);
    format!("{extension}:{destination_group}")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    use crate::model::{
        BuiltInMode, Certainty, ConflictState, OperationType, PlanMode, PlanOperation,
        SourceSnapshot,
    };
    use crate::scanner::{scan_folder_to_sink, CancellationToken, ScanOptions};
    use crate::session_store::{PlanOperationFilter, SessionScanSink, SqliteSessionStore};

    #[test]
    fn sqlite_store_persists_scan_records_and_summary() {
        let fixture = TempDir::new().expect("temp dir");
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let mut store = SqliteSessionStore::in_memory().expect("store opens");
        let created_at = Utc.with_ymd_and_hms(2026, 5, 12, 12, 0, 0).unwrap();
        store
            .create_session_with_id(
                "session_test",
                fixture.path(),
                &PlanMode::BuiltIn(BuiltInMode::Type),
                created_at,
            )
            .expect("session creates");

        let mut sink = SessionScanSink::new(&mut store, "session_test");
        let result = scan_folder_to_sink(
            fixture.path(),
            &ScanOptions::default(),
            &CancellationToken::default(),
            &mut sink,
        )
        .expect("scan streams");
        drop(sink);
        store
            .save_scan_summary("session_test", &result.summary)
            .expect("summary saves");

        let records = store
            .scan_records_page("session_test", 0, 10)
            .expect("records load");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "report.pdf");
        assert_eq!(
            store
                .scan_summary("session_test")
                .expect("summary loads")
                .expect("summary exists")
                .records_collected,
            1
        );
    }

    #[test]
    fn cleanup_removes_old_sessions_and_child_rows() {
        let fixture = TempDir::new().expect("temp dir");
        fs::write(fixture.path().join("report.pdf"), b"report").expect("fixture write");
        let mut store = SqliteSessionStore::in_memory().expect("store opens");
        let old_time = Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap();
        let new_time = Utc.with_ymd_and_hms(2026, 5, 12, 12, 0, 0).unwrap();
        store
            .create_session_with_id(
                "session_old",
                fixture.path(),
                &PlanMode::BuiltIn(BuiltInMode::Type),
                old_time,
            )
            .expect("old session creates");
        store
            .create_session_with_id(
                "session_new",
                fixture.path(),
                &PlanMode::BuiltIn(BuiltInMode::Type),
                new_time,
            )
            .expect("new session creates");

        let removed = store
            .cleanup_sessions_before(Utc.with_ymd_and_hms(2026, 5, 10, 0, 0, 0).unwrap())
            .expect("cleanup succeeds");

        assert_eq!(removed, 1);
        assert!(store
            .scan_records_page("session_old", 0, 10)
            .expect("old records query succeeds")
            .is_empty());
        assert!(
            store
                .delete_session("session_new")
                .expect("delete succeeds")
                == 1
        );
    }

    #[test]
    fn filtered_plan_pages_count_ready_and_attention_rows() {
        let fixture = TempDir::new().expect("temp dir");
        let mut store = SqliteSessionStore::in_memory().expect("store opens");
        let created_at = Utc.with_ymd_and_hms(2026, 5, 12, 12, 0, 0).unwrap();
        store
            .create_session_with_id(
                "session_plan_filters",
                fixture.path(),
                &PlanMode::BuiltIn(BuiltInMode::Type),
                created_at,
            )
            .expect("session creates");

        let ready_operation = PlanOperation {
            operation_id: "op_000001".to_string(),
            operation_type: OperationType::Move,
            source: fixture.path().join("report.pdf"),
            destination: fixture.path().join("Documents").join("report.pdf"),
            reason: "Built-in rule: Type".to_string(),
            certainty: Certainty::High,
            conflict: ConflictState::None,
            selected: true,
            source_snapshot: SourceSnapshot {
                size_bytes: 10,
                modified_at: None,
            },
        };
        let conflict_operation = PlanOperation {
            operation_id: "op_000002".to_string(),
            operation_type: OperationType::Move,
            source: fixture.path().join("duplicate.pdf"),
            destination: fixture.path().join("Documents").join("report.pdf"),
            reason: "Built-in rule: Type".to_string(),
            certainty: Certainty::High,
            conflict: ConflictState::DestinationExists {
                path: PathBuf::from("Documents/report.pdf"),
            },
            selected: false,
            source_snapshot: SourceSnapshot {
                size_bytes: 20,
                modified_at: None,
            },
        };

        store
            .insert_plan_operation(
                "session_plan_filters",
                &ready_operation,
                "documents/report.pdf",
            )
            .expect("ready operation inserts");
        store
            .insert_plan_operation(
                "session_plan_filters",
                &conflict_operation,
                "documents/report-copy.pdf",
            )
            .expect("conflict operation inserts");

        assert_eq!(
            store
                .plan_operation_count("session_plan_filters", PlanOperationFilter::All)
                .expect("all count loads"),
            2
        );
        assert_eq!(
            store
                .plan_operation_count("session_plan_filters", PlanOperationFilter::Ready)
                .expect("ready count loads"),
            1
        );
        assert_eq!(
            store
                .plan_operation_count("session_plan_filters", PlanOperationFilter::NeedsAttention)
                .expect("attention count loads"),
            1
        );

        let ready_page = store
            .plan_operations_page_filtered(
                "session_plan_filters",
                PlanOperationFilter::Ready,
                0,
                10,
            )
            .expect("ready page loads");
        assert_eq!(ready_page, vec![ready_operation]);

        let attention_page = store
            .plan_operations_page_filtered(
                "session_plan_filters",
                PlanOperationFilter::NeedsAttention,
                0,
                10,
            )
            .expect("attention page loads");
        assert_eq!(attention_page, vec![conflict_operation]);
    }

    #[test]
    fn representative_examples_prefer_diverse_ready_operations() {
        let fixture = TempDir::new().expect("temp dir");
        let mut store = SqliteSessionStore::in_memory().expect("store opens");
        let created_at = Utc.with_ymd_and_hms(2026, 5, 12, 12, 0, 0).unwrap();
        store
            .create_session_with_id(
                "session_examples",
                fixture.path(),
                &PlanMode::BuiltIn(BuiltInMode::Type),
                created_at,
            )
            .expect("session creates");

        for (index, (name, destination)) in [
            ("report.pdf", "Documents"),
            ("notes.pdf", "Documents"),
            ("photo.jpg", "Images"),
            ("clip.mp4", "Videos"),
        ]
        .into_iter()
        .enumerate()
        {
            let operation = PlanOperation {
                operation_id: format!("op_{index:06}"),
                operation_type: OperationType::Move,
                source: fixture.path().join(name),
                destination: fixture.path().join(destination).join(name),
                reason: "Built-in rule: Type".to_string(),
                certainty: Certainty::High,
                conflict: ConflictState::None,
                selected: true,
                source_snapshot: SourceSnapshot {
                    size_bytes: 10,
                    modified_at: None,
                },
            };
            store
                .insert_plan_operation(
                    "session_examples",
                    &operation,
                    &format!("{destination}/{name}"),
                )
                .expect("operation inserts");
        }

        let examples = store
            .representative_plan_examples("session_examples", 3)
            .expect("examples load");

        assert_eq!(examples.len(), 3);
        assert_eq!(examples[0].source, fixture.path().join("report.pdf"));
        assert_eq!(examples[1].source, fixture.path().join("photo.jpg"));
        assert_eq!(examples[2].source, fixture.path().join("clip.mp4"));
    }
}
