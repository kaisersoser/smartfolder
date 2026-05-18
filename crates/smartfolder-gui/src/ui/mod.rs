//! Reusable GUI presentation modules for smartfolder.
//!
//! The GUI is moving from a single-file utility layout to a design-system-backed
//! desktop product. This module tree owns shared visual tokens and widgets so
//! Organize, Activity, Rules, and Settings can converge on one implementation
//! language without changing shared-core behavior.

#[allow(dead_code)]
pub(crate) mod components;
#[allow(dead_code)]
pub(crate) mod icons;
pub(crate) mod screens;
pub(crate) mod shell;
pub(crate) mod theme;
