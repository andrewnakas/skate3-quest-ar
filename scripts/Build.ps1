param([switch]$StageOnly, [string]$TargetDirectory = (Join-Path $env:LOCALAPPDATA 'Skate3RustEngine/target'))
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$ErrorActionPreference = 'Stop'
Push-Location $ProjectRoot
try {
    if (-not $StageOnly) {
        & cargo build -p skate-game -p skate-steam-relay --locked --target-dir $TargetDirectory
        if ($LASTEXITCODE -ne 0) { throw 'Build failed; see the compiler output above.' }
    }
    $debugDirectory = Join-Path $TargetDirectory 'debug'
    $executable = Join-Path $debugDirectory 'skate3rust.exe'
    if (-not (Test-Path -LiteralPath $executable)) { throw "Missing executable: $executable" }
    $readobj = @(
        (Join-Path $env:ProgramFiles 'LLVM/bin/llvm-readobj.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/2022/BuildTools/VC/Tools/Llvm/x64/bin/llvm-readobj.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/2022/BuildTools/VC/Tools/Llvm/bin/llvm-readobj.exe')
    ) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $readobj) { throw 'LLVM llvm-readobj is required to stage exact runtime DLL dependencies.' }
    $rustLibraries = (& rustc --print target-libdir).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not locate Rust runtime libraries.' }
    $binDirectory = Join-Path $ProjectRoot 'bin'
    New-Item -ItemType Directory -Path $binDirectory -Force | Out-Null
    $queue = [System.Collections.Generic.Queue[string]]::new()
    $queue.Enqueue($executable)
    $seen = @{}
    $staged = @()
    while ($queue.Count -gt 0) {
        $source = $queue.Dequeue()
        $name = Split-Path -Leaf $source
        if ($seen.ContainsKey($name)) { continue }
        $seen[$name] = $true
        $destination = Join-Path $binDirectory $name
        Copy-Item -LiteralPath $source -Destination $destination -Force
        $staged += @{name = $name; sha256 = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash}
        # Keep development backtraces useful after staging the EXE and Bevy DLLs.
        $pdb = [IO.Path]::ChangeExtension($source, '.pdb')
        if (Test-Path -LiteralPath $pdb) {
            $symbolDestination = Join-Path $binDirectory (Split-Path -Leaf $pdb)
            Copy-Item -LiteralPath $pdb -Destination $symbolDestination -Force
            $staged += @{name = (Split-Path -Leaf $pdb); sha256 = (Get-FileHash -LiteralPath $symbolDestination -Algorithm SHA256).Hash}
        }
        $imports = & $readobj --coff-imports $source
        if ($LASTEXITCODE -ne 0) { throw "Could not inspect DLL imports: $source" }
        foreach ($line in $imports) {
            if ($line -match '^\s+Name: (.+\.dll)$') {
                $dependency = $Matches[1]
                $found = $false
                foreach ($directory in @($debugDirectory, (Join-Path $debugDirectory 'deps'), $rustLibraries)) {
                    $candidate = Join-Path $directory $dependency
                    if (Test-Path -LiteralPath $candidate) {
                        $queue.Enqueue($candidate)
                        $found = $true
                        break
                    }
                }
                if (-not $found -and $dependency -notmatch '^(api-ms-|ext-ms-)' -and
                    -not (Test-Path -LiteralPath (Join-Path $env:WINDIR "System32/$dependency"))) {
                    throw "Runtime dependency not found: $dependency"
                }
            }
        }
    }
    & (Join-Path $PSScriptRoot 'Stage-SteamRelay.ps1') -TargetDirectory $TargetDirectory -BinDirectory $binDirectory
    foreach ($name in @('steam-relay/skate-steam-relay.exe', 'steam-relay/steam_api64.dll')) {
        $staged += @{name = $name; sha256 = (Get-FileHash -LiteralPath (Join-Path $binDirectory $name) -Algorithm SHA256).Hash}
    }
    $staged | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $binDirectory 'manifest.json') -Encoding UTF8
    Write-Host "Ready: $binDirectory/skate3rust.exe"
} finally { Pop-Location }
