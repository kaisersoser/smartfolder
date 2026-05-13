# smartfolder v2.0 implementation plan

## Status

This plan is finalized for v2.0 release-candidate review.

The original v2 technical foundation is implemented: GUI launch, folder selection, bounded-memory analysis, paged preview, safe apply, undo, profile import/editing, Explorer launcher script, and portable packaging script exist.

The v2 priority changed from capability expansion to a major UX rewrite. UX-1 through UX-9 are now implemented as the release-candidate track. Automated validation is complete; the remaining release gate is manual GUI smoke review from the Windows desktop workflow.

## Product direction

smartfolder 2.0 should become a Windows-first desktop product that helps users organize files safely without requiring them to understand sessions, journals, transactions, SQLite storage, or recovery internals.

The primary workflow is:

```text
Right-click folder in Explorer -> smartfolder opens with that folder preselected -> choose style -> analyze -> preview -> organize -> undo if needed
```

The product should feel:

- calm
- safe
- modern
- understandable
- reversible
- powerful without intimidation

## Settled UX decisions

These decisions are approved for the v2 UX rewrite:

| Area | Decision |
|---|---|
| Rewrite shape | Full GUI rewrite, while keeping the main build green throughout |
| First implementation slice | Organize screen only |
| Default user model | Balanced: beginner-friendly default, power-user depth available |
| Advanced disclosure | Per-section advanced panels, not one global advanced mode |
| Default preview | `File`, `Destination`, `Status` only |
| Rule editor scope | Current/simple profile editor is enough for 2.0 |
| Visual tone | Bold modern product |
| Copy tone | Warm, reassuring, plain language |
| Launch model | Explorer/folder-context launch remains a primary entry path; clicked folder must prefill the Organize screen |
| First UX gate | Manual flow passes without documentation: right-click folder -> open smartfolder -> analyze -> preview -> organize -> undo |

## Non-negotiables

- Keep the engine metadata-only.
- Do not add AI, duplicate detection, content extraction, autonomous organization, or regex-heavy workflows in 2.0.
- Do not weaken preview, explicit confirmation, no-overwrite behavior, journaling, or undo.
- Do not hide safety failures to make the UI look cleaner.
- Do not expose internal storage concepts as primary workflow language.
- Keep CLI compatibility and shared-core behavior intact.
- Keep the project compiling after each implementation slice.

## Terminology direction

Primary UI terminology should move away from implementation language:

| Current/internal term | Primary UX term |
|---|---|
| Apply ready moves | Organize Files |
| Transaction | Activity |
| Journal | History |
| Undo transaction | Undo Changes |
| Ambiguous | Needs Review |
| Selected | Ready |
| Recovery log | Restore History |
| Session | Hidden from primary UI |

Technical terms may still appear in advanced panels, exported data, logs, and diagnostics.

## Current foundation already implemented

These shipped pieces become the implementation substrate for the UX rewrite:

- `eframe` / `egui` GUI crate exists.
- GUI can be launched standalone or with a preloaded folder argument.
- Folder picker exists.
- Built-in organization modes exist.
- Current-folder-only scanning is the default, with explicit subfolder opt-in.
- SQLite-backed session storage exists for bounded-memory scan and preview workflows.
- GUI analysis runs through the shared core.
- Planning and preview are paged from SQLite.
- Safe apply runs through the shared core and writes transaction journals.
- Undo runs through shared recovery logic.
- Current-folder activity and technical recovery details exist.
- Rule profile import and a simple one-rule visual profile editor exist.
- Explorer launcher registration script exists.
- Explorer launch already provides the selected folder as startup context and must remain supported in the rewrite.
- Portable packaging script and portable package documentation exist.

## Finalized milestone record

### Milestone UX-0 - Baseline stabilization before rewrite

**Status:** implemented - baseline and release-candidate validation complete for the required UX test stack

Purpose:

Freeze the current functional GUI baseline before the major UX rewrite starts.

Scope:

- update documentation so the v2 roadmap points to the UX rewrite track
- run and record baseline validation commands
- identify reusable GUI functions and state that must survive the rewrite
- mark the current single-page GUI as the functional baseline, not the final product design
- preserve Explorer/context-launch folder preselection behavior as a first-class workflow requirement

