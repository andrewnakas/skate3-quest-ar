# Launch against the install that actually has maps built.
#
# There are two partial installs: the audio-work one has a complete audio set but
# an empty maps/ folder, and this one has all ten stock maps. There is no stock-map
# CLI flag -- --map takes custom .skate files only and crashes on a stock map path
# -- so the map comes from <install>/settings/default-map.json.
param(
    [string]$Install = 'C:\s3\installations\84bb8943f4a0439a80a7cc5e2e4489ca',
    [switch]$Trace
)
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$ErrorActionPreference = 'Stop'
Push-Location $ProjectRoot
try {
    $executable = Join-Path $ProjectRoot 'bin/skate3rust.exe'
    if (-not (Test-Path -LiteralPath $executable)) { throw 'Game is not built. Run BUILD.bat first.' }
    $assets = Join-Path $Install 'assets'
    if (-not (Test-Path -LiteralPath $assets)) { throw "No assets at $assets" }

    $mod = Join-Path $ProjectRoot 'mods/endless-tricks.zip'
    if (-not (Test-Path -LiteralPath $mod)) {
        Write-Warning 'mods/endless-tricks.zip is missing; run tools/package_mod.py first.'
    }
    $env:SKATE3_MODS = Join-Path $ProjectRoot 'mods'

    New-Item -ItemType Directory -Path (Join-Path $ProjectRoot 'logs') -Force | Out-Null
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $log = Join-Path $ProjectRoot "logs/endless-tricks-$stamp.log"
    $err = Join-Path $ProjectRoot "logs/endless-tricks-$stamp.err"
    if ($Trace) {
        # Without this a playtest log contains no scoring lines at all, which reads as a scoring
        # failure and is not one.
        $env:SKATE_SCORING_TRACE = '1'
    }

    Write-Host ''
    Write-Host 'ENDLESS TRICKS' -ForegroundColor Green
    Write-Host '  1. Escape -> Mods -> enable "Endless Tricks", then resume.'
    Write-Host '  2. Find some height, then hold the kickflip or heelflip flick through the air.'
    Write-Host ''
    Write-Host '  Flat ground still stops at a double. That is correct, not a bug: the authored'
    Write-Host '  air-time gates are untouched, so the ladder needs the pop to have time left.'
    Write-Host '  A fifth flip wants a real drop; the higher you get, the further it runs.'
    Write-Host ''
    Write-Host '  Disable the mod to get the stock four-rung ladder back exactly.'
    Write-Host '  Run with -Trace to write SCORE_TRICK lines into the .err log.'
    Write-Host ''
    Write-Host "  University loads in ~40 s and settles around 1.5 GB." -ForegroundColor DarkGray
    Write-Host "  Most output goes to stderr: $err" -ForegroundColor DarkGray
    Write-Host ''

    $game = Start-Process -FilePath $executable -WorkingDirectory $ProjectRoot `
        -ArgumentList @('--assets', ('"' + $assets + '"')) -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $log -RedirectStandardError $err
    if ($game.ExitCode -ne 0) {
        Get-Content -LiteralPath $err -Tail 30
        throw "Game exited with code $($game.ExitCode). Log: $err"
    }
} finally { Pop-Location }
