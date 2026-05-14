# Architecture

This document describes the structure, data flow, tech stack, and key design boundaries of `smartfolder`.

---

## Overview

`smartfolder` is a safe, deterministic folder organizer. It analyzes a directory using metadata only, generates an explicit plan of file moves, previews proposed operations, applies confirmed moves with a transaction journal, and supports undo.

The project is a **Rust workspace** with a shared engine crate and two frontend crates.

---

## Workspace layout

```text
smartfolder/
├── crates/
│   ├── smartfolder-core/   # Engine: scanning, planning, execution, recovery
│   ├── smartfolder-cli/    # CLI frontend (binary: smartfolder)
│   └── smartfolder-gui/    # Windows-first desktop GUI frontend
├── tests/
│   └── fixtures/           # Folder fixtures for integration tests
├── scripts/
│   ├── test-mvp.ps1                  # MVP regression runner
│   ├── package-portable.ps1          # Builds portable dist/ package
│   └── register-explorer-launcher.ps1 # Registers/removes Explorer context-menu entry
└── docs/
    ├── v1-mvp/             # v1 design and implementation history
    ├── v2-roadmap/         # v2 design decisions and UX plans
    └── release/            # Release notes and portable-package docs
```

---

## Three-phase workflow

Every organize operation follows this invariant pipeline:

```
┌─────────────┐     ┌──────────────┐     ┌──────────────┐
│  1. Scanner │────▶│  2. Planner  │────▶│  3. Executor │
└─────────────┘     └──────────────┘     └──────────────┘
  Collect metadata    Generate plan         Apply safely
  (no file reads)     + detect conflicts    + write journal
```

### 1. Scanner (`scanner.rs`)

- Recursively traverses a directory tree.
- Collects file metadata: name, extension, size, timestamps, inferred type.
- Never reads file contents.
- Supports depth limits, hidden-file filtering, and symlink exclusion.
- Progress is streamed; cancellation is supported via `CancellationToken`.
- For large trees, scan results are stored in a SQLite-backed session (`session_store.rs`) to keep memory bounded.

### 2. Planner (`planner.rs`)

- Consumes a `ScanResult` and `PlanOptions`.
- Applies rule matching to assign each file a destination path inside the root.
- Built-in modes: `type`, `date`, `extension`, `type-year`, `type-year-month-day`.
- Custom TOML rule profiles (`rules.rs`) allow user-defined destination patterns with `{year}`, `{month}`, `{day}` placeholders.
- Performs conflict detection: destination exists, case-only rename, path too long.
- Conflicting operations are marked not-selected, never silently skipped.
- Produces a `PlanRecord` (serializable to JSON).

### 3. Executor (`apply.rs`)

- Reads a `PlanRecord`.
- Revalidates source path, size, and modified time before moving.
- Writes a transaction journal to disk before each move.
- Never overwrites an existing destination.
- Never writes outside the selected root.
- Cloud-synced roots require explicit confirmation before apply.
- Supports cancellation mid-apply; the journal records partial state for resumption.

### Recovery (`recovery.rs`)

- Reads a transaction journal to inspect, resume, or undo completed operations.
- Undo moves files back to their original locations in reverse journal order.
- Undo is safe only when the recorded pre-move state still matches the filesystem.

---

## Crate responsibilities

### `smartfolder-core`

The engine. Contains all filesystem-touching logic.

| Module | Responsibility |
|--------|---------------|
| `scanner.rs` | Directory traversal and metadata collection |
| `planner.rs` | Rule matching and plan generation |
| `apply.rs` | Safe plan execution with journaling |
| `recovery.rs` | Transaction inspection, undo, cleanup |
| `rules.rs` | Built-in and TOML-defined rule matching |
| `model.rs` | Shared data structures (`ScanResult`, `PlanRecord`, `TransactionJournal`, …) |
| `paths.rs` | Path safety validation (root confinement, length, symlink checks) |
| `storage.rs` | Persistent storage: transaction journals, rule profiles |
| `session_store.rs` | SQLite-backed session storage for large scan/plan results |
| `error.rs` | Typed error hierarchy |
| `lib.rs` | Public API surface and crate-level documentation |

