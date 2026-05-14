# smartfolder portable Windows package

This package runs smartfolder as a local Windows desktop app. It does not install services, start background tasks, or apply file moves from Explorer.

## Run the app

```powershell
.\smartfolder-gui.exe
```

The package also includes the CLI:

```powershell
.\smartfolder.exe --help
```

You can also pass a folder path to preload it:

```powershell
.\smartfolder-gui.exe "D:\OneDrive\Documents"
```

## Explorer launcher

The optional Explorer launcher adds an `Organize with smartfolder` entry for folders. It only opens the GUI with the selected folder preloaded.

```powershell
.\scripts\register-explorer-launcher.ps1 -AppPath .\smartfolder-gui.exe
```

Remove it with:

```powershell
.\scripts\register-explorer-launcher.ps1 -Unregister
```

## Optional per-user install

To copy the portable package into `%LOCALAPPDATA%\Programs\smartfolder`, create shortcuts, and register Explorer launch:

```powershell
.\scripts\install-windows.ps1 -SkipBuild
```

## Data location

Analysis sessions, restore history, and saved rule profiles are stored in app-local data. Set `SMARTFOLDER_DATA_DIR` before launch to use a custom location.

## Safety model

smartfolder scans metadata only, previews planned moves before Organize Files, never overwrites existing files, and writes restore history so completed organization sessions can be undone.
