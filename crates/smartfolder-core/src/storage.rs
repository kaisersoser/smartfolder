use std::path::PathBuf;

use directories::ProjectDirs;

use crate::{Result, SmartfolderError};

pub fn app_data_dir() -> Result<PathBuf> {
    let project_dirs = ProjectDirs::from("dev", "smartfolder", "smartfolder")
        .ok_or(SmartfolderError::AppDataDirectoryUnavailable)?;

    Ok(project_dirs.data_local_dir().to_path_buf())
}

pub fn journals_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("journals"))
}

pub fn plans_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("plans"))
}

pub fn ensure_journals_dir() -> Result<PathBuf> {
    let directory = journals_dir()?;
    std::fs::create_dir_all(&directory)
        .map_err(|source| SmartfolderError::io(&directory, source))?;
    Ok(directory)
}

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
    use crate::storage::{app_data_dir, journal_path, journals_dir, plans_dir};

    #[test]
    fn storage_dirs_are_under_app_data_dir() {
        let app_data = app_data_dir().expect("app data dir should resolve");

        assert!(journals_dir()
            .expect("journals dir should resolve")
            .starts_with(&app_data));
        assert!(plans_dir()
            .expect("plans dir should resolve")
            .starts_with(&app_data));
    }

    #[test]
    fn journal_path_rejects_path_like_ids() {
        let err = journal_path("..\\evil").expect_err("path-like ids are invalid");

        assert!(err.to_string().contains("transaction journal path"));
    }
}
