param([switch]$LiveReadVerification, [switch]$UseRunningFrontend, [switch]$BuildOnly, [string]$MessageBridgeDirectory, [string]$SqlCipherWorker, [string]$SqlCipherEvidence)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
$project = Split-Path $PSScriptRoot -Parent
$previous = @{}
foreach ($name in @('TEMP','TMP','CARGO_TARGET_DIR','TAURI_CONFIG','WERC_WORKER_SHA256','WERC_SQLCIPHER_WORKER_SHA256','WERC_MESSAGE_HOST_SHA256','WERC_VOICE_CODEC_SHA256','WERC_EMOTICON_CODEC_SHA256')) {
    $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$frontend = $null
function Get-Sha([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Require-Hash([string]$Path, [string]$Hash) {
    if (-not $Hash -or (Get-Sha $Path) -ne $Hash.ToLowerInvariant()) { throw ('Source/component changed; rebuild its component first: ' + $Path) }
}
try {
    $scratch = Join-Path $project 'target/tmp'
    New-Item -ItemType Directory -Force -Path $scratch | Out-Null
    $env:TEMP = $scratch
    $env:TMP = $scratch
    Initialize-MsvcEnvironment
    Set-Location -LiteralPath $project
    $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
    $target = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($target)
    $env:CARGO_TARGET_DIR = $target
    $destination = Join-Path $target 'debug'
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    $components = Join-Path $project 'target/release'
    # Existing source-built native components are reused only when their source
    # adapters and binary hashes still match. No seven-component rebuild here.
    if (-not $MessageBridgeDirectory -and (Test-Path -LiteralPath (Join-Path $project 'target/components/message-bridge/build-evidence.json'))) { $MessageBridgeDirectory = Join-Path $project 'target/components/message-bridge' }
    $bridgeComponents = if ($MessageBridgeDirectory) { $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($MessageBridgeDirectory) } else { $components }
    $bridgeEvidence = if ($MessageBridgeDirectory) { 'build-evidence.json' } else { 'message-build-evidence.json' }
    $bridge = Get-Content (Join-Path $bridgeComponents $bridgeEvidence) -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($source in $bridge.adapter_sources) { Require-Hash (Join-Path $project ('native/message-bridge/' + $source.path)) $source.sha256 }
    Require-Hash (Join-Path $bridgeComponents 'werc-message-host.exe') $bridge.host.sha256
    Require-Hash (Join-Path $bridgeComponents 'werc-message-bridge.dll') $bridge.sha256
    $voice = Get-Content (Join-Path $components 'voice-codec-build-evidence.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $emoticon = Get-Content (Join-Path $components 'emoticon-codec-build-evidence.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    Require-Hash (Join-Path $project 'native/voice-codec/codec.cpp') $voice.adapter_source_sha256
    Require-Hash (Join-Path $project 'native/emoticon-codec/codec.cpp') $emoticon.adapter_source_sha256
    Require-Hash (Join-Path $project 'native/emoticon-codec/CMakeLists.txt') $emoticon.cmake_sha256
    Require-Hash (Join-Path $components 'werc-emoticon-codec-source.zip') $emoticon.source_archive_sha256
    Require-Hash (Join-Path $components 'werc-voice-codec.exe') $voice.sha256
    Require-Hash (Join-Path $components 'werc-emoticon-codec.exe') $emoticon.sha256
    foreach ($name in @('werc-message-host.exe','werc-message-bridge.dll','werc-voice-codec.exe','werc-emoticon-codec.exe','werc-emoticon-codec-source.zip')) {
        $componentSource = if ($name -in @('werc-message-host.exe','werc-message-bridge.dll')) { $bridgeComponents } else { $components }
        Copy-Item -LiteralPath (Join-Path $componentSource $name) -Destination (Join-Path $destination $name)
    }
    if (-not $SqlCipherWorker -and -not $SqlCipherEvidence -and (Test-Path -LiteralPath (Join-Path $project 'target/components/sqlcipher/debug/werc-sqlcipher-worker.exe'))) {
        $SqlCipherWorker=Join-Path $project 'target/components/sqlcipher/debug/werc-sqlcipher-worker.exe'
        $SqlCipherEvidence=Join-Path $project 'target/components/sqlcipher/source-evidence.json'
        if (-not (Test-Path -LiteralPath $SqlCipherEvidence)) { $SqlCipherEvidence=Join-Path $project 'target/current-source-data-worker.json' }
    }
    if ([bool]$SqlCipherWorker -ne [bool]$SqlCipherEvidence) { throw 'Supply both -SqlCipherWorker and -SqlCipherEvidence' }
    $sql = if ($SqlCipherWorker) { $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SqlCipherWorker) } else { Join-Path $project 'workers/sqlcipher/target/release/werc-sqlcipher-worker.exe' }
    $sqlEvidencePath = if ($SqlCipherEvidence) { $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SqlCipherEvidence) } else { Join-Path $project 'docs/verification/data-worker-result.json' }
    $sqlBuild = Get-Content $sqlEvidencePath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $sqlBuild.passed) { throw 'Run the source SQLCipher worker checks before desktop development' }
    Require-Hash $sql $sqlBuild.executable_sha256
    foreach ($source in $sqlBuild.source_files) { Require-Hash (Join-Path $project $source.path) $source.sha256 }
    Copy-Item -LiteralPath $sql -Destination (Join-Path $destination 'werc-sqlcipher-worker.exe')
    & cargo.exe build --locked -p werc-native-worker -p werc-http-api --bins
    if ($LASTEXITCODE -ne 0) { throw 'Native reader source build failed' }
    $env:WERC_WORKER_SHA256 = Get-Sha (Join-Path $destination 'werc-native-worker.exe')
    $env:WERC_SQLCIPHER_WORKER_SHA256 = $sqlBuild.executable_sha256
    $env:WERC_MESSAGE_HOST_SHA256 = $bridge.host.sha256
    $env:WERC_VOICE_CODEC_SHA256 = $voice.sha256
    $env:WERC_EMOTICON_CODEC_SHA256 = $emoticon.sha256
    $configName = if ($LiveReadVerification) { 'tauri.liveverify.conf.json' } else { 'tauri.smoke.conf.json' }
    $env:TAURI_CONFIG = Get-Content (Join-Path $project ('apps/desktop/src-tauri/' + $configName)) -Raw -Encoding UTF8
    & cargo.exe build --locked -p werc-desktop
    if ($LASTEXITCODE -ne 0) { throw 'Desktop source build failed' }
    if ($BuildOnly) { Write-Output 'Source desktop built in debug; no app launched or installer generated.'; return }
    if (-not $UseRunningFrontend) {
        $node = (Get-Command node.exe -ErrorAction Stop).Source
        $vite = Join-Path $project 'node_modules/vite/bin/vite.js'
        $config = Join-Path $project 'apps/desktop/vite.config.ts'
        $frontend = Start-Process -FilePath $node -ArgumentList @(('"'+$vite+'"'),'--config',('"'+$config+'"'),'--host','127.0.0.1') -WorkingDirectory $project -WindowStyle Hidden -PassThru
        $ready = $false
        for ($attempt=0; $attempt -lt 40; $attempt++) {
            if ($frontend.HasExited) { throw 'Source frontend exited; use -UseRunningFrontend when Vite is already running' }
            try { $response=Invoke-WebRequest 'http://127.0.0.1:5173/' -UseBasicParsing; $ready=$response.StatusCode -eq 200 } catch {}
            if ($ready) { break }
            Start-Sleep -Milliseconds 250
        }
        if (-not $ready) { throw 'Source frontend did not become ready' }
    }
    & (Join-Path $destination 'werc-desktop.exe')
    if ($LASTEXITCODE -ne 0) { throw 'Source desktop exited with an error' }
} finally {
    if ($frontend -and -not $frontend.HasExited) { Stop-Process -Id $frontend.Id }
    Restore-WeRCEnvironment $previous
}
