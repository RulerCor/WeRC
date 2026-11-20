param([string]$OutputDirectory = 'target/asr', [switch]$Cuda, [string]$CudaArchitectures = '75;80;86;89;90;120', [string]$BuildDirectory)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
Initialize-MsvcEnvironment
$output = if ([IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory } else { Join-Path $project $OutputDirectory }
$source = Join-Path $project 'native/asr/whisper.cpp-1.9.5'
if (-not $BuildDirectory) { $BuildDirectory = if ($Cuda) { 'target/asr-build-cuda' } else { 'target/asr-build' } }
$build = if ([IO.Path]::IsPathRooted($BuildDirectory)) { $BuildDirectory } else { Join-Path $project $BuildDirectory }
if ($Cuda) { $output = Join-Path $output 'cuda' }
$gpuOptions = if ($Cuda) { @('-DGGML_CUDA=ON', ('-DCMAKE_CUDA_ARCHITECTURES=' + $CudaArchitectures)) } else { @('-DGGML_CUDA=OFF') }
if (-not (Test-Path -LiteralPath (Join-Path $source 'CMakeLists.txt'))) { throw 'Missing pinned whisper.cpp source; see native/asr/SOURCE.json' }
& cmake -S $source -B $build -G 'NMake Makefiles' '-DCMAKE_BUILD_TYPE=Release' '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded' '-DBUILD_SHARED_LIBS=OFF' '-DGGML_NATIVE=OFF' '-DGGML_AVX=OFF' '-DGGML_AVX2=OFF' '-DGGML_FMA=OFF' '-DGGML_F16C=OFF' '-DGGML_AVX512=OFF' '-DGGML_OPENMP=OFF' '-DGGML_CCACHE=OFF' '-DWHISPER_BUILD_TESTS=OFF' '-DWHISPER_BUILD_SERVER=OFF' '-DWHISPER_CURL=OFF' @gpuOptions
if ($LASTEXITCODE -ne 0) { throw 'ASR configure failed' }
& cmake --build $build --target whisper-cli
if ($LASTEXITCODE -ne 0) { throw 'ASR source build failed' }
New-Item -ItemType Directory -Force -Path $output | Out-Null
Copy-Item -LiteralPath (Join-Path $build 'bin/whisper-cli.exe') -Destination (Join-Path $output 'whisper-cli.exe')
# Whisper expects UTF-8 filenames, while Windows narrow argv otherwise uses the
# user's legacy code page. Merge into the existing executable manifest so paths
# containing Chinese work for models, input audio and output JSON alike.
$utf8Manifest = Join-Path $project 'native/asr/windows-utf8.manifest'
$cli = Join-Path $output 'whisper-cli.exe'
& mt.exe -nologo -manifest $utf8Manifest ('-inputresource:'+$cli+';#1') ('-outputresource:'+$cli+';#1')
if ($LASTEXITCODE -ne 0) { throw 'ASR UTF-8 application manifest failed' }
Copy-Item -LiteralPath (Join-Path $source 'LICENSE') -Destination (Join-Path $output 'LICENSE-whisper.txt')
if ($Cuda) {
    $nvcc = (Get-Command nvcc.exe -ErrorAction Stop).Source
    $cudaBin = Split-Path $nvcc -Parent
    foreach ($pattern in @('cudart64_*.dll','cublas64_*.dll','cublasLt64_*.dll')) {
        $libraries = @(Get-ChildItem -LiteralPath $cudaBin -Filter $pattern -File)
        if ($libraries.Count -eq 0) { throw ('Missing CUDA runtime: ' + $pattern) }
        $libraries | Copy-Item -Destination $output
    }
    [ordered]@{ backend='cuda'; architectures=$CudaArchitectures; files=@(Get-ChildItem -LiteralPath $output -File | Where-Object Extension -in '.exe','.dll' | ForEach-Object { @{path=$_.Name;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()} }) } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $output 'cuda-runtime.json') -Encoding UTF8
}
[ordered]@{source_tag='v1.9.5';cpu_baseline='x64 without AVX/FMA/F16C';backend=$(if($Cuda){'cuda'}else{'cpu'});utf8_manifest_sha256=(Get-FileHash -LiteralPath $utf8Manifest -Algorithm SHA256).Hash.ToLowerInvariant();sha256=(Get-FileHash -LiteralPath (Join-Path $output 'whisper-cli.exe') -Algorithm SHA256).Hash.ToLowerInvariant();source_manifest=(Get-Content (Join-Path $project 'native/asr/SOURCE.json') -Raw | ConvertFrom-Json)} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $output 'build-evidence.json') -Encoding UTF8
