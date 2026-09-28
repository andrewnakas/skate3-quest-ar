param([string]$TargetDirectory = (Join-Path (Split-Path $PSScriptRoot -Parent) 'target'))
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$ErrorActionPreference = 'Stop'
Push-Location $ProjectRoot
try {
    # Cargo cache restoration can prune Python package files under target.
    # Hosted builds get a fresh packaging environment outside that cache.
    $packageEnvironment = if ($env:GITHUB_ACTIONS -eq 'true') {
        if (-not $env:RUNNER_TEMP) { throw 'RUNNER_TEMP is required in GitHub Actions' }
        Join-Path $env:RUNNER_TEMP ('skate-package-' + $env:GITHUB_RUN_ID + '-' + $env:GITHUB_RUN_ATTEMPT)
    } else { Join-Path $ProjectRoot 'target/package-venv' }
    $packagePython = Join-Path $packageEnvironment 'Scripts/python.exe'
    if (-not (Test-Path -LiteralPath $packagePython)) {
        & python -m venv $packageEnvironment
        if ($LASTEXITCODE -ne 0) { throw 'Could not create packaging environment' }
    }
    & $packagePython -m pip install -r tools/requirements-setup.txt
    if ($LASTEXITCODE -ne 0) { throw 'Could not install packaging dependencies' }
    & $packagePython -m unittest tools.test_setup_assets tools.asset_pipeline.test_marquee_assets tools.asset_pipeline.test_customiser_setup tools.asset_pipeline.test_setup_recovery tools.asset_pipeline.test_versions tools.asset_pipeline.test_optional_content
    if ($LASTEXITCODE -ne 0) { throw 'Setup extraction regression checks failed' }
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS = '-C target-feature=+crt-static'
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-ffff'
    $stage = Join-Path $ProjectRoot "target/release-packages/$stamp/skate3rust-windows-x64"
    $symbols = Join-Path $ProjectRoot "target/release-packages/$stamp/symbols"
    New-Item -ItemType Directory -Path "$stage/support",$symbols -Force | Out-Null
    # Link this invocation directly into private staging; never copy a generic cache EXE.
    & cargo rustc --release --locked --target x86_64-pc-windows-msvc --target-dir $TargetDirectory -p skate-game --bin skate3rust --no-default-features -- -C extra-filename= -o "$stage/skate3rust.exe" -C "link-arg=/PDB:$symbols/skate3rust.pdb"
    if ($LASTEXITCODE -ne 0) { throw 'Release compilation failed' }
    & cargo build --release --locked --target x86_64-pc-windows-msvc --target-dir $TargetDirectory -p skate-steam-relay
    if ($LASTEXITCODE -ne 0) { throw 'Steam relay compilation failed' }
    & (Join-Path $PSScriptRoot 'Stage-SteamRelay.ps1') `
        -TargetDirectory $TargetDirectory `
        -BinDirectory $stage `
        -RelayExecutable (Join-Path $TargetDirectory 'x86_64-pc-windows-msvc/release/skate-steam-relay.exe')
    # rustc also emits a dep-info file beside -o; it contains local source paths.
    $depInfo = Join-Path $stage 'skate3rust.d'
    if (Test-Path -LiteralPath $depInfo) { Remove-Item -LiteralPath $depInfo }
    New-Item -ItemType Directory -Path target/native -Force | Out-Null
    & rustc --edition 2024 --crate-type cdylib -C opt-level=3 -C panic=abort -C target-feature=+crt-static tools/asset_pipeline/refpack_native.rs -o target/native/refpack.dll
    if ($LASTEXITCODE -ne 0) { throw 'Native converter compilation failed' }
    if (-not (Test-Path -LiteralPath "$stage/skate3rust.exe") -or -not (Test-Path -LiteralPath "$symbols/skate3rust.pdb")) { throw 'Fresh executable or matching symbols missing.' }
    # Preserve the exact PE and PDB pair privately; symbols are not in the player ZIP.
    Copy-Item -LiteralPath "$stage/skate3rust.exe" -Destination $symbols
    @{
        revision = (& git rev-parse HEAD).Trim()
        source_status = @(& git status --porcelain)
        executable_sha256 = (Get-FileHash -LiteralPath "$stage/skate3rust.exe" -Algorithm SHA256).Hash
        pdb_sha256 = (Get-FileHash -LiteralPath "$symbols/skate3rust.pdb" -Algorithm SHA256).Hash
        compiler = (& rustc --version).Trim()
    } | ConvertTo-Json | Set-Content -LiteralPath "$symbols/build.json" -Encoding UTF8
    New-Item -ItemType Directory -Path "$stage/mods" -Force | Out-Null
    Copy-Item -LiteralPath mods/native-trainer.zip,mods/mario-kart.zip,mods/README.md -Destination "$stage/mods"
    $sourceStage = Join-Path $stage '../setup-source'
    $toolsRoot = Join-Path $ProjectRoot 'tools'
    foreach ($source in Get-ChildItem -LiteralPath $toolsRoot -File -Recurse) {
        if ($source.FullName -match '[\\/]__pycache__[\\/]') { continue }
        if ($source.Extension -notin '.py','.json','.txt','.md','.toml','.rs' -and $source.Name -ne 'LICENSE') { continue }
        $relative = [IO.Path]::GetRelativePath($toolsRoot, $source.FullName)
        $portableName = $relative.Replace('\','/')
        # Calibration is derived from private models, never a release resource.
        if ($portableName -like 'mixamo_to_skate/*.json') { continue }
        if ($portableName -match '(^|/)blender[^/]*(/|$)' -or
            $portableName -in @('asset_pipeline/build_map.py','asset_pipeline/finish_character.py',
                'add_onboard_ik_targets.py','apply_default_skater_materials.py','export_bevy_glb.py')) { continue }
        $destination = Join-Path "$sourceStage/tools" $relative
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        Copy-Item -LiteralPath $source.FullName -Destination $destination
    }
    $importerBinaries = Join-Path $ProjectRoot 'target/importer-runtime'
    & "$PSScriptRoot/Prepare-CharacterImporter.ps1" -Destination $importerBinaries
    & $packagePython -m PyInstaller --noconfirm --clean --onefile --name skate3setup `
        --icon "$ProjectRoot/docs/images/skating-crab.ico" --paths $ProjectRoot `
        --hidden-import numpy --hidden-import PIL.Image --hidden-import tkinter `
        --add-binary "$ProjectRoot/target/native/refpack.dll;tools/asset_pipeline" `
        --add-binary "$importerBinaries/FBX2glTF.exe;tools/mixamo_to_skate/tools" `
        --add-binary "$importerBinaries/*.dll;tools/mixamo_to_skate/tools" `
        --exclude-module bpy --exclude-module mathutils `
        --copy-metadata numpy --copy-metadata Pillow --copy-metadata PyInstaller `
        --add-data "$sourceStage/tools;tools" --add-data "$ProjectRoot/docs/images/skating-crab.ico;docs/images" `
        --distpath "$stage/support" --workpath target/setup-build/work --specpath target/setup-build tools/setup.py
    if ($LASTEXITCODE -ne 0) { throw 'Setup packaging failed' }
    & $packagePython tools/mixamo_to_skate/check_package.py --setup "$stage/support/skate3setup.exe"
    if ($LASTEXITCODE -ne 0) { throw 'Packaged character importer verification failed' }
    & $packagePython -m PyInstaller --noconfirm --clean --onefile --windowed --name skate3update `
        --distpath "$stage/support" --workpath target/updater-build/work --specpath target/updater-build tools/updater.py
    if ($LASTEXITCODE -ne 0) { throw 'Updater packaging failed' }
    # GitHub run number is monotonic across releases, including prereleases. Re-runs
    # deliberately retain identity; publish a new run/tag for a new eligible build.
    $build = if ($env:GITHUB_RUN_NUMBER) { [long]$env:GITHUB_RUN_NUMBER } else { 0 }
    $tag = if ($env:RELEASE_TAG) { $env:RELEASE_TAG } else { 'development' }
    $assetPipelinesJson = & $packagePython "$sourceStage/tools/asset_pipeline/versions.py" --tools "$sourceStage/tools"
    if ($LASTEXITCODE -ne 0) { throw 'Could not identify asset extractors' }
    $assetPipelines = $assetPipelinesJson | ConvertFrom-Json
    $characterCustomiser = (& $packagePython -m tools.asset_pipeline.customiser_setup --fingerprint).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not identify character customiser resources' }
    Copy-Item -LiteralPath README.md,docs/THIRD_PARTY_NOTICES.md -Destination $stage
    New-Item -ItemType Directory -Path "$stage/docs/images" -Force | Out-Null
    Copy-Item -LiteralPath docs/images/skating-crab.png -Destination "$stage/docs/images/skating-crab.png"
    Copy-Item -LiteralPath docs/installation.md -Destination "$stage/docs/installation.md"
    Copy-Item -LiteralPath docs/retail-renderer.md -Destination "$stage/docs/retail-renderer.md"
    Copy-Item -LiteralPath docs/crash-reports.md -Destination "$stage/docs/crash-reports.md"
    Copy-Item -LiteralPath docs/performance-tracing.md -Destination "$stage/docs/performance-tracing.md"
    Copy-Item -LiteralPath docs/updates.md -Destination "$stage/docs/updates.md"
    Copy-Item -LiteralPath docs/custom-models.md,docs/mixamo-to-skate.md -Destination "$stage/docs"
    Copy-Item -LiteralPath docs/character-customisation.md -Destination "$stage/docs"
    New-Item -ItemType Directory -Path "$stage/licenses" -Force | Out-Null
    Copy-Item -LiteralPath tools/mixamo_to_skate/licenses/FBX2glTF.txt -Destination "$stage/licenses/FBX2glTF.txt"
    Copy-Item -LiteralPath tools/vendor/utt/LICENSE -Destination "$stage/licenses/UTT.txt"
    Copy-Item -LiteralPath tools/vendor/university/LICENSE-PROJECT.md -Destination "$stage/licenses/CustomEngineLayer.txt"
    Copy-Item -LiteralPath vendor/bevy_pbr/LICENSE-MIT -Destination "$stage/licenses/Bevy-MIT.txt"
    Copy-Item -LiteralPath vendor/bevy_pbr/LICENSE-APACHE -Destination "$stage/licenses/Bevy-APACHE.txt"
    $pythonBase = (& $packagePython -c 'import sys; print(sys.base_prefix)').Trim()
    Copy-Item -LiteralPath "$pythonBase/LICENSE.txt" -Destination "$stage/licenses/Python.txt"
    foreach ($license in Get-ChildItem -LiteralPath "$pythonBase/tcl" -Filter license.terms -Recurse -ErrorAction SilentlyContinue) {
        Copy-Item -LiteralPath $license.FullName -Destination "$stage/licenses/$($license.Directory.Name).txt"
    }
    # Own every shipped program component, including future tools and libraries.
    # Mods are user-editable content and remain outside program replacement.
    $files = @{}
    foreach ($file in Get-ChildItem -LiteralPath $stage -File -Recurse) {
        $name = [IO.Path]::GetRelativePath($stage, $file.FullName).Replace('\', '/')
        if ($name -eq 'release.json' -or $name.StartsWith('mods/')) { continue }
        $files[$name] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLower()
    }
    @{
        schema = 1; repository = 'SK8-ENGINE/skate-3-rust-engine'; target = 'windows-x64'
        build = $build; tag = $tag; revision = (& git rev-parse HEAD).Trim(); files = $files; asset_pipelines = $assetPipelines; character_customiser = $characterCustomiser
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$stage/release.json" -Encoding utf8
    Copy-Item -LiteralPath "$stage/release.json" -Destination (Join-Path $ProjectRoot 'target/release.json')
    $zip = Join-Path $ProjectRoot 'target/skate3rust-windows-x64.zip'
    Compress-Archive -LiteralPath $stage -DestinationPath $zip -Force
    (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLower() + '  skate3rust-windows-x64.zip' |
        Set-Content -LiteralPath "$zip.sha256" -Encoding ascii
    Write-Host "Release package: $zip"
} finally { Pop-Location }
