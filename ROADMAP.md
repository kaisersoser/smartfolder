# Roadmap

This document captures the strategic direction for `smartfolder` across release tracks. For implementation details and settled design decisions, see [`docs/v2-roadmap/`](docs/v2-roadmap/).

---

## Current status

| Track | Status |
|-------|--------|
| v1 MVP (CLI) | ✅ Released |
| v2.0 UX rewrite (GUI-first) | 🔶 Release-candidate — manual smoke review remaining |
| v2.1+ (installer, AI, cross-platform) | 📋 Planned |

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

- Windows installer (MSI or NSIS) with optional silent install.
- Auto-update check (opt-in).
- Pinned `smartfolder` on PATH after install.

---

## v2.x — AI-assisted organization

After 2.0 ships and the engine is stable:

- Optional local-model (Ollama) suggestions for ambiguous files.
- Confidence-scored recommendations shown in the preview panel.
- No autonomous moves — AI suggestions feed the same preview → confirm → undo workflow.
- Privacy preserved: metadata only, no file contents sent to any model.

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

- No file content reads.
- No overwriting existing files.
- No destinations outside the selected root.
- No autonomous organization without explicit preview and confirmation.
- No symlink or junction following by default.
