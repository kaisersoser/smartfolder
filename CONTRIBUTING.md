# Contributing

`smartfolder` prioritizes correctness, transparency, and data safety over convenience.

Before changing behavior, preserve these v1 constraints:

- Do not read file contents.
- Do not move files without preview and explicit confirmation.
- Do not overwrite existing files.
- Do not allow destinations outside the selected root.
- Do not follow symlinks or junctions by default.
- Keep undo and transaction journal behavior reliable.

Run the workspace checks before submitting changes:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
