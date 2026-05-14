# smartfolder 2.1 Windows installation

smartfolder 2.1 includes a per-user Windows installer script for the desktop app and CLI. It does not install services, start background tasks, or organize files directly from Explorer.

## Install

From a source checkout:

```powershell
.\scripts\install-windows.ps1
```

Useful options:

```powershell
.\scripts\install-windows.ps1 -AddToPath
.\scripts\install-windows.ps1 -DesktopShortcut
.\scripts\install-windows.ps1 -NoExplorerRegistration
.\scripts\install-windows.ps1 -NoShortcuts
.\scripts\install-windows.ps1 -SkipBuild
```

Default install location:

```text
%LOCALAPPDATA%\Programs\smartfolder
```

The installer copies:

- `smartfolder-gui.exe`
- `smartfolder.exe`
- Explorer registration script
- uninstall script
- license and installation notes

## Shortcuts and Explorer

The installer creates a Start Menu shortcut by default. `-DesktopShortcut` also creates a desktop shortcut.

Unless `-NoExplorerRegistration` is used, it registers `Organize with smartfolder` for folder and folder-background right-clicks. That entry only opens the GUI with the selected folder preloaded. It never organizes files directly.

## CLI parity

The installed CLI supports the same main organization scope as the desktop app:

- built-in organization modes
- current-folder-only default
- optional subfolder analysis
- custom TOML profiles
- saved app-local profiles via `--profile-id`
- preview, apply, undo, restore history inspection, and cleanup

## Uninstall

```powershell
.\scripts\uninstall-windows.ps1
```

From an installed copy:

```powershell
%LOCALAPPDATA%\Programs\smartfolder\scripts\uninstall-windows.ps1
```

Use `-RemoveData` only when you also want to delete local sessions, restore history, settings, and saved profiles.
