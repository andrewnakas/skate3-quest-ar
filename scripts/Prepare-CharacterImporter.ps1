param([Parameter(Mandatory)][string]$Destination)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$cache = Join-Path $root 'target/importer-deps'
New-Item -ItemType Directory -Path $cache,$Destination -Force | Out-Null
$exe = Join-Path $cache 'FBX2glTF.exe'
$sha = '8d90fb5e0a8d186a3d9a7ff8c75eaee541c3975ce4df0d80351f20092ae0877f'
if (-not (Test-Path -LiteralPath $exe)) {
    Invoke-WebRequest 'https://github.com/facebookincubator/FBX2glTF/releases/download/v0.9.7/FBX2glTF-windows-x64.exe' -OutFile $exe
}
if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLower() -ne $sha) {
    throw 'FBX2glTF checksum mismatch; remove target/importer-deps/FBX2glTF.exe and retry.'
}
Copy-Item -LiteralPath $exe -Destination $Destination
# FBX2glTF uses the VC runtime. Ship the redistributable DLLs beside it so
# players do not need Visual Studio or a separate runtime installer.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = (& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
$crt = Get-ChildItem -Path "$vs/VC/Redist/MSVC/*/x64/Microsoft.VC*.CRT" -Directory |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $crt) { throw 'Visual C++ x64 redistributable files are required to package the importer.' }
foreach ($name in @('msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll')) {
    Copy-Item -LiteralPath (Join-Path $crt.FullName $name) -Destination $Destination
}
