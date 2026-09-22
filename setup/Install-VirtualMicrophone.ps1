[CmdletBinding()]
param([switch]$ValidateOnly, [switch]$NoDialog)
$ErrorActionPreference = 'Stop'
function Fail([string]$text) { throw $text }
$addedCertificates = @()
$selected = $null
$changedSystem = $false
$exe = $null
try {
    . (Join-Path $PSScriptRoot 'SetupValidation.ps1')
    $package = Split-Path $PSScriptRoot -Parent
    $serviceSource = Join-Path $package 'service\ToasterMic.Service.exe'
    if (!(Test-Path -LiteralPath $serviceSource -PathType Leaf)) { Fail 'ToasterMic.Service.exe fehlt im Paket.' }
    if ([Environment]::OSVersion.Version.Build -lt 19041) { Fail 'Der integrierte Treiber benötigt Windows 10 Version 2004 oder neuer (x64).' }
    if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() -ne 'X64') { Fail 'Dieses Treiberpaket benötigt Windows auf einem x64-Prozessor.' }
    $selected = Get-ToasterDriverPackage $package
    $driverSource = $selected.Directory
    try { Assert-ToasterDriverPolicy $selected.IsTest ([ToasterMic.SetupEnvironment]::CodeIntegrityOptions()) }
    catch {
        if ($_.Exception.Data['ExitCode'] -ne 2 -or $ValidateOnly -or $NoDialog) { throw }
        Add-Type -AssemblyName PresentationFramework
        $choice = [Windows.MessageBox]::Show(
            "Der enthaltene Treiber ist entwicklungssigniert. Windows-Testmodus ist noch nicht aktiv.`n`nSoll der Testmodus für diesen Windows-Starteintrag eingeschaltet werden? Er erlaubt systemweit auch Testtreiber und benötigt einen Neustart. Secure Boot kann diese Einstellung blockieren und wird nicht verändert.`n`nNach dem Neustart die Einrichtung erneut starten. Jetzt werden noch keine Treiber oder Zertifikate installiert.",
            'Toaster Mic - Testmodus vorbereiten?', [Windows.MessageBoxButton]::YesNo, [Windows.MessageBoxImage]::Warning, [Windows.MessageBoxResult]::No)
        if ($choice -ne [Windows.MessageBoxResult]::Yes) { throw }
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
        if (!(New-Object Security.Principal.WindowsPrincipal($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { Fail 'Zum Vorbereiten des Testmodus muss die Einrichtung als Administrator gestartet werden.' }
        $bootOutput = & "$env:SystemRoot\System32\bcdedit.exe" /set '{current}' testsigning on 2>&1
        if ($LASTEXITCODE -ne 0) { Fail ('Windows konnte den Testmodus nicht aktivieren. Bei aktivem Secure Boot ist diese Änderung gesperrt; alternativ wird ein Microsoft-signierter Treiber benötigt. Windows-Meldung: ' + ($bootOutput | Out-String)) }
        [Windows.MessageBox]::Show('Testmodus vorbereitet. Bitte Windows selbst neu starten und anschließend in Toaster Mic erneut auf Installieren klicken. Es wurden noch keine Treiber oder Zertifikate installiert.', 'Toaster Mic - Neustart erforderlich') | Out-Null
        exit 3010
    }
    if ($ValidateOnly) { Write-Output 'Signatur, Paket und Windows-Voraussetzungen geprüft. Keine Änderungen durchgeführt.'; exit 0 }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    if (!(New-Object Security.Principal.WindowsPrincipal($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { Fail 'Bitte die Einrichtung als Administrator starten.' }
    $target = Join-Path $env:ProgramFiles 'ToasterMicBridge'
    if ((Test-Path -LiteralPath $target) -and ((Get-Item -LiteralPath $target).Attributes -band [IO.FileAttributes]::ReparsePoint)) { Fail 'Unerwartete Verknüpfung im Installationspfad.' }
    $exe = Join-Path $target 'ToasterMic.Service.exe'
    $existing = Get-CimInstance Win32_Service -Filter "Name='ToasterMicBridge'"
    if ($existing -and $existing.PathName.Trim('"') -ne $exe) { Fail 'Ein fremder Dienst belegt den Namen ToasterMicBridge.' }
    foreach ($name in @('ToasterMic.Service.exe','ToasterMic.inf','ToasterMic.sys','ToasterMic.cat')) {
        $path = Join-Path $target $name
        if ((Test-Path -LiteralPath $path) -and ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint)) { Fail 'Unerwartete Verknüpfung im Installationsziel.' }
    }
    if ($selected.IsTest) {
        foreach ($storeName in @('Root','TrustedPublisher')) {
            $store = [Security.Cryptography.X509Certificates.X509Store]::new($storeName, 'LocalMachine')
            try {
                $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
                $found = $store.Certificates.Find([Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint, $selected.Certificate.Thumbprint, $false)
                if ($found.Count -eq 0) { $store.Add($selected.Certificate); $addedCertificates += $storeName }
            } finally { $store.Close() }
        }
    }
    if ($existing) { Stop-Service ToasterMicBridge -ErrorAction Stop }
    $changedSystem = $true
    New-Item -ItemType Directory -Force $target | Out-Null
    Copy-Item -LiteralPath $serviceSource -Destination $exe -Force
    foreach ($name in @('ToasterMic.inf','ToasterMic.sys','ToasterMic.cat')) { Copy-Item -LiteralPath (Join-Path $driverSource $name) -Destination (Join-Path $target $name) -Force }
    # Windows independently verifies package/catalog membership during driver installation.
    $installOutput = & $exe --install-driver (Join-Path $target 'ToasterMic.inf') 2>&1
    if ($LASTEXITCODE -ne 0) { Fail ('Windows konnte den Treiber nicht einrichten oder laden: ' + ($installOutput | Out-String)) }
    if (!$existing) {
        & sc.exe create ToasterMicBridge binPath= ('"' + $exe + '"') start= auto obj= LocalSystem DisplayName= 'Toaster Mic - Mikrofonsteuerung'
        if ($LASTEXITCODE -ne 0) { Fail 'Windows-Dienst konnte nicht angelegt werden.' }
    }
    & sc.exe failure ToasterMicBridge reset= 86400 actions= restart/1000/restart/1000/restart/5000
    if ($LASTEXITCODE -ne 0) { Fail 'Dienst-Wiederherstellung konnte nicht eingerichtet werden.' }
    & sc.exe failureflag ToasterMicBridge 1
    if ($LASTEXITCODE -ne 0) { Fail 'Dienst-Wiederherstellung konnte nicht aktiviert werden.' }
    if (![Diagnostics.EventLog]::SourceExists('ToasterMicBridge')) { New-EventLog -LogName Application -Source ToasterMicBridge }
    Start-Service ToasterMicBridge
    (Get-Service ToasterMicBridge).WaitForStatus('Running', [TimeSpan]::FromSeconds(10))
    $verifyOutput = & $exe --verify-installation 2>&1
    if ($LASTEXITCODE -ne 0) { Fail ('Abschlussprüfung fehlgeschlagen: ' + ($verifyOutput | Out-String)) }
    $message = 'Toaster Mic ist eingerichtet. Das Windows-Mikrofon bleibt bis zur Handy-Verbindung deaktiviert.'
    if (!$NoDialog) { Add-Type -AssemblyName PresentationFramework; [Windows.MessageBox]::Show($message, 'Toaster Mic') | Out-Null }
    Write-Output $message
} catch {
    $failure = $_.Exception
    $message = $failure.Message
    if ($changedSystem -and $exe -and (Test-Path -LiteralPath $exe)) {
        try {
            Stop-Service ToasterMicBridge -ErrorAction SilentlyContinue
            & $exe --disable-device 2>&1 | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'Windows hat die Deaktivierung nicht bestätigt.' }
        } catch { $message += "`nDas Gerät konnte nach dem Fehler nicht deaktiviert werden: " + $_.Exception.Message }
    }
    foreach ($storeName in $addedCertificates) {
        $store = [Security.Cryptography.X509Certificates.X509Store]::new($storeName, 'LocalMachine')
        try { $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite); $store.Remove($selected.Certificate) }
        catch { $message += "`nDas neue Entwicklungszertifikat konnte nicht zurückgenommen werden: " + $_.Exception.Message }
        finally { $store.Close() }
    }
    if (!$NoDialog) { Add-Type -AssemblyName PresentationFramework; [Windows.MessageBox]::Show($message, 'Toaster Mic - Einrichtung nicht abgeschlossen') | Out-Null }
    Write-Error $message -ErrorAction Continue
    if ($failure.Data['ExitCode']) { exit [int]$failure.Data['ExitCode'] }
    exit 1
} finally { if ($selected -and $selected.Certificate) { $selected.Certificate.Dispose() } }
