# smartfolder

`smartfolder` is a CLI-first folder organizer for power users and developers. It scans a selected folder, generates deterministic organization plans, previews proposed file moves, applies confirmed operations with a transaction journal, and supports undo.

## v1 scope

The first version focuses on safe, transparent, reversible organization:

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

An experimental Windows-first GUI crate also exists and can be started with:

```powershell
cargo run -p smartfolder-gui -- <root>
```

```powershell
smartfolder analyze <root> --output plan.json
smartfolder analyze <root> --profile rules.toml --output plan.json
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
```

Use `--json` with commands that support machine-readable output. If an error occurs while `--json` is present, errors are emitted as JSON on stderr.

Built-in `type-year`, `type-date`, and `type-year-month-day` modes currently produce `Type\Year\Month\Day\filename` with full month names, for example `Documents\2026\May\11\report.pdf`. Custom TOML rule destinations support `{year}`, `{month}`, and `{day}` placeholders.

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
```

Expected checks:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
.\scripts\test-mvp.ps1
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
- [copilot-instructions.md](copilot-instructions.md) – AI-assisted development guidelines

## License

GPL-3.0-only.
