param(
    [string]$TargetDirectory = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target'),
    [string]$BinDirectory = (Join-Path (Split-Path $PSScriptRoot -Parent) 'bin'),
    [string]$RelayExecutable = ''
)
$ErrorActionPreference = 'Stop'
$relayExecutable = if ($RelayExecutable) { $RelayExecutable } else { Join-Path $TargetDirectory 'debug/skate-steam-relay.exe' }
if (-not (Test-Path -LiteralPath $relayExecutable)) { throw 'Build skate-steam-relay before staging.' }
$relayMetadata = (& cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc) -join "`n"
if ($LASTEXITCODE -ne 0) { throw 'Could not locate the locked Steam SDK redistributable.' }
$sdk = [regex]::Match($relayMetadata, '"name":"steamworks-sys","version":"0\.13\.0".*?"manifest_path":("(?:[^"\\]|\\.)*")')
if (-not $sdk.Success) { throw 'Expected steamworks-sys 0.13.0 in Cargo metadata.' }
$sdkManifest = ConvertFrom-Json -InputObject $sdk.Groups[1].Value
$steamDll = Join-Path (Split-Path -Parent $sdkManifest) 'lib/steam/redistributable_bin/win64/steam_api64.dll'
if (-not (Test-Path -LiteralPath $steamDll)) { throw "Missing Steam SDK DLL: $steamDll" }
$relayDirectory = Join-Path $BinDirectory 'steam-relay'
New-Item -ItemType Directory -Path $relayDirectory -Force | Out-Null
Copy-Item -LiteralPath $relayExecutable -Destination (Join-Path $relayDirectory 'skate-steam-relay.exe') -Force
Copy-Item -LiteralPath $steamDll -Destination (Join-Path $relayDirectory 'steam_api64.dll') -Force
Write-Host "Steam relay ready: $relayDirectory"
