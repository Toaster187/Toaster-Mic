$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
    $env:DOTNET_CLI_HOME = Join-Path $projectRoot '.tools\dotnet-home'
    $env:NUGET_PACKAGES = Join-Path $projectRoot '.tools\nuget'
    $dotnet = if (Test-Path '.tools\dotnet\dotnet.exe') { (Resolve-Path '.tools\dotnet\dotnet.exe').Path } else { 'dotnet' }
    if (Test-Path '.tools\jdk') { $env:JAVA_HOME = (Get-ChildItem '.tools\jdk' -Directory | Select-Object -First 1).FullName }
    $javac = if ($env:JAVA_HOME) { Join-Path $env:JAVA_HOME 'bin\javac.exe' } else { 'javac' }
    $java = if ($env:JAVA_HOME) { Join-Path $env:JAVA_HOME 'bin\java.exe' } else { 'java' }
    New-Item -ItemType Directory -Force 'artifacts\tests' | Out-Null
    & $javac -d 'artifacts\tests' 'android\app\src\main\java\de\toaster\mic\ChannelCalibration.java' 'tests\ChannelCalibrationTests.java'
    if ($LASTEXITCODE -ne 0) { throw 'Kalibrierungstest konnte nicht gebaut werden.' }
    & $java -cp 'artifacts\tests' ChannelCalibrationTests
    if ($LASTEXITCODE -ne 0) { throw 'Kalibrierungstest fehlgeschlagen.' }
    & $javac -d 'artifacts\tests' 'android\app\src\main\java\de\toaster\mic\Wire.java' 'android\app\src\main\java\de\toaster\mic\KeyExchange.java' 'tests\WireInterop.java'
    if ($LASTEXITCODE -ne 0) { throw 'Java-Protokolltest konnte nicht gebaut werden.' }
    & $java -cp 'artifacts\tests' WireInterop 'artifacts\tests\wire-vectors.txt'
    if ($LASTEXITCODE -ne 0) { throw 'Java-Protokolltest fehlgeschlagen.' }
    & $dotnet run --project 'tests\ToasterMic.Tests' -c Release -- 'artifacts\tests\wire-vectors.txt'
    if ($LASTEXITCODE -ne 0) { throw 'Protokoll-/Audiotests fehlgeschlagen.' }
    & $dotnet run --project 'tests\ToasterMic.Windows.Tests' -c Release
    if ($LASTEXITCODE -ne 0) { throw 'Windows-Lebenszyklus-/Oberflächentests fehlgeschlagen.' }
} finally { Pop-Location }
