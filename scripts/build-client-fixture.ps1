$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
Initialize-MsvcEnvironment
$project = Split-Path $PSScriptRoot -Parent
$source = Join-Path $project 'tests/fixtures/client'
$output = Join-Path $project 'target/fixtures/client'
New-Item -ItemType Directory -Force -Path $output | Out-Null
& rc.exe /nologo /fo (Join-Path $output 'client.res') (Join-Path $source 'client.rc')
if ($LASTEXITCODE -ne 0) { throw 'Synthetic resource compilation failed' }
$object = Join-Path $output 'client.obj'
$executable = Join-Path $output 'Weixin.exe'
& cl.exe /nologo /W4 (Join-Path $source 'client.c') (Join-Path $output 'client.res') "/Fo$object" "/Fe$executable" /link /SUBSYSTEM:WINDOWS
if ($LASTEXITCODE -ne 0) { throw 'Synthetic executable compilation failed' }
Write-Output ('Synthetic version fixture built; do not execute: ' + (Join-Path $output 'Weixin.exe'))
