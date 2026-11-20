param([switch]$PrepareOnly)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
$project = Split-Path $PSScriptRoot -Parent
$target = Join-Path $project 'target'
$components = Join-Path $target 'components'
$release = Join-Path $target 'release'
$previous = @{}
foreach ($name in @('CARGO_TARGET_DIR','OPENSSL_NO_ASM','LC_ALL','PATH')) {
    $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
function Sha([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Copy-Component([string]$Directory, [string]$Name, [string[]]$Files) {
    foreach ($file in $Files) { Copy-Item -LiteralPath (Join-Path $Directory $file) -Destination (Join-Path $release $file) }
    Copy-Item -LiteralPath (Join-Path $Directory 'build-evidence.json') -Destination (Join-Path $release "$Name-build-evidence.json")
}
function Get-Asset([uri]$Url, [string]$Path, [string]$Hash) {
    if (Test-Path -LiteralPath $Path) {
        if ((Sha $Path) -ne $Hash) { throw "Remove the corrupt cached download: $Path" }
        return
    }
    $pending = "$Path.download"
    & curl.exe -fL --retry 2 --output $pending $Url.AbsoluteUri
    if ($LASTEXITCODE -ne 0 -or (Sha $pending) -ne $Hash) { throw "Cannot verify $Url" }
    Move-Item -LiteralPath $pending -Destination $Path
}
Push-Location $project
try {
    Initialize-MsvcEnvironment
    $perl = Split-Path (Find-WeRCNativePerl) -Parent
    New-Item -ItemType Directory -Force -Path $components,$release | Out-Null
    $scratch = Join-Path $target ('source-build-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $scratch | Out-Null
    $bridge = Join-Path $scratch 'message-bridge'
    $voice = Join-Path $scratch 'voice'
    $emoticon = Join-Path $scratch 'emoticon'
    & (Join-Path $PSScriptRoot 'build-message-bridge.ps1') -OutputDirectory $bridge
    & (Join-Path $PSScriptRoot 'build-voice-codec.ps1') -OutputDirectory $voice
    & (Join-Path $PSScriptRoot 'build-emoticon-codec.ps1') -OutputDirectory $emoticon
    $bridgeDestination = Join-Path $components 'message-bridge'
    New-Item -ItemType Directory -Force -Path $bridgeDestination | Out-Null
    foreach ($file in @('build-evidence.json','werc-message-host.exe','werc-message-bridge.dll')) {
        Copy-Item -LiteralPath (Join-Path $bridge $file) -Destination $bridgeDestination
    }
    Copy-Component $bridge 'message' @('werc-message-host.exe','werc-message-bridge.dll')
    Copy-Component $voice 'voice-codec' @('werc-voice-codec.exe')
    Copy-Component $emoticon 'emoticon-codec' @('werc-emoticon-codec.exe','werc-emoticon-codec-source.zip')

    $sql = Join-Path $components 'sqlcipher'
    $sqlBuild = Join-Path $target 'build/sqlcipher'
    $env:CARGO_TARGET_DIR = $sqlBuild
    $env:OPENSSL_NO_ASM = '1'
    $env:LC_ALL = 'C'
    $oldPath = $env:PATH
    $env:PATH = $perl + ';' + $env:PATH
    try {
        & cargo.exe build --locked --release --manifest-path workers/sqlcipher/Cargo.toml
        if ($LASTEXITCODE -ne 0) { throw 'SQLCipher worker build failed' }
        & cargo.exe test --locked --release --manifest-path workers/sqlcipher/Cargo.toml
        if ($LASTEXITCODE -ne 0) { throw 'SQLCipher worker tests failed' }
    } finally { $env:PATH = $oldPath }
    $sqlDestination = Join-Path $sql 'debug'
    New-Item -ItemType Directory -Force -Path $sqlDestination | Out-Null
    $worker = Join-Path $sqlDestination 'werc-sqlcipher-worker.exe'
    Copy-Item -LiteralPath (Join-Path $sqlBuild 'release/werc-sqlcipher-worker.exe') -Destination $worker
    [ordered]@{
        passed=$true; executable_sha256=(Sha $worker)
        source_files=@(Get-ChildItem -LiteralPath 'workers/sqlcipher' -Recurse -File | Where-Object {
            $_.FullName -notmatch '[\\/]target[\\/]' -and $_.Name -match '^(Cargo\.(toml|lock)|.+\.rs)$'
        } | ForEach-Object { @{path=$_.FullName.Substring($project.Length + 1).Replace('\','/');sha256=(Sha $_.FullName)} })
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $sql 'source-evidence.json') -Encoding UTF8
    Restore-WeRCEnvironment $previous
    if ($PrepareOnly) { return }

    & (Join-Path $PSScriptRoot 'build-asr.ps1')
    $download = Join-Path $target 'downloads'
    $modelDir = Join-Path $target 'asr/models'
    $sherpaDir = Join-Path $target 'sherpa-verification/components/sherpa'
    New-Item -ItemType Directory -Force -Path $download,$modelDir,$sherpaDir | Out-Null
    $model = Get-Content -LiteralPath 'native/asr/MODEL.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    Get-Asset "https://huggingface.co/$($model.repository)/resolve/$($model.revision)/$($model.model)" (Join-Path $modelDir $model.model) $model.sha256
    $sherpa = Get-Content -LiteralPath 'native/asr/SHERPA.json' -Raw -Encoding UTF8 | ConvertFrom-Json
    $archive = Join-Path $download 'sherpa-runtime.tar.bz2'
    Get-Asset $sherpa.url $archive $sherpa.sha256
    $items = @(& tar.exe -tf $archive | Where-Object { $_ -match '^[a-zA-Z0-9._-]+/bin/sherpa-onnx-offline\.exe$' })
    if ($LASTEXITCODE -ne 0 -or $items.Count -ne 1) { throw 'Expected one Sherpa executable' }
    & tar.exe -xf $archive -C $scratch $items[0]
    if ($LASTEXITCODE -ne 0) { throw 'Sherpa extraction failed' }
    Copy-Item -LiteralPath (Join-Path $scratch $items[0]) -Destination $sherpaDir
    & (Join-Path $PSScriptRoot 'release-v1.ps1')
} finally {
    Restore-WeRCEnvironment $previous
    Pop-Location
}
