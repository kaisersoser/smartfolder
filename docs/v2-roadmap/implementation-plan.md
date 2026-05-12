# smartfolder v2.0 implementation plan

## Status

Implementation started. Milestone 0 is complete, and Milestone 2 is currently in progress.

## Problem and approach

Build `smartfolder` 2.0 as a Windows-first portable desktop application that makes the current safe organization engine available through a GUI-first workflow, while retaining the CLI for advanced and scripted use.

The implementation plan intentionally keeps 2.0 focused:

- keep the engine metadata-only
- do not add AI, duplicate detection, or content extraction
- do not require installer or auto-update infrastructure
- do add GUI-first scan/preview/apply/undo workflows
- do add a GUI rule editor
- do add Explorer right-click launching that preloads a folder into the app
- do allow schema evolution, but only with explicit migration strategy

## Delivery principles

| Principle | Meaning |
|---|---|
| Shared core first | GUI and CLI must reuse the same Rust logic |
| Safety preserved | GUI cannot weaken preview, confirmation, no-overwrite, or undo guarantees |
| Windows-first | Optimize the first release for Windows packaging, shell launch, and UX |
| Bounded memory | Large scan, plan, and transaction data must be page-oriented instead of retained wholesale in memory |
| Incremental migration | Evolve schemas deliberately and version them clearly |
| Product over experimentation | Defer AI and deeper engine expansion until after a successful GUI release |

## Proposed milestone plan

### Milestone 0 - Architecture spike and GUI technology decision

**Status:** done

Scope:

- compare viable GUI approaches
- evaluate Windows packaging constraints
- verify shared-core integration strategy
- verify Explorer-launch feasibility
- choose GUI framework and project structure

Acceptance criteria:

- a single GUI stack is chosen
- the repo layout for GUI work is agreed
- the shell-launch approach is technically validated

Implementation outcome:

- selected `eframe` / `egui` as the Windows-first GUI stack
- added `crates\smartfolder-gui`
- validated argument-based folder preloading as the basis for later Explorer launching

### Milestone 1 - Shared core boundary and schema migration foundation

**Status:** planned

Scope:

- identify any CLI-only assumptions that must move out of UI code
- define a stable interface from GUI to core orchestration
- add schema migration/versioning strategy for plans and journals
- preserve CLI compatibility through shared model changes

Acceptance criteria:

- core logic remains UI-agnostic
- GUI and CLI can use the same plan/journal model layer
- migration behavior is documented and testable

### Milestone 1A - Bounded-memory session storage

**Status:** in_progress

Scope:

- introduce an embedded SQLite session store for large working sets
- stream scan records into durable storage instead of retaining every record in memory
- generate plans from stored scan rows in bounded pages
- store plan operations for paged GUI retrieval
- batch database writes so large scans do not stall on per-row commits
- expose progress, cancellation, and current-work detail during scan and planning
- provide cleanup/compaction paths for stale working-session data
- preserve existing CLI JSON workflows during migration

Acceptance criteria:

- large GUI scans do not retain all scan records in process memory
- plan previews can be queried page-by-page from storage
- duplicate destination detection uses indexed storage instead of a process-wide destination set
- scan and plan work can be cancelled from the GUI
- stale session data can be deleted and the database can be compacted
- existing in-memory CLI APIs remain compatible until CLI migration is intentional

Progress so far:

- added SQLite-backed session database under app-local data
- added streaming scan sink API
- added paged plan generation into session storage
- moved GUI analysis to session-backed scan and preview storage
- added live scan/planning progress and cancellation controls
- added stale-session cleanup and database compaction APIs

### Milestone 2 - Desktop app shell and portable packaging baseline

**Status:** in_progress

Scope:

- add the desktop app crate and window shell
- create basic navigation and app state management
- implement folder picker / preload handling
- produce a portable Windows build artifact

Acceptance criteria:

- app launches as a standalone Windows desktop program
- a root folder can be selected or preloaded
- a portable release build can be produced locally

Progress so far:

- GUI crate scaffolded in the workspace
- initial window shell implemented
- folder preloading by startup argument implemented
- folder picker and built-in mode selection implemented
- shared-core analyze flow and preview rendering implemented in the GUI

### Milestone 3 - Analyze experience and plan summary UI

**Status:** in_progress

Scope:

- run metadata-only scans from the GUI
- show progress and cancellation state
- expose built-in rule mode selection
- render high-level plan summaries
- surface warnings, ambiguous files, and conflicts

Acceptance criteria:

- a user can select a folder and generate a plan from the GUI
- long-running scans keep the UI responsive
- warnings and exclusions are visible

Progress so far:

