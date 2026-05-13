//! Persistent storage locations for journals and plans.
//!
//! Determines platform-specific app data directories using the `directories` crate.
//! Provides functions to locate journals (transaction records) and plans (organization plans).
//!
//! Default locations:
//! - Windows: `%LOCALAPPDATA%\dev\smartfolder\smartfolder\data`
//! - Linux/Mac: `~/.local/share/smartfolder` or equivalent
//!
//! Can be overridden with `SMARTFOLDER_DATA_DIR` environment variable.

use std::path::PathBuf;

use directories::ProjectDirs;

use crate::{Result, SmartfolderError};

/// Get the app data directory for storing journals and plans.
///
/// Respects `SMARTFOLDER_DATA_DIR` environment variable if set.
///
/// # Errors
///
/// Returns error if directories cannot be determined (e.g., on headless systems).
pub fn app_data_dir() -> Result<PathBuf> {
    if let Some(override_dir) = std::env::var_os("SMARTFOLDER_DATA_DIR") {
        return Ok(PathBuf::from(override_dir));
    }

    let project_dirs = ProjectDirs::from("dev", "smartfolder", "smartfolder")
        .ok_or(SmartfolderError::AppDataDirectoryUnavailable)?;

    Ok(project_dirs.data_local_dir().to_path_buf())
}

/// Get the directory containing transaction journals.
pub fn journals_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("journals"))
}

/// Get the directory containing saved plans.
pub fn plans_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("plans"))
}

/// Get the directory containing saved rule profiles.
pub fn profiles_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("profiles"))
}

/// Get the path to the SQLite working-session database.
///
/// The database stores large scan and plan working sets so GUI workflows can
/// page through results without retaining every row in process memory.
pub fn session_db_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("sessions.sqlite3"))
}

/// Ensure journals directory exists, creating it if necessary.
pub fn ensure_journals_dir() -> Result<PathBuf> {
    let directory = journals_dir()?;
    std::fs::create_dir_all(&directory)
        .map_err(|source| SmartfolderError::io(&directory, source))?;
    Ok(directory)
}

/// Ensure rule profile directory exists, creating it if necessary.
pub fn ensure_profiles_dir() -> Result<PathBuf> {
    let directory = profiles_dir()?;
    std::fs::create_dir_all(&directory)
        .map_err(|source| SmartfolderError::io(&directory, source))?;
    Ok(directory)
}

/// Get the full path to a transaction journal file.
///
/// Validates transaction ID to prevent path traversal attacks.
///
/// # Errors
///
/// Returns error if transaction ID contains path separators or is empty.
pub fn journal_path(transaction_id: &str) -> Result<PathBuf> {
    if transaction_id.trim().is_empty()
        || transaction_id.contains(std::path::MAIN_SEPARATOR)
        || transaction_id.contains('/')
        || transaction_id.contains('\\')
    {
        return Err(SmartfolderError::InvalidTransactionId {
            transaction_id: transaction_id.to_string(),
        });
    }

    Ok(ensure_journals_dir()?.join(format!("{transaction_id}.json")))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::storage::{
        app_data_dir, journal_path, journals_dir, plans_dir, profiles_dir, session_db_path,
    };

    #[test]
    fn storage_dirs_are_under_app_data_dir() {
        let app_data = app_data_dir().expect("app data dir should resolve");

        assert!(journals_dir()
            .expect("journals dir should resolve")
            .starts_with(&app_data));
        assert!(plans_dir()
            .expect("plans dir should resolve")
            .starts_with(&app_data));
        assert!(profiles_dir()
            .expect("profiles dir should resolve")
            .starts_with(&app_data));
        assert!(session_db_path()
            .expect("session db path should resolve")
            .starts_with(&app_data));
    }

    #[test]
    fn journal_path_rejects_path_like_ids() {
        let err = journal_path("..\\evil").expect_err("path-like ids are invalid");

        assert!(err.to_string().contains("transaction journal path"));
    }

    #[test]
    fn app_data_dir_prefers_explicit_override() {
        let key = "SMARTFOLDER_DATA_DIR";
        let original = std::env::var_os(key);
        std::env::set_var(key, "D:\\override-dir");

        let directory = app_data_dir().expect("override dir should resolve");

        assert_eq!(directory, PathBuf::from("D:\\override-dir"));

        match original {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}
