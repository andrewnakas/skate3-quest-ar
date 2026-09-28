param([string]$OutputDirectory = 'bin/multiplayer')
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $workspace
$output = [IO.Path]::GetFullPath((Join-Path $workspace $OutputDirectory))
$relay = Join-Path $output 'steam-relay'
$build = Join-Path $output 'build'
New-Item -ItemType Directory -Path $relay,$build -Force | Out-Null
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static'
# Share dependency artifacts, but link directly into this checkout's staging folder.
& cargo rustc --release --target x86_64-pc-windows-msvc --target-dir (Join-Path $workspace 'target') -p skate-game --bin skate3-multiplayer --no-default-features --locked -- -C extra-filename=-multiplayer-test -o (Join-Path $build 'skate3.exe')
if ($LASTEXITCODE -ne 0) { throw 'Multiplayer build failed.' }
& cargo rustc --release --target x86_64-pc-windows-msvc --target-dir (Join-Path $workspace 'target') -p skate-steam-relay --bin skate-steam-relay --locked -- -C extra-filename=-multiplayer-test -o (Join-Path $build 'relay.exe')
if ($LASTEXITCODE -ne 0) { throw 'Steam relay build failed.' }
Copy-Item -LiteralPath (Join-Path $build 'skate3-multiplayer-test.exe') -Destination (Join-Path $output 'skate3-multiplayer.exe') -Force
Copy-Item -LiteralPath (Join-Path $build 'relay-multiplayer-test.exe') -Destination (Join-Path $relay 'skate-steam-relay.exe') -Force
$metadata = (& cargo metadata --format-version 1 --locked) -join "`n"
if ($LASTEXITCODE -ne 0) { throw 'Could not locate the Steam redistributable.' }
# Extract only the JSON string we need. Windows PowerShell's JSON object parser
# rejects unrelated dependency feature keys that differ only in letter case.
$sdk = [regex]::Match($metadata, '"name":"steamworks-sys","version":"0\.13\.0".*?"manifest_path":("(?:[^"\\]|\\.)*")')
if (-not $sdk.Success) { throw 'Steamworks SDK package was not found in Cargo metadata.' }
$manifest = ConvertFrom-Json -InputObject $sdk.Groups[1].Value
$dll = Join-Path (Split-Path -Parent $manifest) 'lib/steam/redistributable_bin/win64/steam_api64.dll'
Copy-Item -LiteralPath $dll -Destination $relay
Write-Host "Staged multiplayer build: $output"
Write-Host 'Build only; no game or Steam process was launched.'
