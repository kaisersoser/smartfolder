#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate
)]

//! Core folder analysis and organization logic for `smartfolder`.
//!
//! The core crate intentionally avoids terminal UI concerns. It will own the
//! scanner, rule engine, plan generation, transaction journal, apply, and undo
//! logic as the implementation milestones progress.

pub mod apply;
pub mod error;
pub mod model;
pub mod paths;
pub mod planner;
pub mod recovery;
pub mod rules;
pub mod scanner;
pub mod storage;

pub use error::{Result, SmartfolderError};

/// Returns the package version compiled into the core crate.
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
