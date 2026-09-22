param([ValidateSet('All', 'Windows', 'Android')][string]$Target = 'All')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    $env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
    $env:DOTNET_CLI_HOME = Join-Path $projectRoot '.tools\dotnet-home'
    $env:NUGET_PACKAGES = Join-Path $projectRoot '.tools\nuget'
    $dotnet = if (Test-Path '.tools\dotnet\dotnet.exe') { (Resolve-Path '.tools\dotnet\dotnet.exe').Path } else { 'dotnet' }
    if ($Target -in @('All', 'Windows')) {
        # Start with a clean distribution so old driver/service installers cannot leak in.
        $windowsOutput = [IO.Path]::GetFullPath((Join-Path $projectRoot 'artifacts\windows'))
        $artifactsRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'artifacts')) + [IO.Path]::DirectorySeparatorChar
        if (!$windowsOutput.StartsWith($artifactsRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Ungültiges Build-Ausgabeverzeichnis.'
        }
        if (Test-Path -LiteralPath $windowsOutput) { Remove-Item -LiteralPath $windowsOutput -Recurse -Force }
        & $dotnet publish 'desktop\ToasterMic.Windows' -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:DebugType=None -o 'artifacts\windows'
        if ($LASTEXITCODE -ne 0) { throw 'Windows-Build fehlgeschlagen.' }
        New-Item -ItemType Directory -Force 'artifacts\windows\licenses' | Out-Null
        Copy-Item 'licenses\*.txt' 'artifacts\windows\licenses' -Force
        New-Item -ItemType Directory -Force 'artifacts\windows\docs' | Out-Null
        Copy-Item 'docs\PROTOCOL.md','docs\DEVICE-TEST.md' 'artifacts\windows\docs' -Force
        Copy-Item -LiteralPath 'README.md' -Destination 'artifacts\windows\README.md' -Force
        $archive = Join-Path $projectRoot 'artifacts\ToasterMic-Windows-x64.zip'
        if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
        Compress-Archive -Path 'artifacts\windows\*' -DestinationPath $archive -CompressionLevel Optimal
        Copy-Item -LiteralPath $archive -Destination (Join-Path $projectRoot 'ToasterMic-Windows-x64.zip') -Force
    }
    if ($Target -in @('All', 'Android')) {
        if (Test-Path '.tools\jdk') { $env:JAVA_HOME = (Get-ChildItem '.tools\jdk' -Directory | Select-Object -First 1).FullName }
        if (Test-Path '.tools\android-sdk') { $env:ANDROID_HOME = (Resolve-Path '.tools\android-sdk').Path }
        $env:GRADLE_USER_HOME = Join-Path $projectRoot '.tools\gradle-home'
        $env:ANDROID_USER_HOME = Join-Path $projectRoot '.tools\android-user'
        $gradle = if (Test-Path '.tools\gradle-8.11.1\bin\gradle.bat') { (Resolve-Path '.tools\gradle-8.11.1\bin\gradle.bat').Path } else { (Resolve-Path 'android\gradlew.bat').Path }
        Push-Location android
        try { & $gradle --no-daemon assembleDebug lintDebug; if ($LASTEXITCODE -ne 0) { throw 'Android-Build oder Lint fehlgeschlagen.' } }
        finally { Pop-Location }
        New-Item -ItemType Directory -Force 'artifacts\android' | Out-Null
        Copy-Item -LiteralPath 'android\app\build\outputs\apk\debug\app-debug.apk' -Destination 'artifacts\android\ToasterMic-debug.apk' -Force
    }
    Write-Host 'Build erfolgreich. Ergebnisse liegen in artifacts/.'
} finally { Pop-Location }
