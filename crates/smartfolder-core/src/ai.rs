//! Optional AI assistance primitives.
//!
//! The AI layer is deliberately advisory. Provider output is validated and converted
//! into existing deterministic rule profiles before it can affect planning.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use chrono::Datelike;
use serde::{Deserialize, Serialize};

use crate::model::{FileEntryKind, FileInventoryRecord, FileTypeBucket};
use crate::paths::normalize_relative;
use crate::rules::{CustomRule, RuleProfile};
use crate::{Result, SmartfolderError};

const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";
const DEFAULT_AI_TIMEOUT_SECS: u64 = 60;
const AI_CONTEXT_RECORD_LIMIT: usize = 500;
const AI_CONTEXT_EVIDENCE_LIMIT: usize = 24;
const AI_CONTEXT_CONTENT_FILE_LIMIT: usize = 12;
const AI_CONTEXT_CONTENT_BYTES_PER_FILE: u64 = 4096;
const AI_CONTEXT_CONTENT_CHARS_PER_FILE: usize = 2000;
const PREFERRED_OLLAMA_MODELS: &[&str] = &[
    "llama3.1", "llama3", "mistral", "qwen2.5", "qwen2", "gemma2", "gemma",
];
const TEXT_LIKE_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "csv", "tsv", "json", "jsonl", "toml", "yaml", "yml", "xml", "html",
    "htm", "css", "scss", "js", "jsx", "ts", "tsx", "rs", "py", "ps1", "sh", "bat", "cmd", "sql",
    "log", "ini", "cfg", "conf", "rtf",
];
const ALLOWED_DESTINATION_TOKENS: &[&str] = &[
    "{type}",
    "{year}",
    "{month}",
    "{day}",
    "{extension}",
    "{filename}",
];
const AI_ANALYSIS_SYSTEM_PROMPT: &str = "You are smartfolder's advisory folder analyst. Use only the provided folder context. Do not invent files. Do not suggest direct moves. Explain useful organization patterns, risks, and a recommended deterministic sorting strategy.";
const AI_PROFILE_SYSTEM_PROMPT: &str = "You are smartfolder's rule drafting assistant. Produce only a deterministic rule profile draft using the allowed schema and tokens. Do not invent unsupported semantic tokens. Do not use absolute paths or parent traversal.";
const AI_EXPLAIN_SYSTEM_PROMPT: &str = "You are smartfolder's rule explainer. Explain the provided deterministic rule profile using the selected folder context. Do not modify rules and do not claim that files will move without preview.";
const AI_PROMPT_REFINEMENT_SYSTEM_PROMPT: &str = "You are smartfolder's prompt editor. Rewrite the user's rule-building prompt so it is clearer, more specific, and easier for a deterministic rule generator to follow. Preserve user intent. Do not generate rules. Do not invent unsupported semantic tokens.";
const FOLDER_ANALYSIS_SCHEMA: &str = r#"{
  "summary": "short plain-language assessment",
  "patterns": [{"title": "pattern", "detail": "why it matters", "examples": ["relative/path.ext"]}],
  "risks": [{"title": "risk", "detail": "what needs review", "examples": ["relative/path.ext"]}],
  "recommended_strategy": "recommended deterministic sorting strategy",
  "confidence": "low|medium|high",
  "evidence": ["relative/path.ext"],
  "scope_used": "folder-relative scope description",
  "content_inspection_used": false
}"#;
const PROFILE_DRAFT_SCHEMA: &str = r#"{
  "profile_id": "safe-profile-id",
  "rationale": "why these rules fit the folder",
  "rules": [{
    "name": "rule name",
    "destination": "literal/{year}/{month}",
    "priority": 10,
    "match_all": false,
    "extensions": ["pdf"],
    "filename_contains": ["invoice"],
    "path_contains": [],
    "min_size_bytes": null,
    "max_size_bytes": null,
    "year": null
  }]
}"#;
const RULE_EXPLANATION_SCHEMA: &str = r#"{
  "summary": "plain-language rule summary",
  "rule_order": [{"rule_name": "rule name", "explanation": "how it matches and why order matters"}],
  "likely_matches": [{"rule_name": "rule name", "relative_path": "file.ext", "destination_example": "folder/file.ext"}],
  "warnings": ["potential issue"],
  "scope_used": "folder-relative scope description"
}"#;
const PROMPT_REFINEMENT_SCHEMA: &str = r#"{
  "refined_prompt": "clear user-facing prompt for drafting deterministic rules",
  "notes": ["short note about clarification made"]
}"#;

/// Settings needed to connect to a local Ollama provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub enabled: bool,
    pub endpoint: String,
    pub selected_model: Option<String>,
    pub timeout_seconds: u64,
    pub content_inspection_enabled: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: DEFAULT_OLLAMA_ENDPOINT.to_string(),
            selected_model: None,
            timeout_seconds: DEFAULT_AI_TIMEOUT_SECS,
            content_inspection_enabled: false,
        }
    }
}

