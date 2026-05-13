# RC2 Font and Icon Review

## Selected assets

- Inter variable font bundled from the upstream `rsms/inter` project.
- Phosphor icons integrated through the `egui-phosphor` crate.

## License review

- Inter is published as free and open source under the SIL Open Font License 1.1.
- The vendored Inter license text is stored next to this note in `Inter-OFL-1.1.txt`.
- `egui-phosphor` is published under `MIT OR Apache-2.0`.
- The bundled Phosphor icon set used by `egui-phosphor` is MIT licensed.

## Security review

- Inter is vendored as a static font asset. It is loaded at runtime through `include_bytes!` and does not execute code.
- `egui-phosphor` registers icon font data with egui. It does not add network access, shell execution, or external process handling to GUI startup.
- Integration is limited to startup font registration and icon glyph constants; core scanning, planning, apply, and undo behavior are unchanged.

## Implementation notes

- Inter is added as the first proportional font so the GUI still keeps egui fallback fonts for broad glyph coverage.
- Phosphor regular icons are added as fallback fonts so icons can be mixed into ordinary `RichText` labels.
- The first RC2 integration applies the icon set to the primary shell: Organize, Activity, Rules, and Settings.