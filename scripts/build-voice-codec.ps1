param([string]$OutputDirectory)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
Initialize-MsvcEnvironment
$project=Split-Path $PSScriptRoot -Parent
$source=Join-Path $project 'native/silk'
$manifest=Get-Content -LiteralPath (Join-Path $source 'SOURCE.json') -Raw | ConvertFrom-Json
foreach ($entry in $manifest.files) {if ((Get-FileHash -LiteralPath (Join-Path $source $entry.path)).Hash.ToLowerInvariant() -ne $entry.sha256) {throw 'SILK source hash mismatch'}}
if (-not $OutputDirectory) {$OutputDirectory=Join-Path $project ('target/voice-codec/'+[Guid]::NewGuid().ToString('N'))}
$OutputDirectory=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) {throw 'Output must be a new directory'}
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$compiler=(Get-Command cl.exe).Source
$librarian=(Get-Command lib.exe).Source
Push-Location $OutputDirectory
try {
    $files=@(Get-ChildItem -LiteralPath (Join-Path $source 'src') -Filter '*.c' -File | Sort-Object Name)
    # PowerShell 7's utf8 omits the BOM; cl.exe then reads non-ASCII paths in
    # the system code page. Write explicit UTF-8 BOM under both PS5.1 and PS7.
    [IO.File]::WriteAllLines((Join-Path $OutputDirectory 'sources.rsp'), [string[]]@($files | ForEach-Object {'"'+$_.FullName+'"'}), [Text.UTF8Encoding]::new($true))
    & $compiler /nologo /c /TC /O2 /MT /DNDEBUG /W3 /utf-8 "/I$source/interface" "/I$source/src" '@sources.rsp'
    if ($LASTEXITCODE -ne 0) {throw 'SILK C source compilation failed'}
    (Get-ChildItem -Filter '*.obj' -File | ForEach-Object {$_.Name}) | Set-Content -LiteralPath objects.rsp -Encoding ascii
    & $librarian /nologo /out:silk.lib '@objects.rsp'
    if ($LASTEXITCODE -ne 0) {throw 'SILK static archive failed'}
    & $compiler /nologo /std:c++17 /EHsc /W4 /utf-8 /O2 /MT "/I$source/interface" "$project/native/voice-codec/codec.cpp" /Fe:werc-voice-codec.exe /Fo:codec.obj /link /DYNAMICBASE /NXCOMPAT silk.lib
    if ($LASTEXITCODE -ne 0) {throw 'Codec controller build failed'}
    & (Join-Path $OutputDirectory 'werc-voice-codec.exe') --self-test | Tee-Object -FilePath fixture-result.json
    if ($LASTEXITCODE -ne 0) {throw 'SILK roundtrip self-test failed'}
    & node.exe "$project/scripts/voice-codec-smoke.mjs" (Join-Path $OutputDirectory 'werc-voice-codec.exe') | Tee-Object -FilePath ipc-result.json
    if ($LASTEXITCODE -ne 0) {throw 'SILK cross-process validation failed'}
    [ordered]@{schema_version=1;built_at=(Get-Date).ToString('o');source_archive_sha256=$manifest.archive_sha256;source_manifest_sha256=(Get-FileHash -LiteralPath (Join-Path $source 'SOURCE.json')).Hash.ToLowerInvariant();adapter_source_sha256=(Get-FileHash -LiteralPath "$project/native/voice-codec/codec.cpp").Hash.ToLowerInvariant();c_sources=$files.Count;compiler=$compiler;sha256=(Get-FileHash -LiteralPath 'werc-voice-codec.exe').Hash.ToLowerInvariant();real_wechat_accessed=$false;messages_sent=$false;complete_release=$false} | ConvertTo-Json | Set-Content -LiteralPath build-evidence.json -Encoding utf8
    Get-Content -LiteralPath build-evidence.json
} finally {Pop-Location}
