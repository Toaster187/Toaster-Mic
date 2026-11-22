# Build environment for Toaster Mic (dot-source: . ./scripts/env.ps1).
# Uses rustup's x86_64-pc-windows-gnu toolchain and the Android NDK's LLVM
# (clang, lld, llvm-dlltool). No Visual Studio, no MinGW installation needed.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (Test-Path $cargoBin) { $env:Path = "$cargoBin;$env:Path" }
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
$ndk = if ($env:ANDROID_NDK_HOME) { $env:ANDROID_NDK_HOME } else { (Get-ChildItem (Join-Path $sdk 'ndk') -Directory | Sort-Object Name -Descending | Select-Object -First 1).FullName }
if (!$ndk -or !(Test-Path $ndk)) { throw 'Android NDK nicht gefunden (ANDROID_NDK_HOME setzen).' }
$llvm = Join-Path $ndk 'toolchains\llvm\prebuilt\windows-x86_64\bin'
$sysroot = (& rustc --print sysroot).Trim()
$selfContained = Join-Path $sysroot 'lib\rustlib\x86_64-pc-windows-gnu\lib\self-contained'
$tools = Join-Path $root '.tools\bin'
New-Item -ItemType Directory -Force $tools | Out-Null
$dlltool = Join-Path $tools 'dlltool.exe'
if (!(Test-Path $dlltool)) { Copy-Item (Join-Path $llvm 'llvm-dlltool.exe') $dlltool }
$hostlink = Join-Path $tools 'hostlink.exe'
$source = Join-Path $PSScriptRoot 'tools\hostlink.rs'
if (!(Test-Path $hostlink) -or (Get-Item $source).LastWriteTime -gt (Get-Item $hostlink).LastWriteTime) {
    $env:HOSTLINK_CLANG = Join-Path $llvm 'clang.exe'
    $env:HOSTLINK_LIBS = $selfContained
    & rustc -O --edition 2021 $source -o $hostlink
    if ($LASTEXITCODE -ne 0) { throw 'hostlink konnte nicht gebaut werden.' }
}
$env:Path = "$tools;$env:Path"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $hostlink
$clang = Join-Path $llvm 'clang.exe'
foreach ($t in @('aarch64-linux-android', 'x86_64-linux-android')) {
    $upper = $t.ToUpper().Replace('-', '_')
    Set-Item "env:CARGO_TARGET_${upper}_LINKER" $clang
    Set-Item "env:CARGO_TARGET_${upper}_RUSTFLAGS" "-Clink-arg=--target=${t}28"
    $u = $t.Replace('-', '_')
    Set-Item "env:CC_$u" $clang
    Set-Item "env:CFLAGS_$u" "--target=${t}28"
    Set-Item "env:AR_$u" (Join-Path $llvm 'llvm-ar.exe')
}
if (!$env:JAVA_HOME) {
    $jbr = 'C:\Program Files\Android\Android Studio\jbr'
    if (Test-Path $jbr) { $env:JAVA_HOME = $jbr } else { throw 'JDK nicht gefunden (JAVA_HOME setzen).' }
}
$env:Path = "$(Join-Path $env:JAVA_HOME 'bin');$env:Path"
$env:ANDROID_HOME = $sdk
$env:TOASTER_NDK = $ndk
$env:TOASTER_SDK = $sdk
