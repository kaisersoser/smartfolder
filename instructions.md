# Generic Project Instructions

This file exists so coding CLIs that do not read `copilot-instructions.md` can still find
the critical workspace guidance for this repository.

## Explorer Launcher Registration

For this worktree, the only reliable way to make the Explorer context-menu entry launch the
correct GUI executable is to register Explorer against the worktree release binary with these
exact commands:

```powershell
Set-Location D:\User\Projects\TinkerBox\FlexSorterApp.worktrees\agents-explorer-launcher-setup-update
.\scripts\register-explorer-launcher.ps1 -AppPath "D:\User\Projects\TinkerBox\FlexSorterApp.worktrees\agents-explorer-launcher-setup-update\target\release\smartfolder-gui.exe"
```

Do not rely on other registration methods for Explorer-context launches in this worktree.
