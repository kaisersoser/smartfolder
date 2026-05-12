# smartfolder v1 MVP implementation plan

## Status

Delivered. All MVP milestones were completed.

## Problem and approach

Build `smartfolder` as a public Rust project in `D:\User\Projects\TinkerBox\FlexSorterApp`. The v1 product is a CLI-first, safety-focused folder organizer that uses metadata-only analysis, deterministic rules, explicit preview, confirmed apply, transaction journaling, and undo.

The plan deliberately narrowed scope to make a safe first release:

- metadata/filename/path analysis only
- deterministic rules before AI
- no desktop GUI
- no shell integration
- no content extraction
- no duplicate detection
- no automatic unsupervised organization

## Fixed product decisions

| Area | Decision |
|---|---|
| Project name | `smartfolder` |
| Audience | Power users and developers |
| Interface | CLI-first |
| Stack | Rust core plus Rust CLI |
| Operations | Real file moves only after preview and explicit confirmation |
| Journals | Stored in app-local data by default, optional export supported |
| Undo retention | Kept until explicit cleanup |
| Rules | Type/date/extension rules plus simple TOML custom profiles |
| Rule safety | Safe primitives only, no regex in v1 |
| Analysis | Metadata, filename, path, extension, size, timestamps only |
| AI | No AI behavior in v1 |
| Revalidation | Path + size + modified time |
| Destination scope | Destinations must stay inside the selected root |
| Symlinks/junctions | Include entries but do not follow them by default |
| Default exclusions | Hidden/system/project/dependency folders excluded unless included |
| Conflicts | Skip conflicted files and report them |
| Per-file failures | Continue when safe; stop only on fatal/systemic errors |
| Confirmation | `apply` requires interactive confirmation unless `--yes` is supplied |
| Output | Human-readable by default, JSON available with `--json` |
| Scale target | Metadata-only scans up to roughly 100k files |
| Cloud folders | Warn and require explicit confirmation before apply |

## Milestone plan

### Milestone 0 - Repository and project bootstrap

**Status:** done

Scope:

- initialize the Rust workspace
- add `smartfolder-core` and `smartfolder-cli`
- configure `smartfolder` as the CLI binary name
- add `.gitignore`, `.editorconfig`, README, GPL-3.0 LICENSE, and contributing docs
- add GitHub Actions for format/lint/test
- initialize local git repository

Acceptance criteria:

- CI runs `cargo fmt --check`, `cargo clippy`, and `cargo test`
- the repository builds from a fresh clone
- README states v1 scope and safety constraints

### Milestone 1 - Domain model and file inventory

**Status:** done

Scope:

- plan schema
- inventory schema
- transaction journal schema
- error taxonomy
- JSON serialization/deserialization
- app-local storage resolution
- cross-platform path normalization helpers

Acceptance criteria:

- schema round-trip tests exist
- path helpers reject escaping destinations
- schema versions are present in plan and journal records

### Milestone 2 - Metadata scanner

**Status:** done

Scope:

- recursive scan or current-folder-only scan
- max depth support
- default exclusions
- explicit include flags for hidden/system/project folders
- symlink/junction entries recorded without following them
- warnings for unreadable entries
- progress-ready scan summary
- cancellation support

Acceptance criteria:

- tests cover exclusions, depth handling, symlink behavior, and cancellation
- scanner remains metadata-only
- large synthetic scans are supported without reading file contents

### Milestone 3 - Built-in rules and TOML rule profiles

**Status:** done

Scope:

- built-in type/date/extension/type-year behavior
- TOML custom rules with safe primitives
- rule priority handling
- clear validation errors
- ambiguous files reported and left in place

Acceptance criteria:

- TOML rule profiles parse deterministically
- invalid rules fail before planning
- no regex support exists in v1
- ambiguous files are reported but not moved

### Milestone 4 - Plan generation and preview

**Status:** done

Scope:

- convert scan results and rules into safe operations
- enforce in-root destinations
- detect conflicts
- mark unsafe operations as unselected
- generate human-readable preview output
- generate JSON output
- export plans to JSON

Acceptance criteria:

- `analyze` writes a valid plan file
- `preview` shows source, destination, reason, and status
- conflicted files are reported and not auto-selected
- planning remains deterministic for identical inputs

### Milestone 5 - Apply engine and transaction journal

**Status:** done

Scope:

- interactive confirmation by default
- `--yes` for non-interactive scripting
- cloud-folder warning and confirmation
- source revalidation before move
- destination directory creation
- journal creation before file moves
- one-operation-at-a-time execution
- per-operation journal updates
- recoverable failure handling
- interruption handling

Acceptance criteria:

- apply never overwrites destinations
- journal captures completed, skipped, failed, and interrupted operations
- recoverable per-file failures do not corrupt the transaction record

### Milestone 6 - Undo, inspect, cleanup, and crash recovery

**Status:** done

Scope:

- `undo`
- `transactions list`
- `transactions inspect`
- `transactions cleanup`
- incomplete transaction detection
- safe rollback of completed moves
- source-state revalidation before undo
- overwrite refusal during undo
- partial rollback reporting

Acceptance criteria:

- fixture apply/undo restores the original layout
- interrupted transactions can be inspected
- cleanup keeps incomplete journals unless explicitly told otherwise

### Milestone 7 - CLI polish, safety hardening, and performance target

**Status:** done

Scope:

- consistent exit codes
- machine-readable JSON errors
- clear human-readable error messages
- scan/apply progress output
- safe Ctrl+C behavior
- large-scan performance validation
- Windows path and case-only rename hardening
- cloud-folder detection heuristics

Acceptance criteria:

- exit codes are documented
- large scans remain responsive and cancellable
- safety warnings are covered by tests where practical

### Milestone 8 - Public GitHub repository submission

**Status:** done

Scope:

- create the public `smartfolder` repository
- push the initialized local repository
- confirm GitHub Actions
- verify README, license, and repository visibility
- capture deferred roadmap follow-up

Acceptance criteria:

- public repository exists as `smartfolder`
- main contains the complete MVP
- CI is passing or failures are understood and fixed

## Testing strategy

The implementation plan required:

- unit tests for path safety, rules, schemas, and conflict handling
- integration tests for analyze/preview/apply/undo
- CLI-oriented tests for user-facing output and option handling
- JSON contract tests
- synthetic large-folder tests

The required end-to-end proof was:

```text
fixture folder
  -> smartfolder analyze
  -> smartfolder preview
  -> smartfolder apply
  -> smartfolder undo
  -> assert original layout restored
```

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Data loss from incorrect moves | Preview, confirmation, no overwrite, journal-before-move, undo |
| Race between analyze and apply | Revalidate path, size, and modified time |
| Destination escaping root | Normalize and reject out-of-root destinations |
| Symlink traversal surprises | Record links but do not follow them |
| Cloud sync conflicts | Detect cloud folders and require explicit confirmation |
| Large folder responsiveness | Metadata-only scan, progress output, cancellation |
| Repo leakage | Commit only project files, not user data or journals |

## Delivery outcome

This plan is complete. The MVP now exists as the current project baseline, and future work should build from this implementation rather than reopening v1 scope.