Acceptance criteria:

- `cargo fmt --check` passes
- `cargo test -p smartfolder-core` passes
- `cargo test -p smartfolder-cli` passes
- `cargo test -p smartfolder-gui` passes
- `cargo build -p smartfolder-gui --release` passes
- current GUI can still complete analyze -> preview -> organize -> undo
- launching from Explorer or equivalent preloaded-folder path still opens the app with the folder already selected

Implementation notes:

- Do not change core behavior in this milestone unless required to keep tests green.
- The visual redesign was implemented after the baseline was established and validated.

### Milestone UX-1 - New Organize screen shell

**Status:** implemented - release-candidate scope complete

Purpose:

Replace the current dense single-page utility layout with the first slice of the new product shell, focused only on the Organize screen.

Scope:

- introduce the new Organize screen layout
- keep the main build green throughout
- keep existing app state and shared-core operations reusable
- create a bold modern visual direction using egui styling available in the current stack
- preserve existing analyze/apply/undo behavior behind the new layout
- make preloaded-folder launch feel native to the Organize screen, not like a secondary path

Out of scope:

- full Activity screen redesign
- full Rules screen redesign
- Settings screen implementation
- multi-rule editor expansion

Acceptance criteria:

- app opens directly to Organize
- if launched from Explorer, the clicked folder is already populated and ready for Analyze Folder
- folder selection is visually prominent
- organization style selection is visible as cards, not radio buttons
- Analyze Folder is the dominant call to action
- existing analysis still works through the shared core
- existing apply and undo flows still work
- no internal terms like session, transaction, or journal dominate the primary screen

Manual proof:

```text
Right-click folder in Explorer -> Organize with smartfolder -> folder is already selected -> choose style -> Analyze Folder
```

### Milestone UX-2 - Guided folder and style selection

**Status:** implemented - release-candidate scope complete

Purpose:

Make the starting point and organization choice obvious to a first-time user.

Scope:

- redesign folder input as `Choose a folder to organize`
- keep Browse support
- preserve Explorer preload support
- treat Explorer-preloaded folder as the default entry path to optimize for, not just a compatibility feature
- add recent-folder presentation if storage support is straightforward; otherwise stub the UI for later
- replace built-in mode combo/radio UI with style cards
- map style cards to existing built-in modes:
  - By Type -> `BuiltInMode::Type`
  - By Date -> `BuiltInMode::Date`
  - Type + Date -> `BuiltInMode::TypeYear`
  - Custom Rules -> loaded/simple profile flow
- move Include subfolders into an Organize advanced/details area

Acceptance criteria:

- folder selection is the first obvious step
- a preloaded Explorer folder is clearly shown as the active folder without requiring extra clicks
- style cards include short examples of resulting folders
- selected style is visually clear
- Analyze Folder is disabled until a folder is provided
- current-folder-only remains the default
- include-subfolders remains available but no longer clutters the primary workflow

Manual proof:

```text
User can understand the first two actions within 30 seconds: confirm the preselected folder, choose style.
```

### Milestone UX-3 - Analysis progress and summary cards

**Status:** implemented - release-candidate scope complete

Purpose:

Replace dense technical analysis feedback with clear progress and outcome framing.

Scope:

- redesign scan/planning progress into plain-language progress feedback
- show current work without exposing implementation terms
- keep cancellation visible
- replace summary text with summary cards:
  - Ready to organize
  - Needs review
  - Left untouched
- add confidence framing that explains cautious behavior as protection
- keep warnings available through a details area

Acceptance criteria:

- long-running analysis visibly progresses
- cancellation remains available
- summary is scannable at a glance
- ambiguous/unplanned files are framed as Needs Review or Left untouched
- users can still inspect warnings/details when needed

Manual proof:

```text
After analysis, user can answer: how many files are safe to organize, how many need review, and why smartfolder held some files back.
```

### Milestone UX-4 - Simplified preview with progressive details

**Status:** implemented - release-candidate scope complete

Purpose:

Make planned changes understandable without overwhelming users.

Scope:

- default preview columns:
  - File
  - Destination
  - Status
