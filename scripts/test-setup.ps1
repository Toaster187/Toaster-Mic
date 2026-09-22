$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
. (Join-Path $projectRoot 'setup\SetupValidation.ps1')
function Check([bool]$condition, [string]$name) { if (!$condition) { throw $name }; Write-Host "PASS $name" }
$package = Get-ToasterDriverPackage (Join-Path $projectRoot 'artifacts\windows')
try { Check ($package.IsTest -and $null -ne $package.Certificate) 'development package and matching certificate accepted' }
finally { if ($package.Certificate) { $package.Certificate.Dispose() } }
Assert-ToasterDriverPolicy $false 1
Check $true 'production signing does not require test mode'
Assert-ToasterDriverPolicy $true 3
Check $true 'development signing allowed when kernel test mode is active'
$blocked = $false
try { Assert-ToasterDriverPolicy $true 1 } catch { $blocked = $_.Exception.Data['ExitCode'] -eq 2 }
Check $blocked 'normal Windows blocks development driver with actionable exit code'
$scratch = Join-Path $projectRoot ('artifacts\tests\setup-' + [Guid]::NewGuid().ToString('N'))
$driver = Join-Path $scratch 'driver-testsigned'
New-Item -ItemType Directory -Force $driver | Out-Null
Copy-Item (Join-Path $projectRoot 'artifacts\driver-testsigned\*') -Destination $driver
$inf = Join-Path $driver 'ToasterMic.inf'
Add-Content -LiteralPath $inf -Value '; changed after signing'
$blocked = $false
try { $unexpected = Get-ToasterDriverPackage $scratch; $unexpected.Certificate.Dispose() } catch { $blocked = $_.Exception.Message -match 'Prüfsumme' }
Check $blocked 'modified INF rejected before certificate import'
Copy-Item -LiteralPath (Join-Path $projectRoot 'artifacts\driver-testsigned\ToasterMic.inf') -Destination $inf -Force
$sys = Join-Path $driver 'ToasterMic.sys'
$bytes = [IO.File]::ReadAllBytes($sys); $bytes[1024] = $bytes[1024] -bxor 1; [IO.File]::WriteAllBytes($sys, $bytes)
$blocked = $false
try { $unexpected = Get-ToasterDriverPackage $scratch; $unexpected.Certificate.Dispose() } catch { $blocked = $_.Exception.Message -match 'signiert|Prüfsumme' }
Check $blocked 'modified SYS rejected before certificate import'
New-Item -ItemType Directory -Force (Join-Path $scratch 'driver') | Out-Null
$blocked = $false
try { $null = Get-ToasterDriverPackage $scratch } catch { $blocked = $_.Exception.Message -match 'Treiberdatei fehlt' }
Check $blocked 'broken production package never falls back to test certificate'
$options = [ToasterMic.SetupEnvironment]::CodeIntegrityOptions()
Write-Host ('Read-only host check: CodeIntegrityOptions=0x{0:X}; test mode active={1}' -f $options, (($options -band 2) -ne 0))