impl AiSettings {
    /// Resolve a bounded timeout for provider requests.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_seconds.clamp(5, 300))
    }
}

/// User-facing provider readiness state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiProviderStatus {
    pub available: bool,
    pub state: AiProviderState,
    pub selected_model: Option<String>,
    pub models: Vec<String>,
    pub message: String,
}

impl AiProviderStatus {
    fn unavailable(state: AiProviderState, message: impl Into<String>) -> Self {
        Self {
            available: false,
            state,
            selected_model: None,
            models: Vec::new(),
            message: message.into(),
        }
    }
}

/// Coarse status values for settings and feature gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProviderState {
    Disabled,
    EndpointUnavailable,
    NoModels,
    ModelMissing,
    RequestFailed,
    Ready,
}

/// Minimal Ollama client used by the GUI feature gate and AI requests.
#[derive(Debug, Clone)]
pub struct OllamaClient {
    endpoint: String,
    timeout: Duration,
}

impl OllamaClient {
    /// Create a client for a base endpoint such as `http://localhost:11434`.
    #[must_use]
    pub fn new(endpoint: impl Into<String>, timeout: Duration) -> Self {
        let endpoint = endpoint.into();
        Self {
            endpoint: trim_endpoint(&endpoint),
            timeout,
        }
    }

    /// Check endpoint/model readiness with a tiny structured request.
    pub fn check_status(&self, configured_model: Option<&str>) -> AiProviderStatus {
        let models = match self.list_models() {
            Ok(models) => models,
            Err(message) => {
                return AiProviderStatus::unavailable(
                    AiProviderState::EndpointUnavailable,
                    message,
                );
            }
        };

        let Some(selected_model) = select_ollama_model(&models, configured_model) else {
            return AiProviderStatus::unavailable(
                AiProviderState::NoModels,
                "No local Ollama models were found.",
            );
        };

        if configured_model.is_some_and(|model| !models.iter().any(|candidate| candidate == model))
        {
            return AiProviderStatus {
                available: false,
                state: AiProviderState::ModelMissing,
                selected_model: Some(selected_model),
                models,
                message: "The configured Ollama model is not installed.".to_string(),
            };
        }

        match self.generate_json(
            &selected_model,
            "Return exactly this JSON object: {\"ready\":true}",
        ) {
            Ok(value) if value.get("ready").and_then(serde_json::Value::as_bool) == Some(true) => {
                AiProviderStatus {
                    available: true,
                    state: AiProviderState::Ready,
                    selected_model: Some(selected_model),
                    models,
                    message: "Ollama is ready.".to_string(),
                }
            }
            Ok(_) => AiProviderStatus {
                available: false,
                state: AiProviderState::RequestFailed,
                selected_model: Some(selected_model),
                models,
                message: "Ollama responded, but the structured readiness check failed.".to_string(),
            },
            Err(error) => AiProviderStatus {
                available: false,
                state: AiProviderState::RequestFailed,
                selected_model: Some(selected_model),
                models,
                message: error.to_string(),
            },
        }
    }

    /// Return installed local model names from `/api/tags`.
    pub fn list_models(&self) -> std::result::Result<Vec<String>, String> {
        let url = format!("{}/api/tags", self.endpoint);
        let response = self
            .agent()
            .get(&url)
            .call()
            .map_err(|error| format!("Failed to reach Ollama at {url}: {error}"))?;
        let body: OllamaTagsResponse = response
            .into_json()
            .map_err(|error| format!("Failed to parse Ollama model list: {error}"))?;
        Ok(body.models.into_iter().map(|model| model.name).collect())
    }

    /// Generate a structured JSON response using `/api/generate`.
    pub fn generate_json(&self, model: &str, prompt: &str) -> Result<serde_json::Value> {
        let response = self.generate_raw(model, prompt)?;
        match serde_json::from_str(&response) {
            Ok(value) => Ok(value),
            Err(error) => {
                let repair_prompt = json_repair_prompt(&response, &error.to_string());
                let repaired = self.generate_raw(model, &repair_prompt)?;
                serde_json::from_str(&repaired).map_err(SmartfolderError::from)
            }
        }
    }

    fn generate_raw(&self, model: &str, prompt: &str) -> Result<String> {
        let url = format!("{}/api/generate", self.endpoint);
        let response = self
            .agent()
            .post(&url)
            .send_json(ureq::json!({
                "model": model,
                "prompt": prompt,
                "stream": false,
                "format": "json",
            }))
            .map_err(|error| SmartfolderError::AiProvider {
                message: format!("Failed to call Ollama at {url}: {error}"),
            })?;
        let body: OllamaGenerateResponse =
            response
                .into_json()
                .map_err(|error| SmartfolderError::AiProvider {
                    message: format!("Failed to parse Ollama response: {error}"),
                })?;
        Ok(body.response)
    }