- hide reason, original path, exact destination path, and metadata by default
- provide row expansion or a side/detail panel for advanced details
- keep paging or incremental loading internally, but avoid exposing paging terminology prominently
- preserve filters:
  - All
  - Ready
  - Needs Review
  - Conflicts, if distinguishable from needs-review state
- add search/filtering if feasible in the existing SQLite-backed preview model

Acceptance criteria:

- default preview is visually simple
- power users can inspect exact move details quickly
- full source/destination paths remain accessible
- rule reasons remain accessible
- preview remains paged/bounded for large folders
- current safety semantics are unchanged

Manual proof:

```text
User can understand where files will go without reading full absolute paths or rule internals.
```

### Milestone UX-5 - Organize Files apply flow and immediate undo

**Status:** implemented - release-candidate scope complete

Purpose:

Turn apply/undo into the central trust-building moment of the product.

Scope:

- replace Apply ready moves copy with Organize Files
- redesign confirmation dialog around safety guarantees:
  - files will be moved into organized folders
  - existing files will never be overwritten
  - undo will be available after completion
- keep explicit confirmation required
- show apply progress in plain language
- completion state must prominently show:
  - Organization complete
  - count of files organized
  - Undo Changes button
  - View Details button
- keep journal-backed undo behavior unchanged

Acceptance criteria:

- user sees Undo Changes immediately after organization completes
- user does not need to find transaction/activity history to undo the just-applied action
- confirmation explains safety and reversibility
- failures/skips are reported clearly and honestly
- no overwrite and journal-before-move behavior remain intact

Manual proof:

```text
Analyze -> preview -> Organize Files -> Undo Changes restores the original layout for a disposable folder.
```

### Milestone UX-6 - Activity screen and restore history language

**Status:** implemented - release-candidate scope complete

Purpose:

Reframe transaction history as user-readable activity and restore history.

Scope:

- introduce or redesign Activity navigation section
- default activity list uses human-readable entries:
  - Organized 122 files in Documents
  - Undone organization in Downloads
- keep activity scoped to the selected/current folder where helpful
- keep advanced transaction/journal details available in an expandable panel
- rename technical recovery log to Restore History in the user-facing UI
- preserve transaction ids and journal paths in advanced details

Acceptance criteria:

- a user can find recent organization actions without understanding transactions
- undo availability is obvious for completed activities
- failed/interrupted activities are clearly explained
- advanced details retain all recovery/debug information

Manual proof:

```text
User can find a prior organization action and understand whether it can be undone.
```

### Milestone UX-7 - Rules screen cleanup

**Status:** implemented - release-candidate scope complete

Purpose:

Make rule/profile management approachable without blocking the UX rewrite on a full advanced designer.

Scope:

- introduce or refine Rules section
- present built-in styles visually
- list saved profiles clearly
- keep the simple one-rule profile editor
- keep TOML import/export available as advanced actions
- preserve shared-core `RuleProfile` validation

Out of scope for 2.0:

- full multi-rule drag/drop editor
- reusable rule templates beyond simple examples
- regex or advanced rule language

Acceptance criteria:

- user can create a simple valid profile without raw TOML
- imported profiles can still be used
- invalid profiles are blocked with clear validation
- profile behavior matches CLI/core semantics

Manual proof:

```text
Create simple PDF profile -> Analyze Folder with Custom Rules -> preview expected destination.
```

### Milestone UX-8 - Settings, advanced controls, and desktop integration polish

**Status:** implemented - release-candidate scope complete

Purpose:

Move less-common controls out of the primary workflow while keeping them accessible.

Scope:

- define sparse Settings sections:
  - Appearance
  - Safety
  - History
  - Advanced
- decide which settings are actually persisted in 2.0
- keep Clean old session data out of the primary Organize path
- keep subfolder/exclusion behavior discoverable through contextual advanced panels
- rename Explorer context action to `Organize with smartfolder`
- ensure Explorer registration remains launch-only and never applies changes directly

Acceptance criteria:

- primary Organize path is uncluttered
- advanced controls remain discoverable
- Explorer action uses human product language
- no shell integration mutates files directly

Manual proof:

```text
Explorer context menu opens smartfolder with the selected folder preloaded and no file changes are made.
```

### Milestone UX-9 - Hardening, accessibility, and release readiness

**Status:** implemented - automated validation complete; manual GUI smoke review remains recommended before tagging release

