# Make the zip the Quest app imports, from YOUR OWN converted Skate 3 disc.
#
# Easiest: put this file (and MAKE-QUEST-ZIP.bat) in the folder with skate3rust.exe and
# double-click MAKE-QUEST-ZIP.bat. It finds the converted game data by itself and writes
# Skate3-Quest.zip next to itself. Copy that zip to the headset's Download\Skate3 folder.
#
#   powershell -ExecutionPolicy Bypass -File make-bundle.ps1 [-Assets <dir>] [-Cache <dir>] [-Out <dir>] [-NoZip]
#
# -Assets  a converted installation's assets folder (has private\game.json); found by itself if left out
# -Cache   the decoded sound banks (%LOCALAPPDATA%\Skate3RustEngine\audio-pcm-cache)
# -Out     where the zip goes (default: this script's folder)
# -NoZip   leave a quest-data folder instead of a zip (for scripts/quest/push-assets.sh)
param(
    [string]$Assets = "",
    [string]$Cache = (Join-Path $env:LOCALAPPDATA "Skate3RustEngine\audio-pcm-cache"),
    [string]$Out = "",
    [switch]$NoZip
)
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
if (-not $Out) { $Out = $here }

function Find-Installation {
    # Look beside this script (the game folder), above it, and in the usual places, for a
    # converted installation: an assets folder holding private\game.json. Newest wins.
    $roots = @($here, (Split-Path -Parent $here), (Split-Path -Parent (Split-Path -Parent $here)), "C:\s3", "$env:USERPROFILE\Downloads", "$env:USERPROFILE\Desktop") |
        Where-Object { $_ -and (Test-Path $_) } | Select-Object -Unique
    $found = foreach ($r in $roots) {
        Get-ChildItem -Path $r -Recurse -Depth 5 -Filter game.json -ErrorAction SilentlyContinue |
            Where-Object { $_.Directory.Name -eq "private" -and $_.Directory.Parent.Name -eq "assets" }
    }
    $best = $found | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($best) { return $best.Directory.Parent.FullName }
    return $null
}

Write-Host ""
Write-Host "Skate 3 AR - making the Quest import zip from your own converted disc" -ForegroundColor Cyan
Write-Host ""

if (-not $Assets) { $Assets = Find-Installation }
if (-not $Assets -or -not (Test-Path (Join-Path $Assets "private\game.json"))) {
    Write-Host "Couldn't find your converted Skate 3 data." -ForegroundColor Red
    Write-Host "1. Run skate3rust.exe once and pick your Skate 3 Xbox 360 ISO; let the setup finish."
    Write-Host "2. Put this script in the same folder as skate3rust.exe and run it again,"
    Write-Host "   or pass the folder yourself:  -Assets <...\installations\<id>\assets>"
    exit 1
}
Write-Host "Game data:   $Assets"

$haveCache = [bool](Get-ChildItem $Cache -Filter *.v3.pcm -ErrorAction SilentlyContinue | Select-Object -First 1)
if ($haveCache) {
    Write-Host "Sound banks: $Cache"
} else {
    Write-Host "Sound banks: none yet - the retail sounds will be missing on the Quest." -ForegroundColor Yellow
    Write-Host "             Play the desktop game for a minute (skate around), then run this again."
}

$stage = Join-Path $Out "quest-data"
New-Item -ItemType Directory -Force (Join-Path $stage "assets") | Out-Null
Write-Host ""
Write-Host "Copying the game data..."
robocopy $Assets (Join-Path $stage "assets") /MIR /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "copying the game data failed (robocopy $LASTEXITCODE)" }
if ($haveCache) {
    New-Item -ItemType Directory -Force (Join-Path $stage "audio-pcm-cache") | Out-Null
    robocopy $Cache (Join-Path $stage "audio-pcm-cache") *.v3.pcm /MIR /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "copying the sound banks failed (robocopy $LASTEXITCODE)" }
}
$global:LASTEXITCODE = 0

if ($NoZip) {
    $size = (Get-ChildItem $stage -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
    Write-Host ("quest-data ready: {0:N0} MB in {1}" -f $size, $stage) -ForegroundColor Green
    exit 0
}

$zip = Join-Path $Out "Skate3-Quest.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Write-Host "Zipping (this can take a few minutes)..."
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [System.IO.Compression.CompressionLevel]::Fastest, $false)
Remove-Item $stage -Recurse -Force
$size = (Get-Item $zip).Length / 1MB
Write-Host ""
Write-Host ("Done: {0} ({1:N0} MB)" -f $zip, $size) -ForegroundColor Green
Write-Host ""
Write-Host "Next:"
Write-Host "  1. Plug the Quest into this PC, allow file access in the headset."
Write-Host "  2. Copy Skate3-Quest.zip into the Quest's  Download\Skate3  folder (make the Skate3 folder)."
Write-Host "  3. In Skate 3 AR: menu > Skate 3 data > Import, then close and reopen the app."
