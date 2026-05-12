# smartfolder v1 MVP detailed design

## Status

Implemented and published as the current CLI-first product.

## Product summary

`smartfolder` v1 is a safe, deterministic, CLI-first folder organizer for power users and developers. It analyzes a selected root folder using metadata only, generates an explicit move plan, previews proposed operations, applies only confirmed moves, writes transaction journals during execution, and supports undo.

The v1 design intentionally favors safety, transparency, and reversibility over aggressive automation.

## Goals and success criteria

### Primary goals

1. Let a user scan a folder without reading file contents.
2. Generate deterministic move plans from built-in or TOML-defined rules.
3. Preview every proposed move before changing the filesystem.
4. Apply moves safely with explicit confirmation and journaled recovery state.
5. Undo completed transactions when the recorded state is still safe to reverse.

### Success criteria

- A user can run `analyze -> preview -> apply -> undo` on a fixture folder and end with the original layout restored.
- No operation overwrites an existing destination.
- All generated destinations stay inside the selected root.
- Cloud-synced roots require explicit confirmation before apply.
- The scanner remains metadata-only and responsive on large folder trees.

## Intended users

- Power users managing large personal folders
- Developers comfortable with CLI workflows
- Early adopters who value explicit preview and recovery over automation magic

## Scope

### Included in v1

- Rust workspace with shared core and CLI frontend
- Metadata-only scanning
- Built-in organization modes:
  - `type`
  - `date`
  - `extension`
  - `type-year`
  - CLI aliases `type-date` and `type-year-month-day`
- Simple custom TOML rules
- Human-readable preview output
- JSON output for plans, previews, and machine-readable errors
- Apply, undo, transaction list, transaction inspect, and transaction cleanup
- Cross-platform-safe path handling
- CLI safety hardening and progress reporting

### Explicit non-goals for v1

- Desktop GUI
- Shell or file-manager integrations
- AI or Ollama-driven behavior
- Content extraction from files
- Duplicate detection
- Regex-based custom rules
- Enterprise policy management or managed deployment
- Automatic unsupervised organization

## Core user flows

### 1. Analyze a folder

The user selects a root folder and runs:

```powershell
smartfolder analyze <root> --output plan.json
```

The scanner walks the tree, applies exclusion policy, records metadata-safe inventory records, and generates a deterministic plan.

### 2. Preview the plan

The user reviews:

```powershell
smartfolder preview plan.json
```

The preview shows the source, destination, reason, and whether each operation is selected or conflicted.

### 3. Apply the plan

The user confirms:

```powershell
smartfolder apply plan.json
```

or uses:

```powershell
smartfolder apply plan.json --yes
```

The app revalidates the source state, writes a transaction journal before movement, performs operations one at a time, and records final status for each operation.

### 4. Undo a transaction

The user runs:

```powershell
smartfolder undo <transaction-id>
```

The undo flow reverses only previously completed moves and refuses to overwrite an existing original source path.

## Command surface

### Commands

```powershell
smartfolder analyze <root> --output plan.json
smartfolder analyze <root> --profile rules.toml --output plan.json
smartfolder preview plan.json
smartfolder apply plan.json
smartfolder apply plan.json --yes
smartfolder undo <transaction-id>
smartfolder transactions list
smartfolder transactions inspect <transaction-id>
smartfolder transactions cleanup
```

### Important flags

```text
--json
--include-hidden
--include-system
--include-project-folders
--max-depth <n>
--current-folder-only
--exclude <name>
--profile <rules.toml>
--journal-export <path>
--confirm-cloud-folder
--yes
```

## Rules model

### Built-in rules

- `type` -> `Type\filename`
- `date` -> `Year\MonthName\Day\filename`
- `extension` -> `extension\filename`
- `type-year` / `type-date` / `type-year-month-day` -> `Type\Year\MonthName\Day\filename`

Month folders use full English month names, for example `May`.

### Custom TOML rules

Supported fields:

- `name`
- `destination`
- `priority`
- `extensions`
- `filename_contains`
- `path_contains`
- `min_size_bytes`
- `max_size_bytes`
- `year`

Supported destination tokens:

- `{type}`
- `{year}`
- `{month}`
- `{day}`
- `{extension}`
- `{filename}`

Regex is intentionally unsupported in v1. Unknown rule fields fail validation.

## System architecture

## Workspace structure

```text
smartfolder/
  Cargo.toml
  crates/
    smartfolder-core/
    smartfolder-cli/
  tests/
  .github/
  README.md
  LICENSE
```

### `smartfolder-core`

Owns domain logic and is intentionally terminal-agnostic:

- filesystem scanning
- file inventory record generation
- rule profile parsing and matching
- plan generation
- preview JSON rendering
- safe destination validation
- transaction journal writing
- apply orchestration
- undo and transaction recovery
- shared errors and data contracts

### `smartfolder-cli`

Owns command parsing and user interaction:

- command dispatch
- confirmation prompts
- human-readable output
- JSON output and JSON error formatting
- exit codes
- Ctrl+C handling

## Data contracts

### File inventory record

Tracks metadata-safe fields only:

- stable file id
- root-relative path
- file name and extension
- detected type bucket
- size
- created / modified / accessed timestamps where available
- directory depth
- file entry kind
- scan warnings

### Plan record

Contains:

- schema version
- plan id
- root path
- built-in mode or rule profile id
- created timestamp
- operations
- ambiguous files
- warnings
- summary counts

Each operation includes:

- operation id
- source path
- destination path
- reason
- certainty
- conflict state
- selected state
- source snapshot for revalidation

### Transaction journal

Contains:

- schema version
- transaction id
- plan id
- root path
- transaction status
- started / completed timestamps
- per-operation status
- operation errors
- same-volume metadata where known

## Safety model

### Destination safety

- All generated destinations must remain inside the selected root.
- Parent traversal and out-of-root destinations are rejected.
- Case-only rename conflicts are marked unsafe.
- Existing destination conflicts are skipped.
- Long legacy Windows path risks are flagged as unsafe where practical.

### Apply safety

- Preview-first workflow
- Interactive confirmation by default
- `--yes` only for explicit non-interactive use
- Source revalidation using path, size, and modified time
- No overwrite policy
- Journal written before and during apply
- Continue on recoverable per-file failures
- Stop only on fatal/systemic failures

### Undo safety

- Only completed operations are eligible for rollback
- Undo refuses to overwrite an already-present original source path
- Partial rollback is reported explicitly

### Cloud-folder safety

- Common cloud-folder names are detected heuristically
- Apply requires explicit confirmation for detected cloud-synced roots

## Performance and scale

- Metadata-only scanning only; no file content reads
- Targeted responsiveness up to roughly 100k files
- Progress output for scan and apply
- Safe cancellation support

## Privacy posture

- No content extraction
- No AI behavior in v1
- No telemetry by default
- Transaction journals stay local unless explicitly exported

## Testing strategy

Layered testing is part of the design:

- unit tests for path safety, rule logic, schemas, and conflicts
- integration tests for `analyze`, `preview`, `apply`, `undo`, and transactions
- JSON contract tests for plans and journals
- synthetic large-folder tests for metadata-only scan behavior
- CLI parser and end-to-end regression coverage

## Current implementation outcome

The delivered v1 matches the MVP design:

- public GitHub repository exists
- CI is green
- CLI commands are implemented
- documentation covers safety model and source-checkout usage
- comprehensive MVP regression tests exist in the workspace
