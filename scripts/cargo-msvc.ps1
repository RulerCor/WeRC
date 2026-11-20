$CargoArguments = $args
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
Initialize-MsvcEnvironment
Set-Location (Split-Path $PSScriptRoot -Parent)
& cargo.exe @CargoArguments
exit $LASTEXITCODE
