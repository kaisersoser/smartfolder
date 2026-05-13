//! Icon tokens for the smartfolder GUI.
//!
//! RC2 uses Phosphor icons through `egui-phosphor`. These semantic mappings keep
//! screen code tied to product meaning instead of raw icon symbol names.

/// Folder or organize action icon.
pub(crate) const FOLDER: &str = egui_phosphor::regular::FOLDER_SIMPLE;
/// Activity/history icon.
pub(crate) const ACTIVITY: &str = egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE;
/// Rules icon.
pub(crate) const RULES: &str = egui_phosphor::regular::SLIDERS_HORIZONTAL;
/// Settings icon.
pub(crate) const SETTINGS: &str = egui_phosphor::regular::GEAR_SIX;

/// Combine an icon glyph with a product label.
pub(crate) fn label(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}
