$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    $vc = 'C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231'
    if ($env:VCToolsInstallDir) { $vc = $env:VCToolsInstallDir.TrimEnd('\') }
    $sdk = 'C:\Program Files (x86)\Windows Kits\10'
    $wdk = Join-Path $root '.tools\wdk\c'
    if (!(Test-Path "$wdk\Include\10.0.26100.0\km\ntddk.h")) { throw 'WDK 10.0.26100.6584 NuGet package must be extracted to .tools/wdk.' }
    $source = Join-Path $root 'driver\ToasterMic.Driver\Source'
    $out = Join-Path $root 'artifacts\driver-unsigned'
    $obj = Join-Path $root 'artifacts\driver-obj'
    $outRel = 'artifacts\driver-unsigned'
    $objRel = 'artifacts\driver-obj'
    New-Item -ItemType Directory -Force $out,$obj | Out-Null
    $includes = @("$wdk\Include\10.0.26100.0\km", "$wdk\Include\10.0.26100.0\km\crt", "$wdk\Include\wdf\kmdf\1.15", "$sdk\Include\10.0.26100.0\shared", "$sdk\Include\10.0.26100.0\ucrt", "$vc\include", "$source\Inc", "$source\Main", "$source\Filters", "$source\Utilities")
    $sourcePaths = Get-ChildItem "$source\Main","$source\Filters","$source\Utilities" -Filter '*.cpp' | ForEach-Object FullName
    $compileRsp = '.tools\driver-compile.rsp'
    $compileLines = @('/nologo','/c','/kernel','/std:c++17','/O2','/GS','/Zl','/GR-','/EHs-c-','/W3','/D_AMD64_','/DAMD64','/D_WIN64','/D_WIN32','/DUNICODE','/D_UNICODE','/DPC_IMPLEMENTATION','/DNTDDI_VERSION=0x0A00000B','/D_WIN32_WINNT=0x0A00','/DWINVER=0x0A00','/DDEPRECATE_DDK_FUNCTIONS=1','/DPOOL_NX_OPTIN=1','/D_NEW_DELETE_OPERATORS_','/Foartifacts\driver-obj\')
    $compileLines += $includes | ForEach-Object { "/I`"$_`"" }
    $compileLines += $sourcePaths | ForEach-Object { "`"$_`"" }
    Set-Content -LiteralPath $compileRsp -Value $compileLines -Encoding ASCII
    & "$vc\bin\Hostx64\x64\cl.exe" "@$compileRsp"
    if ($LASTEXITCODE -ne 0) { throw 'Driver compilation failed.' }
    $linkRsp = '.tools\driver-link.rsp'
    $linkLines = @('/nologo','/DRIVER','/SUBSYSTEM:NATIVE,10.00','/ENTRY:FxDriverEntry','/NODEFAULTLIB','/INCREMENTAL:NO','/DYNAMICBASE','/NXCOMPAT','/MACHINE:X64','/OUT:artifacts\driver-unsigned\ToasterMic.sys',"/LIBPATH:`"$wdk\Lib\10.0.26100.0\km\x64`"","/LIBPATH:`"$wdk\Lib\wdf\kmdf\x64\1.15`"")
    $linkLines += $sourcePaths | ForEach-Object { Join-Path $objRel ([IO.Path]::GetFileNameWithoutExtension($_) + '.obj') }
    $linkLines += @('ntoskrnl.lib','hal.lib','wmilib.lib','portcls.lib','stdunk.lib','libcntpr.lib','wdmsec.lib','BufferOverflowFastFailK.lib','WdfDriverEntry.lib','WdfLdr.lib')
    Set-Content -LiteralPath $linkRsp -Value $linkLines -Encoding ASCII
    & "$vc\bin\Hostx64\x64\link.exe" "@$linkRsp"
    if ($LASTEXITCODE -ne 0) { throw 'Driver link failed.' }
    Copy-Item 'driver\ToasterMic.Driver\ToasterMic.inf' "$out\ToasterMic.inf" -Force
    & "$wdk\bin\10.0.26100.0\x86\Inf2Cat.exe" "/driver:$outRel" /os:10_GE_X64 /uselocaltime
    if ($LASTEXITCODE -ne 0) { throw 'Driver catalog validation failed.' }
    $env:INCLUDE = "$vc\include;$sdk\Include\10.0.26100.0\ucrt;$sdk\Include\10.0.26100.0\shared;$sdk\Include\10.0.26100.0\um"
    $env:LIB = "$vc\lib\x64;$sdk\Lib\10.0.26100.0\ucrt\x64;$sdk\Lib\10.0.26100.0\um\x64"
    $testRsp = '.tools\driver-test.rsp'
    Set-Content -LiteralPath $testRsp -Value @('/nologo','/EHsc','/std:c++17','"tests\DriverRingTests.cpp"','/Feartifacts\driver-obj\DriverRingTests.exe','/Foartifacts\driver-obj\DriverRingTests.obj') -Encoding ASCII
    & "$vc\bin\Hostx64\x64\cl.exe" "@$testRsp"
    if ($LASTEXITCODE -ne 0) { throw 'Driver ring test build failed.' }
    & "$obj\DriverRingTests.exe"
    if ($LASTEXITCODE -ne 0) { throw 'Driver ring test failed.' }
    & (Join-Path $PSScriptRoot 'sign-driver.ps1')
    & (Join-Path $PSScriptRoot 'test-signing.ps1')
    Write-Host 'Driver built and development-signed. Microsoft signing and hardware tests remain required for production.'
} finally { Pop-Location }
