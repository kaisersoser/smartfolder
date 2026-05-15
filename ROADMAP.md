# Roadmap

This document captures the strategic direction for `smartfolder` across release tracks. For implementation details and settled design decisions, see [`docs/v2-roadmap/`](docs/v2-roadmap/).

---

## Current status

| Track | Status |
|-------|--------|
| v1 MVP (CLI) | ✅ Released |
| v2.0 UX rewrite (GUI-first) | ✅ Released |
| v2.1 installer and distribution | ✅ Released |
| v2.2 AI-assisted organization | ✅ Released |
| v2.3 CLI AI parity | 📋 Planned |
| v2.x cross-platform tracks | 📋 Planned |

---

## v2.0 — Windows-first desktop release

**Goal:** Make `smartfolder` usable without terminal knowledge while preserving the safety model and CLI workflow that power users rely on.

**Must-win workflow:**

```text
Right-click folder in Explorer
  → Organize with smartfolder
  → Choose style
  → Analyze Folder
  → Preview
  → Organize Files
  → Undo Changes (if needed)
```

### Included in v2.0

- Full GUI shell with Organize, Activity, Rules, and Settings sections.
- Explorer context-menu launcher — opens GUI with selected folder preloaded.
- Style selection cards: By Type, By Date, Type + Date.
- Simplified preview (File / Destination / Status by default; full details on expand).
- Plain-language safety confirmation and immediate Undo Changes affordance after organizing.
- GUI rule editor (create and edit simple profiles without writing TOML).
- TOML rule import/export in advanced Rules actions.
- Portable Windows package (`scripts/package-portable.ps1`).
- Bounded-memory session storage (SQLite) for large folder trees.
- Include-subfolders toggle in CLI and GUI.
- CLI retained and fully compatible.

### Not in v2.0

- Installer or auto-update pipeline (v2.1+).
- AI-assisted organization.
- Duplicate detection.
- Content extraction or semantic classification.
- Deep Explorer shell verbs that directly apply operations.
- Cross-platform desktop release.
- Telemetry (none by default; local-first posture).

### Release gate

Automated test stack must pass **and** a manual disposable-folder proof must complete:

```
select or preload folder → analyze → preview → organize → undo → original layout restored
```

---

## v2.1 — Installer and distribution

**Goal:** make the Windows desktop app easy to install, update, and launch without relying on source checkouts or manual script registration.

- Windows installer (MSI or NSIS) with optional silent install.
- Start Menu shortcut and optional desktop shortcut.
- Explorer context-menu registration from the installer, with uninstall cleanup.
- Optional `smartfolder` CLI on PATH after install.
- Portable package retained for users who do not want installation.
- Release signing and version metadata for distributed binaries.

### Included in v2.1

- Per-user Windows installer script.
- Matching uninstall script.
- Installer-managed Explorer registration and cleanup.
- Start Menu shortcut and optional desktop shortcut.
- Optional CLI `PATH` registration.
- Portable package with GUI and CLI binaries.
- CLI saved-profile parity with GUI profile storage.
- CLI profile list/import/inspect/validate commands.

### Deferred after v2.1

- MSI/NSIS wrapper around the scripted installer.
- Code signing certificate and signed artifacts.
- Auto-update check; implementation remains opt-in only.

---

## v2.2 — AI-assisted organization

**Goal:** add optional AI assistance that helps users understand messy folders, generate validated custom rule profiles, and explain existing rules without weakening the deterministic preview-first organizer.

**Product boundary:**

- AI is optional and hidden outside Settings unless a usable provider/model connection is confirmed.
- The standard rules-based workflow remains the default experience.
- AI never moves files directly.
- AI output must become a recommendation or a draft profile that passes deterministic validation before use.
- Preview, conflict checks, confirmation, and undo history remain authoritative.
- v2.2 is GUI-first; CLI AI parity is deferred to v2.3.

### Included in v2.2

- AI Settings panel:
  - Enable AI assistance toggle.
  - Ollama provider support.
  - Default endpoint: `http://localhost:11434`.
  - Model selection from installed Ollama models.
  - Preferred model allowlist fallback, then first installed model.
  - Manual `Test connection` action.
  - Provider/model status states such as unavailable, ready, model missing, and request failed.
  - Configurable request timeout, defaulting to 60 seconds.
- Provider availability gate:
  - Provider is enabled.
  - Endpoint health check succeeds.
  - Configured model is reachable.
  - Tiny structured test request succeeds.
  - Contextual AI actions appear only after this gate passes.
- Optional content inspection:
  - Off by default.
  - Persistent setting with warnings.
  - No per-run confirmation.
  - Text-like files only in v2.2.
  - No OCR or broad binary/media inspection.
  - Large files are sampled, not sent whole.
  - AI runs visibly indicate whether content inspection is on.
- Organize flow integration:
  - `Analyze with AI` appears only after provider readiness is confirmed.
  - AI analysis uses the existing deterministic scan/analysis output as input.
  - Output includes summary, detected patterns, risks, recommended strategy, confidence, and evidence examples.
  - Draft profile generation is explicit and user-triggered.
  - Generated drafts open in the existing Profile workspace.
