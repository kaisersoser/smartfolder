# Changelog

All notable changes to `smartfolder` are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Version numbers use [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Planned

- v2.25 UX refinement and trust pass for the desktop workflow.
- Cross-platform GUI packaging research.

---

## [2.2.0] — 2026-05-16

### Added

- **Optional Ollama-backed AI assistance** in the GUI, gated behind provider readiness and explicit user enablement.
- **AI-assisted folder analysis** with summary, detected patterns, risks, recommended deterministic strategy, confidence, and evidence examples.
- **AI rule drafting workflow** for building deterministic custom profiles from prompts, including draft review and raw JSON inspection.
- **AI prompt refinement** to rewrite user prompts before generating draft rule profiles.
- **AI rule explanation** for existing deterministic profiles against the selected folder context.
- **AI settings and diagnostics** including endpoint, model selection, timeout, content-inspection preference, connection testing, and diagnostic export.
- **Bounded content sampling** for text-like files when content inspection is explicitly enabled.
- **Validation and repair path** for AI drafts, including schema validation, safety checks, applicability warnings, and one-shot JSON repair.

### Changed

- AI-assisted flows remain advisory only and preserve the existing preview, confirmation, conflict, journal, and undo safety model.
- Portable packaging script now resolves its default output path reliably when invoked directly in PowerShell.

### Verified

- Automated quality gates pass: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo build --release`.
- Manual verification completed with AI enabled and disabled, including Ollama-backed end-to-end checks and deterministic organize/undo regression coverage.

---

## [2.1.0] — 2026-05-14

### Added

- **Per-user Windows installer** — `install-windows.ps1` installs the GUI and CLI under `%LOCALAPPDATA%\Programs\smartfolder`.
- **Uninstall flow** — `uninstall-windows.ps1` removes shortcuts, Explorer registration, PATH entry, and installed binaries, with optional data removal.
- **Installer-managed Explorer registration** — install registers the launch-only `Organize with smartfolder` Explorer entry by default.
- **Start Menu and optional desktop shortcuts** — installer creates Start Menu shortcut and supports `-DesktopShortcut`.
- **Optional CLI on PATH** — installer supports `-AddToPath`.
- **CLI saved profile parity** — `analyze --profile-id <id>` can use app-local profiles saved by the GUI.
- **CLI profile management** — `profiles list`, `profiles inspect`, `profiles import`, and `profiles validate`.

### Changed

- Portable package now includes both `smartfolder-gui.exe` and `smartfolder.exe`.
- Portable package naming follows the workspace version.

---

## [2.0.0] — 2026-05-14

The v2.0 Windows-first desktop release completes the GUI-first UX rewrite while preserving the existing preview, safety, and undo model.

### Added

- **GUI shell rewrite** — new Organize-first shell with Activity, Rules, and Settings sections.
- **Explorer launcher** — `register-explorer-launcher.ps1` registers an "Organize with smartfolder" Windows Explorer context-menu entry for folders. `smartfolder-gui.exe` opens with the selected folder preloaded. Remove with `-Unregister`.
- **Portable Windows package** — `package-portable.ps1` produces a self-contained `dist/` package.
- **GUI preferences and theme** — per-user theme and preference persistence.
- **Inter font and Phosphor icons** — Inter typeface and `egui-phosphor` icon set added with licensing and security reviews.
- **Tree view icons** — folder tree in the GUI uses Phosphor icons; context menu registration enhanced.
- **`--include-subfolders` / `--recursive`** — CLI and GUI analysis can now span nested folders, with optional `--max-depth <n>` cap.
- **SQLite-backed session storage** — large scan and plan sessions are stored in a bundled SQLite database so memory usage stays bounded on very large folder trees.
- **GUI crate** (`smartfolder-gui`) — `eframe`/`egui`-based Windows-first desktop frontend added to the workspace.
- **MVP regression suite** — end-to-end regression coverage added in `tests/`.

### Changed

- Analysis defaults to the selected folder only; subfolders require an explicit flag.
- Preview default shows `File`, `Destination`, and `Status`. Full details (exact source path, rule reason, timestamps) are available in the selected-change panel.
- Safety confirmation before Organize Files states: previewed, no overwrites, restore history recorded, undo available.
- Activity and Restore History use plain user-facing language; technical details are in disclosure sections.
- Rules section moves TOML import/export into an advanced actions area.

### Fixed

- Cross-platform test path handling corrected for Windows vs Unix fixture paths.

---

## [1.0.0] — 2026-05-11

Initial public release of the CLI-first `smartfolder` v1 MVP.

### Added

- **`smartfolder analyze`** — metadata-only folder scan with rule-based plan generation.
  - Built-in modes: `type`, `date`, `extension`, `type-year`, `type-date` (`type-year`), `type-year-month-day`.
  - Custom TOML rule profiles with `{year}`, `{month}`, `{day}` destination placeholders.
  - `--output plan.json` for machine-readable plans.
  - `--quiet` to suppress progress output.
- **`smartfolder preview`** — human-readable preview of a saved plan.
- **`smartfolder apply`** — apply a plan with explicit confirmation or `--yes` for scripting.
- **`smartfolder resume` / `smartfolder continue`** — resume a partially applied transaction.
- **`smartfolder undo`** — roll back a completed transaction using its journal.
- **`smartfolder transactions list/inspect/cleanup`** — manage and inspect the transaction store.
- **JSON output** (`--json`) for all commands; errors emit structured JSON on stderr when `--json` is present.
- **Safety model**: preview-before-action, revalidation before move, no overwrites, destinations confined to root, cloud-folder confirmation gate, transaction journal.
- **Exit codes**: `0` success, `1` runtime/IO/safety error, `2` invalid input or declined confirmation.
- **Rust workspace** with `smartfolder-core` (engine) and `smartfolder-cli` (CLI frontend) crates.
- **GPL-3.0-only** license.
