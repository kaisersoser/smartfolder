[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$AppPath = (Join-Path $PSScriptRoot "..\target\release\smartfolder-gui.exe"),
    [switch]$Unregister
)

$ErrorActionPreference = "Stop"

$resolvedAppPath = [System.IO.Path]::GetFullPath($AppPath)
$folderShellKey = "Registry::HKEY_CURRENT_USER\Software\Classes\Directory\shell\smartfolder"
$folderCommandKey = Join-Path $folderShellKey "command"
$menuLabel = "Open with smartfolder"

function Remove-SmartfolderExplorerLauncher {
    if (Test-Path $folderShellKey) {
        if ($PSCmdlet.ShouldProcess($folderShellKey, "Remove Explorer context menu entry")) {
            Remove-Item -Path $folderShellKey -Recurse -Force
        }
    }
}

if ($Unregister) {
    Remove-SmartfolderExplorerLauncher
    Write-Host "Removed smartfolder Explorer launcher registration."
    return
}

if (-not (Test-Path $resolvedAppPath -PathType Leaf)) {
    throw "smartfolder GUI executable not found: $resolvedAppPath. Build it first with: cargo build -p smartfolder-gui --release"
}

if ($PSCmdlet.ShouldProcess($folderShellKey, "Register Explorer context menu entry")) {
    New-Item -Path $folderCommandKey -Force | Out-Null
    New-ItemProperty -Path $folderShellKey -Name "MUIVerb" -Value $menuLabel -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $folderShellKey -Name "Icon" -Value $resolvedAppPath -PropertyType String -Force | Out-Null
    Set-Item -Path $folderCommandKey -Value "`"$resolvedAppPath`" `"%1`""
}

Write-Host "Registered smartfolder Explorer launcher for folders."