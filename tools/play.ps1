# Starts the translated game directly on Windows, in a window, without going through WSL for
# the graphics: wgpu talks to Vulkan or Direct3D 12.
#
# Usage, from PowerShell (the project folder is reached as \\wsl.localhost\<distro>\...):
#   powershell -ExecutionPolicy Bypass -File tools\play.ps1
#   powershell -ExecutionPolicy Bypass -File tools\play.ps1 -Backend dx12
#   powershell -ExecutionPolicy Bypass -File tools\play.ps1 -Game "D:\...\AssassinsCreed_Dx9.exe"
#
# Parameters: -Backend vulkan|dx12|gl (default: Vulkan, then Direct3D 12);
#             -Game    path to AssassinsCreed_Dx9.exe;  -NoAudio;  -NoVideo.
# In the window: F8 shows or hides the diagnostics overlay (fps),
# the opening videos are skipped like the intro one,
#                F10 captures or releases the mouse pointer.
# Executable and DLL are built from WSL with tools/build-win.sh.
# The game's log ends up in run\win\log.txt (it can be read from WSL too).
param(
    [string]$Backend = "",
    [string]$Game = "E:\Giochi\Steam\steamapps\common\Assassins Creed\AssassinsCreed_Dx9.exe",
    [switch]$NoAudio,
    [switch]$NoVideo
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target\x86_64-pc-windows-gnullvm\release\altair-run.exe"
$dll = Join-Path $root "build\win\altair_ac1.dll"
foreach ($f in @($exe, $dll, $Game)) {
    if (-not (Test-Path -LiteralPath $f)) { Write-Error "Missing $f"; exit 1 }
}

# One instance at a time.
Get-Process -Name "altair-run" -ErrorAction SilentlyContinue | Stop-Process -Force

$dir = Join-Path $root "run\win"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$log = Join-Path $dir "log.txt"

$env:ALTAIR_WINDOW = "1"
$env:ALTAIR_STATS = "1"
if ($Backend) { $env:ALTAIR_BACKEND = $Backend } else { Remove-Item Env:ALTAIR_BACKEND -ErrorAction SilentlyContinue }
if ($NoAudio) { $env:ALTAIR_AUDIO = "0" } else { Remove-Item Env:ALTAIR_AUDIO -ErrorAction SilentlyContinue }
if ($NoVideo) { $env:ALTAIR_VIDEO = "0" } else { Remove-Item Env:ALTAIR_VIDEO -ErrorAction SilentlyContinue }

Write-Host "Starting: $exe"
Write-Host "Log: $log"
# The working directory is the project's: profiles and saves live in run\c.
$p = Start-Process -FilePath $exe -ArgumentList @("`"$Game`"", "--lib", "`"$dll`"") `
    -WorkingDirectory $root -NoNewWindow -Wait -PassThru -RedirectStandardError $log
Write-Host "Exited with code $($p.ExitCode). Last lines of the log:"
Get-Content -LiteralPath $log -Tail 15
