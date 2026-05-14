[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\smartfolder"),
    [string]$SourceRoot = "",
    [switch]$SkipBuild,
    [switch]$NoExplorerRegistration,
    [switch]$NoShortcuts,
    [switch]$AddToPath,
    [switch]$DesktopShortcut,
    [switch]$Quiet
)

$ErrorActionPreference = "Stop"

function Write-InstallMessage {
    param([string]$Message)
    if (-not $Quiet) {
        Write-Host $Message
    }
}

function New-SmartfolderShortcut {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ShortcutPath,

        [Parameter(Mandatory = $true)]
        [string]$TargetPath,

        [string]$WorkingDirectory = (Split-Path -Parent $TargetPath)
    )

    $shortcutDirectory = Split-Path -Parent $ShortcutPath
    New-Item -ItemType Directory -Path $shortcutDirectory -Force | Out-Null

    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($ShortcutPath)
    $shortcut.TargetPath = $TargetPath
    $shortcut.WorkingDirectory = $WorkingDirectory
    $shortcut.IconLocation = $TargetPath
    $shortcut.Save()
}

function Add-SmartfolderToUserPath {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = @()
    if (-not [string]::IsNullOrWhiteSpace($currentPath)) {
        $entries = $currentPath -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    }

    $alreadyPresent = $entries | Where-Object { $_.TrimEnd("\") -ieq $Directory.TrimEnd("\") }
    if (-not $alreadyPresent) {
        $updatedPath = (@($entries) + $Directory) -join ";"
        [Environment]::SetEnvironmentVariable("Path", $updatedPath, "User")
    }
}

$scriptRoot = if ([string]::IsNullOrWhiteSpace($PSScriptRoot)) {
    (Get-Location).Path
} else {
    $PSScriptRoot
}
$sourceRootInput = if ([string]::IsNullOrWhiteSpace($SourceRoot)) {
    Join-Path $scriptRoot ".."
} else {
    $SourceRoot
}

$resolvedSourceRoot = [System.IO.Path]::GetFullPath($sourceRootInput)
$resolvedInstallDir = [System.IO.Path]::GetFullPath($InstallDir)
$portableGuiExe = Join-Path $resolvedSourceRoot "smartfolder-gui.exe"
$portableCliExe = Join-Path $resolvedSourceRoot "smartfolder.exe"
$isPortableSource = (Test-Path $portableGuiExe -PathType Leaf) -and (Test-Path $portableCliExe -PathType Leaf)
$releaseDir = if ($isPortableSource) {
    $resolvedSourceRoot
} else {
    Join-Path $resolvedSourceRoot "target\release"
}
$guiExe = Join-Path $releaseDir "smartfolder-gui.exe"
$cliExe = Join-Path $releaseDir "smartfolder.exe"
$registrationScript = Join-Path $resolvedSourceRoot "scripts\register-explorer-launcher.ps1"
$installerReadme = Join-Path $resolvedSourceRoot "docs\release\windows-installer.md"
if (-not (Test-Path $installerReadme -PathType Leaf)) {
    $installerReadme = Join-Path $resolvedSourceRoot "README.md"
}

if ((-not $SkipBuild) -and (-not $isPortableSource)) {
    Push-Location $resolvedSourceRoot
    try {
        cargo build --workspace --release
    }
    finally {
        Pop-Location
    }
}

foreach ($binary in @($guiExe, $cliExe)) {
    if (-not (Test-Path $binary -PathType Leaf)) {
        throw "Required release binary not found: $binary. Build first or run without -SkipBuild."
    }
}

if (-not (Test-Path $registrationScript -PathType Leaf)) {
    throw "Explorer registration script not found: $registrationScript"
}

if ($PSCmdlet.ShouldProcess($resolvedInstallDir, "Install smartfolder")) {
    New-Item -ItemType Directory -Path $resolvedInstallDir -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $resolvedInstallDir "scripts") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $resolvedInstallDir "docs") -Force | Out-Null

    Copy-Item -Path $guiExe -Destination (Join-Path $resolvedInstallDir "smartfolder-gui.exe") -Force
    Copy-Item -Path $cliExe -Destination (Join-Path $resolvedInstallDir "smartfolder.exe") -Force
    Copy-Item -Path $registrationScript -Destination (Join-Path $resolvedInstallDir "scripts\register-explorer-launcher.ps1") -Force
    Copy-Item -Path (Join-Path $resolvedSourceRoot "scripts\uninstall-windows.ps1") -Destination (Join-Path $resolvedInstallDir "scripts\uninstall-windows.ps1") -Force
    Copy-Item -Path $installerReadme -Destination (Join-Path $resolvedInstallDir "README.md") -Force
    Copy-Item -Path (Join-Path $resolvedSourceRoot "LICENSE") -Destination (Join-Path $resolvedInstallDir "LICENSE") -Force

    [ordered]@{
        install_dir = $resolvedInstallDir
        explorer_registered = -not $NoExplorerRegistration
        path_registered = [bool]$AddToPath
        shortcuts_created = -not $NoShortcuts
    } | ConvertTo-Json | Set-Content -Path (Join-Path $resolvedInstallDir ".install.json") -Encoding UTF8

    if (-not $NoShortcuts) {
        $startMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\smartfolder"
        New-SmartfolderShortcut `
            -ShortcutPath (Join-Path $startMenuDir "smartfolder.lnk") `
            -TargetPath (Join-Path $resolvedInstallDir "smartfolder-gui.exe")

        if ($DesktopShortcut) {
            New-SmartfolderShortcut `
                -ShortcutPath (Join-Path ([Environment]::GetFolderPath("Desktop")) "smartfolder.lnk") `
                -TargetPath (Join-Path $resolvedInstallDir "smartfolder-gui.exe")
        }
    }

    if ($AddToPath) {
        Add-SmartfolderToUserPath -Directory $resolvedInstallDir
    }

    if (-not $NoExplorerRegistration) {
        & (Join-Path $resolvedInstallDir "scripts\register-explorer-launcher.ps1") `
            -AppPath (Join-Path $resolvedInstallDir "smartfolder-gui.exe")
    }
}

Write-InstallMessage "smartfolder installed to: $resolvedInstallDir"
