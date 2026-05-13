# smartfolder 2.0 UX Product Vision

## Status

Refined from `ux-enhancement-design.md` after product-direction review.

## Product Thesis

smartfolder 2.0 should feel like a calm, modern desktop product that helps users organize files safely without forcing them to understand sessions, journals, transactions, SQLite storage, or recovery internals.

The core experience is:

```text
Right-click folder in Explorer -> smartfolder opens with that folder preselected -> choose style -> analyze -> preview -> organize -> undo if needed
```

The app should remain deterministic, local-first, reversible, and power-user capable, but the default interface should communicate safety through plain language and strong visual structure instead of technical detail density.

## Settled UX Decisions

### Rewrite Shape

The UX redesign should replace the current GUI with a new shell rather than continue layering small changes onto the existing single-page utility.

Constraint: the main build must stay green throughout the rewrite. The implementation can be incremental internally, but each committed slice should compile and preserve the safe core workflow.

### Default User Model

The product should use a balanced model:

- beginner-friendly by default
- power-user controls available through progressive disclosure
- no loss of CLI interoperability or detailed inspection capability

This means the default screens optimize for confidence and clarity, while advanced panels preserve exact paths, rule reasons, logs, technical history, and export options.

### First Implementation Slice

The first redesign slice should focus on the Organize screen only.

Do not start by building every sidebar section as placeholders. A convincing Organize workflow is the product proof point.

Minimum first slice:

- new application shell structure
- Organize screen as the primary view
- Explorer/context launch support with the clicked folder preselected in the Organize screen
- folder selection for standalone launch
- style selection cards
- prominent Analyze Folder CTA
- simplified result summary
- simple preview table
- Organize Files apply flow
- immediate Undo Changes affordance after completion

### Advanced Disclosure

Advanced controls should be per-section, not a single global mode.

Examples:

- Organize: include subfolders, exclusions, exact scan details
- Preview: original path, exact destination, rule reason, conflict detail
- Activity: transaction id, journal path, technical restore log
- Rules: TOML import/export, raw validation details

This keeps each screen focused while preserving depth where it is contextually useful.

### Preview Default

The default preview row should show only:

```text
File | Destination | Status
```

Original folder, exact destination path, rule reason, timestamps, and transaction metadata belong in row expansion or a detail side panel.

### Rule Management Scope for 2.0

The current simple profile editor is sufficient for 2.0 if the Organize and Activity experiences are strong.

Rule management should remain approachable and deterministic, but full multi-rule editing/reordering is not required before the UX redesign can proceed.

### Visual Tone

The app should move beyond plain utility styling into a bold modern product direction.

Target feel:

- spacious
- confident
- visually structured
- warm but not playful
- polished enough to feel like a product, not a debug harness

### Copy Tone

The app should speak in warm, reassuring, plain language.

Product copy should emphasize:

- what will happen
- what will not happen
- what remains safe
- how to undo

Avoid humor in product UI. Humor can remain in internal planning docs, but the app itself should build trust through clarity.

### Definition of Done for the First UX Slice

The first redesigned Organize flow is successful when the manual flow is obvious without documentation:

```text
right-click folder in Explorer -> smartfolder opens with that folder preselected -> analyze -> preview -> organize -> undo
```

This manual usability proof is the first gate. Automated tests remain necessary for safety behavior, but product clarity is judged by whether the workflow feels self-explanatory.

## Information Architecture

The final app shell should use persistent primary navigation:

```text
Organize
Activity
Rules
Settings
```

However, the first implementation should only fully build Organize. Activity can continue using existing functionality until the Organize flow proves the new design language.

## Organize Screen Vision

### 1. Folder

The folder picker should be the obvious starting point.

Desired presentation:

```text
Choose a folder to organize
[ D:\Documents                                  ] [ Browse ]

Recent folders
Downloads   Documents   Desktop
```

Important behavior:

- Explorer preload should fill this field automatically
- if launched from Explorer, the preselected folder should already be ready for Analyze Folder without requiring re-selection
- current-folder-only remains the default
- include subfolders belongs in advanced options

### 2. Organization Style

Replace radio buttons and mode names with cards.

Suggested cards:

- By Type
- By Date
- Type + Date
- Custom Rules

Each card should show a compact example of the resulting folders. The selected style should be visually obvious.

### 3. Analyze

The primary action should be a single dominant button:

```text
Analyze Folder
```

Disable it until a folder is selected. When analysis is stale, say so explicitly.

### 4. Results Summary

Replace dense text summaries with cards:

```text
Ready to organize | Needs review | Left untouched
122               | 74           | 0
```

Use confidence framing:

```text
122 files can be safely organized.
74 files were left untouched because smartfolder was not confident enough to move them.
```

This turns uncertainty into a safety feature.

### 5. Preview

Default preview:

```text
File            Destination             Status
IMG_2033.jpg    Images / 2025 / March   Ready
```

Details on expand:

- original path
- exact destination path
- rule reason
- conflict detail
- file metadata

### 6. Organize

Replace `Apply ready moves` with `Organize Files`.

Confirmation copy should say:

```text
Ready to organize 122 files

smartfolder will:
- Move files into organized folders
- Never overwrite existing files
- Keep undo available after this finishes
```

### 7. Completion and Undo

Completion must prominently show:

```text
Organization complete
122 files organized successfully.

[ Undo Changes ] [ View Details ]
```

Undo is not a secondary recovery feature. It is a primary trust feature.

## Activity Screen Vision

Activity should present human-readable history first:

```text
Today
Organized 122 files in Documents
Undone organization in Downloads
```

Advanced details may expose:

- transaction id
- journal path
- operation counts
- exact source/destination rows
- restore log

Default language should use `Activity`, `History`, and `Restore`, not `Transaction`, `Journal`, and `Recovery Log`.

## Rules Screen Vision

For 2.0, rules should remain simple:

- built-in styles explained visually
- saved profiles listed clearly
- simple profile editor available
- import/export supported

Do not block the UX redesign on a full advanced multi-rule designer.

## Settings Vision

Settings should stay sparse.

Suggested groups:

- Appearance
- Safety
- History
- Advanced

Avoid turning Settings into a control panel for every internal system.

## Implementation Priority

### Phase 1: Organize Screen Rewrite

Build the new guided Organize flow while keeping the project compiling.

### Phase 2: Trust and Safety Copy

Improve apply, undo, warnings, and error language.

### Phase 3: Progressive Details

Move technical details into contextual expanders and panels.

### Phase 4: Activity, Rules, Settings

Bring the remaining navigation sections into the same design language.

### Phase 5: Polish and Accessibility

Improve spacing, typography, semantic color, keyboard navigation, contrast, and focus behavior.

## Non-Negotiables

- No AI in 2.0.
- No autonomous organization.
- No file content extraction.
- No weakening of preview, confirmation, no-overwrite, journaling, or undo.
- No hiding safety failures to make the UI look cleaner.
- No exposing technical internals as primary workflow concepts.

## Product Standard

The redesigned app succeeds when a user can say:

> I understand what this app will do, I trust that it will not overwrite my files, and I know exactly how to undo the changes.