[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$OutputRoot = (Join-Path $PSScriptRoot "..\dist"),
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$releaseDir = Join-Path $repoRoot "target\release"
$guiExe = Join-Path $releaseDir "smartfolder-gui.exe"
$cliExe = Join-Path $releaseDir "smartfolder.exe"
$workspaceManifest = Get-Content -Path (Join-Path $repoRoot "Cargo.toml") -Raw
$versionMatch = [regex]::Match($workspaceManifest, '(?m)^version\s*=\s*"([^"]+)"')
if (-not $versionMatch.Success) {
    throw "Could not determine workspace version from Cargo.toml"
}
$packageName = "smartfolder-$($versionMatch.Groups[1].Value)-portable-windows"
$packageDir = Join-Path ([System.IO.Path]::GetFullPath($OutputRoot)) $packageName

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        cargo build --workspace --release
    }
    finally {
        Pop-Location
    }
}

foreach ($binary in @($guiExe, $cliExe)) {
    if (-not (Test-Path $binary -PathType Leaf)) {
        throw "Required release binary not found: $binary. Build it first or run without -SkipBuild."
    }
}

if ($PSCmdlet.ShouldProcess($packageDir, "Create portable smartfolder package")) {
    if (Test-Path $packageDir) {
        Remove-Item -Path $packageDir -Recurse -Force
    }

    New-Item -ItemType Directory -Path $packageDir | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $packageDir "scripts") | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $packageDir "docs\release") | Out-Null

    Copy-Item -Path $guiExe -Destination (Join-Path $packageDir "smartfolder-gui.exe")
    Copy-Item -Path $cliExe -Destination (Join-Path $packageDir "smartfolder.exe")
    Copy-Item -Path (Join-Path $repoRoot "scripts\register-explorer-launcher.ps1") -Destination (Join-Path $packageDir "scripts\register-explorer-launcher.ps1")
    Copy-Item -Path (Join-Path $repoRoot "scripts\install-windows.ps1") -Destination (Join-Path $packageDir "scripts\install-windows.ps1")
    Copy-Item -Path (Join-Path $repoRoot "scripts\uninstall-windows.ps1") -Destination (Join-Path $packageDir "scripts\uninstall-windows.ps1")
    Copy-Item -Path (Join-Path $repoRoot "docs\release\portable-windows.md") -Destination (Join-Path $packageDir "README.md")
    Copy-Item -Path (Join-Path $repoRoot "docs\release\portable-windows.md") -Destination (Join-Path $packageDir "docs\release\portable-windows.md")
    Copy-Item -Path (Join-Path $repoRoot "docs\release\windows-installer.md") -Destination (Join-Path $packageDir "docs\release\windows-installer.md")
    Copy-Item -Path (Join-Path $repoRoot "LICENSE") -Destination (Join-Path $packageDir "LICENSE")
}

Write-Host "Portable package created at: $packageDir"
