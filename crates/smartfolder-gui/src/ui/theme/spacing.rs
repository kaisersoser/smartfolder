#![allow(dead_code)]

//! Spacing and sizing tokens for smartfolder GUI layouts.
//!
//! The design system uses a small 8-pixel-derived scale so screens can add
//! breathing room without drifting into arbitrary local spacing.

/// Extra-small spacing token.
pub(crate) const XS: f32 = 4.0;
/// Small spacing token.
pub(crate) const SM: f32 = 8.0;
/// Medium spacing token.
pub(crate) const MD: f32 = 12.0;
/// Large spacing token.
pub(crate) const LG: f32 = 16.0;
/// Extra-large spacing token.
pub(crate) const XL: f32 = 24.0;
/// Double-extra-large spacing token.
pub(crate) const XXL: f32 = 32.0;
/// Maximum centered content width for the main area.
pub(crate) const CONTENT_MAX_WIDTH: f32 = 1200.0;
/// Minimum interactive target size for accessible controls.
pub(crate) const MIN_TARGET: f32 = 40.0;
/// Default sidebar width from the design system.
pub(crate) const SIDEBAR_WIDTH: f32 = 256.0;
