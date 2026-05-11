use std::ffi::OsStr;
use std::fs::{self, DirEntry, Metadata};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use chrono::{DateTime, Utc};

use crate::model::{
    FileEntryKind, FileInventoryRecord, FileTypeBucket, ScanWarning, ScanWarningCode,
};
use crate::{Result, SmartfolderError};

#[cfg(windows)]
const HIDDEN_ATTRIBUTE: u32 = 0x2;
#[cfg(windows)]
const SYSTEM_ATTRIBUTE: u32 = 0x4;

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub include_hidden: bool,
    pub include_system: bool,
    pub include_project_folders: bool,
    pub max_depth: Option<usize>,
    pub current_folder_only: bool,
    pub exclude_names: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub root: PathBuf,
    pub records: Vec<FileInventoryRecord>,
    pub warnings: Vec<ScanWarning>,
    pub summary: ScanSummary,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanSummary {
    pub entries_seen: usize,
    pub records_collected: usize,
    pub entries_skipped: usize,
    pub folders_scanned: usize,
    pub warnings: usize,
}

pub fn scan_folder(root: impl AsRef<Path>, options: &ScanOptions) -> Result<ScanResult> {
    scan_folder_with_cancellation(root, options, &CancellationToken::default())
}

pub fn scan_folder_with_cancellation(
    root: impl AsRef<Path>,
    options: &ScanOptions,
    cancellation: &CancellationToken,
) -> Result<ScanResult> {
    let root = root.as_ref();
    let metadata = fs::metadata(root).map_err(|source| SmartfolderError::io(root, source))?;

    if !metadata.is_dir() {
        return Err(SmartfolderError::ScanRootNotDirectory {
            path: root.to_path_buf(),
        });
    }

    let mut scanner = Scanner {
        root: root.to_path_buf(),
        options,
        cancellation,
        result: ScanResult {
            root: root.to_path_buf(),
            ..ScanResult::default()
        },
    };

    scanner.scan_dir(root, 0)?;
    scanner.result.summary.records_collected = scanner.result.records.len();
    scanner.result.summary.warnings = scanner.result.warnings.len();
    Ok(scanner.result)
}

struct Scanner<'a> {
    root: PathBuf,
    options: &'a ScanOptions,
    cancellation: &'a CancellationToken,
    result: ScanResult,
}

impl Scanner<'_> {
    fn scan_dir(&mut self, directory: &Path, depth: usize) -> Result<()> {
        if self.cancellation.is_cancelled() {
            self.result.cancelled = true;
            return Ok(());
        }

        if self.should_stop_at_depth(depth) {
            return Ok(());
        }

        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(source) => {
                self.warn(
                    ScanWarningCode::UnreadableEntry,
                    Some(directory.to_path_buf()),
                    format!("could not read directory: {source}"),
                );
                return Ok(());
            }
        };

        self.result.summary.folders_scanned += 1;

        for entry in entries {
            if self.cancellation.is_cancelled() {
                self.result.cancelled = true;
                break;
            }

            match entry {
                Ok(entry) => self.scan_entry(&entry, depth + 1)?,
                Err(source) => self.warn(
                    ScanWarningCode::UnreadableEntry,
                    Some(directory.to_path_buf()),
                    format!("could not read directory entry: {source}"),
                ),
            }
        }

        Ok(())
    }

    fn scan_entry(&mut self, entry: &DirEntry, depth: usize) -> Result<()> {
        self.result.summary.entries_seen += 1;

        let path = entry.path();
        let file_name = entry.file_name();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) => {
                self.warn(
                    ScanWarningCode::UnreadableEntry,
                    Some(path),
                    format!("could not read metadata: {source}"),
                );
                return Ok(());
            }
        };

        if self.should_skip(&file_name, &metadata) {
            self.result.summary.entries_skipped += 1;
            return Ok(());
        }

        let entry_kind = entry_kind(&metadata);
        self.result.records.push(record_from_metadata(
            &self.root,
            &path,
            file_name.to_string_lossy().as_ref(),
            &metadata,
            depth,
            entry_kind,
        ));

        if metadata.is_dir() && entry_kind != FileEntryKind::Symlink {
            self.scan_dir(&path, depth)?;
        }

        Ok(())
    }

    fn should_stop_at_depth(&self, depth: usize) -> bool {
        if self.options.current_folder_only && depth > 0 {
            return true;
        }

        self.options
            .max_depth
            .is_some_and(|max_depth| depth >= max_depth)
    }

    fn should_skip(&self, file_name: &OsStr, metadata: &Metadata) -> bool {
        let name = file_name.to_string_lossy();

        if self
            .options
            .exclude_names
            .iter()
            .any(|excluded| excluded.eq_ignore_ascii_case(&name))
        {
            return true;
        }

        if !self.options.include_project_folders && is_default_excluded_name(&name) {
            return true;
        }

        if !self.options.include_hidden && is_hidden_name(&name) {
            return true;
        }

        if !self.options.include_hidden && has_hidden_attribute(metadata) {
            return true;
        }

        !self.options.include_system && has_system_attribute(metadata)
    }

    fn warn(&mut self, code: ScanWarningCode, path: Option<PathBuf>, message: String) {
        self.result.warnings.push(ScanWarning {
            code,
            path,
            message,
        });
    }
}