**Key dependencies:** `serde`/`serde_json`, `rusqlite` (bundled), `chrono`, `toml`, `thiserror`, `directories`.

### `smartfolder-cli`

The terminal frontend. Thin shell over the core API.

- Single binary: `smartfolder`.
- Commands: `analyze`, `preview`, `apply`, `resume`, `continue`, `undo`, `transactions`.
- Human-readable output by default; `--json` for machine-readable.
- Progress and scan counts on stderr; `--quiet` suppresses them.
- Exit codes: `0` success, `1` runtime/safety error, `2` invalid input or declined confirmation.

**Key dependencies:** `smartfolder-core`, `serde_json`, `chrono`, `ctrlc`, `thiserror`.

### `smartfolder-gui`

The Windows-first desktop frontend built with `eframe`/`egui`.

- Shell sections: Organize, Activity, Rules, Settings.
- Organize screen: folder selection, style cards (By Type, By Date, Type + Date), Analyze Folder, Preview table, Organize Files, Undo Changes.
- Explorer launcher: opens the GUI with a preloaded folder argument.
- Rule editor: create and edit simple profiles; TOML import/export in advanced actions.
- Preferences and theme persistence (`preferences.rs`).
- UI typography: Inter font; icons: `egui-phosphor` (Phosphor icon set).

**Key dependencies:** `smartfolder-core`, `eframe`, `egui-phosphor`, `rfd` (native file dialogs), `serde`/`serde_json`, `chrono`.

---

## Data flow — full organize cycle

```
User selects folder
         │
         ▼
    ScanOptions
         │
    scanner::scan_folder()
         │
    ScanResult ──► (SQLite session if large)
         │
    PlanOptions (mode or profile)
         │
    planner::generate_plan()
         │
    PlanRecord (saved to plan.json / GUI memory)
         │
    User reviews preview
         │
    apply::apply_plan()
         │
    TransactionJournal (written to app-local storage)
         │
    Filesystem moves
         │
    TransactionSummary
         │
    recovery::undo_transaction()  ◄── (if user requests undo)
         │
    Original layout restored
```

---

## Safety invariants

These are enforced in the engine and must not be weakened:

1. **No file content reads** — scanner is metadata-only.
2. **Preview before action** — a plan must be generated and reviewed before any moves.
3. **Revalidation before move** — source path, size, and modified time are rechecked at apply time.
4. **No overwrites** — the executor refuses to move a file if the destination already exists.
5. **Root confinement** — all destinations are validated to stay inside the selected root.
6. **Journal before move** — the transaction journal is written before each filesystem operation.
7. **Symlink exclusion** — symlinks and junctions are not followed by default.
8. **Cloud-folder gate** — cloud-synced paths require an explicit confirmation flag.

---

## Storage locations

| Data | Location |
|------|----------|
| Transaction journals and profiles | App-local data directory (via `directories` crate) |
| Override with | `SMARTFOLDER_DATA_DIR` environment variable |
| SQLite session files | Same app-local data directory |
| GUI preferences | Same app-local data directory |

---

## Tech stack

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust 1.80+ | Workspace edition 2021 |
| GUI framework | `eframe` 0.28 / `egui` | Immediate-mode GUI, Windows-first |
| Icons | `egui-phosphor` 0.6 | Phosphor icon set (regular weight) |
| File dialogs | `rfd` 0.15 | Native OS file/folder picker |
| Serialization | `serde` + `serde_json` | Plans and journals as JSON |
| Config format | `toml` 0.9 | Custom rule profiles |
| Date/time | `chrono` 0.4 | File timestamps and plan dates |
| Persistent storage | `rusqlite` 0.32 (bundled) | Session store and transaction journals |
| Error handling | `thiserror` 2 | Typed error hierarchy |
| Platform dirs | `directories` 6 | App-local data path resolution |
| License | GPL-3.0-only | |

---

## Build and validation

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --no-deps --lib
.\scripts\test-mvp.ps1
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full documentation standard and [DEVELOPMENT_STANDARDS.md](DEVELOPMENT_STANDARDS.md) for the quick-reference checklist.