    /// Analyze a folder context and return advisory recommendations.
    pub fn analyze_folder(
        &self,
        model: &str,
        context: &AiFolderContext,
    ) -> Result<AiFolderAnalysis> {
        let mut analysis: AiFolderAnalysis =
            self.generate_typed_json(model, &folder_analysis_prompt(context))?;
        analysis.content_inspection_used = context.content_inspection_enabled;
        analysis.content_samples_included = context.content_samples_included;
        analysis
            .content_sample_warnings
            .clone_from(&context.content_sample_warnings);
        Ok(analysis)
    }

    /// Generate a draft profile from a user prompt and selected folder context.
    pub fn draft_profile(
        &self,
        model: &str,
        user_prompt: &str,
        context: &AiFolderContext,
        existing_profile: Option<&RuleProfile>,
    ) -> Result<AiRuleProfileDraft> {
        self.generate_typed_json(
            model,
            &profile_draft_prompt(user_prompt, context, existing_profile),
        )
    }

    /// Refine a user prompt before generating a draft profile.
    pub fn refine_prompt(
        &self,
        model: &str,
        user_prompt: &str,
        context: &AiFolderContext,
    ) -> Result<AiPromptRefinement> {
        self.generate_typed_json(model, &prompt_refinement_prompt(user_prompt, context))
    }

    /// Explain a deterministic rule profile against selected folder context.
    pub fn explain_profile(
        &self,
        model: &str,
        profile: &RuleProfile,
        context: &AiFolderContext,
    ) -> Result<AiRuleExplanation> {
        self.generate_typed_json(model, &rule_explanation_prompt(profile, context))
    }

    fn generate_typed_json<T>(&self, model: &str, prompt: &str) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let value = self.generate_json(model, prompt)?;
        serde_json::from_value(value).map_err(SmartfolderError::from)
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout(self.timeout).build()
    }
}

/// Structured folder analysis returned by AI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiFolderAnalysis {
    pub summary: String,
    #[serde(default)]
    pub patterns: Vec<AiFinding>,
    #[serde(default)]
    pub risks: Vec<AiFinding>,
    pub recommended_strategy: String,
    pub confidence: AiConfidence,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub scope_used: String,
    pub content_inspection_used: bool,
    #[serde(default)]
    pub content_samples_included: usize,
    #[serde(default)]
    pub content_sample_warnings: Vec<String>,
}

/// One pattern or risk discovered by AI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiFinding {
    pub title: String,
    pub detail: String,
    #[serde(default)]
    pub examples: Vec<String>,
}

/// Coarse confidence value for AI advisory output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiConfidence {
    Low,
    Medium,
    High,
}

/// Structured explanation of an existing deterministic rule profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiRuleExplanation {
    pub summary: String,
    #[serde(default)]
    pub rule_order: Vec<AiRuleOrderExplanation>,
    #[serde(default)]
    pub likely_matches: Vec<AiRuleExample>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub scope_used: String,
}

/// Explanation of one rule in profile order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiRuleOrderExplanation {
    pub rule_name: String,
    pub explanation: String,
}

/// Example file match for a rule explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiRuleExample {
    pub rule_name: String,
    pub relative_path: String,
    pub destination_example: String,
}

/// Structured prompt rewrite returned by AI before profile generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiPromptRefinement {
    pub refined_prompt: String,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Select a saved model, preferred model, or first installed model.
#[must_use]
pub fn select_ollama_model(models: &[String], configured_model: Option<&str>) -> Option<String> {
    if let Some(configured_model) = configured_model {
        if models.iter().any(|model| model == configured_model) {
            return Some(configured_model.to_string());
        }
    }

    for preferred in PREFERRED_OLLAMA_MODELS {
        if let Some(model) = models
            .iter()
            .find(|model| model_name_matches(model, preferred))
        {
            return Some(model.clone());
        }
    }

    models.first().cloned()
}

/// Metadata-only context passed to AI prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AiFolderContext {
    pub scope_root: String,
    pub records_in_context: usize,
    pub total_records: usize,
    pub sampled: bool,
    pub content_inspection_enabled: bool,
    pub content_samples_included: usize,
    pub content_sample_warnings: Vec<String>,
    pub file_type_counts: BTreeMap<String, usize>,
    pub extension_counts: BTreeMap<String, usize>,
    pub examples: Vec<AiFileContext>,
}

impl AiFolderContext {
    /// Build a bounded, relative-path-only context from scan records.
    #[must_use]
    pub fn from_records(scope_root: &Path, records: &[FileInventoryRecord]) -> Self {
        Self::from_records_with_content_mode(scope_root, records, false)
    }

