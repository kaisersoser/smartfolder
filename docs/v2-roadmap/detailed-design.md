# smartfolder v2.0+ detailed design

## Status

Planned. This document captures the current 2.0+ direction agreed before implementation planning.

## Product thesis

`smartfolder` 2.0 should turn the current safe Rust engine into a real Windows-first desktop product without abandoning the trusted CLI workflow. The defining 2.0 outcome is a GUI-first experience that lets a user analyze a folder, preview a plan, apply changes safely, and undo them without opening a terminal.

The key principle is to improve product accessibility and usability without taking on high-risk engine expansion in the same release.

## Fixed product decisions for 2.0

| Area | Decision |
|---|---|
| Release definition | Windows-first desktop GUI release |
| Product shape | GUI primary, CLI retained for power users |
| Must-win workflow | Analyze -> preview -> apply -> undo wizard in the GUI |
| Engine scope | Stay metadata-only in 2.0 |
| Rules | Keep the current rule model, but add a GUI rule editor |
| AI / Ollama | Not in 2.0; later 2.x track |
| Explorer integration | Right-click should launch the GUI with the selected folder preloaded |
| Distribution | Portable app in 2.0, installer in 2.1+ |
| Compatibility | CLI remains; schema changes are allowed with migration |
| Performance | Match current large-folder responsiveness target |
| Privacy | No telemetry by default; local-first posture |
| GUI technology | `eframe` / `egui` chosen for the first Windows-first implementation track |

## Product goals

### 2.0 goals

1. Make smartfolder usable without terminal knowledge.
2. Preserve the current safety model and deterministic planning model.
3. Keep the Rust core as the source of truth for filesystem behavior.
4. Retain the CLI for advanced users and scripting.
5. Prepare the product for future 2.x expansion without overloading 2.0 scope.

### 2.0 success criteria

- A Windows user can launch the app, select a folder, run analysis, inspect the proposed organization plan, apply confirmed changes, and undo them from the GUI.
- The GUI remains responsive on large folder trees consistent with the current performance target.
- Existing core safety guarantees remain intact.
- Explorer right-click launching can preload a chosen folder into the app.
- Existing CLI users are not stranded; schema changes, if any, are migrated.

## Non-goals for 2.0

- AI-assisted organization
- duplicate detection
- content extraction or semantic classification
- fully cross-platform desktop release from day one
- installer or auto-update pipeline
- deep Explorer verbs that directly apply or preview operations from the shell
- enterprise admin or managed deployment features

## Primary users and flows

### Primary users

- current CLI users who want a faster visual workflow
- Windows users uncomfortable with terminal-first tools
- power users who still want inspectable plans and reversible operations

### Must-win flow

1. Launch smartfolder GUI.
2. Choose or receive a root folder.
3. Run metadata-only analysis.
4. Review plan summary and per-file operations.
5. Optionally adjust built-in mode or select a saved rule profile.
6. Confirm apply.
7. Review completion result and transaction reference.
8. Undo if needed.

### Secondary flows

- create and edit rule profiles visually
- open the app from Explorer with a selected folder preloaded
- inspect prior transactions and recover from interrupted operations
- export or inspect plan/journal data for power-user workflows

## Architecture direction

## High-level structure

The current Rust core remains the domain authority. 2.0 adds a desktop frontend and likely introduces a clearer boundary between product shell and shared engine orchestration.

```text
smartfolder/
  crates/
    smartfolder-core/
    smartfolder-cli/
    smartfolder-gui/          # planned
    smartfolder-platform/     # optional, if shell/platform hooks justify separation
```

### Shared core responsibilities

- scanner
- rule parsing and matching
- plan generation
- path and destination safety
- apply orchestration
- journal persistence
- undo and recovery
- shared schemas and migration logic

### GUI responsibilities

- folder selection
- analyze / preview / apply / undo flow
- transaction history UX
- rule editor UX
- progress and cancellation UI
- error presentation
- Explorer-launch handling

### CLI responsibilities in 2.0

- keep automation and scripting scenarios working
- preserve machine-readable output
- serve as a fallback and validation surface for the shared core

## GUI design boundaries

The GUI should not become a second implementation of core behavior. It should call shared Rust logic and expose the same safety semantics:

- preview before apply
- no overwrite
- explicit confirmation
- visible conflicts and skipped items
- undo through transaction journals

## Data model and migration

2.0 may evolve plan and journal schemas, but compatibility must be managed intentionally.

### Required migration stance

- schema changes are allowed
- schema versions must remain explicit
- the app should either read older data directly or migrate it with an explicit flow
- CLI and GUI should agree on the same underlying schemas

### Likely migration touchpoints

- richer plan metadata for GUI presentation
- richer transaction metadata for history views
- persisted GUI preferences
- rule-profile storage format and location

## Performance and UX requirements

- maintain current large-folder responsiveness expectations
- keep long-running analysis and apply flows visibly active
- support cancellation where safe
- avoid blocking the UI thread during scan or apply
- preserve metadata-only scanning in 2.0

## Privacy and trust model

- local-first by default
- no telemetry by default
- no file-content extraction in 2.0
- no cloud AI or local AI dependency in 2.0
- clear visibility into what will happen before filesystem changes occur

## Roadmap topics considered

| Topic | Decision |
|---|---|
| Desktop GUI | 2.0 core |
| Shared Rust core + retained CLI | 2.0 core |
| Analyze/preview/apply/undo GUI wizard | 2.0 core |
| GUI rule editor/profile management | 2.0 core |
| Explorer right-click launcher | 2.0 core |
| Plan/journal migration tooling | 2.0 core |
| Transaction history UX | likely 2.0 or early 2.1, needs final planning |
| Portable Windows release | 2.0 core |
| Installer and auto-update | 2.1+ |
| Cross-platform desktop support | later 2.x |
| AI/Ollama recommendations | later 2.x |
| Duplicate detection | later 2.x |
| Content extraction/classification | later 2.x |
| Regex or advanced rule language | later 2.x |
| Reusable rule templates/presets | later 2.x |
| Deep shell integration beyond launcher | later 2.x |
| Enterprise/admin features | deferred |

## Open design questions

These are intentionally unresolved and should be answered during implementation planning:

1. Whether transaction history UX is part of 2.0 core or 2.1.
2. How far schema migration should go automatically versus explicitly.
3. Whether rule profiles remain TOML under the hood or adopt a richer internal storage layer.

## GUI framework evaluation outcome

The first implementation track chooses `eframe` / `egui` because it best matches the current constraints:

- Rust-native UI without adding a web runtime dependency
- practical Windows-first portable distribution
- direct reuse of the existing Rust core
- straightforward argument-based folder preloading for later Explorer integration
- low-friction scaffolding for fast iteration

The initial GUI crate is `crates\smartfolder-gui`.

## Release philosophy

Version 2.0 should not try to solve every deferred idea from v1. It should deliver a decisive product upgrade by making the current trusted engine accessible, visual, and easier to operate on Windows, while leaving AI, duplicates, content understanding, and advanced automation for later 2.x releases.