- added GUI analysis through the shared core
- added bounded-memory progress reporting for scan and planning
- added a high-level plan summary with ready, attention, ambiguous, and warning counts
- added paged preview controls for all operations, ready operations, and operations needing attention
- kept preview rows truncated and loaded from SQLite by page

### Milestone 4 - Preview details and safe apply flow

**Status:** in_progress

Scope:

- render detailed operation previews
- show selected vs conflicted items clearly
- add explicit confirmation UX before apply
- surface cloud-folder confirmation requirement
- run apply with progress updates and final result summary

Acceptance criteria:

- a user can inspect planned operations before applying
- apply uses the shared safety model
- apply progress and failures are visible in the GUI

Progress so far:

- added a core stored-plan apply path that pages ready operations from SQLite
- reused the existing journaled move executor and no-overwrite safety checks
- added explicit GUI confirmation before applying file moves
- added cloud-synced folder caution in the confirmation step
- added apply progress and final transaction summary in the GUI

### Milestone 5 - Undo, transaction history, and recovery UX

**Status:** in_progress

Scope:

- expose recent transactions in the GUI
- inspect transaction details
- trigger undo from the GUI
- show interrupted/failed transaction states

Acceptance criteria:

- a GUI user can undo a prior completed transaction
- journal-backed recovery states are visible
- rollback results are clearly reported

Progress so far:

- added recent transaction history to the GUI from journal storage
- added a bounded transaction detail inspector for journal metadata and recorded operations
- added transaction status visibility and refresh controls
- added explicit undo confirmation before rollback
- wired GUI undo through the shared recovery model
- added undo result summaries with rolled back, skipped, failed, and journal path details

### Milestone 6 - GUI rule editor and profile management

**Status:** planned

Scope:

- create and edit profiles visually
- validate supported rule conditions in the UI
- preserve the current rule model
- import/export or persist profiles predictably

Acceptance criteria:

- a user can create a valid rule profile without editing raw TOML
- invalid rule inputs are blocked with clear validation
- profile behavior matches CLI/core rule semantics

### Milestone 7 - Explorer launcher integration

**Status:** planned

Scope:

- add a Windows Explorer right-click entry point
- pass selected folder context into the GUI
- launch the app with the folder preloaded

Acceptance criteria:

- right-clicking a folder can launch smartfolder
- the GUI opens with that folder already selected
- no shell verb applies filesystem changes directly

### Milestone 8 - Performance, hardening, and accessibility pass

**Status:** planned

Scope:

- validate large-folder responsiveness against current expectations
- harden cancellation and error presentation
- handle edge cases around cloud folders and path conflicts in the GUI
- improve keyboard usability and accessibility basics

Acceptance criteria:

- large scans remain responsive
- GUI error states remain actionable
- core safety warnings are surfaced consistently

### Milestone 9 - Portable 2.0 release documentation and publication

**Status:** planned

Scope:

- document portable app usage
- document CLI/GUI coexistence
- document migration behavior
- publish the Windows portable 2.0 release

Acceptance criteria:

- release notes explain what changed from v1
- users can install or run the portable build without CLI knowledge
- compatibility and migration behavior are documented

## Testing strategy

2.0 should expand the existing test stack rather than replace it.

### Core requirements

- keep existing core unit and integration tests green
- add migration tests for any schema changes
- add GUI integration tests for analyze/preview/apply/undo flows
- add Explorer-launch tests where practical
- add regression coverage for GUI rule editing
- preserve CLI regression coverage as a compatibility signal

### Required end-to-end proof for 2.0

```text
Explorer or app launch
  -> root folder preloaded or selected
  -> analyze
  -> preview
  -> apply
  -> undo
  -> assert original layout restored
```

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| GUI duplicates or diverges from core logic | Keep all filesystem behavior in shared Rust core |
| Schema changes strand CLI users | Explicit schema versions and migration path |
| GUI becomes sluggish on large folders | Background work, progress reporting, performance gates |
| Explorer integration increases risk | Limit shell integration to launching the app with context |
| 2.0 scope expands uncontrollably | Keep AI, duplicates, content extraction, and installer work out of 2.0 |
| Windows-first choices make later cross-platform support harder | Keep framework choice open until architecture spike is complete |

## Explicitly out of scope for 2.0

- AI recommendations
- automatic organization without user preview
- duplicate detection
- content extraction
- deep shell verbs that apply actions from Explorer
- installer and auto-update
- enterprise administration features

## Follow-on roadmap after 2.0

### Candidate 2.1+ work

- installer
- updater
- deeper transaction history UX
- reusable templates and presets

### Candidate later 2.x work

- AI/Ollama suggestions
- duplicate detection
- content extraction/classification
- advanced rule language or regex
- cross-platform desktop expansion
