[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$AppPath = (Join-Path $PSScriptRoot "..\target\release\smartfolder-gui.exe"),
    [switch]$Unregister
)

$ErrorActionPreference = "Stop"

$resolvedAppPath = [System.IO.Path]::GetFullPath($AppPath)
$folderShellKey = "Registry::HKEY_CURRENT_USER\Software\Classes\Directory\shell\smartfolder"
$folderBackgroundShellKey = "Registry::HKEY_CURRENT_USER\Software\Classes\Directory\Background\shell\smartfolder"
$folderCommandKey = Join-Path $folderShellKey "command"
$folderBackgroundCommandKey = Join-Path $folderBackgroundShellKey "command"
$menuLabel = "Organize with smartfolder"

function Register-SmartfolderExplorerLauncher {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ShellKey,

        [Parameter(Mandatory = $true)]
        [string]$CommandKey,

        [Parameter(Mandatory = $true)]
        [string]$CommandArgument,

        [Parameter(Mandatory = $true)]
        [string]$RegistrationLabel
    )

    if ($PSCmdlet.ShouldProcess($ShellKey, $RegistrationLabel)) {
        New-Item -Path $CommandKey -Force | Out-Null
        New-ItemProperty -Path $ShellKey -Name "MUIVerb" -Value $menuLabel -PropertyType String -Force | Out-Null
        New-ItemProperty -Path $ShellKey -Name "Icon" -Value $resolvedAppPath -PropertyType String -Force | Out-Null
        $commandValue = '"{0}" "{1}"' -f $resolvedAppPath, $CommandArgument
        Set-Item -Path $CommandKey -Value $commandValue
    }
}

function Remove-SmartfolderExplorerLauncher {
    foreach ($shellKey in @($folderShellKey, $folderBackgroundShellKey)) {
        if (Test-Path $shellKey) {
            if ($PSCmdlet.ShouldProcess($shellKey, "Remove Explorer context menu entry")) {
                Remove-Item -Path $shellKey -Recurse -Force
            }
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

Register-SmartfolderExplorerLauncher `
    -ShellKey $folderShellKey `
    -CommandKey $folderCommandKey `
    -CommandArgument '%1' `
    -RegistrationLabel "Register folder context menu entry"

Register-SmartfolderExplorerLauncher `
    -ShellKey $folderBackgroundShellKey `
    -CommandKey $folderBackgroundCommandKey `
    -CommandArgument '%V' `
    -RegistrationLabel "Register folder background context menu entry"

Write-Host "Registered smartfolder Explorer launcher for folders and folder backgrounds."