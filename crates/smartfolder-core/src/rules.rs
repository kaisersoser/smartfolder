//! File organization rules and matching logic.
//!
//! Provides both built-in rules (organize by type, date, extension) and support for
//! custom rule profiles loaded from TOML files.
//!
//! # Built-in modes
//!
//! - `Type`: Group by `FileTypeBucket` (Documents, Images, Videos, etc.)
//! - `Date`: Organize by year and month from file modification time
//! - `Extension`: Folder per file extension
//! - `TypeYear`: Combine type and year (e.g., Documents/2024/report.pdf)
//!
//! # Custom rules
//!
//! Define rules in TOML with conditions like:
//! - File extensions (e.g., "pdf", "docx")
//! - Filename patterns (e.g., contains "invoice")
//! - Path patterns (e.g., contains "downloads")
//! - File size ranges
//! - Year from modification time
//!
//! Rules are matched in priority order; first match wins.

use std::path::PathBuf;

use chrono::Datelike;
use serde::{Deserialize, Serialize};

use crate::model::{BuiltInMode, Certainty, FileEntryKind, FileInventoryRecord, FileTypeBucket};
use crate::paths::normalize_relative;
use crate::{Result, SmartfolderError};

const DEFAULT_RULE_PRIORITY: u32 = 1_000;

/// Result of applying a rule to a file.
///
/// Contains the destination folder path, human-readable reason, and certainty level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleMatch {
    pub destination: PathBuf,
    pub reason: String,
    pub certainty: Certainty,
}

/// Collection of custom rules for organizing files.
///
/// Loaded from TOML and applied in priority order (ascending).
/// Can be validated and searched for matching rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleProfile {
    pub profile_id: String,
    #[serde(default)]
    pub rules: Vec<CustomRule>,
}

impl RuleProfile {
    /// Parse a rule profile from TOML string and validate it.
    pub fn from_toml(input: &str) -> Result<Self> {
        let profile: Self = toml::from_str(input)?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<()> {
        if self.profile_id.trim().is_empty() {
            return Err(SmartfolderError::InvalidRuleProfile {
                message: "profile_id must not be empty".to_string(),
            });
        }

        if self.rules.is_empty() {
            return Err(SmartfolderError::InvalidRuleProfile {
                message: "at least one rule is required".to_string(),
            });
        }

        for rule in &self.rules {
            rule.validate()?;
        }

        Ok(())
    }

    /// Find the first rule matching a file record (ordered by priority).
    pub fn first_match(&self, record: &FileInventoryRecord) -> Option<RuleMatch> {
        let mut ordered_rules = self.rules.iter().enumerate().collect::<Vec<_>>();
        ordered_rules
            .sort_by_key(|(index, rule)| (rule.priority.unwrap_or(DEFAULT_RULE_PRIORITY), *index));

        ordered_rules
            .into_iter()
            .find_map(|(_, rule)| rule.match_record(record))
    }
}

/// A custom rule for organizing files.
///
/// Matches files based on multiple conditions (all must match):
/// - Extensions (e.g., "pdf", "jpg")
/// - Filename patterns (substring match)
/// - Path patterns (substring match)
/// - File size range
/// - Year from modification date
///
/// When a file matches all conditions, it is moved to the `destination` folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomRule {
    pub name: String,
    pub destination: String,
    pub priority: Option<u32>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub filename_contains: Vec<String>,
    #[serde(default)]
    pub path_contains: Vec<String>,
    pub min_size_bytes: Option<u64>,
    pub max_size_bytes: Option<u64>,
    pub year: Option<i32>,
}

