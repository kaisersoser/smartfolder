[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$OutputRoot = (Join-Path $PSScriptRoot "..\dist"),
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$releaseExe = Join-Path $repoRoot "target\release\smartfolder-gui.exe"
$packageName = "smartfolder-2.0-portable-windows"
$packageDir = Join-Path ([System.IO.Path]::GetFullPath($OutputRoot)) $packageName

if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        cargo build -p smartfolder-gui --release
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path $releaseExe -PathType Leaf)) {
    throw "smartfolder GUI executable not found: $releaseExe. Build it first or run without -SkipBuild."
}

if ($PSCmdlet.ShouldProcess($packageDir, "Create portable smartfolder package")) {
    if (Test-Path $packageDir) {
        Remove-Item -Path $packageDir -Recurse -Force
    }

    New-Item -ItemType Directory -Path $packageDir | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $packageDir "scripts") | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $packageDir "docs") | Out-Null

    Copy-Item -Path $releaseExe -Destination (Join-Path $packageDir "smartfolder-gui.exe")
    Copy-Item -Path (Join-Path $repoRoot "scripts\register-explorer-launcher.ps1") -Destination (Join-Path $packageDir "scripts\register-explorer-launcher.ps1")
    Copy-Item -Path (Join-Path $repoRoot "docs\release\portable-windows.md") -Destination (Join-Path $packageDir "README.md")
    Copy-Item -Path (Join-Path $repoRoot "LICENSE") -Destination (Join-Path $packageDir "LICENSE")
}

Write-Host "Portable package created at: $packageDir"