. (Join-Path $PSScriptRoot 'DriverSignature.ps1')
if (-not ('ToasterMic.SetupEnvironment' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace ToasterMic {
    public static class SetupEnvironment {
        [StructLayout(LayoutKind.Sequential)] private struct Integrity { public uint Length, Options; }
        [DllImport("ntdll.dll")] private static extern int NtQuerySystemInformation(int kind, ref Integrity info, int length, out int returned);
        public static uint CodeIntegrityOptions() {
            var info = new Integrity { Length=8 }; int returned;
            int status = NtQuerySystemInformation(103, ref info, 8, out returned);
            if (status < 0) throw new InvalidOperationException("Windows-Code-Integrity-Status konnte nicht gelesen werden: " + status);
            return info.Options;
        }
    }
}
'@
}

function Get-ToasterDriverPackage([string]$PackageRoot) {
    $driver = Join-Path $PackageRoot 'driver'
    $isTest = !(Test-Path -LiteralPath $driver -PathType Container)
    if ($isTest) { $driver = Join-Path $PackageRoot 'driver-testsigned' }
    foreach ($name in @('ToasterMic.inf','ToasterMic.sys','ToasterMic.cat')) {
        if (!(Test-Path -LiteralPath (Join-Path $driver $name) -PathType Leaf)) { throw "Treiberdatei fehlt: $name. Bitte das vollständige Windows-Paket entpacken." }
    }
    $catalog = Get-AuthenticodeSignature -LiteralPath (Join-Path $driver 'ToasterMic.cat')
    if (!$isTest) {
        if ($catalog.Status -ne 'Valid' -or !$catalog.SignerCertificate -or $catalog.SignerCertificate.Subject -notmatch 'CN=Microsoft Windows Hardware Compatibility Publisher(?:,|$)') {
            throw 'Der Ordner driver enthält kein gültiges Microsoft-signiertes Paket. Es wird nicht auf den Testtreiber zurückgefallen.'
        }
        return [pscustomobject]@{ Directory=$driver; IsTest=$false; Certificate=$null }
    }
    $certPath = Join-Path $driver 'ToasterMic-Development.cer'
    $manifest = Get-Content -Raw -LiteralPath (Join-Path $driver 'signing.json') | ConvertFrom-Json
    $cert = [Security.Cryptography.X509Certificates.X509Certificate2]::new($certPath)
    try {
        if ($cert.Subject -ne 'CN=Toaster Mic Development Only' -or $cert.Thumbprint -ne $manifest.certificateThumbprint -or $cert.HasPrivateKey) {
            throw 'Das Entwicklungszertifikat passt nicht zum Treiberpaket.'
        }
        foreach ($name in @('ToasterMic.sys','ToasterMic.cat')) {
            $path = Join-Path $driver $name
            $sig = Get-AuthenticodeSignature -LiteralPath $path
            $result = [ToasterMic.BuildSignature]::Verify($path)
            if (!$sig.SignerCertificate -or $sig.SignerCertificate.Thumbprint -ne $cert.Thumbprint -or $result -notin @(0,[uint32]2148204809)) {
                throw "Treiberdatei beschädigt oder falsch signiert: $name"
            }
        }
        foreach ($entry in @(@('ToasterMic.sys','sysSha256'),@('ToasterMic.cat','catalogSha256'),@('ToasterMic.inf','infSha256'))) {
            if ((Get-FileHash -LiteralPath (Join-Path $driver $entry[0]) -Algorithm SHA256).Hash -ne $manifest.($entry[1])) {
                throw ('Prüfsumme stimmt nicht: ' + $entry[0])
            }
        }
        return [pscustomobject]@{ Directory=$driver; IsTest=$true; Certificate=$cert }
    } catch { $cert.Dispose(); throw }
}

function Assert-ToasterDriverPolicy([bool]$IsTest, [uint32]$IntegrityOptions) {
    if ($IsTest -and ($IntegrityOptions -band 2) -eq 0) {
        $policyFailure = [InvalidOperationException]::new('Windows erlaubt aktuell keine Testtreiber. Das beiliegende Paket ist entwicklungssigniert. Benötigt wird entweder ein Microsoft-signiertes Paket oder ein bereits aktivierter Windows-Testmodus mit anschließendem Neustart. Secure Boot kann den Testmodus verhindern. Es wurden keine Treiber oder Zertifikate installiert. Anleitung: docs/DRIVER.md im entpackten Paket.')
        $policyFailure.Data['ExitCode'] = 2
        throw $policyFailure
    }
}