impl CustomRule {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(SmartfolderError::InvalidRuleProfile {
                message: "rule name must not be empty".to_string(),
            });
        }

        if self.destination.trim().is_empty() {
            return Err(SmartfolderError::InvalidRuleProfile {
                message: format!("rule '{}' destination must not be empty", self.name),
            });
        }

        normalize_relative(&self.destination)?;

        if self.condition_count() == 0 {
            return Err(SmartfolderError::InvalidRuleProfile {
                message: format!("rule '{}' must define at least one condition", self.name),
            });
        }

        if let (Some(min), Some(max)) = (self.min_size_bytes, self.max_size_bytes) {
            if min > max {
                return Err(SmartfolderError::InvalidRuleProfile {
                    message: format!("rule '{}' min_size_bytes exceeds max_size_bytes", self.name),
                });
            }
        }

        Ok(())
    }

    pub fn match_record(&self, record: &FileInventoryRecord) -> Option<RuleMatch> {
        if record.entry_kind != FileEntryKind::File {
            return None;
        }

        if !self.matches_extensions(record)
            || !self.matches_filename(record)
            || !self.matches_path(record)
            || !self.matches_size(record)
            || !self.matches_year(record)
        {
            return None;
        }

        let destination = render_destination_template(&self.destination, record).ok()?;

        Some(RuleMatch {
            destination,
            reason: format!("Rule: {}", self.name),
            certainty: Certainty::High,
        })
    }

    fn condition_count(&self) -> usize {
        usize::from(!self.extensions.is_empty())
            + usize::from(!self.filename_contains.is_empty())
            + usize::from(!self.path_contains.is_empty())
            + usize::from(self.min_size_bytes.is_some())
            + usize::from(self.max_size_bytes.is_some())
            + usize::from(self.year.is_some())
    }

    fn matches_extensions(&self, record: &FileInventoryRecord) -> bool {
        self.extensions.is_empty()
            || record.extension.as_ref().is_some_and(|extension| {
                self.extensions
                    .iter()
                    .any(|candidate| normalize_extension(candidate).eq_ignore_ascii_case(extension))
            })
    }

    fn matches_filename(&self, record: &FileInventoryRecord) -> bool {
        self.filename_contains
            .iter()
            .all(|needle| contains_case_insensitive(&record.name, needle))
    }

    fn matches_path(&self, record: &FileInventoryRecord) -> bool {
        let path = record.root_relative_path.to_string_lossy();
        self.path_contains
            .iter()
            .all(|needle| contains_case_insensitive(&path, needle))
    }

    fn matches_size(&self, record: &FileInventoryRecord) -> bool {
        self.min_size_bytes
            .map_or(true, |minimum| record.size_bytes >= minimum)
            && self
                .max_size_bytes
                .map_or(true, |maximum| record.size_bytes <= maximum)
    }

    fn matches_year(&self, record: &FileInventoryRecord) -> bool {
        self.year.map_or(true, |year| {
            record
                .modified_at
                .is_some_and(|modified_at| modified_at.year() == year)
        })
    }
}

pub fn builtin_rule_match(record: &FileInventoryRecord, mode: BuiltInMode) -> Option<RuleMatch> {
    if record.entry_kind != FileEntryKind::File {
        return None;
    }

    let date = record_date_parts(record);
    let destination = match mode {
        BuiltInMode::Type => PathBuf::from(type_folder(record.detected_type)).join(&record.name),
        BuiltInMode::Date => PathBuf::from(&date.year)
            .join(&date.month)
            .join(&date.day)
            .join(&record.name),
        BuiltInMode::Extension => PathBuf::from(record_extension(record)).join(&record.name),
        BuiltInMode::TypeYear => PathBuf::from(type_folder(record.detected_type))
            .join(&date.year)
            .join(&date.month)
            .join(&date.day)
            .join(&record.name),
    };

    Some(RuleMatch {
        destination,
        reason: format!("Built-in rule: {mode:?}"),
        certainty: Certainty::High,
    })
}

fn render_destination_template(template: &str, record: &FileInventoryRecord) -> Result<PathBuf> {
    let extension = record.extension.as_deref().unwrap_or("no-extension");
    let date = record_date_parts(record);
    let type_name = type_folder(record.detected_type);
    let includes_filename = template.contains("{filename}");

    let rendered = template
        .replace("{year}", &date.year)
        .replace("{month}", &date.month)
        .replace("{day}", &date.day)
        .replace("{extension}", extension)
        .replace("{type}", type_name)
        .replace("{filename}", &record.name);

    let normalized = normalize_relative(rendered)?;
    if includes_filename {
        Ok(normalized)
    } else {
        Ok(normalized.join(&record.name))
    }
}

fn type_folder(file_type: FileTypeBucket) -> &'static str {
    match file_type {
        FileTypeBucket::Document => "Documents",
        FileTypeBucket::Image => "Images",
        FileTypeBucket::Video => "Videos",
        FileTypeBucket::Audio => "Audio",
        FileTypeBucket::Archive => "Archives",
        FileTypeBucket::Spreadsheet => "Spreadsheets",
        FileTypeBucket::Presentation => "Presentations",
        FileTypeBucket::Code => "Code",
        FileTypeBucket::Directory => "Directories",
        FileTypeBucket::Link => "Links",
        FileTypeBucket::Other => "Other",
    }
}

