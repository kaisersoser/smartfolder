# smartfolder 2.0 UX release notes

smartfolder 2.0 is a Windows-first desktop workflow for organizing a folder safely from Explorer or from the app itself.

## Main workflow

```text
Right-click folder -> Organize with smartfolder -> Analyze Folder -> Preview -> Organize Files -> Undo Changes if needed
```

## What changed

- The app opens to an Organize-first shell with Activity, Rules, and Settings sections.
- Explorer launch preselects the clicked folder and keeps Analyze Folder ready without re-selection.
- Built-in organization styles are shown as cards: By Type, By Date, and Type + Date.
- Preview defaults to File, Destination, and Status, with exact source, destination, and rule details shown in the selected-change panel.
- Organize Files uses a safety confirmation that states files are previewed, existing files are not overwritten, restore history is recorded first, and Undo Changes is available afterward.
- Activity and Restore History use user-facing event language while keeping activity ids and technical details in disclosure sections.
- Rules keeps the simple profile editor and moves TOML import/export into advanced actions.
- Settings explains safety defaults, restore history, Explorer integration, and storage maintenance.

## Compatibility

- The shared Rust core, CLI behavior, no-overwrite policy, and journal-backed undo model remain intact.
- The Explorer launcher only opens the GUI with the selected folder preloaded. It never organizes files directly.
- The app remains metadata-only and does not read file contents.

## Validation

Release readiness requires the full automated test stack plus a manual disposable-folder proof:

```text
select or preload folder -> analyze -> preview -> organize -> undo -> original layout restored
```