fn record_from_metadata(
    root: &Path,
    path: &Path,
    name: &str,
    metadata: &Metadata,
    depth: usize,
    entry_kind: FileEntryKind,
) -> FileInventoryRecord {
    let root_relative_path = path
        .strip_prefix(root)
        .map_or_else(|_| path.to_path_buf(), Path::to_path_buf);

    FileInventoryRecord {
        file_id: root_relative_path.to_string_lossy().replace('\\', "/"),
        root_relative_path,
        name: name.to_string(),
        extension: path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase()),
        detected_type: detect_type(path, entry_kind),
        size_bytes: metadata.len(),
        created_at: metadata.created().ok().map(DateTime::<Utc>::from),
        modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
        accessed_at: metadata.accessed().ok().map(DateTime::<Utc>::from),
        depth,
        entry_kind,
        scan_warnings: Vec::new(),
    }
}

fn entry_kind(metadata: &Metadata) -> FileEntryKind {
    let file_type = metadata.file_type();

    if file_type.is_symlink() {
        FileEntryKind::Symlink
    } else if metadata.is_file() {
        FileEntryKind::File
    } else if metadata.is_dir() {
        FileEntryKind::Directory
    } else {
        FileEntryKind::Other
    }
}

fn detect_type(path: &Path, entry_kind: FileEntryKind) -> FileTypeBucket {
    if entry_kind == FileEntryKind::Directory {
        return FileTypeBucket::Directory;
    }

    if entry_kind == FileEntryKind::Symlink {
        return FileTypeBucket::Link;
    }

    match path
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf" | "doc" | "docx" | "txt" | "rtf" | "odt" | "md") => FileTypeBucket::Document,
        Some("jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tiff" | "heic") => {
            FileTypeBucket::Image
        }
        Some("mp4" | "mov" | "mkv" | "avi" | "wmv" | "webm") => FileTypeBucket::Video,
        Some("mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a") => FileTypeBucket::Audio,
        Some("zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz") => FileTypeBucket::Archive,
        Some("xls" | "xlsx" | "ods" | "csv") => FileTypeBucket::Spreadsheet,
        Some("ppt" | "pptx" | "odp") => FileTypeBucket::Presentation,
        Some("rs" | "go" | "py" | "js" | "ts" | "tsx" | "jsx" | "java" | "cs" | "cpp" | "c") => {
            FileTypeBucket::Code
        }
        _ => FileTypeBucket::Other,
    }
}

fn is_default_excluded_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".svn"
            | ".hg"
            | "node_modules"
            | "vendor"
            | "venv"
            | ".venv"
            | "target"
            | "build"
            | "dist"
            | ".idea"
            | ".vscode"
            | "AppData"
            | "Library"
    )
}

fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.') && name != "." && name != ".."
}

#[cfg(windows)]
fn has_hidden_attribute(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes() & HIDDEN_ATTRIBUTE != 0
}

#[cfg(not(windows))]
fn has_hidden_attribute(_metadata: &Metadata) -> bool {
    false
}

#[cfg(windows)]
fn has_system_attribute(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes() & SYSTEM_ATTRIBUTE != 0
}