struct DateParts {
    year: String,
    month: String,
    day: String,
}

fn record_date_parts(record: &FileInventoryRecord) -> DateParts {
    record.modified_at.map_or_else(
        || DateParts {
            year: "unknown-year".to_string(),
            month: "unknown-month".to_string(),
            day: "unknown-day".to_string(),
        },
        |modified_at| DateParts {
            year: modified_at.year().to_string(),
            month: month_name(modified_at.month()).to_string(),
            day: format!("{:02}", modified_at.day()),
        },
    )
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "unknown-month",
    }
}

fn record_extension(record: &FileInventoryRecord) -> String {
    record
        .extension
        .as_deref()
        .map_or_else(|| "no-extension".to_string(), ToOwned::to_owned)
}

fn normalize_extension(extension: &str) -> String {
    extension.trim_start_matches('.').to_ascii_lowercase()
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};

    use crate::model::{BuiltInMode, FileEntryKind, FileInventoryRecord, FileTypeBucket};
    use crate::rules::{builtin_rule_match, RuleProfile};

    fn path(parts: &[&str]) -> PathBuf {
        let mut path = PathBuf::new();
        for part in parts {
            path.push(part);
        }
        path
    }

    #[test]
    fn builtin_type_year_rule_places_file_in_type_year_month_day() {
        let record = record("report.pdf", FileTypeBucket::Document);
        let matched = builtin_rule_match(&record, BuiltInMode::TypeYear).expect("rule match");

        assert_eq!(
            matched.destination,
            path(&["Documents", "2026", "May", "11", "report.pdf"])
        );
    }

    #[test]
    fn builtin_date_rule_places_file_in_year_month_day() {
        let record = record("report.pdf", FileTypeBucket::Document);
        let matched = builtin_rule_match(&record, BuiltInMode::Date).expect("rule match");

        assert_eq!(
            matched.destination,
            path(&["2026", "May", "11", "report.pdf"])
        );
    }

    #[test]
    fn toml_rule_profile_matches_first_priority_rule() {
        let profile = RuleProfile::from_toml(
            r#"
profile_id = "downloads"

[[rules]]
name = "Invoices"
priority = 10
destination = "Documents/Invoices/{year}/{month}/{day}"
extensions = ["pdf"]
filename_contains = ["invoice"]

[[rules]]
name = "PDFs"
priority = 20
destination = "Documents/PDFs"
extensions = ["pdf"]
"#,
        )
        .expect("valid profile");

        let record = record("invoice-acme.pdf", FileTypeBucket::Document);
        let matched = profile.first_match(&record).expect("rule match");

        assert_eq!(
            matched.destination,
            path(&[
                "Documents",
                "Invoices",
                "2026",
                "May",
                "11",
                "invoice-acme.pdf"
            ])
        );
        assert_eq!(matched.reason, "Rule: Invoices");
    }

    #[test]
    fn custom_rule_rejects_parent_traversal_destination() {
        let err = RuleProfile::from_toml(
            r#"
profile_id = "bad"

[[rules]]
name = "Escape"
destination = "../outside"
extensions = ["txt"]
"#,
        )
        .expect_err("invalid destination should fail");

        assert!(err.to_string().contains("inside the selected root"));
    }

    #[test]
    fn regex_field_is_rejected_as_unknown() {
        let err = RuleProfile::from_toml(
            r#"
profile_id = "bad"

[[rules]]
name = "Regex"
destination = "Documents"
regex = ".*"
"#,
        )
        .expect_err("regex is not part of v1 rules");

        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn non_matching_rule_returns_none() {
        let profile = RuleProfile::from_toml(
            r#"
profile_id = "downloads"

[[rules]]
name = "Images"
destination = "Images"
extensions = ["jpg"]
"#,
        )
        .expect("valid profile");

        assert!(profile
            .first_match(&record("report.pdf", FileTypeBucket::Document))
            .is_none());
    }

    fn record(name: &str, detected_type: FileTypeBucket) -> FileInventoryRecord {
        FileInventoryRecord {
            file_id: name.to_string(),
            root_relative_path: PathBuf::from(name),
            name: name.to_string(),
            extension: PathBuf::from(name)
                .extension()
                .map(|extension| extension.to_string_lossy().to_ascii_lowercase()),
            detected_type,
            size_bytes: 42,
            created_at: None,
            modified_at: Some(Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap()),
            accessed_at: None,
            depth: 1,
            entry_kind: FileEntryKind::File,
            scan_warnings: Vec::new(),
        }
    }
}
