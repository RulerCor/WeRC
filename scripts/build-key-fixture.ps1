$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
Initialize-MsvcEnvironment
$project = Split-Path $PSScriptRoot -Parent
$source = Join-Path $project 'tests/fixtures/key-client'
$output = Join-Path $project 'target/fixtures/key-client'
New-Item -ItemType Directory -Force -Path $output | Out-Null
& rc.exe /nologo /fo (Join-Path $output 'client.res') (Join-Path $source 'client.rc')
if ($LASTEXITCODE -ne 0) { throw 'Owned memory fixture resource compilation failed' }
$object = Join-Path $output 'client.obj'
$executable = Join-Path $output 'Weixin.exe'
& cl.exe /nologo /W4 /WX /utf-8 (Join-Path $source 'client.c') (Join-Path $output 'client.res') "/Fo$object" "/Fe$executable" /link /SUBSYSTEM:CONSOLE
if ($LASTEXITCODE -ne 0) { throw 'Owned memory fixture compilation failed' }
$sources = @('tests/fixtures/key-client/client.c', 'tests/fixtures/key-client/client.rc') | ForEach-Object {
    [ordered]@{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $project $_) -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$evidence = [ordered]@{
    format = 1
    synthetic_only = $true
    executable_sha256 = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
    sources = @($sources)
}
$json = $evidence | ConvertTo-Json -Depth 4
[System.IO.File]::WriteAllText((Join-Path $output 'build-evidence.json'), $json, (New-Object System.Text.UTF8Encoding($false)))
Write-Output 'Built an owned synthetic memory target; not a real WeChat executable.'
