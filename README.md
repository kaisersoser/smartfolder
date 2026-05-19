# smartfolder

`smartfolder` is a Windows-first desktop folder organizer with a compatible CLI for power users and automation. It scans a selected folder, generates deterministic organization plans, previews proposed file moves, applies confirmed operations with a transaction journal, and supports undo.

## v2 desktop workflow

The current desktop app focuses on the safe Explorer-to-preview workflow:

- Right-click a folder in Explorer and choose `Organize with smartfolder`.
- Open the GUI with that folder preselected.
- Choose a built-in style or saved custom rule profile.
- Analyze, preview, organize, and restore from the same app.
- Review Activity, Rules, and Settings for restore history, rule profiles, AI assistance, and local preferences.
- Use preview tree mode, command palette actions, and richer ready/review/untouched explanations when inspecting larger folders.

## v1 CLI scope

The original CLI version focuses on safe, transparent, reversible organization:

- Metadata, filename, path, extension, size, and timestamp analysis only.
- Deterministic type/date/extension organization rules.
- Simple custom rule profiles in TOML.
- Human-readable CLI output by default and JSON output with `--json`.
- Explicit confirmation before real moves, with `--yes` for scripts.
- Transaction journals stored in app-local data by default.
- Undo history retained until explicit cleanup.

## Non-goals for v1

- Desktop GUI.
- Shell integrations.
- AI recommendations.
- Content extraction.
- Duplicate detection.
- Regex rules.
- Deleting files or rewriting file contents.

## CLI

From a source checkout, the repository does **not** include a prebuilt `smartfolder.exe` in the project root. Run it in one of these ways:

```powershell
cargo run -p smartfolder-cli -- analyze <root> --output plan.json
cargo build --release
.\target\release\smartfolder.exe analyze <root> --output plan.json
```

When built without `--release`, the executable is created at `.\target\debug\smartfolder.exe`.

The Windows-first GUI crate can be started with:

```powershell
cargo run -p smartfolder-gui -- <root>
```

The GUI can analyze with built-in modes, import TOML rule profiles, create saved rule profiles visually from the Profile editor, and optionally use Ollama-backed AI assistance for folder analysis, rule drafting, prompt refinement, and rule explanation.

To add a per-user Windows Explorer launcher after building the release GUI:

```powershell
cargo build -p smartfolder-gui --release
.\scripts\register-explorer-launcher.ps1
```

This adds an "Organize with smartfolder" Explorer context-menu entry for both folder right-clicks and folder-background right-clicks, and launches the GUI with that folder preloaded. Remove it with `.\scripts\register-explorer-launcher.ps1 -Unregister`.

To create a portable Windows package under `dist`:

```powershell
.\scripts\package-portable.ps1
```

```powershell
smartfolder analyze <root> --output plan.json
smartfolder analyze <root> --include-subfolders --output plan.json
smartfolder analyze <root> --profile rules.toml --output plan.json
smartfolder analyze <root> --profile-id my-profile --output plan.json
smartfolder analyze <root> --mode type-date --quiet --output plan.json
smartfolder preview plan.json
smartfolder apply plan.json
smartfolder apply plan.json --yes
smartfolder resume <transaction-id>
smartfolder continue <transaction-id>
smartfolder undo <transaction-id>
smartfolder undo <transaction-id> --yes
smartfolder transactions list
smartfolder transactions inspect <transaction-id>
smartfolder transactions cleanup
smartfolder profiles list
smartfolder profiles import rules.toml
smartfolder profiles inspect my-profile
smartfolder profiles validate rules.toml
smartfolder ai status
smartfolder ai analyze <root>
smartfolder ai analyze <root> --json
smartfolder ai draft-profile <root> --prompt "Put invoices under Invoices/{year}" --save-as invoices
smartfolder profiles explain invoices --folder <root>
```

Analyze scans only the selected folder by default. Add `--include-subfolders` or `--recursive` when you want to include nested folders, and combine it with `--max-depth <n>` to limit recursion. Use `--json` with commands that support machine-readable output. If an error occurs while `--json` is present, errors are emitted as JSON on stderr.

Built-in `type-year`, `type-date`, and `type-year-month-day` modes currently produce `Type\Year\Month\Day\filename` with full month names, for example `Documents\2026\May\11\report.pdf`. Custom TOML rule destinations support `{year}`, `{month}`, and `{day}` placeholders.

CLI AI commands reuse the desktop app's local AI settings and Ollama readiness checks. They do not add API-key support, do not move files, and only read file contents when the persisted AI content-inspection setting is enabled. AI-generated profiles are validated as deterministic TOML rule profiles before they can be saved with `--save-as`.

## Safety model

`smartfolder` is designed around preview-before-action:

1. Analyze the folder without reading file contents.
2. Generate a plan where every proposed destination stays inside the selected root.
3. Preview every proposed move.
4. Revalidate source path, size, and modified time before applying.
5. Write a transaction journal before moving files.
6. Never overwrite existing files.
7. Use the journal to inspect, recover, or undo operations.

## Exit codes

| Code | Meaning |
|---:|---|
| 0 | Success |
| 1 | Runtime error, I/O error, serialization error, or core safety error |
| 2 | Invalid command/options, declined confirmation, or missing explicit cloud-folder confirmation |

## Progress and scale

`analyze` reports scan counts on stderr unless `--quiet` is used. The scanner is metadata-only and streams directory traversal without reading file contents; v1 targets large-folder scans up to 100k files before GUI work or indexing features are introduced.

## Development

This repository is a Rust workspace:

```text
crates/
  smartfolder-core/
  smartfolder-cli/
  smartfolder-gui/
```

Expected checks:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
.\scripts\test-mvp.ps1
```

## Windows install

Build and install the desktop app plus CLI for the current Windows user:

```powershell
.\scripts\install-windows.ps1 -AddToPath
```

This creates a Start Menu shortcut, optionally adds the CLI to user `PATH`, and registers the launch-only Explorer context menu unless `-NoExplorerRegistration` is provided. Remove the installed copy with:

```powershell
.\scripts\uninstall-windows.ps1
```

## Contributing

All contributions must follow the development standards in [CONTRIBUTING.md](CONTRIBUTING.md) and [DEVELOPMENT_STANDARDS.md](DEVELOPMENT_STANDARDS.md).

**Mandatory requirement:** All code must be comprehensively documented:
- Module-level doc comments explaining purpose
- Every public function documented with logical flow, parameters, errors, and examples
- Every public type documented with field/variant descriptions
- Clear, concise documentation explaining *why* and *what*

Documentation is **not optional**. Pull requests without proper documentation will be rejected.

See:
- [CONTRIBUTING.md](CONTRIBUTING.md) – Code standards and documentation guidelines
- [DEVELOPMENT_STANDARDS.md](DEVELOPMENT_STANDARDS.md) – Quick reference for standards
- [ARCHITECTURE.md](ARCHITECTURE.md) – System design, crate responsibilities, and data flow
- [CHANGELOG.md](CHANGELOG.md) – Release history
- [ROADMAP.md](ROADMAP.md) – Planned releases and future direction
- [copilot-instructions.md](copilot-instructions.md) – AI-assisted development guidelines

## License

GPL-3.0-only.
