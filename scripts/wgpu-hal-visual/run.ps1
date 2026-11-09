[CmdletBinding()]
param(
    [string]$OutputDirectory,
    [string]$TargetDirectory,
    [string]$Toolchain
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Run this test on an interactive Windows desktop.' }
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
if (-not $TargetDirectory) {
    $TargetDirectory = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repository 'target/wgpu-hal-visual' }
}
$TargetDirectory = [IO.Path]::GetFullPath($TargetDirectory)
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $TargetDirectory ('results/' + [guid]::NewGuid().ToString('N')) }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if ((Test-Path -LiteralPath $OutputDirectory) -and (Get-ChildItem -LiteralPath $OutputDirectory -Force | Select-Object -First 1)) {
    throw 'Output directory must be new or empty; old captures must not pass a new run.'
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$vendor = Join-Path $repository 'vendor/wgpu-hal'
$savedTarget = $env:CARGO_TARGET_DIR
$savedBackend = $env:WGPU_BACKEND
$cargoOptions = @()
if ($Toolchain) { $cargoOptions += "+$Toolchain" }
$executables = @{}
$metadata = @{}

try {
    $env:CARGO_TARGET_DIR = $TargetDirectory
    $env:WGPU_BACKEND = 'dx12'
    foreach ($side in @('before', 'after')) {
        $build = Join-Path $OutputDirectory "build/$side"
        New-Item -ItemType Directory -Path $build -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Cargo.toml'), (Join-Path $PSScriptRoot 'Cargo.lock') -Destination $build
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'src') -Destination $build -Recurse
        $manifest = Join-Path $build 'Cargo.toml'
        if ($side -eq 'after') {
            # Only the second temporary manifest selects the vendor copy. The
            # repository and its published baseline manifest are never edited.
            $tomlPath = ConvertTo-Json -InputObject ($vendor.Replace('\', '/')) -Compress
            [IO.File]::AppendAllText($manifest, "`n[patch.crates-io]`nwgpu-hal = { path = $tomlPath }`n", [Text.UTF8Encoding]::new($false))
            # Cargo replaces the registry entry with the path entry. Offline
            # resolution cannot fetch new dependencies; graph equality below
            # rejects any unrelated version/source change before either run.
            & cargo @cargoOptions build --manifest-path $manifest --offline
        } else {
            & cargo @cargoOptions build --manifest-path $manifest --locked
        }
        if ($LASTEXITCODE -ne 0) { throw "Could not build $side variant." }
        $text = & cargo @cargoOptions metadata --manifest-path $manifest --format-version 1 --locked
        if ($LASTEXITCODE -ne 0) { throw "Could not read $side dependency graph." }
        $metadata[$side] = $text | ConvertFrom-Json
        $binary = Join-Path $build "$side.exe"
        Copy-Item -LiteralPath (Join-Path $TargetDirectory 'debug/dx12-egui-smoke.exe') -Destination $binary
        $executables[$side] = $binary
    }

    $beforeGraph = @($metadata.before.packages | Where-Object name -ne 'wgpu-hal' | ForEach-Object { "$($_.name)@$($_.version)|$($_.source)" } | Sort-Object)
    $afterGraph = @($metadata.after.packages | Where-Object name -ne 'wgpu-hal' | ForEach-Object { "$($_.name)@$($_.version)|$($_.source)" } | Sort-Object)
    if (Compare-Object $beforeGraph $afterGraph) { throw 'A/B dependency versions or sources differ beyond wgpu-hal.' }
    $originalHal = @($metadata.before.packages | Where-Object name -eq 'wgpu-hal')
    $patchedHal = @($metadata.after.packages | Where-Object name -eq 'wgpu-hal')
    if ($originalHal.Count -ne 1 -or $patchedHal.Count -ne 1) { throw 'Expected exactly one wgpu-hal in each graph.' }
    if ($originalHal[0].version -ne '30.0.1' -or $patchedHal[0].version -ne '30.0.1' -or -not $originalHal[0].source.StartsWith('registry+')) {
        throw 'The baseline must be the published wgpu-hal 30.0.1.'
    }
    if ([IO.Path]::GetFullPath($patchedHal[0].manifest_path) -ne [IO.Path]::GetFullPath((Join-Path $vendor 'Cargo.toml'))) {
        throw 'The patched graph did not select this checkout vendor/wgpu-hal.'
    }
    foreach ($file in Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'src') -File) {
        $a = (Get-FileHash -LiteralPath (Join-Path $OutputDirectory "build/before/src/$($file.Name)") -Algorithm SHA256).Hash
        $b = (Get-FileHash -LiteralPath (Join-Path $OutputDirectory "build/after/src/$($file.Name)") -Algorithm SHA256).Hash
        if ($a -ne $b) { throw "A/B harness source differs: $($file.Name)" }
    }
    @{
        baseline = 'crates.io wgpu-hal 30.0.1'
        patched = 'this checkout vendor/wgpu-hal'
        vendor_dx12_sha256 = (Get-FileHash -LiteralPath (Join-Path $vendor 'src/dx12/mod.rs') -Algorithm SHA256).Hash
        other_dependency_versions_and_sources_match = $true
        harness_sources_match = $true
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $OutputDirectory 'provenance.json') -Encoding UTF8

    foreach ($side in @('before', 'after')) {
        & $executables[$side] capture (Join-Path $OutputDirectory $side) *> (Join-Path $OutputDirectory "$side.log")
        if ($LASTEXITCODE -ne 0) { throw "Capture failed for $side. Inspect $OutputDirectory/$side.log." }
    }
    & $executables.after compare $OutputDirectory
    $comparisonExit = $LASTEXITCODE
    $report = Join-Path $OutputDirectory 'RESULTS.md'
    if ((Test-Path -LiteralPath $report) -and $env:GITHUB_STEP_SUMMARY) {
        Get-Content -LiteralPath $report | Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Encoding UTF8
    }
    Write-Host "Images and numerical report: $OutputDirectory"
    if ($comparisonExit -eq 2) { throw 'INCONCLUSIVE: unpatched blur did not reproduce; this is not a regression pass.' }
    if ($comparisonExit -ne 0) { throw 'FAIL: inspect RESULTS.md and the captures.' }
} finally {
    $env:CARGO_TARGET_DIR = $savedTarget
    $env:WGPU_BACKEND = $savedBackend
}
