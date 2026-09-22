$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'DriverSignature.ps1')
$directory = Join-Path $projectRoot 'artifacts\driver-testsigned'
$manifest = Get-Content -Raw -LiteralPath (Join-Path $directory 'signing.json') | ConvertFrom-Json
$scratch = Join-Path $projectRoot 'artifacts\tests'
New-Item -ItemType Directory -Force $scratch | Out-Null
foreach ($name in @('ToasterMic.sys','ToasterMic.cat')) {
    $source = Join-Path $directory $name
    $signature = Get-AuthenticodeSignature -LiteralPath $source
    if ($signature.SignerCertificate.Thumbprint -ne $manifest.certificateThumbprint -or
        [ToasterMic.BuildSignature]::Verify($source) -notin @(0, [uint32]2148204809)) { throw "Ungültige Entwicklungssignatur: $name" }
    Write-Host "PASS $name development signature and file digest"
    $test = Join-Path $scratch ([Guid]::NewGuid().ToString('N') + [IO.Path]::GetExtension($name))
    try {
        $bytes = [IO.File]::ReadAllBytes($source)
        $offset = if ($name.EndsWith('.sys')) { 1024 } else { $bytes.Length - 1 }
        $bytes[$offset] = $bytes[$offset] -bxor 1
        [IO.File]::WriteAllBytes($test, $bytes)
        if ([ToasterMic.BuildSignature]::Verify($test) -in @(0, [uint32]2148204809)) { throw "Manipulierte Datei akzeptiert: $name" }
        Write-Host "PASS $name tamper rejected"
    } finally { if (Test-Path -LiteralPath $test) { Remove-Item -LiteralPath $test -Force } }
}
if (Get-ChildItem -LiteralPath (Join-Path $projectRoot '.tools') -Filter 'driver-signing-*.pfx') { throw 'Temporärer Signierschlüssel wurde nicht gelöscht.' }
Write-Host 'PASS no temporary signing private key left behind'