Purpose:

Turn the redesigned GUI into a release candidate.

Scope:

- validate responsiveness on large folders
- harden cancellation and error presentation
- test cloud-folder warnings and confirmation language
- test conflict/no-overwrite messaging
- improve keyboard navigation
- improve visible focus states
- improve contrast and scalable text behavior where egui allows
- run disposable-folder end-to-end apply/undo tests
- finalize portable package docs and release notes

Acceptance criteria:

- large scans remain responsive
- GUI errors are actionable and safety-oriented
- keyboard users can complete the core flow
- warning language is consistent with the new terminology
- portable package can be built locally
- release notes explain the v2 GUI workflow and compatibility with the CLI

Required end-to-end proof:

```text
Explorer or app launch
  -> folder preloaded or selected
  -> Analyze Folder
  -> Preview
  -> Organize Files
  -> Undo Changes
  -> original layout restored
```

Automated validation completed 2026-05-13:

- `cargo fmt --check`
- `cargo test -p smartfolder-core`
- `cargo test -p smartfolder-cli`
- `cargo test -p smartfolder-gui`
- `cargo build -p smartfolder-gui --release`
- PowerShell parser checks for `scripts/register-explorer-launcher.ps1` and `scripts/package-portable.ps1`
- portable package smoke test with `scripts/package-portable.ps1 -SkipBuild -OutputRoot .\target\tmp\ux9-portable-smoke`

Manual release review still recommended:

- launch the release GUI from Explorer and confirm the clicked folder is preselected
- run the disposable-folder GUI proof: Analyze Folder -> Preview -> Organize Files -> Undo Changes -> original layout restored
- visually review the Organize, Activity, Rules, and Settings screens for text clipping at the target window size

## Testing strategy

The UX rewrite must preserve the existing test stack while adding product-flow validation.

Required automated checks:

- `cargo fmt --check`
- `cargo test -p smartfolder-core`
- `cargo test -p smartfolder-cli`
- `cargo test -p smartfolder-gui`
- `cargo build -p smartfolder-gui --release`
- PowerShell parser checks for release and Explorer scripts

Required manual checks:

- fresh launch with no folder selected
- Explorer/preloaded folder launch
- Explorer/preloaded folder launch with immediate Analyze eligibility and no forced re-selection
- current-folder-only analysis
- include-subfolders analysis from advanced controls
- style card selection for all built-in modes
- custom profile analysis
- preview detail expansion
- organize files confirmation
- immediate Undo Changes
- Activity restore for an older action
- portable package smoke test

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Rewrite breaks existing safe behavior | Keep shared core unchanged and run core/CLI/GUI tests after each slice |
| UI hides important safety details | Move technical data into details panels, not out of the product |
| Visual redesign expands scope | Implement Organize screen first and defer full Activity/Rules/Settings polish |
| egui limits styling ambition | Use spacing, color, cards, hierarchy, and layout before considering framework changes |
| Users cannot find advanced controls | Use contextual advanced panels with clear labels |
| Undo becomes hidden again | Treat immediate Undo Changes as a required completion-state control |
| Large previews become sluggish | Preserve SQLite paging/incremental loading internally |
| Technical terminology leaks into primary UX | Enforce the terminology table in UI copy reviews |

## Explicitly out of scope for v2.0 UX rewrite

- AI recommendations
- automatic organization without preview
- duplicate detection
- content extraction or semantic classification
- regex-heavy workflows
- full multi-rule designer
- installer and auto-update
- enterprise/admin features

## Final release gate

The UX rewrite plan is approved and implemented through UX-9. Do not expand v2.0 scope before release-candidate review; keep AI recommendations, automatic organization, duplicate detection, content extraction, regex-heavy workflows, and a full multi-rule designer out of this release.

Release-candidate signoff should focus on the manual desktop proof that automated tests cannot cover:

- Explorer context menu opens `Organize with smartfolder` with the clicked folder preselected.
- A disposable folder completes Analyze Folder -> Preview -> Organize Files -> Undo Changes.
- Undo restores the original layout.
- Organize, Activity, Rules, and Settings remain readable at the target window size.

If those checks pass, the v2.0 UX rewrite is ready to tag as a release candidate.