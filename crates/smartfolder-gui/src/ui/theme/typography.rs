#![allow(dead_code)]

//! Typography tokens for the smartfolder GUI.
//!
//! egui does not use CSS-style named text classes, so this module maps the RC2
//! type scale to `TextStyle` entries that can be applied consistently across
//! screens.

use std::collections::BTreeMap;

use eframe::egui::{FontFamily, FontId, TextStyle};

/// Page title font size.
pub(crate) const PAGE_TITLE: f32 = 30.0;
/// Section title font size.
pub(crate) const SECTION_TITLE: f32 = 20.0;
/// Card title font size.
pub(crate) const CARD_TITLE: f32 = 17.0;
/// Body font size.
pub(crate) const BODY: f32 = 14.0;
/// Caption font size.
pub(crate) const CAPTION: f32 = 12.0;

/// Build the egui text-style table for smartfolder screens.
pub(crate) fn text_styles() -> BTreeMap<TextStyle, FontId> {
    BTreeMap::from([
        (
            TextStyle::Heading,
            FontId::new(PAGE_TITLE, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(BODY, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(BODY, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(CAPTION, FontFamily::Proportional),
        ),
        (TextStyle::Monospace, FontId::monospace(BODY)),
    ])
}