#[cfg(not(windows))]
fn has_system_attribute(_metadata: &Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use crate::model::{FileEntryKind, FileTypeBucket};
    use crate::scanner::{
        is_default_excluded_name, scan_folder, scan_folder_with_cancellation, CancellationToken,
        ScanOptions,
    };

    #[test]
    fn scans_metadata_without_file_contents() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("report.pdf"), b"content").expect("fixture write");
        fs::create_dir(fixture.path().join("nested")).expect("fixture dir");
        fs::write(
            fixture.path().join("nested").join("main.rs"),
            b"fn main() {}",
        )
        .expect("fixture write");

        let result = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");

        assert_eq!(result.summary.records_collected, 3);
        assert!(result
            .records
            .iter()
            .any(|record| record.detected_type == FileTypeBucket::Document));
        assert!(result
            .records
            .iter()
            .any(|record| record.detected_type == FileTypeBucket::Code));
    }

    #[test]
    fn excludes_project_folders_by_default() {
        let fixture = fixture_dir();
        fs::create_dir(fixture.path().join(".git")).expect("fixture dir");
        fs::write(fixture.path().join(".git").join("config"), b"config").expect("fixture write");
        fs::write(fixture.path().join("keep.txt"), b"keep").expect("fixture write");

        let result = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");

        assert_eq!(result.summary.entries_skipped, 1);
        assert!(result
            .records
            .iter()
            .all(|record| !record.root_relative_path.starts_with(Path::new(".git"))));
    }

    #[test]
    fn include_project_folders_flag_overrides_default_exclusions() {
        let fixture = fixture_dir();
        fs::create_dir(fixture.path().join("target")).expect("fixture dir");
        fs::write(
            fixture.path().join("target").join("artifact.bin"),
            b"artifact",
        )
        .expect("fixture write");

        let options = ScanOptions {
            include_project_folders: true,
            ..ScanOptions::default()
        };
        let result = scan_folder(fixture.path(), &options).expect("scan succeeds");

        assert!(result
            .records
            .iter()
            .any(|record| record.root_relative_path.starts_with(Path::new("target"))));
    }

    #[test]
    fn max_depth_limits_recursion() {
        let fixture = fixture_dir();
        fs::create_dir(fixture.path().join("nested")).expect("fixture dir");
        fs::write(fixture.path().join("nested").join("too_deep.txt"), b"deep")
            .expect("fixture write");

        let options = ScanOptions {
            max_depth: Some(1),
            ..ScanOptions::default()
        };
        let result = scan_folder(fixture.path(), &options).expect("scan succeeds");

        assert!(result.records.iter().any(|record| record.name == "nested"));
        assert!(!result
            .records
            .iter()
            .any(|record| record.name == "too_deep.txt"));
    }

    #[test]
    fn cancellation_returns_partial_result() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("first.txt"), b"first").expect("fixture write");
        let token = CancellationToken::default();
        token.cancel();

        let result = scan_folder_with_cancellation(fixture.path(), &ScanOptions::default(), &token)
            .expect("cancelled scan returns result");

        assert!(result.cancelled);
        assert!(result.records.is_empty());
    }

    #[test]
    fn symlink_entries_are_not_followed_when_creation_is_available() {
        let fixture = fixture_dir();
        fs::write(fixture.path().join("target.txt"), b"target").expect("fixture write");
        let link = fixture.path().join("link.txt");

        if create_file_symlink(fixture.path().join("target.txt"), &link).is_err() {
            return;
        }

        let result = scan_folder(fixture.path(), &ScanOptions::default()).expect("scan succeeds");

        assert!(
            result
                .records
                .iter()
                .any(|record| record.name == "link.txt"
                    && record.entry_kind == FileEntryKind::Symlink)
        );
    }

    #[test]
    fn known_project_exclusions_are_documented_in_code() {
        assert!(is_default_excluded_name(".git"));
        assert!(is_default_excluded_name("node_modules"));
        assert!(is_default_excluded_name("target"));
    }

    fn fixture_dir() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[cfg(windows)]
    fn create_file_symlink(
        original: impl AsRef<Path>,
        link: impl AsRef<Path>,
    ) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(original, link)
    }

    #[cfg(unix)]
    fn create_file_symlink(
        original: impl AsRef<Path>,
        link: impl AsRef<Path>,
    ) -> std::io::Result<()> {
        std::os::unix::fs::symlink(original, link)
    }

    #[cfg(not(any(unix, windows)))]
    fn create_file_symlink(
        _original: impl AsRef<Path>,
        _link: impl AsRef<Path>,
    ) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "symlink creation is not supported on this platform",
        ))
    }
}
