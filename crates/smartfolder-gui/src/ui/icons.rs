//! Icon tokens for the smartfolder GUI.
//!
//! The v2 desktop UI uses Phosphor icons through `egui-phosphor`. These semantic mappings keep
//! screen code tied to product meaning instead of raw icon symbol names.

/// Folder or organize action icon.
pub(crate) const FOLDER: &str = egui_phosphor::regular::FOLDER_SIMPLE;
/// Generic folder icon for tree views.
pub(crate) const TREE_FOLDER: &str = egui_phosphor::regular::FOLDER_SIMPLE;
/// Generic file icon for tree views.
pub(crate) const TREE_FILE: &str = egui_phosphor::regular::FILE_TEXT;
/// Activity/history icon.
pub(crate) const ACTIVITY: &str = egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE;
/// Rules icon.
pub(crate) const RULES: &str = egui_phosphor::regular::SLIDERS_HORIZONTAL;
/// Settings icon.
pub(crate) const SETTINGS: &str = egui_phosphor::regular::GEAR_SIX;
/// Help or explanatory details icon.
pub(crate) const HELP: &str = egui_phosphor::regular::QUESTION;
/// Expand a collapsed navigation/sidebar panel.
pub(crate) const EXPAND: &str = egui_phosphor::regular::CARET_RIGHT;
/// Collapse an expanded navigation/sidebar panel.
pub(crate) const COLLAPSE: &str = egui_phosphor::regular::CARET_LEFT;

/// Combine an icon glyph with a product label.
pub(crate) fn label(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}