    /// Build a bounded, relative-path-only context and record content-inspection mode.
    #[must_use]
    pub fn from_records_with_content_mode(
        scope_root: &Path,
        records: &[FileInventoryRecord],
        content_inspection_enabled: bool,
    ) -> Self {
        Self::from_records_with_optional_content(scope_root, records, content_inspection_enabled)
    }

    /// Build AI context and optionally sample bounded text-like file contents from disk.
    #[must_use]
    pub fn from_records_with_optional_content(
        scope_root: &Path,
        records: &[FileInventoryRecord],
        content_inspection_enabled: bool,
    ) -> Self {
        let mut file_type_counts = BTreeMap::new();
        let mut extension_counts = BTreeMap::new();
        let mut examples = Vec::new();
        let mut content_samples_included = 0;
        let mut content_sample_warnings = Vec::new();

        for record in records {
            *file_type_counts
                .entry(file_type_label(record.detected_type).to_string())
                .or_insert(0) += 1;
            if let Some(extension) = &record.extension {
                *extension_counts.entry(extension.clone()).or_insert(0) += 1;
            }
            if examples.len() < AI_CONTEXT_EVIDENCE_LIMIT {
                let mut file_context = AiFileContext::from_record(record);
                if content_inspection_enabled
                    && content_samples_included < AI_CONTEXT_CONTENT_FILE_LIMIT
                    && is_text_like_record(record)
                {
                    match sample_text_content(scope_root, record) {
                        Ok(sample) => {
                            file_context.content_sample = Some(sample.text);
                            file_context.content_sample_truncated = sample.truncated;
                            content_samples_included += 1;
                        }
                        Err(message) => {
                            content_sample_warnings
                                .push(format!("{}: {message}", file_context.relative_path));
                        }
                    }
                }
                examples.push(file_context);
            }
        }

        Self {
            scope_root: scope_root
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().to_string()),
            records_in_context: records.len().min(AI_CONTEXT_RECORD_LIMIT),
            total_records: records.len(),
            sampled: records.len() > AI_CONTEXT_RECORD_LIMIT,
            content_inspection_enabled,
            content_samples_included,
            content_sample_warnings,
            file_type_counts,
            extension_counts,
            examples,
        }
    }
}

/// One AI-safe file metadata row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AiFileContext {
    pub relative_path: String,
    pub name: String,
    pub extension: Option<String>,
    pub detected_type: String,
    pub size_bytes: u64,
    pub modified_year: Option<i32>,
    pub depth: usize,
    pub kind: String,
    pub content_sample: Option<String>,
    pub content_sample_truncated: bool,
}

impl AiFileContext {
    fn from_record(record: &FileInventoryRecord) -> Self {
        Self {
            relative_path: record
                .root_relative_path
                .to_string_lossy()
                .replace('\\', "/"),
            name: record.name.clone(),
            extension: record.extension.clone(),
            detected_type: file_type_label(record.detected_type).to_string(),
            size_bytes: record.size_bytes,
            modified_year: record.modified_at.map(|modified| modified.year()),
            depth: record.depth,
            kind: entry_kind_label(record.entry_kind).to_string(),
            content_sample: None,
            content_sample_truncated: false,
        }
    }
}

/// Strict JSON profile draft expected from AI rule generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiRuleProfileDraft {
    pub profile_id: String,
    pub rationale: Option<String>,
    #[serde(default)]
    pub rules: Vec<AiRuleDraft>,
}

/// Strict JSON rule draft expected from AI rule generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiRuleDraft {
    pub name: String,
    pub destination: String,
    pub priority: Option<u32>,
    #[serde(default)]
    pub match_all: bool,
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

/// Result of validating an AI-generated profile draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiProfileValidation {
    pub profile: Option<RuleProfile>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl AiProfileValidation {
    /// Whether the draft can be opened as a profile draft.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.profile.is_some() && self.errors.is_empty()
    }
}

/// Validate and convert an AI-generated profile draft.
#[must_use]
pub fn validate_ai_profile_draft(
    draft: AiRuleProfileDraft,
    records: &[FileInventoryRecord],
) -> AiProfileValidation {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for rule in &draft.rules {
        if let Err(message) = validate_destination_tokens(&rule.destination) {
            errors.push(format!("Rule '{}': {message}", rule.name));
        }
    }

    let profile = RuleProfile {
        profile_id: draft.profile_id.trim().to_string(),
        rules: draft
            .rules
            .into_iter()
            .map(AiRuleDraft::into_rule)
            .collect(),
    };

    if let Err(error) = profile.validate() {
        errors.push(error.to_string());
    }

    if errors.is_empty() {
        warnings.extend(applicability_warnings(&profile, records));
        AiProfileValidation {
            profile: Some(profile),
            errors,
            warnings,
        }
    } else {
        AiProfileValidation {
            profile: None,
            errors,
            warnings,
        }
    }
}

