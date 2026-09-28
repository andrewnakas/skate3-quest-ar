# Collect the game data the Quest app needs into quest-data\ (gitignored),
# ready to copy to the Mac and push with scripts/quest/push-assets.sh.
#   powershell -File scripts\quest\make-bundle.ps1 [-Assets <dir>] [-Cache <dir>]
param(
    [string]$Assets = "C:\s3\installations\70eda9dc4644496d81ae73af95ff4285\assets",
    [string]$Cache = (Join-Path $env:LOCALAPPDATA "Skate3RustEngine\audio-pcm-cache")
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$out = Join-Path $root "quest-data"
if (-not (Test-Path (Join-Path $Assets "private\game.json"))) {
    throw "$Assets is not a converted installation (no private\game.json)"
}
if (-not (Get-ChildItem $Cache -Filter *.v3.pcm -ErrorAction SilentlyContinue)) {
    throw "No decoded banks in $Cache. Run the desktop game (or the headless player-audio test) once first."
}
New-Item -ItemType Directory -Force (Join-Path $out "assets"), (Join-Path $out "audio-pcm-cache") | Out-Null
robocopy $Assets (Join-Path $out "assets") /MIR /NFL /NDL /NJH /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy assets failed ($LASTEXITCODE)" }
robocopy $Cache (Join-Path $out "audio-pcm-cache") *.v3.pcm /MIR /NFL /NDL /NJH /NP | Out-Null
if ($LASTEXITCODE -ge 8) { throw "robocopy cache failed ($LASTEXITCODE)" }
$global:LASTEXITCODE = 0
$size = (Get-ChildItem $out -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
"quest-data ready: {0:N0} MB in {1}" -f $size, $out
