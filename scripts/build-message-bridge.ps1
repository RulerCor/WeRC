param([string]$OutputDirectory, [switch]$UpstreamOnly)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
Initialize-MsvcEnvironment
$project = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'message-bridge-source-check.ps1')
$addressCheck=Test-WeRCMessageBridgeAddresses (Join-Path $project 'native/message-bridge')
$source = Join-Path $project 'native/helper'
$manifest = Get-Content -LiteralPath (Join-Path $source 'SOURCE.json') -Raw | ConvertFrom-Json
foreach ($item in $manifest.files) {
    $path = Join-Path $source $item.path
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $item.sha256) {
        throw ('Upstream source changed: ' + $item.path)
    }
}
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $project ('target/message-bridge/' + [Guid]::NewGuid().ToString('N'))
}
$OutputDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Build output must be a new directory' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$compiler = (Get-Command cl.exe).Source
$librarian = (Get-Command lib.exe).Source
$dumpbin = (Get-Command dumpbin.exe).Source
$native = Join-Path $source 'native'
$minhook = Join-Path $native 'vendor/minhook-1.3.4'
# No downloaded build scripts or prebuilt DLLs are run. The working directory is
# our new output directory, and all executables are resolved from the MSVC SDK.
Push-Location $OutputDirectory
try {
    & $compiler /nologo /c /TC /O2 /MT /DNDEBUG /W3 /utf-8 `
        "/I$minhook/include" "/I$minhook/src" "/I$minhook/src/hde" `
        "$minhook/src/buffer.c" "$minhook/src/hook.c" `
        "$minhook/src/trampoline.c" "$minhook/src/hde/hde64.c"
    if ($LASTEXITCODE -ne 0) { throw 'MinHook compilation failed' }
    & $librarian /nologo /out:minhook.lib buffer.obj hook.obj trampoline.obj hde64.obj
    if ($LASTEXITCODE -ne 0) { throw 'MinHook archive failed' }
    & $compiler /nologo /std:c++17 /EHsc /W4 /utf-8 /LD /MT /O2 /DNDEBUG `
        "$native/text_bridge.cpp" /Fe:helper-text-bridge.dll /Fo:helper-text-bridge.obj `
        /link /MACHINE:X64 /DYNAMICBASE /NXCOMPAT bcrypt.lib advapi32.lib minhook.lib
    if ($LASTEXITCODE -ne 0) { throw 'Native text bridge compilation failed' }
    $outputName = 'helper-text-bridge.dll'
    if (-not $UpstreamOnly) {
        & $compiler /nologo /std:c++17 /EHsc /W4 /wd4505 /utf-8 /LD /MT /O2 /DNDEBUG `
            "$project/native/message-bridge/bridge.cpp" /Fe:werc-message-bridge.dll /Fo:werc-message-bridge.obj `
            /link /MACHINE:X64 /DYNAMICBASE /NXCOMPAT bcrypt.lib advapi32.lib minhook.lib
        if ($LASTEXITCODE -ne 0) { throw 'WeRC message adapter compilation failed' }
        $outputName = 'werc-message-bridge.dll'
        $bridgeHash = (Get-FileHash -LiteralPath $outputName).Hash.ToLowerInvariant()
        ('#define WERC_BRIDGE_SHA256 "' + $bridgeHash + '"') | Set-Content -LiteralPath build_identity.h -Encoding ascii
        & $compiler /nologo /std:c++17 /EHsc /W4 /utf-8 /MT /O2 /DNDEBUG "/FI$OutputDirectory/build_identity.h" `
            "$project/native/message-bridge/host.cpp" /Fe:werc-message-host.exe /Fo:werc-message-host.obj `
            /link /MACHINE:X64 /DYNAMICBASE /NXCOMPAT bcrypt.lib advapi32.lib
        if ($LASTEXITCODE -ne 0) { throw 'Message host compilation failed' }
        & $compiler /nologo /std:c++17 /EHsc /W4 /wd4505 /utf-8 /MT /O2 /DNDEBUG `
            "$project/native/message-bridge/fixture.cpp" /Fe:werc-message-fixture.exe /Fo:werc-message-fixture.obj `
            /link /MACHINE:X64 /DYNAMICBASE /NXCOMPAT bcrypt.lib advapi32.lib minhook.lib
        if ($LASTEXITCODE -ne 0) { throw 'Message fixture compilation failed' }
        & (Join-Path $OutputDirectory 'werc-message-fixture.exe') | Tee-Object -FilePath fixture-result.json
        if ($LASTEXITCODE -ne 0) { throw 'Message fixture failed' }
    }
    & $dumpbin /nologo /exports $outputName | Set-Content -LiteralPath exports.txt
    if ($LASTEXITCODE -ne 0) { throw 'Export inspection failed' }
    $dll = Get-Item -LiteralPath $outputName
    [ordered]@{
        schema_version=1; built_at=[DateTimeOffset]::Now.ToString('o')
        source_zip_sha256=$manifest.zip_sha256
        source_manifest_sha256=(Get-FileHash -LiteralPath (Join-Path $source 'SOURCE.json')).Hash.ToLowerInvariant()
        compiler=$compiler; compiler_version=(Get-Item -LiteralPath $compiler).VersionInfo.FileVersion
        runtime='static MSVC runtime (/MT)'; architecture='x64'
        output=$dll.FullName; bytes=$dll.Length
        sha256=(Get-FileHash -LiteralPath $dll.FullName).Hash.ToLowerInvariant()
        host=if (-not $UpstreamOnly) { @{path=(Join-Path $OutputDirectory 'werc-message-host.exe');sha256=(Get-FileHash -LiteralPath 'werc-message-host.exe').Hash.ToLowerInvariant()} } else { $null }
        adapter_sources=@(Get-ChildItem -LiteralPath "$project/native/message-bridge" -File | ForEach-Object { @{path=$_.Name;sha256=(Get-FileHash -LiteralPath $_.FullName).Hash.ToLowerInvariant()} })
        address_resolution_check=$addressCheck
        source_build_succeeded=$true; injected=$false; real_wechat_verified=$false
        production_provider_integrated=$false; messages_sent=$false; complete_release=$false
    } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath build-evidence.json -Encoding utf8
    Get-Content -LiteralPath build-evidence.json
} finally { Pop-Location }
