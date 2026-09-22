[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'DriverSignature.ps1')
$inputDirectory = Join-Path $projectRoot 'artifacts\driver-unsigned'
$outputDirectory = Join-Path $projectRoot 'artifacts\driver-testsigned'
$sdkBin = 'C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64'
$signTool = Join-Path $sdkBin 'signtool.exe'
$inf2cat = Join-Path $projectRoot '.tools\wdk\c\bin\10.0.26100.0\x86\Inf2Cat.exe'
foreach ($path in @($signTool, $inf2cat, (Join-Path $inputDirectory 'ToasterMic.sys'), (Join-Path $inputDirectory 'ToasterMic.inf'))) {
    if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw "Benötigte Datei fehlt: $path" }
}
New-Item -ItemType Directory -Force $outputDirectory | Out-Null
$pfxPath = Join-Path $projectRoot ('.tools\driver-signing-' + [Guid]::NewGuid().ToString('N') + '.pfx')
$rsa = $null; $certificate = $null
try {
    # Only an in-memory certificate and a temporary password-protected PFX.
    # No certificate store, boot configuration or system trust is modified.
    $rsa = [Security.Cryptography.RSA]::Create(3072)
    $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new(
        'CN=Toaster Mic Development Only', $rsa, [Security.Cryptography.HashAlgorithmName]::SHA256,
        [Security.Cryptography.RSASignaturePadding]::Pkcs1)
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false, $false, 0, $true))
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new([Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature, $true))
    $oids = [Security.Cryptography.OidCollection]::new()
    $null = $oids.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.3'))
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($oids, $true))
    $certificate = $request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddMinutes(-10), [DateTimeOffset]::UtcNow.AddDays(365))
    $password = [Guid]::NewGuid().ToString('N') + [Guid]::NewGuid().ToString('N')
    [IO.File]::WriteAllBytes($pfxPath, $certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Pfx, $password))
    foreach ($name in @('ToasterMic.inf','ToasterMic.sys')) {
        Copy-Item -LiteralPath (Join-Path $inputDirectory $name) -Destination (Join-Path $outputDirectory $name) -Force
    }
    $binary = Join-Path $outputDirectory 'ToasterMic.sys'
    & $signTool sign /fd SHA256 /f $pfxPath /p $password $binary
    if ($LASTEXITCODE -ne 0) { throw 'Entwicklungssignatur der SYS-Datei fehlgeschlagen.' }
    # The catalog must hash the signed SYS, not the original unsigned file.
    & $inf2cat "/driver:$outputDirectory" /os:10_GE_X64 /uselocaltime
    if ($LASTEXITCODE -ne 0) { throw 'Katalog für den signierten Treiber konnte nicht erzeugt werden.' }
    $catalog = Join-Path $outputDirectory 'ToasterMic.cat'
    & $signTool sign /fd SHA256 /f $pfxPath /p $password $catalog
    if ($LASTEXITCODE -ne 0) { throw 'Entwicklungssignatur des Katalogs fehlgeschlagen.' }
    foreach ($file in @($binary, $catalog)) {
        $signature = Get-AuthenticodeSignature -LiteralPath $file
        $result = [ToasterMic.BuildSignature]::Verify($file)
        if ($signature.SignerCertificate.Thumbprint -ne $certificate.Thumbprint -or $result -notin @(0, [uint32]2148204809)) {
            throw ('Signaturprüfung fehlgeschlagen: {0} (WinVerifyTrust 0x{1:X8})' -f $file, $result)
        }
    }
    [IO.File]::WriteAllBytes((Join-Path $outputDirectory 'ToasterMic-Development.cer'), $certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert))
    @{
        purpose = 'Development only - not Microsoft signed'
        certificateThumbprint = $certificate.Thumbprint
        sysSha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
        catalogSha256 = (Get-FileHash -LiteralPath $catalog -Algorithm SHA256).Hash
        infSha256 = (Get-FileHash -LiteralPath (Join-Path $outputDirectory 'ToasterMic.inf') -Algorithm SHA256).Hash
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $outputDirectory 'signing.json') -Encoding UTF8
    Write-Host 'SYS und Katalog automatisch entwicklungssigniert. Keine Microsoft-Freigabe; kein Systemvertrauen oder Testmodus verändert.'
} finally {
    if (Test-Path -LiteralPath $pfxPath) { Remove-Item -LiteralPath $pfxPath -Force }
    if ($certificate) { $certificate.Dispose() }
    if ($rsa) { $rsa.Dispose() }
    $password = $null
}
