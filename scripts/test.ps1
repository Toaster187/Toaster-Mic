# Runs all automated checks: protocol/crypto/jitter buffer (toaster-core),
# Android shared pieces (host build), Windows receiver lifecycle with the
# simulated phone, and the studio DSP (release, for realistic timing).
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'env.ps1')
Push-Location $projectRoot
try {
    & cargo test --workspace
    if ($LASTEXITCODE -ne 0) { throw 'Tests fehlgeschlagen.' }
    & cargo test --release -p toaster-dsp -- --nocapture --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw 'Studio-DSP-Tests (Release) fehlgeschlagen.' }
    & cargo check -p toaster-android-engine -p toaster-android-ui --target aarch64-linux-android
    if ($LASTEXITCODE -ne 0) { throw 'Android-Prüfbuild fehlgeschlagen.' }
    Write-Host 'Alle Prüfungen erfolgreich.'
} finally { Pop-Location }
