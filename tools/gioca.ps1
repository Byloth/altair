# Avvia il gioco tradotto direttamente su Windows, in una finestra, senza passare da WSL per
# la grafica: wgpu parla a Vulkan o a Direct3D 12.
#
# Uso, da PowerShell (la cartella del progetto si raggiunge come \\wsl.localhost\<distro>\...):
#   powershell -ExecutionPolicy Bypass -File tools\gioca.ps1
#   powershell -ExecutionPolicy Bypass -File tools\gioca.ps1 -Backend dx12
#   powershell -ExecutionPolicy Bypass -File tools\gioca.ps1 -Gioco "D:\...\AssassinsCreed_Dx9.exe"
#
# Parametri: -Backend vulkan|dx12|gl (predefinito: Vulkan, poi Direct3D 12);
#            -Gioco   percorso di AssassinsCreed_Dx9.exe;  -SenzaAudio;  -SenzaVideo.
# Nella finestra: F8 mostra o nasconde i fotogrammi al secondo (nel titolo),
#                F10 cattura o libera il puntatore del mouse.
# Eseguibile e DLL si compilano da WSL con tools/build-win.sh.
# Il registro del gioco finisce in run\win\log.txt (lo si può leggere anche da WSL).
param(
    [string]$Backend = "",
    [string]$Gioco = "E:\Giochi\Steam\steamapps\common\Assassins Creed\AssassinsCreed_Dx9.exe",
    [switch]$SenzaAudio,
    [switch]$SenzaVideo
)
$ErrorActionPreference = "Stop"
$radice = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $radice "target\x86_64-pc-windows-gnullvm\release\altair-run.exe"
$dll = Join-Path $radice "build\win\altair_ac1.dll"
foreach ($f in @($exe, $dll, $Gioco)) {
    if (-not (Test-Path -LiteralPath $f)) { Write-Error "Manca $f"; exit 1 }
}

# Una sola istanza alla volta.
Get-Process -Name "altair-run" -ErrorAction SilentlyContinue | Stop-Process -Force

$cartella = Join-Path $radice "run\win"
New-Item -ItemType Directory -Force -Path $cartella | Out-Null
$registro = Join-Path $cartella "log.txt"

$env:ALTAIR_WINDOW = "1"
$env:ALTAIR_STATS = "1"
if ($Backend) { $env:ALTAIR_BACKEND = $Backend } else { Remove-Item Env:ALTAIR_BACKEND -ErrorAction SilentlyContinue }
if ($SenzaAudio) { $env:ALTAIR_AUDIO = "0" } else { Remove-Item Env:ALTAIR_AUDIO -ErrorAction SilentlyContinue }
if ($SenzaVideo) { $env:ALTAIR_VIDEO = "0" } else { Remove-Item Env:ALTAIR_VIDEO -ErrorAction SilentlyContinue }

Write-Host "Avvio: $exe"
Write-Host "Registro: $registro"
# La cartella di lavoro è quella del progetto: profili e salvataggi stanno in run\c.
$p = Start-Process -FilePath $exe -ArgumentList @("`"$Gioco`"", "--lib", "`"$dll`"") `
    -WorkingDirectory $radice -NoNewWindow -Wait -PassThru -RedirectStandardError $registro
Write-Host "Uscito con codice $($p.ExitCode). Ultime righe del registro:"
Get-Content -LiteralPath $registro -Tail 15
