[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\smartfolder"),
    [switch]$RemoveData,
    [switch]$Quiet
)

$ErrorActionPreference = "Stop"

function Write-UninstallMessage {
    param([string]$Message)
    if (-not $Quiet) {
        Write-Host $Message
    }
}

function Remove-SmartfolderFromUserPath {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ([string]::IsNullOrWhiteSpace($currentPath)) {
        return
    }

    $entries = $currentPath -split ";" | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_) -and $_.TrimEnd("\") -ine $Directory.TrimEnd("\")
    }
    [Environment]::SetEnvironmentVariable("Path", ($entries -join ";"), "User")
}

$resolvedInstallDir = [System.IO.Path]::GetFullPath($InstallDir)
$registrationScript = Join-Path $resolvedInstallDir "scripts\register-explorer-launcher.ps1"
$installManifest = Join-Path $resolvedInstallDir ".install.json"
$startMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\smartfolder"
$desktopShortcut = Join-Path ([Environment]::GetFolderPath("Desktop")) "smartfolder.lnk"
$defaultDataDir = Join-Path $env:LOCALAPPDATA "dev\smartfolder\smartfolder\data"
$shouldRemoveExplorerRegistration = $true

if (Test-Path $installManifest -PathType Leaf) {
    try {
        $manifest = Get-Content -Path $installManifest -Raw | ConvertFrom-Json
        $shouldRemoveExplorerRegistration = [bool]$manifest.explorer_registered
    }
    catch {
        $shouldRemoveExplorerRegistration = $true
    }
}

if ($shouldRemoveExplorerRegistration -and (Test-Path $registrationScript -PathType Leaf)) {
    & $registrationScript -Unregister
} elseif ($shouldRemoveExplorerRegistration) {
    foreach ($shellKey in @(
        "Registry::HKEY_CURRENT_USER\Software\Classes\Directory\shell\smartfolder",
        "Registry::HKEY_CURRENT_USER\Software\Classes\Directory\Background\shell\smartfolder"
    )) {
        if (Test-Path $shellKey) {
            if ($PSCmdlet.ShouldProcess($shellKey, "Remove Explorer context menu entry")) {
                Remove-Item -Path $shellKey -Recurse -Force
            }
        }
    }
}

if (Test-Path $startMenuDir) {
    if ($PSCmdlet.ShouldProcess($startMenuDir, "Remove Start Menu shortcuts")) {
        Remove-Item -Path $startMenuDir -Recurse -Force
    }
}

if (Test-Path $desktopShortcut -PathType Leaf) {
    if ($PSCmdlet.ShouldProcess($desktopShortcut, "Remove desktop shortcut")) {
        Remove-Item -Path $desktopShortcut -Force
    }
}

Remove-SmartfolderFromUserPath -Directory $resolvedInstallDir

if (Test-Path $resolvedInstallDir) {
    if ($PSCmdlet.ShouldProcess($resolvedInstallDir, "Remove smartfolder install directory")) {
        Remove-Item -Path $resolvedInstallDir -Recurse -Force
    }
}

if ($RemoveData -and (Test-Path $defaultDataDir)) {
    if ($PSCmdlet.ShouldProcess($defaultDataDir, "Remove smartfolder app data")) {
        Remove-Item -Path $defaultDataDir -Recurse -Force
    }
}

Write-UninstallMessage "smartfolder uninstalled."
