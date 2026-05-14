#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::redundant_closure_for_method_calls
)]

//! Core folder analysis and organization logic for `smartfolder`.
//!
//! This crate provides the primary functionality for intelligently organizing folder structures
//! by analyzing file properties and applying configurable rules. It intentionally avoids
//! terminal UI concerns, focusing on the core business logic.
//!
//! # Overview
//!
//! The smartfolder workflow consists of three main phases:
//!
//! 1. **Scanning** ([`scanner`]): Recursively scan a directory tree, collecting metadata about files
//!    (size, timestamps, type, path). Supports filtering by depth, file attributes, and exclusions.
//!
//! 2. **Planning** ([`planner`]): Generate a plan of file moves based on built-in modes (type, date, extension)
//!    or custom rule profiles. Each operation includes conflict detection and certainty levels.
//!
//! 3. **Execution** ([`apply`]): Apply the plan to the file system while maintaining a transaction
//!    journal that enables rollback ([`recovery`]). Supports cancellation and progress tracking.
//!
//! # Example workflow
//!
//! ```ignore
//! // 1. Scan the directory
//! let scan = scan_folder("./Downloads", &ScanOptions::default())?;
//!
//! // 2. Generate a plan (e.g., organize by file type)
//! let plan = generate_plan("./Downloads", &scan, &plan_options)?;
//!
//! // 3. Apply the plan with transaction support
//! let summary = apply_plan(&plan, &apply_options)?;
//!
//! // Later: Inspect or rollback the transaction
//! let journal = inspect_transaction(&summary.transaction_id)?;
//! undo_transaction(&journal.transaction_id)?;
//! ```
//!
//! # Modules
//!
//! - [`scanner`]: File system scanning with filtering and metadata collection.
//! - [`planner`]: Plan generation using built-in modes or custom rule profiles.
//! - [`apply`]: Safe plan execution with transaction journaling.
//! - [`recovery`]: Transaction inspection, rollback, and cleanup.
//! - [`rules`]: Rule profile definitions and matching logic.
//! - [`model`]: Core data structures for records, plans, journals, and operations.
//! - [`paths`]: Path validation and normalization to prevent unsafe operations.
//! - [`storage`]: Persistent storage locations for journals and plans.
//! - [`error`]: Error types used throughout the crate.

pub mod ai;
pub mod apply;
pub mod error;
pub mod model;
pub mod paths;
pub mod planner;
pub mod recovery;
pub mod rules;
pub mod scanner;
pub mod session_store;
pub mod storage;

pub use error::{Result, SmartfolderError};

/// Returns the package version compiled into the core crate.
///
/// This version string is determined at compile time from `Cargo.toml` and
/// should match the version exposed by the CLI.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn exposes_package_version() {
        assert!(!version().is_empty());
    }
}
