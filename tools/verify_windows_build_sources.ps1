param(
    [Parameter(Mandatory=$true)][string]$Pdb,
    [string]$SourceRoot = (Split-Path -Parent $PSScriptRoot),
    [string]$ReportPath
)
$ErrorActionPreference = 'Stop'
$sourcePrefix = [IO.Path]::GetFullPath($SourceRoot).TrimEnd('\', '/') + '\'
$pdbutil = Join-Path $env:ProgramFiles 'LLVM/bin/llvm-pdbutil.exe'
$dump = & $pdbutil dump --files $Pdb
if ($LASTEXITCODE -ne 0) { throw 'PDB source inspection failed' }
$checked = @{}
$failures = [Collections.Generic.List[string]]::new()
foreach ($line in $dump) {
    if ($line -notmatch '^- \(SHA-256: ([0-9A-Fa-f]{64})\) (.+)$') { continue }
    $compiledHash = $Matches[1]
    $compiledPath = $Matches[2].Replace('/', '\')
    if ($compiledPath -notmatch '\\((?:crates\\skate-[^\\]+|vendor\\bevy_[^\\]+)\\.+)$') { continue }
    $relative = $Matches[1]
    $key = "$compiledPath|$compiledHash"
    if ($checked.ContainsKey($key)) { continue }
    $expectedPath = Join-Path $sourcePrefix $relative
    $sourceHash = if (Test-Path -LiteralPath $expectedPath -PathType Leaf) {
        (Get-FileHash -LiteralPath $expectedPath -Algorithm SHA256).Hash
    } else { $null }
    $matchesRoot = $compiledPath.StartsWith($sourcePrefix, [StringComparison]::OrdinalIgnoreCase)
    $matchesHash = $sourceHash -eq $compiledHash
    $checked[$key] = [ordered]@{ file=$relative; compiledPath=$compiledPath; compiledSha256=$compiledHash; sourceSha256=$sourceHash; matchesRoot=$matchesRoot; matchesHash=$matchesHash }
    if (-not $matchesRoot -or -not $matchesHash) { $failures.Add($relative) }
}
foreach ($crate in @('skate-core', 'skate-data', 'skate-game')) {
    if (-not ($checked.Values | Where-Object { $_.file.StartsWith("crates\$crate\") })) {
        $failures.Add("No source checksums found for $crate")
    }
}
if ($ReportPath) {
    [ordered]@{ pdb=[IO.Path]::GetFullPath($Pdb); sourceRoot=$sourcePrefix; passed=($failures.Count -eq 0); files=@($checked.Values | Sort-Object file,compiledPath) } |
        ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $ReportPath -Encoding utf8
}
if ($failures.Count) { throw "PDB source provenance failed for $($failures.Count) entries: $(($failures | Select-Object -First 8) -join ', ')" }
Write-Output "Verified $($checked.Count) workspace source checksums and paths in $Pdb"
