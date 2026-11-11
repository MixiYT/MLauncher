param(
    [switch]$SkipBuild,
    [switch]$Lite
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$versionLine = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "([^"]+)"' | Select-Object -First 1
if (-not $versionLine) { throw 'Cannot find version in Cargo.toml' }
$version = $versionLine.Matches[0].Groups[1].Value
$edition = if ($Lite) { 'MLauncher-Lite' } else { 'MLauncher' }
$binaryName = "$edition.exe"
$targetRoot = if ($Lite) { Join-Path $root 'target\lite' } else { Join-Path $root 'target' }
$profile = if ($Lite) { 'lite' } else { 'release' }

if (-not $SkipBuild) {
    Write-Host "Building release $version ..."
    if (-not $Lite) {
        & (Join-Path $PSScriptRoot 'build-overlay.ps1')
        if ($LASTEXITCODE -ne 0) { throw 'Native overlay build failed' }
        $env:MLAUNCHER_OVERLAY_SHA256=(Get-FileHash (Join-Path $root 'native\bin\mlauncher-overlay.dll') -Algorithm SHA256).Hash
    } else { $env:MLAUNCHER_OVERLAY_SHA256=$null }
    if ($Lite) { cargo build --profile lite --features lite --target-dir $targetRoot }
    else { cargo build --release }
    if ($LASTEXITCODE -ne 0) { throw 'cargo build --release failed' }
}

$exe = Join-Path $targetRoot "$profile\mlauncher.exe"
if (-not (Test-Path $exe)) { throw "Release EXE not found: $exe" }

$dist = Join-Path $root 'dist'
$temporaryRoot = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Temp\opencode'
$stage = Join-Path $temporaryRoot ("package-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $stage | Out-Null
New-Item -ItemType Directory -Force -Path $dist | Out-Null

$stageExe = Join-Path $stage $binaryName
Copy-Item $exe $stageExe
$readme = if ($Lite) { 'README-Lite.md' } else { 'README.md' }
Copy-Item (Join-Path $root $readme) (Join-Path $stage 'README.md')
if (-not $Lite) { Copy-Item (Join-Path $root 'README-Lite.md') $stage }
Copy-Item (Join-Path $root 'LICENSE') $stage
if (-not $Lite) {
    Copy-Item (Join-Path $root 'native\bin\mlauncher-overlay.dll') $stage
    Copy-Item (Join-Path $root 'native\bin\LICENSE-*.txt') $stage
    New-Item -ItemType Directory -Force -Path (Join-Path $stage 'native') | Out-Null
    Copy-Item (Join-Path $root 'native\README.md') (Join-Path $stage 'native\README.md')
}

# Copy non-system DLLs the GNU build might depend on (usually none for Rust GNU).
$objdump = Get-Command objdump.exe -ErrorAction SilentlyContinue
$mingwBin = $null
$gcc = Get-Command gcc.exe -ErrorAction SilentlyContinue
if ($gcc) { $mingwBin = Split-Path -Parent $gcc.Source }
if ($objdump) {
    $dlls = & $objdump.Source -p $stageExe 2>$null |
        Select-String 'DLL Name:\s*(.+)$' |
        ForEach-Object { $_.Matches[0].Groups[1].Value.Trim() } |
        Sort-Object -Unique
    foreach ($dll in $dlls) {
        # API-set / UCRT forwarders входят в Windows 10+; копировать их нельзя.
        if ($dll -like 'api-ms-win-*') { continue }
        $already = Join-Path $stage $dll
        $system = Join-Path $env:WINDIR "System32\$dll"
        if ((Test-Path $already) -or (Test-Path $system)) { continue }
        $candidates = @()
        if ($mingwBin) { $candidates += (Join-Path $mingwBin $dll) }
        $command = Get-Command $dll -ErrorAction SilentlyContinue
        if ($command) { $candidates += $command.Source }
        $source = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
        if ($source) {
            Copy-Item $source $stage
            Write-Host "Bundled runtime DLL: $dll"
        } else {
            Write-Warning "Runtime DLL not found (may be part of Windows): $dll"
        }
    }
}

$zip = Join-Path $dist "$edition-$version-win64.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -CompressionLevel Optimal

# Publish only generated files. Never delete the old directory or stop a running launcher.
$publish = Join-Path $dist "$edition-$version-win64"
New-Item -ItemType Directory -Force -Path $publish | Out-Null
try {
    Copy-Item $stageExe (Join-Path $publish $binaryName) -Force
} catch {
    $publish = Join-Path $dist ("$edition-$version-win64-new-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    New-Item -ItemType Directory -Force -Path $publish | Out-Null
    Copy-Item $stageExe (Join-Path $publish $binaryName)
    Write-Warning "Previous EXE is in use or not writable; left it unchanged. New files: $publish"
}
Get-ChildItem $stage -File | Where-Object { $_.Name -ne $binaryName } | Copy-Item -Destination $publish -Force
if (-not $Lite) {
    New-Item -ItemType Directory -Force -Path (Join-Path $publish 'native') | Out-Null
    Copy-Item (Join-Path $stage 'native\README.md') (Join-Path $publish 'native\README.md') -Force
}

$hash = Get-FileHash $zip -Algorithm SHA256
Write-Host ""
Write-Host "Built: $zip"
Write-Host "EXE: $(Join-Path $publish $binaryName)"
Write-Host "SHA256: $($hash.Hash)"
Write-Host "EXE size: $([math]::Round((Get-Item $stageExe).Length / 1MB, 2)) MiB"
$resolvedStage = (Resolve-Path -LiteralPath $stage).ProviderPath
$resolvedTemporaryRoot = [IO.Path]::GetFullPath($temporaryRoot).TrimEnd('\') + '\'
if (-not $resolvedStage.StartsWith($resolvedTemporaryRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove staging directory outside temporary root: $resolvedStage"
}
Remove-Item -LiteralPath $resolvedStage -Recurse -Force
