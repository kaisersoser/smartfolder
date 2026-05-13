# smartfolder 2.0 portable Windows package

This package runs smartfolder as a local Windows desktop app. It does not install services, start background tasks, or apply file moves from Explorer.

## Run the app

```powershell
.\smartfolder-gui.exe
```

You can also pass a folder path to preload it:

```powershell
.\smartfolder-gui.exe "D:\OneDrive\Documents"
```

## Explorer launcher

The optional Explorer launcher adds an `Open with smartfolder` entry for folders. It only opens the GUI with the selected folder preloaded.

```powershell
.\scripts\register-explorer-launcher.ps1 -AppPath .\smartfolder-gui.exe
```

Remove it with:

```powershell
.\scripts\register-explorer-launcher.ps1 -Unregister
```

## Data location

Analysis sessions, transaction journals, and saved rule profiles are stored in app-local data. Set `SMARTFOLDER_DATA_DIR` before launch to use a custom location.

## Safety model

smartfolder scans metadata only, previews planned moves before apply, never overwrites existing files, and writes transaction journals so completed apply sessions can be undone.