- Rules/Profile workspace integration:
  - `Build with AI` prompt-to-rules action.
  - AI prompt refinement action for clearer prompt wording before profile generation.
  - Requires selected folder or selected subfolder context.
  - AI receives the allowed rule schema, current folder context, user prompt, and existing profile context when relevant.
  - AI-generated rules are validated before appearing in the builder.
  - `Explain current rules` action generated on demand and cached until folder context or profile changes.
- Rule-generation constraints:
  - AI returns structured JSON, not freeform TOML.
  - App converts validated JSON into the existing rule profile model.
  - Allowed destination tokens: `{type}`, `{year}`, `{month}`, `{day}`, `{extension}`, `{filename}`.
  - Literal folder segments are allowed.
  - Conditions are limited to existing deterministic rule fields.
  - No AI-invented semantic tokens such as `{vendor}`, `{project}`, or `{client}` in v2.2.
- Validation stack:
  - Schema validation.
  - Safety validation for unsafe paths, unsupported tokens, invalid profile IDs, and invalid priorities.
  - Logical validation for contradictory or unreachable rules.
  - Applicability validation against selected folder metadata.
  - Zero-match rules are allowed with warnings.
  - Overlapping or shadowed rules are allowed with warnings and examples.
  - Existing preview validation remains required before organizing.
- Failure handling:
  - If Ollama/model/provider readiness fails, AI actions hide outside Settings.
  - Invalid AI JSON triggers one repair attempt.
  - Failed repair or failed validation leaves current profiles untouched; invalid drafts remain reviewable without replacing the current profile.
  - Timeouts stop cleanly; user cancellation drops UI interest and ignores late provider responses.
  - Unreadable content samples are skipped and reported as warnings.
- Observability:
  - Do not log file contents or full prompts by default.
  - Keep sanitized operational events only: provider, model, request type, duration, success/failure class, and validation results.
  - Settings includes an AI diagnostic export that excludes sensitive payloads by default.
  - Advanced users can expand raw AI draft JSON during draft review.

### Implementation status

1. ✅ Add an AI core module with request/response models, validation-safe schemas, and deterministic conversion into `RuleProfile`.
2. ✅ Implement the Ollama provider:
   - health check
   - model listing
   - model selection
   - tiny structured readiness test
   - analysis/explain/rule-draft requests
3. ✅ Add persisted AI settings:
   - enabled
   - endpoint
   - selected model
   - timeout
   - content inspection enabled
4. ✅ Add the provider availability state machine.
5. ✅ Add folder-context builders that derive AI-safe metadata from existing deterministic scan output.
6. ✅ Add scoped content sampling for text-like files behind the persistent content-inspection setting.
7. ✅ Add prompt builders and strict response schemas for:
   - folder analysis
   - prompt-to-profile
   - prompt refinement
   - rule explanation
8. ✅ Add deterministic validation for AI-generated profiles, including warnings for zero-match and shadowed rules.
9. ✅ Add Settings UI for AI configuration, status, diagnostics, and content-inspection warnings.
10. ✅ Add contextual AI actions to Organize and Rules/Profile workspace behind the availability gate.
11. ✅ Add cancellation, timeout, one-shot JSON repair, and validation-error surfaces.
12. ✅ Add tests for model selection, schema prompt coverage, safety validation, applicability warnings, content sampling, and no absolute-path leakage.
13. ✅ Run release verification:
   - AI unavailable behavior matches v2.1 except Settings.
   - Ollama available path works end to end.
   - Existing deterministic organize/undo tests still pass.

### Deferred after v2.2

- API key handling and authenticated cloud providers.
- Secure persisted credential storage.
- OpenAI-compatible custom endpoints that require API-key or Chat Completions compatibility.
- CLI AI parity.
- AI-native organizer separate from deterministic profiles.
- Semantic tokens such as `{vendor}`, `{project}`, and `{client}`.
- OCR, media analysis, and deep binary/document extraction.
- Automatic model downloads.
- Persisted AI prompt/history storage.

## v2.3 — CLI AI parity

**Goal:** expose the v2.2 AI capabilities to CLI users once the GUI-first AI workflow is stable.

Candidate commands:

- `smartfolder ai status`
- `smartfolder ai analyze <folder> [--json]`
- `smartfolder ai draft-profile <folder> --prompt "..." [--save-as <profile-id>]`
- `smartfolder profiles explain <profile-id> --folder <folder>`

No API-key support is planned until secure credential storage exists.

---

## Future considerations

The following topics are tracked but not committed to a specific release:

| Topic | Notes |
|-------|-------|
| Duplicate detection | Hash-based; requires content reads — out of scope until explicitly unlocked |
| Regex rules | Complex rule authoring for power users |
| Cross-platform GUI | Linux/macOS desktop builds once Windows is stable |
| Enterprise managed deployment | Admin policy and managed rule profiles |
| Scheduler / watch mode | Periodic or inotify-triggered auto-organize on confirmed profiles |

---

## Non-goals (permanent)

These are deliberate constraints of the `smartfolder` design and will not change without a major product direction review:

- No file content reads unless the user explicitly enables AI content inspection.
- No overwriting existing files.
- No destinations outside the selected root.
- No autonomous organization without explicit preview and confirmation.
- No symlink or junction following by default.
