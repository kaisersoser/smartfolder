//! GUI preference persistence for smartfolder.
//!
//! Preferences are stored as a small JSON document under the core app-local
//! configuration directory. They cover appearance and wizard defaults only;
//! analysis sessions, plans, and restore history remain owned by the shared core
//! storage and recovery systems.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use smartfolder_core::model::BuiltInMode;
use smartfolder_core::storage::{ensure_config_dir, gui_preferences_path};

use crate::ui::theme::VisualTheme;

const RECENT_FOLDER_LIMIT: usize = 8;

/// Persisted user preference document for the desktop GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct GuiPreferences {
    /// Preferred application theme.
    pub(crate) theme: ThemePreference,
    /// Preferred motion level for transitions and animated feedback.
    pub(crate) motion: MotionPreference,
    /// Recently selected folder roots for the wizard folder step.
    pub(crate) recent_folders: Vec<PathBuf>,
    /// Last chosen organization style.
    pub(crate) last_style: StylePreference,
    /// Whether advanced wizard controls should default open.
    pub(crate) advanced_options_open: bool,
}

impl Default for GuiPreferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            motion: MotionPreference::Subtle,
            recent_folders: Vec::new(),
            last_style: StylePreference::TypeYear,
            advanced_options_open: false,
        }
    }
}

impl GuiPreferences {
    /// Load preferences from the app-local configuration file.
    pub(crate) fn load() -> std::result::Result<Self, String> {
        let path = gui_preferences_path()
            .map_err(|error| format!("Failed to resolve GUI preferences path: {error}"))?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(&path).map_err(|error| {
            format!("Failed to read GUI preferences {}: {error}", path.display())
        })?;
        serde_json::from_str(&content).map_err(|error| {
            format!(
                "Failed to parse GUI preferences {}: {error}",
                path.display()
            )
        })
    }

    /// Save preferences to the app-local configuration file.
    pub(crate) fn save(&self) -> std::result::Result<(), String> {
        let directory = ensure_config_dir()
            .map_err(|error| format!("Failed to create GUI preferences directory: {error}"))?;
        let path = gui_preferences_path()
            .map_err(|error| format!("Failed to resolve GUI preferences path: {error}"))?;
        let content = serde_json::to_string_pretty(self)
            .map_err(|error| format!("Failed to serialize GUI preferences: {error}"))?;
        fs::write(&path, content).map_err(|error| {
            format!(
                "Failed to write GUI preferences in {}: {error}",
                directory.display()
            )
        })
    }

    /// Resolve the current theme to a concrete visual theme.
    pub(crate) fn visual_theme(&self, system_theme: Option<eframe::Theme>) -> VisualTheme {
        match self.theme {
            ThemePreference::System => match system_theme {
                Some(eframe::Theme::Dark) => VisualTheme::Dark,
                Some(eframe::Theme::Light) | None => VisualTheme::Light,
            },
            ThemePreference::Light => VisualTheme::Light,
            ThemePreference::Dark => VisualTheme::Dark,
        }
    }

    /// Add a folder to the recent-folder list, keeping newest entries first.
    pub(crate) fn remember_folder(&mut self, folder: impl AsRef<Path>) {
        let folder = folder.as_ref().to_path_buf();
        self.recent_folders.retain(|entry| entry != &folder);
        self.recent_folders.insert(0, folder);
        self.recent_folders.truncate(RECENT_FOLDER_LIMIT);
    }
}

/// Theme preference exposed in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ThemePreference {
    /// Follow the system theme when platform support is available.
    System,
    /// Use the warm light theme.
    Light,
    /// Use the dark theme.
    Dark,
}

impl Default for ThemePreference {
    fn default() -> Self {
        Self::System
    }
}

impl ThemePreference {
    /// Human-readable label for Settings controls.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

/// Motion preference exposed in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum MotionPreference {
    /// Follow the system reduced-motion preference when platform support is available.
    System,
    /// Minimize nonessential animation.
    Reduced,
    /// Use calm default egui animation.
    Subtle,
    /// Allow the richest animation level supported by the GUI.
    Full,
}

impl Default for MotionPreference {
    fn default() -> Self {
        Self::Subtle
    }
}

impl MotionPreference {
    /// Human-readable label for Settings controls.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Reduced => "Reduced",
            Self::Subtle => "Subtle",
            Self::Full => "Full",
        }
    }
}

/// Persisted organization style preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum StylePreference {
    /// Group files by type.
    Type,
    /// Group files by date.
    Date,
    /// Group files by extension.
    Extension,
    /// Group files by type and date.
    TypeYear,
    /// Use a custom rule profile when one is loaded.
    CustomRules,
}

impl Default for StylePreference {
    fn default() -> Self {
        Self::TypeYear
    }
}

impl From<BuiltInMode> for StylePreference {
    fn from(mode: BuiltInMode) -> Self {
        match mode {
            BuiltInMode::Type => Self::Type,
            BuiltInMode::Date => Self::Date,
            BuiltInMode::Extension => Self::Extension,
            BuiltInMode::TypeYear => Self::TypeYear,
        }
    }
}

impl StylePreference {
    /// Convert a persisted style to a built-in mode when possible.
    pub(crate) fn built_in_mode(self) -> Option<BuiltInMode> {
        match self {
            Self::Type => Some(BuiltInMode::Type),
            Self::Date => Some(BuiltInMode::Date),
            Self::Extension => Some(BuiltInMode::Extension),
            Self::TypeYear => Some(BuiltInMode::TypeYear),
            Self::CustomRules => None,
        }
    }
}
