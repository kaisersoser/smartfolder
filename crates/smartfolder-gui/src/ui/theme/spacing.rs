#![allow(dead_code)]

//! Spacing and sizing tokens for smartfolder GUI layouts.
//!
//! The design system uses a small 4/8-pixel-derived scale so screens can add
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
pub(crate) const XL: f32 = 20.0;
/// Page-level spacing token.
pub(crate) const PAGE: f32 = 24.0;
/// Double-extra-large spacing token.
pub(crate) const XXL: f32 = 32.0;
/// Maximum centered content width for broad pages.
pub(crate) const CONTENT_MAX_WIDTH: f32 = 1120.0;
/// Maximum centered content width for the organize flow and settings.
pub(crate) const FLOW_MAX_WIDTH: f32 = 960.0;
/// Minimum interactive target size for important workflow controls.
pub(crate) const MIN_TARGET: f32 = 40.0;
/// Standard workflow button height.
pub(crate) const CONTROL_HEIGHT: f32 = 36.0;
/// Compact toolbar button height.
pub(crate) const COMPACT_CONTROL_HEIGHT: f32 = 32.0;
/// Default sidebar width from the design system.
pub(crate) const SIDEBAR_WIDTH: f32 = 256.0;
/// Collapsed sidebar rail width.
pub(crate) const SIDEBAR_COLLAPSED_WIDTH: f32 = 56.0;
/// Small radius token.
pub(crate) const RADIUS_SM: f32 = 6.0;
/// Medium radius token.
pub(crate) const RADIUS_MD: f32 = 8.0;
/// Large radius token.
pub(crate) const RADIUS_LG: f32 = 12.0;
/// Pill radius token.
pub(crate) const RADIUS_PILL: f32 = 999.0;
