//! Path validation and normalization for safe file operations.
//!
//! Ensures that destination paths:
//! - Don't escape the root directory (prevents ~/../../etc/passwd attacks)
//! - Don't contain Windows drive prefixes (e.g., `C:`, `\\server\share`)
//! - Are absolute paths starting from the intended root
//!
//! Normalizes paths by:
//! - Removing `.` (current dir) references
//! - Rejecting `..` (parent dir) references
//! - Rejecting absolute paths unless they stay within root
//! - Preserving directory separators appropriate to the platform

use std::path::{Component, Path, PathBuf};

use crate::{Result, SmartfolderError};

/// Compute a safe destination path relative to root.
///
/// Accepts either absolute paths (must stay within root) or relative paths (no `..`).
/// Returns the absolute path ready for file operations.
///
/// # Errors
///
/// - `EmptyDestination`: Destination is empty
/// - `DestinationHasPrefix`: Contains Windows drive letter or UNC prefix
/// - `DestinationEscapesRoot`: Uses `..` or tries to go outside root
pub fn safe_destination_path(
    root: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<PathBuf> {
    let root = normalize_root(root.as_ref());
    let destination = destination.as_ref();

    if destination.as_os_str().is_empty() {
        return Err(SmartfolderError::EmptyDestination);
    }

    if destination.is_absolute() {
        let normalized = normalize_absolute(destination)?;
        if normalized.starts_with(&root) && normalized != root {
            return Ok(normalized);
        }

        return Err(SmartfolderError::DestinationEscapesRoot {
            path: destination.to_path_buf(),
        });
    }

    let relative = normalize_relative(destination)?;
    Ok(root.join(relative))
}

pub fn normalize_relative(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir => {
                return Err(SmartfolderError::DestinationEscapesRoot {
                    path: path.to_path_buf(),
                });
            }
            Component::Prefix(_) => {
                return Err(SmartfolderError::DestinationHasPrefix {
                    path: path.to_path_buf(),
                });
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(SmartfolderError::EmptyDestination);
    }

    Ok(normalized)
}

fn normalize_root(root: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in root.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir
            | Component::Prefix(_)
            | Component::RootDir
            | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    normalized
}

fn normalize_absolute(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(SmartfolderError::DestinationEscapesRoot {
                    path: path.to_path_buf(),
                });
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::paths::{normalize_relative, safe_destination_path};

    fn path(parts: &[&str]) -> PathBuf {
        let mut path = PathBuf::new();
        for part in parts {
            path.push(part);
        }
        path
    }

    fn test_root() -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(r"C:\root")
        }

        #[cfg(not(windows))]
        {
            PathBuf::from("/root")
        }
    }

    fn outside_root() -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(r"C:\other\x.txt")
        }

        #[cfg(not(windows))]
        {
            PathBuf::from("/other/x.txt")
        }
    }

    #[test]
    fn relative_destination_inside_root_is_allowed() {
        let destination = safe_destination_path(test_root(), path(&["Documents", "report.pdf"]))
            .expect("relative destination should be accepted");

        assert_eq!(
            destination,
            test_root().join(path(&["Documents", "report.pdf"]))
        );
    }

    #[test]
    fn parent_components_are_rejected() {
        let err = normalize_relative(path(&["..", "outside.txt"]))
            .expect_err("parent traversal should be rejected");

        assert!(err.to_string().contains("inside the selected root"));
    }

    #[test]
    fn absolute_destination_outside_root_is_rejected() {
        let err = safe_destination_path(test_root(), outside_root())
            .expect_err("outside-root destination should be rejected");

        assert!(err.to_string().contains("inside the selected root"));
    }
}
