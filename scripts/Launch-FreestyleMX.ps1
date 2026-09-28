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

    $mod = Join-Path $ProjectRoot 'mods/freestyle-mx.zip'
    if (-not (Test-Path -LiteralPath $mod)) {
        Write-Warning 'mods/freestyle-mx.zip is missing; run tools/package_mod.py first.'
    }
    $env:SKATE3_MODS = Join-Path $ProjectRoot 'mods'

    New-Item -ItemType Directory -Path (Join-Path $ProjectRoot 'logs') -Force | Out-Null
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $log = Join-Path $ProjectRoot "logs/freestyle-mx-$stamp.log"
    $err = Join-Path $ProjectRoot "logs/freestyle-mx-$stamp.err"
    if ($Trace) {
        $env:SKATE_AUDIO_OBSERVE = '1'
        $env:SKATE_AUDIO_TRACE = Join-Path $ProjectRoot "logs/audio-trace-$stamp.log"
    }

    Write-Host ''
    Write-Host 'FREESTYLE MX' -ForegroundColor Green
    Write-Host '  1. Escape -> Mods -> enable "Freestyle MX", then resume.'
    Write-Host '  2. Press F9 to spawn the bike, then Y (or E) to get on.'
    Write-Host ''
    Write-Host '  LEFT STICK  steer; pull BACK on a jump face and snap forward at the lip to preload'
    Write-Host '  RIGHT STICK rider weight - lean on the ground, roll in the air'
    Write-Host '  RT / LT     throttle / front brake        B rear brake'
    Write-Host '  LB or RB + RIGHT STICK = trick. Release to tuck in BEFORE you land.'
    Write-Host '  R or R3 reset, F9 respawn, Y or E to get off below 3 m/s'
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