/// Build the folder-analysis prompt used by providers.
#[must_use]
pub fn folder_analysis_prompt(context: &AiFolderContext) -> String {
    format!(
        "{system}\n\nReturn JSON matching this schema:\n{schema}\n\nFolder context:\n{context}",
        system = AI_ANALYSIS_SYSTEM_PROMPT,
        schema = FOLDER_ANALYSIS_SCHEMA,
        context = serde_json::to_string_pretty(context).unwrap_or_else(|_| "{}".to_string())
    )
}

/// Build the prompt-to-profile prompt used by providers.
#[must_use]
pub fn profile_draft_prompt(
    user_prompt: &str,
    context: &AiFolderContext,
    existing_profile: Option<&RuleProfile>,
) -> String {
    let existing_profile = existing_profile
        .and_then(|profile| serde_json::to_string_pretty(profile).ok())
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{system}\n\nAllowed destination tokens: {tokens:?}\nAllowed rule fields: profile_id, rules, name, destination, priority, match_all, extensions, filename_contains, path_contains, min_size_bytes, max_size_bytes, year.\nReturn JSON matching this schema:\n{schema}\n\nUser request:\n{user_prompt}\n\nFolder context:\n{context}\n\nExisting profile, if any:\n{existing_profile}",
        system = AI_PROFILE_SYSTEM_PROMPT,
        tokens = ALLOWED_DESTINATION_TOKENS,
        schema = PROFILE_DRAFT_SCHEMA,
        context = serde_json::to_string_pretty(context).unwrap_or_else(|_| "{}".to_string()),
    )
}

