param([switch]$BundleOnly)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
$project=Split-Path $PSScriptRoot -Parent
$previous=@{}
foreach($name in @('TEMP','TMP','CARGO_TARGET_DIR','TAURI_CONFIG','WERC_WORKER_SHA256','WERC_SQLCIPHER_WORKER_SHA256','WERC_MESSAGE_HOST_SHA256','WERC_VOICE_CODEC_SHA256','WERC_EMOTICON_CODEC_SHA256','CARGO_BUILD_JOBS')) {
    $previous[$name]=[Environment]::GetEnvironmentVariable($name,'Process')
}
function Sha([string]$Path){(Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()}
function RequireHash([string]$Path,[string]$Hash){if(-not $Hash -or (Sha $Path) -ne $Hash.ToLowerInvariant()){throw ('Component or source changed: '+$Path)}}
try {
    Set-Location -LiteralPath $project
    Initialize-MsvcEnvironment
    $env:CARGO_BUILD_JOBS='2'
    $env:CARGO_TARGET_DIR=Join-Path $project 'target'
    $env:TEMP=Join-Path $project 'target/tmp'
    $env:TMP=$env:TEMP
    New-Item -ItemType Directory -Force $env:TEMP | Out-Null
    Remove-Item Env:TAURI_CONFIG -ErrorAction SilentlyContinue
    if(-not $BundleOnly){
        $destination=Join-Path $project 'target/release'
        $bridgeRoot=Join-Path $project 'target/components/message-bridge'
        $bridge=Get-Content (Join-Path $bridgeRoot 'build-evidence.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        foreach($source in $bridge.adapter_sources){RequireHash (Join-Path $project ('native/message-bridge/'+$source.path)) $source.sha256}
        RequireHash (Join-Path $bridgeRoot 'werc-message-host.exe') $bridge.host.sha256
        RequireHash (Join-Path $bridgeRoot 'werc-message-bridge.dll') $bridge.sha256
        foreach($name in @('werc-message-host.exe','werc-message-bridge.dll')){Copy-Item -LiteralPath (Join-Path $bridgeRoot $name) -Destination (Join-Path $destination $name)}
        Copy-Item -LiteralPath (Join-Path $bridgeRoot 'build-evidence.json') -Destination (Join-Path $destination 'message-build-evidence.json')
        $voice=Get-Content (Join-Path $destination 'voice-codec-build-evidence.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        $emoticon=Get-Content (Join-Path $destination 'emoticon-codec-build-evidence.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        RequireHash (Join-Path $project 'native/voice-codec/codec.cpp') $voice.adapter_source_sha256
        RequireHash (Join-Path $project 'native/emoticon-codec/codec.cpp') $emoticon.adapter_source_sha256
        RequireHash (Join-Path $project 'native/emoticon-codec/CMakeLists.txt') $emoticon.cmake_sha256
        RequireHash (Join-Path $destination 'werc-voice-codec.exe') $voice.sha256
        RequireHash (Join-Path $destination 'werc-emoticon-codec.exe') $emoticon.sha256
        RequireHash (Join-Path $destination 'werc-emoticon-codec-source.zip') $emoticon.source_archive_sha256
        $sql=Get-Content -LiteralPath target/components/sqlcipher/source-evidence.json -Raw -Encoding UTF8 | ConvertFrom-Json
        if(-not $sql.passed){throw 'SQLCipher source checks missing'}
        RequireHash 'target/components/sqlcipher/debug/werc-sqlcipher-worker.exe' $sql.executable_sha256
        foreach($source in $sql.source_files){RequireHash (Join-Path $project $source.path) $source.sha256}
        Copy-Item target/components/sqlcipher/debug/werc-sqlcipher-worker.exe (Join-Path $destination 'werc-sqlcipher-worker.exe')
        & cargo.exe build --locked --release -p werc-native-worker -p werc-http-api --bins
        if($LASTEXITCODE -ne 0){throw 'Worker/MCP build failed'}
        $env:WERC_WORKER_SHA256=Sha (Join-Path $destination 'werc-native-worker.exe')
        $env:WERC_SQLCIPHER_WORKER_SHA256=$sql.executable_sha256
        $env:WERC_MESSAGE_HOST_SHA256=$bridge.host.sha256
        $env:WERC_VOICE_CODEC_SHA256=$voice.sha256
        $env:WERC_EMOTICON_CODEC_SHA256=$emoticon.sha256
        & python.exe (Join-Path $PSScriptRoot 'prepare-release-resources.py')
        if($LASTEXITCODE -ne 0){throw 'Release resource preparation failed'}
        & node node_modules/typescript/bin/tsc --noEmit
        if($LASTEXITCODE -ne 0){throw 'TypeScript failed'}
        & node node_modules/vite/bin/vite.js build --config apps/desktop/vite.config.ts
        if($LASTEXITCODE -ne 0){throw 'Frontend build failed'}
        & cargo.exe build --locked --release -p werc-desktop --features custom-protocol
        if($LASTEXITCODE -ne 0){throw 'Release desktop build failed'}
    }
    & python.exe (Join-Path $PSScriptRoot 'api_docs.py') (Join-Path $project 'target/release-resources')
    if($LASTEXITCODE -ne 0){throw 'API documentation preparation failed'}
    & python.exe (Join-Path $PSScriptRoot 'user_docs.py') (Join-Path $project 'target/release-resources')
    if($LASTEXITCODE -ne 0){throw 'User documentation preparation failed'}
    # Bundling reuses the already compiled normal-identity executable.
    Push-Location (Join-Path $project 'apps/desktop')
    try { & node ../../node_modules/@tauri-apps/cli/tauri.js bundle --config src-tauri/tauri.bundle.conf.json --bundles nsis
        if($LASTEXITCODE -ne 0){throw 'NSIS bundle failed'}
    } finally {Pop-Location}
} finally {Restore-WeRCEnvironment $previous}
