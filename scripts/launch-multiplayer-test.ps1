param([string]$AssetRoot = $env:SKATE3_ASSETS, [switch]$MenuOnly, [string]$MapPath = '', [switch]$TwoControllers, [ValidateRange(2,10)][int]$Players = 2, [string]$BuildDirectory = 'bin/multiplayer')
$ErrorActionPreference = 'Stop'
$instanceCount = if ($MenuOnly) { 1 } else { $Players }
$workspace = Split-Path -Parent $PSScriptRoot
$binary = Join-Path (Join-Path $workspace $BuildDirectory) 'skate3-multiplayer.exe'
if (-not $AssetRoot) { $AssetRoot = Join-Path $workspace 'assets' }
$assets = (Resolve-Path -LiteralPath $AssetRoot).Path
$installation = Split-Path -Parent $assets
if (-not (Test-Path -LiteralPath $binary)) { throw 'Build missing. Run scripts/build-multiplayer-test.ps1 first.' }
if (-not (Test-Path -LiteralPath (Join-Path $assets 'private/game.json'))) { throw 'Prepared game assets are missing. No installer or conversion will be started.' }
if (-not $MapPath) { $MapPath = Join-Path $installation 'maps/University.skate' }
$MapPath = (Resolve-Path -LiteralPath $MapPath).Path
if ([IO.Path]::GetExtension($MapPath) -ne '.skate') { throw 'Expected an existing .skate map.' }
$logs = Join-Path $workspace ('logs/multiplayer/' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Path $logs -Force | Out-Null
# Reserve distinct ports, then release immediately before the games start.
$reservations = @(1..$instanceCount | ForEach-Object { [Net.Sockets.UdpClient]::new([Net.IPEndPoint]::new([Net.IPAddress]::Loopback,0)) })
$ports = @($reservations | ForEach-Object { $_.Client.LocalEndPoint.Port })
$session = Get-Random -Minimum 1 -Maximum ([long]::MaxValue)
$reservations | ForEach-Object { $_.Dispose() }
$common = '--assets "' + $assets + '" --map "' + $MapPath + '" --net-session ' + $session
for ($index = 0; $index -lt $instanceCount; $index++) {
    $label = [string][char](65 + $index)
    $playerArgs = $common
    if ($MenuOnly) {
        # Start offline; Steam initializes only through the multiplayer menu.
        $playerArgs += ' --start-paused'
    } elseif ($index -eq 0) {
        $playerArgs += ' --net-host 127.0.0.1:' + $ports[0]
    } else {
        $playerArgs += ' --net-local 127.0.0.1:' + $ports[$index] + ' 127.0.0.1:' + $ports[0]
        # Guests advertise an unavailable outfit to exercise default fallback.
        $playerArgs += ' --spawn-offset ' + ($index * 2).ToString([Globalization.CultureInfo]::InvariantCulture) + ' --appearance test-uninstalled-outfit'
    }
    $playerArgs += ' --player-title "Multiplayer ' + $label + ' - ' + $Players + ' player test"'
    if ($TwoControllers -and $index -lt 2) { $playerArgs += ' --controller ' + $index }
    $process = Start-Process -FilePath $binary -ArgumentList $playerArgs -WorkingDirectory $workspace -WindowStyle Normal -RedirectStandardOutput (Join-Path $logs ('player-' + $label.ToLower() + '.out.log')) -RedirectStandardError (Join-Path $logs ('player-' + $label.ToLower() + '.err.log')) -PassThru
    Write-Host "Started player $label ($($process.Id))"
}
if ($MenuOnly) { Write-Host "Press Esc, then Multiplayer to host or browse Steam lobbies." }
else { Write-Host "$Players players connect automatically on University; A hosts the lobby." }
Write-Host 'Steam is not required. Click a game window to control that player with your controller.'
Write-Host 'For two pads, use -TwoControllers (XInput slots 0 and 1). Use -Players 10 for the full lobby.'
Write-Host "Logs: $logs"