/// Build the prompt-refinement prompt used by providers.
#[must_use]
pub fn prompt_refinement_prompt(user_prompt: &str, context: &AiFolderContext) -> String {
    format!(
        "{system}\n\nAllowed destination tokens if the user mentions tokens: {tokens:?}\nReturn JSON matching this schema:\n{schema}\n\nUser draft prompt:\n{user_prompt}\n\nFolder context:\n{context}",
        system = AI_PROMPT_REFINEMENT_SYSTEM_PROMPT,
        tokens = ALLOWED_DESTINATION_TOKENS,
        schema = PROMPT_REFINEMENT_SCHEMA,
        context = serde_json::to_string_pretty(context).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn json_repair_prompt(raw_response: &str, parse_error: &str) -> String {
    format!(
        "Repair this invalid JSON response. Return only valid JSON. Preserve all fields and values that can be recovered. Do not add Markdown.\n\nParse error:\n{parse_error}\n\nInvalid response:\n{raw_response}",
    )
}

/// Build the rule-explanation prompt used by providers.
#[must_use]
pub fn rule_explanation_prompt(profile: &RuleProfile, context: &AiFolderContext) -> String {
    format!(
        "{system}\n\nReturn JSON matching this schema:\n{schema}\n\nRule profile:\n{profile}\n\nFolder context:\n{context}",
        system = AI_EXPLAIN_SYSTEM_PROMPT,
        schema = RULE_EXPLANATION_SCHEMA,
        profile = serde_json::to_string_pretty(profile).unwrap_or_else(|_| "{}".to_string()),
        context = serde_json::to_string_pretty(context).unwrap_or_else(|_| "{}".to_string())
    )
}

impl AiRuleDraft {
    fn into_rule(self) -> CustomRule {
        CustomRule {
            name: self.name.trim().to_string(),
            destination: clean_ai_destination(&self.destination),
            priority: self.priority,
            match_all: self.match_all,
            extensions: self.extensions,
            filename_contains: self.filename_contains,
            path_contains: self.path_contains,
            min_size_bytes: self.min_size_bytes,
            max_size_bytes: self.max_size_bytes,
            year: self.year,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContentSample {
    text: String,
    truncated: bool,
}

fn is_text_like_record(record: &FileInventoryRecord) -> bool {
    if record.entry_kind != FileEntryKind::File {
        return false;
    }
    if matches!(record.detected_type, FileTypeBucket::Code) {
        return true;
    }
    record.extension.as_deref().is_some_and(|extension| {
        let normalized = extension.trim_start_matches('.').to_ascii_lowercase();
        TEXT_LIKE_EXTENSIONS
            .iter()
            .any(|candidate| *candidate == normalized)
    })
}

fn sample_text_content(
    scope_root: &Path,
    record: &FileInventoryRecord,
) -> std::result::Result<ContentSample, String> {
    let path = scope_root.join(&record.root_relative_path);
    let mut file = File::open(&path).map_err(|error| format!("content sample skipped: {error}"))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(AI_CONTEXT_CONTENT_BYTES_PER_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("content sample failed: {error}"))?;
    let truncated_by_bytes = bytes.len() as u64 > AI_CONTEXT_CONTENT_BYTES_PER_FILE;
    if truncated_by_bytes {
        bytes.truncate(AI_CONTEXT_CONTENT_BYTES_PER_FILE as usize);
    }
    if looks_binary(&bytes) {
        return Err("content sample skipped: file appears binary".to_string());
    }
    let decoded = String::from_utf8_lossy(&bytes);
    let sanitized = sanitize_content_sample(&decoded);
    if sanitized.trim().is_empty() {
        return Err("content sample skipped: no readable text".to_string());
    }
    let char_count = sanitized.chars().count();
    let truncated_by_chars = char_count > AI_CONTEXT_CONTENT_CHARS_PER_FILE;
    let text = if truncated_by_chars {
        sanitized
            .chars()
            .take(AI_CONTEXT_CONTENT_CHARS_PER_FILE)
            .collect()
    } else {
        sanitized
    };
    Ok(ContentSample {
        text,
        truncated: truncated_by_bytes || truncated_by_chars,
    })
}

fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return true;
    }
    if bytes.is_empty() {
        return false;
    }
    let control_count = bytes
        .iter()
        .filter(|byte| matches!(**byte, 0x01..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F))
        .count();
    control_count * 10 > bytes.len()
}

fn sanitize_content_sample(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .lines()
        .take(40)
        .collect::<Vec<_>>()
        .join("\n")
}

fn clean_ai_destination(destination: &str) -> String {
    destination
        .trim()
        .trim_matches(['/', '\\'])
        .strip_prefix("literal/")
        .or_else(|| {
            destination
                .trim()
                .trim_matches(['/', '\\'])
                .strip_prefix(r"literal\")
        })
        .unwrap_or_else(|| destination.trim().trim_matches(['/', '\\']))
        .to_string()
}

fn applicability_warnings(profile: &RuleProfile, records: &[FileInventoryRecord]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut matched_by_earlier = BTreeSet::new();

    for (rule_index, rule) in profile.rules.iter().enumerate() {
        let direct_matches = records
            .iter()
            .enumerate()
            .filter(|(_, record)| rule.match_record(record).is_some())
            .map(|(record_index, _)| record_index)
            .collect::<Vec<_>>();
        let first_match_count = direct_matches
            .iter()
            .filter(|record_index| !matched_by_earlier.contains(*record_index))
            .count();

        if direct_matches.is_empty() {
            warnings.push(format!(
                "Rule '{}' does not match any files in the selected folder context.",
                rule.name
            ));
        } else if first_match_count == 0 {
            warnings.push(format!(
                "Rule '{}' is fully shadowed by earlier rules in the selected folder context.",
                rule.name
            ));
        } else if first_match_count < direct_matches.len() {
            warnings.push(format!(
                "Rule '{}' overlaps earlier rules; {} of {} matching files would be handled earlier.",
                rule.name,
                direct_matches.len() - first_match_count,
                direct_matches.len()
            ));
        }

        if rule_index + 1 < profile.rules.len() {
            for record_index in direct_matches {
                matched_by_earlier.insert(record_index);
            }
        }
    }

    warnings
}

fn validate_destination_tokens(destination: &str) -> std::result::Result<(), String> {
    normalize_relative(destination).map_err(|error| error.to_string())?;

    let mut remainder = destination;
    while let Some(start) = remainder.find(['{', '}']) {
        let found = remainder.as_bytes()[start] as char;
        if found == '}' {
            return Err("destination contains an unopened token".to_string());
        }
        let after_start = &remainder[start..];
        let Some(end) = after_start.find('}') else {
            return Err("destination contains an unclosed token".to_string());
        };
        let token = &after_start[..=end];
        if !ALLOWED_DESTINATION_TOKENS.contains(&token) {
            return Err(format!("unsupported destination token '{token}'"));
        }
        remainder = &after_start[end + 1..];
    }

    Ok(())
}

fn trim_endpoint(endpoint: &str) -> String {
    endpoint.trim().trim_end_matches('/').to_string()
}

fn model_name_matches(model: &str, preferred: &str) -> bool {
    model == preferred || model.starts_with(&format!("{preferred}:"))
}

fn file_type_label(file_type: FileTypeBucket) -> &'static str {
    match file_type {
        FileTypeBucket::Document => "document",
        FileTypeBucket::Image => "image",
        FileTypeBucket::Video => "video",
        FileTypeBucket::Audio => "audio",
        FileTypeBucket::Archive => "archive",
        FileTypeBucket::Spreadsheet => "spreadsheet",
        FileTypeBucket::Presentation => "presentation",
        FileTypeBucket::Code => "code",
        FileTypeBucket::Directory => "directory",
        FileTypeBucket::Link => "link",
        FileTypeBucket::Other => "other",
    }
}

fn entry_kind_label(entry_kind: FileEntryKind) -> &'static str {
    match entry_kind {
        FileEntryKind::File => "file",
        FileEntryKind::Directory => "directory",
        FileEntryKind::Symlink => "symlink",
        FileEntryKind::Junction => "junction",
        FileEntryKind::Other => "other",
    }
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    use crate::ai::{
        folder_analysis_prompt, json_repair_prompt, profile_draft_prompt, prompt_refinement_prompt,
        rule_explanation_prompt, select_ollama_model, validate_ai_profile_draft, AiFolderContext,
        AiRuleDraft, AiRuleProfileDraft,
    };
    use crate::model::{FileEntryKind, FileInventoryRecord, FileTypeBucket};

    #[test]
    fn selects_saved_then_preferred_then_first_ollama_model() {
        let models = vec![
            "tinyllama:latest".to_string(),
            "llama3.1:8b".to_string(),
            "mistral:latest".to_string(),
        ];

        assert_eq!(
            select_ollama_model(&models, Some("mistral:latest")),
            Some("mistral:latest".to_string())
        );
        assert_eq!(
            select_ollama_model(&models, Some("missing")),
            Some("llama3.1:8b".to_string())
        );
        assert_eq!(
            select_ollama_model(&["custom".to_string()], None),
            Some("custom".to_string())
        );
    }

    #[test]
    fn folder_context_uses_relative_paths_only() {
        let context = AiFolderContext::from_records(
            Path::new("D:/Users/Alice/Documents"),
            &[record("Invoices/invoice.pdf", FileTypeBucket::Document)],
        );

        assert_eq!(context.scope_root, "Documents");
        assert_eq!(context.examples[0].relative_path, "Invoices/invoice.pdf");
        assert!(!context.examples[0].relative_path.contains("Alice"));
    }

    #[test]
    fn folder_context_samples_text_content_only_when_enabled() {
        let temp = tempdir().expect("temp dir");
        fs::write(
            temp.path().join("notes.txt"),
            "Invoice notes\nClient: Example\nAmount: 42",
        )
        .expect("write notes");
        let records = [record("notes.txt", FileTypeBucket::Document)];

        let disabled =
            AiFolderContext::from_records_with_optional_content(temp.path(), &records, false);
        assert_eq!(disabled.content_samples_included, 0);
        assert!(disabled.examples[0].content_sample.is_none());

        let enabled =
            AiFolderContext::from_records_with_optional_content(temp.path(), &records, true);
        assert_eq!(enabled.content_samples_included, 1);
        assert!(enabled.content_sample_warnings.is_empty());
        assert!(enabled.examples[0]
            .content_sample
            .as_deref()
            .expect("content sample")
            .contains("Invoice notes"));
        assert!(!serde_json::to_string(&enabled)
            .expect("serialize context")
            .contains(temp.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn folder_context_skips_binary_content_samples() {
        let temp = tempdir().expect("temp dir");
        fs::write(temp.path().join("notes.txt"), b"text\0binary").expect("write binary-like file");
        let records = [record("notes.txt", FileTypeBucket::Document)];

        let context =
            AiFolderContext::from_records_with_optional_content(temp.path(), &records, true);

        assert_eq!(context.content_samples_included, 0);
        assert!(context.examples[0].content_sample.is_none());
        assert!(context
            .content_sample_warnings
            .iter()
            .any(|warning| warning.contains("appears binary")));
    }

    #[test]
    fn ai_profile_draft_rejects_unknown_destination_tokens() {
        let validation = validate_ai_profile_draft(
            AiRuleProfileDraft {
                profile_id: "ai-profile".to_string(),
                rationale: None,
                rules: vec![AiRuleDraft {
                    name: "Vendors".to_string(),
                    destination: "Invoices/{vendor}".to_string(),
                    priority: Some(10),
                    match_all: true,
                    extensions: Vec::new(),
                    filename_contains: Vec::new(),
                    path_contains: Vec::new(),
                    min_size_bytes: None,
                    max_size_bytes: None,
                    year: None,
                }],
            },
            &[record("invoice.pdf", FileTypeBucket::Document)],
        );

        assert!(validation.profile.is_none());
        assert!(validation
            .errors
            .iter()
            .any(|error| error.contains("unsupported destination token")));
    }

    #[test]
    fn ai_profile_draft_allows_zero_match_rules_with_warning() {
        let validation = validate_ai_profile_draft(
            AiRuleProfileDraft {
                profile_id: "ai-profile".to_string(),
                rationale: None,
                rules: vec![AiRuleDraft {
                    name: "Images".to_string(),
                    destination: "Images/{year}".to_string(),
                    priority: Some(10),
                    match_all: false,
                    extensions: vec!["png".to_string()],
                    filename_contains: Vec::new(),
                    path_contains: Vec::new(),
                    min_size_bytes: None,
                    max_size_bytes: None,
                    year: None,
                }],
            },
            &[record("invoice.pdf", FileTypeBucket::Document)],
        );

        assert!(validation.is_usable());
        assert!(validation
            .warnings
            .iter()
            .any(|warning| warning.contains("does not match any files")));
    }

    #[test]
    fn ai_profile_draft_warns_for_shadowed_rules() {
        let validation = validate_ai_profile_draft(
            AiRuleProfileDraft {
                profile_id: "ai-profile".to_string(),
                rationale: None,
                rules: vec![
                    AiRuleDraft {
                        name: "All".to_string(),
                        destination: "{type}".to_string(),
                        priority: Some(10),
                        match_all: true,
                        extensions: Vec::new(),
                        filename_contains: Vec::new(),
                        path_contains: Vec::new(),
                        min_size_bytes: None,
                        max_size_bytes: None,
                        year: None,
                    },
                    AiRuleDraft {
                        name: "PDFs".to_string(),
                        destination: "Documents/PDFs".to_string(),
                        priority: Some(20),
                        match_all: false,
                        extensions: vec!["pdf".to_string()],
                        filename_contains: Vec::new(),
                        path_contains: Vec::new(),
                        min_size_bytes: None,
                        max_size_bytes: None,
                        year: None,
                    },
                ],
            },
            &[record("invoice.pdf", FileTypeBucket::Document)],
        );

        assert!(validation.is_usable());
        assert!(validation
            .warnings
            .iter()
            .any(|warning| warning.contains("fully shadowed")));
    }

    #[test]
    fn ai_profile_draft_strips_literal_destination_prefix() {
        let validation = validate_ai_profile_draft(
            AiRuleProfileDraft {
                profile_id: "ai-profile".to_string(),
                rationale: None,
                rules: vec![AiRuleDraft {
                    name: "Documents".to_string(),
                    destination: "literal/{year}/Documents".to_string(),
                    priority: Some(10),
                    match_all: true,
                    extensions: Vec::new(),
                    filename_contains: Vec::new(),
                    path_contains: Vec::new(),
                    min_size_bytes: None,
                    max_size_bytes: None,
                    year: None,
                }],
            },
            &[record("invoice.pdf", FileTypeBucket::Document)],
        );

        let profile = validation.profile.expect("usable profile");
        assert_eq!(profile.rules[0].destination, "{year}/Documents");
    }

    #[test]
    fn prompts_include_schema_and_relative_context() {
        let context = AiFolderContext::from_records(
            Path::new("D:/Users/Alice/Documents"),
            &[record("Invoices/invoice.pdf", FileTypeBucket::Document)],
        );
        let analysis_prompt = folder_analysis_prompt(&context);
        let draft_prompt = profile_draft_prompt("sort invoices", &context, None);
        let refinement_prompt = prompt_refinement_prompt("sort invoices", &context);
        let explanation_prompt = rule_explanation_prompt(
            &validate_ai_profile_draft(
                AiRuleProfileDraft {
                    profile_id: "ai-profile".to_string(),
                    rationale: None,
                    rules: vec![AiRuleDraft {
                        name: "PDFs".to_string(),
                        destination: "Documents/PDFs".to_string(),
                        priority: Some(10),
                        match_all: false,
                        extensions: vec!["pdf".to_string()],
                        filename_contains: Vec::new(),
                        path_contains: Vec::new(),
                        min_size_bytes: None,
                        max_size_bytes: None,
                        year: None,
                    }],
                },
                &[record("Invoices/invoice.pdf", FileTypeBucket::Document)],
            )
            .profile
            .expect("usable profile"),
            &context,
        );

        assert!(analysis_prompt.contains("recommended_strategy"));
        assert!(draft_prompt.contains("Allowed destination tokens"));
        assert!(refinement_prompt.contains("refined_prompt"));
        assert!(explanation_prompt.contains("rule_order"));
        assert!(analysis_prompt.contains("Invoices/invoice.pdf"));
        assert!(refinement_prompt.contains("Invoices/invoice.pdf"));
        assert!(!analysis_prompt.contains("D:/Users/Alice"));
        assert!(!refinement_prompt.contains("D:/Users/Alice"));
    }

    #[test]
    fn json_repair_prompt_does_not_add_payload_instructions() {
        let prompt = json_repair_prompt("{\"ok\": true", "EOF");

        assert!(prompt.contains("Return only valid JSON"));
        assert!(prompt.contains("{\"ok\": true"));
        assert!(!prompt.contains("Markdown table"));
    }

    fn record(path: &str, detected_type: FileTypeBucket) -> FileInventoryRecord {
        let path = PathBuf::from(path);
        let name = path
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .to_string();
        FileInventoryRecord {
            file_id: path.to_string_lossy().replace('\\', "/"),
            root_relative_path: path.clone(),
            name,
            extension: path
                .extension()
                .map(|extension| extension.to_string_lossy().to_ascii_lowercase()),
            detected_type,
            size_bytes: 128,
            created_at: None,
            modified_at: Some(Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap()),
            accessed_at: None,
            depth: 1,
            entry_kind: FileEntryKind::File,
            scan_warnings: Vec::new(),
        }
    }
}
