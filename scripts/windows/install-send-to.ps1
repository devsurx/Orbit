# Adds "Orbit" to the right-click -> Send to menu.
#
# Orbit converts *any* file it understands, so claiming a file association would
# only ever cover one extension and then sit in "Open with" for the rest. The
# Send to menu takes whatever selection you hand it, which is exactly the set of
# things Orbit accepts, and nothing has to be registered machine-wide.
#
# Run once, from a normal (non-elevated) PowerShell:
#
#     powershell -ExecutionPolicy Bypass -File scripts\windows\install-send-to.ps1
#
# Point it at a specific build if it cannot find the installed one:
#
#     ... -File scripts\windows\install-send-to.ps1 -Target C:\path\to\Orbit.exe
#
# And remove it again:
#
#     ... -File scripts\windows\install-send-to.ps1 -Uninstall

[CmdletBinding()]
param(
    [string]$Target = "",
    [switch]$Uninstall
)

$ErrorActionPreference = "Stop"

$sendTo = Join-Path $env:APPDATA "Microsoft\Windows\SendTo"
$link = Join-Path $sendTo "Orbit.lnk"

if ($Uninstall) {
    if (Test-Path -LiteralPath $link) {
        Remove-Item -LiteralPath $link -Force
        Write-Host "Removed $link"
    }
    else {
        Write-Host "Nothing to remove."
    }
    exit 0
}

if (-not $Target) {
    # The NSIS installer is per-user, so %LOCALAPPDATA% is the likely home. The
    # MSI can land in either Program Files. Both possible binary names are tried,
    # because the bundler names it after the product and Cargo names it after the
    # crate, and the two differ.
    $directories = @(
        (Join-Path $env:LOCALAPPDATA "Orbit"),
        (Join-Path $env:LOCALAPPDATA "Programs\Orbit"),
        (Join-Path $env:ProgramFiles "Orbit"),
        (Join-Path ${env:ProgramFiles(x86)} "Orbit")
    )
    $names = @("Orbit.exe", "orbit-app.exe")

    foreach ($directory in $directories) {
        foreach ($name in $names) {
            $candidate = Join-Path $directory $name
            if (Test-Path -LiteralPath $candidate) {
                $Target = $candidate
                break
            }
        }
        if ($Target) { break }
    }

    if (-not $Target) {
        foreach ($name in $names) {
            $onPath = Get-Command $name -ErrorAction SilentlyContinue
            if ($onPath) {
                $Target = $onPath.Source
                break
            }
        }
    }
}

if (-not $Target -or -not (Test-Path -LiteralPath $Target)) {
    throw "Could not find the Orbit executable. Pass -Target 'C:\path\to\Orbit.exe'."
}

New-Item -ItemType Directory -Force -Path $sendTo | Out-Null

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($link)
$shortcut.TargetPath = $Target
$shortcut.WorkingDirectory = Split-Path -Parent $Target
$shortcut.IconLocation = "$Target,0"
$shortcut.Description = "Convert or edit this file with Orbit"
$shortcut.Save()

Write-Host "Added Orbit to the Send to menu:"
Write-Host "  $link"
Write-Host "  -> $Target"
Write-Host ""
Write-Host "Right-click any file (or a selection), Send to, Orbit."
Write-Host "Orbit is a single instance, so a multi-file selection arrives as one"
Write-Host "drop even if Windows launches it once per file."
