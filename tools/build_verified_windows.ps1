param([Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$sourceRoot = Split-Path -Parent $PSScriptRoot
$outputPath = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $outputPath) { throw "Build output already exists: $outputPath" }
New-Item -ItemType Directory -Path $outputPath | Out-Null
# Never share path-dependency artifacts between concurrently edited worktrees.
$targetPath = Join-Path $sourceRoot 'target/verified-windows'
$previousFlags = $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS
Push-Location $sourceRoot
try {
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static'
    cargo rustc --release --locked --target x86_64-pc-windows-msvc --target-dir $targetPath -p skate-game --bin skate3rust --no-default-features -- -C extra-filename= -o "$outputPath/skate3rust.exe" -C "link-arg=/PDB:$outputPath/skate3rust.pdb" *> "$outputPath/compile.log"
    if ($LASTEXITCODE -ne 0) { Get-Content "$outputPath/compile.log" -Tail 40; throw 'Game build failed' }
    & "$PSScriptRoot/verify_windows_build_sources.ps1" -Pdb "$outputPath/skate3rust.pdb" -SourceRoot $sourceRoot -ReportPath "$outputPath/source-checks.json"
} finally {
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = $previousFlags
    Pop-Location
}
