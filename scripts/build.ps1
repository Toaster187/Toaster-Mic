param(
    [ValidateSet('All', 'Windows', 'Android')][string]$Target = 'All',
    # Also build the x86_64 libraries for the Android emulator.
    [switch]$Emulator,
    # Build number (CI run number). Raises the Android versionCode so updates install over older builds.
    [int]$BuildNumber = 0,
    # Also package the Windows installer (Inno Setup, iscc.exe).
    [switch]$Installer
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'env.ps1')
Push-Location $projectRoot
try {
    $artifacts = Join-Path $projectRoot 'artifacts'
    New-Item -ItemType Directory -Force $artifacts | Out-Null
    $baseVersion = ((Select-String -Path 'Cargo.toml' -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value)
    $fullVersion = if ($BuildNumber -gt 0) { "$baseVersion.$BuildNumber" } else { $baseVersion }
    $versionCode = if ($BuildNumber -gt 0) { 1000 + $BuildNumber } else { 6 }
    Write-Host "Version $fullVersion (Android versionCode $versionCode)"

    if ($Target -in @('All', 'Windows')) {
        & cargo build --release -p toaster-windows -p toaster-probe
        if ($LASTEXITCODE -ne 0) { throw 'Windows-Build fehlgeschlagen.' }
        $out = Join-Path $artifacts 'windows'
        if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Recurse -Force }
        New-Item -ItemType Directory -Force $out, (Join-Path $out 'docs'), (Join-Path $out 'licenses') | Out-Null
        Copy-Item 'target\release\toaster-mic.exe' (Join-Path $out 'ToasterMic.exe')
        Copy-Item 'target\release\toaster-probe.exe' (Join-Path $out 'toaster-probe.exe')
        Copy-Item 'README.md' $out
        Copy-Item 'docs\PROTOCOL.md', 'docs\DEVICE-TEST.md' (Join-Path $out 'docs')
        Copy-Item 'licenses\*' (Join-Path $out 'licenses')
        $zip = Join-Path $artifacts 'ToasterMic-Windows-x64.zip'
        if (Test-Path $zip) { Remove-Item $zip -Force }
        Compress-Archive -Path "$out\*" -DestinationPath $zip -CompressionLevel Optimal
        if ($Installer) {
            $iscc = (Get-Command iscc.exe -ErrorAction SilentlyContinue).Source
            if (!$iscc) { $iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1 }
            if (!$iscc) { throw 'Inno Setup (ISCC.exe) nicht gefunden.' }
            & $iscc /Q "/DAppVersion=$fullVersion" "/DSourceDir=$out" "/DOutputDir=$artifacts" (Join-Path $projectRoot 'installer\ToasterMic.iss')
            if ($LASTEXITCODE -ne 0) { throw 'Installer-Build fehlgeschlagen.' }
            Write-Host "Installer: $(Join-Path $artifacts 'ToasterMic-Setup.exe')"
        }
    }

    if ($Target -in @('All', 'Android')) {
        $abis = @{ 'aarch64-linux-android' = 'arm64-v8a' }
        if ($Emulator) { $abis['x86_64-linux-android'] = 'x86_64' }
        foreach ($triple in $abis.Keys) {
            & cargo build --release -p toaster-android-engine -p toaster-android-ui --target $triple
            if ($LASTEXITCODE -ne 0) { throw "Android-Build ($triple) fehlgeschlagen." }
        }
        $buildTools = (Get-ChildItem (Join-Path $env:TOASTER_SDK 'build-tools') -Directory | Sort-Object { [version]($_.Name -replace '[^0-9.]', '') } -Descending | Select-Object -First 1).FullName
        $platform = (Get-ChildItem (Join-Path $env:TOASTER_SDK 'platforms') -Directory | Sort-Object Name -Descending | Select-Object -First 1).FullName
        $androidJar = Join-Path $platform 'android.jar'
        $app = Join-Path $projectRoot 'apps\android'
        $obj = Join-Path $projectRoot 'target\apk'
        if (Test-Path $obj) { Remove-Item $obj -Recurse -Force }
        New-Item -ItemType Directory -Force "$obj\classes", "$obj\dex" | Out-Null

        # Java shell (Android-mandated classes only) -> dex
        $sources = Get-ChildItem (Join-Path $app 'java') -Recurse -Filter *.java | ForEach-Object FullName
        & (Join-Path $env:JAVA_HOME 'bin\javac.exe') --release 11 -nowarn -classpath $androidJar -d "$obj\classes" @sources
        if ($LASTEXITCODE -ne 0) { throw 'javac fehlgeschlagen.' }
        $classes = Get-ChildItem "$obj\classes" -Recurse -Filter *.class | ForEach-Object FullName
        & (Join-Path $buildTools 'd8.bat') --release --min-api 28 --lib $androidJar --output "$obj\dex" @classes
        if ($LASTEXITCODE -ne 0) { throw 'd8 fehlgeschlagen.' }

        # Resources and manifest
        & (Join-Path $buildTools 'aapt2.exe') compile --dir (Join-Path $app 'res') -o "$obj\res.zip"
        if ($LASTEXITCODE -ne 0) { throw 'aapt2 compile fehlgeschlagen.' }
        & (Join-Path $buildTools 'aapt2.exe') link -o "$obj\base.apk" -I $androidJar --manifest (Join-Path $app 'AndroidManifest.xml') `
            --min-sdk-version 28 --target-sdk-version 35 --version-code $versionCode --version-name $fullVersion --replace-version "$obj\res.zip"
        if ($LASTEXITCODE -ne 0) { throw 'aapt2 link fehlgeschlagen.' }

        # Add dex and native libraries
        Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
        $zip = [System.IO.Compression.ZipFile]::Open("$obj\base.apk", 'Update')
        try {
            [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, "$obj\dex\classes.dex", 'classes.dex')
            foreach ($triple in $abis.Keys) {
                foreach ($lib in 'libtoaster_engine.so', 'libtoaster_ui.so') {
                    [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, "target\$triple\release\$lib", "lib/$($abis[$triple])/$lib")
                }
            }
        } finally { $zip.Dispose() }

        & (Join-Path $buildTools 'zipalign.exe') -p -f 4 "$obj\base.apk" "$obj\aligned.apk"
        if ($LASTEXITCODE -ne 0) { throw 'zipalign fehlgeschlagen.' }
        # Release key from the environment (CI secret), otherwise the local debug key.
        if ($env:TOASTER_KEYSTORE -and (Test-Path $env:TOASTER_KEYSTORE)) {
            $keystore = $env:TOASTER_KEYSTORE
            $storePass = $env:TOASTER_KEYSTORE_PASSWORD
            $alias = if ($env:TOASTER_KEY_ALIAS) { $env:TOASTER_KEY_ALIAS } else { 'toastermic' }
            Write-Host 'APK wird mit dem Release-Schlüssel signiert.'
        } else {
            $keystore = Join-Path $env:USERPROFILE '.android\debug.keystore'
            $storePass = 'android'
            $alias = 'androiddebugkey'
            if (!(Test-Path $keystore)) {
                New-Item -ItemType Directory -Force (Split-Path $keystore) | Out-Null
                & (Join-Path $env:JAVA_HOME 'bin\keytool.exe') -genkeypair -keystore $keystore -storepass android -keypass android `
                    -alias androiddebugkey -dname 'CN=Android Debug,O=Android,C=US' -keyalg RSA -keysize 2048 -validity 10000
            }
            Write-Warning 'Kein Release-Schlüssel (TOASTER_KEYSTORE): APK wird mit dem lokalen Debug-Schlüssel signiert.'
        }
        $apkDir = Join-Path $artifacts 'android'
        New-Item -ItemType Directory -Force $apkDir | Out-Null
        $apk = Join-Path $apkDir 'ToasterMic.apk'
        & (Join-Path $buildTools 'apksigner.bat') sign --ks $keystore --ks-pass "pass:$storePass" --key-pass "pass:$storePass" `
            --ks-key-alias $alias --out $apk "$obj\aligned.apk"
        if ($LASTEXITCODE -ne 0) { throw 'apksigner fehlgeschlagen.' }
        Write-Host ("APK: {0} ({1:N1} MB)" -f $apk, ((Get-Item $apk).Length / 1MB))
    }
    Write-Host 'Build erfolgreich. Ergebnisse liegen in artifacts/.'
} finally { Pop-Location }
