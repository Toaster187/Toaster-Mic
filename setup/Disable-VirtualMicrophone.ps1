#Requires -RunAsAdministrator
$ErrorActionPreference = 'Stop'
$exe = Join-Path $env:ProgramFiles 'ToasterMicBridge\ToasterMic.Service.exe'
if (Get-Service ToasterMicBridge -ErrorAction SilentlyContinue) { Stop-Service ToasterMicBridge }
if (Test-Path -LiteralPath $exe) {
    & $exe --disable-device
    if ($LASTEXITCODE -ne 0) { throw 'Windows hat die Geräte-Deaktivierung nicht bestätigt.' }
}
Write-Output 'Toaster-Mic-Dienst angehalten; eigenes Mikrofon deaktiviert.'
