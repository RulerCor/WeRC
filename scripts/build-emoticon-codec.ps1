param([string]$OutputDirectory)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'msvc-env.ps1')
Initialize-MsvcEnvironment
$project=Split-Path $PSScriptRoot -Parent
$source=Join-Path $project 'native/emoticon-codec'
$manifests=@(foreach($name in @('libde265-1.1.3','jbigkit-2.2')){
 $root=Join-Path $source ('source/'+$name)
 $path=Join-Path $root 'SOURCE.json'
 $manifest=Get-Content -LiteralPath $path -Raw -Encoding UTF8|ConvertFrom-Json
 foreach($entry in $manifest.files){if((Get-FileHash -LiteralPath (Join-Path $root $entry.path)).Hash.ToLowerInvariant() -ne $entry.sha256){throw 'Animation dependency source changed'}}
 [ordered]@{name=$name;sha256=(Get-FileHash -LiteralPath $path).Hash.ToLowerInvariant();archive_sha256=$manifest.archive_sha256}
})
if(-not $OutputDirectory){$OutputDirectory=Join-Path $project ('target/emoticon-codec/'+[Guid]::NewGuid().ToString('N'))}
$OutputDirectory=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
if(Test-Path -LiteralPath $OutputDirectory){throw 'Animation codec output must be a new directory'}
& cmake -S $source -B $OutputDirectory -G Ninja -DCMAKE_BUILD_TYPE=Release
if($LASTEXITCODE -ne 0){throw 'Animation codec configuration failed'}
& cmake --build $OutputDirectory --parallel 4
if($LASTEXITCODE -ne 0){throw 'Animation codec source build failed'}
$exe=Join-Path $OutputDirectory 'werc-emoticon-codec.exe'
& node (Join-Path $PSScriptRoot 'emoticon-codec-smoke.mjs') $exe | Tee-Object -FilePath (Join-Path $OutputDirectory 'ipc-result.json')
if($LASTEXITCODE -ne 0){throw 'Animation codec IPC fixture failed'}
$archive=Join-Path $OutputDirectory 'werc-emoticon-codec-source.zip'
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($source,$archive,[IO.Compression.CompressionLevel]::Optimal,$false)
$zip=[IO.Compression.ZipFile]::OpenRead($archive)
try {
 $files=@(Get-ChildItem -LiteralPath $source -File -Recurse -Force)
 if(@($zip.Entries|Where-Object Name).Count -ne $files.Count){throw 'Incomplete animation source archive'}
 foreach($file in $files){
  $relative=$file.FullName.Substring($source.Length+1).Replace('\','/')
  $entry=$zip.GetEntry($relative)
  if(-not $entry){throw 'Missing animation source entry'}
  $stream=$entry.Open();$sha=[Security.Cryptography.SHA256]::Create()
  try {$actual=([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-','').ToLowerInvariant()}finally{$sha.Dispose();$stream.Dispose()}
  if($actual -ne (Get-FileHash -LiteralPath $file.FullName).Hash.ToLowerInvariant()){throw 'Animation source archive mismatch'}
 }
}finally{$zip.Dispose()}
[ordered]@{schema_version=1;built_at=(Get-Date).ToString('o');sha256=(Get-FileHash -LiteralPath $exe).Hash.ToLowerInvariant();adapter_source_sha256=(Get-FileHash -LiteralPath (Join-Path $source 'codec.cpp')).Hash.ToLowerInvariant();cmake_sha256=(Get-FileHash -LiteralPath (Join-Path $source 'CMakeLists.txt')).Hash.ToLowerInvariant();manifests=$manifests;source_archive_sha256=(Get-FileHash -LiteralPath $archive).Hash.ToLowerInvariant();static_msvc_runtime=$true;simd_required=$false;real_wechat_accessed=$false;messages_sent=$false}|ConvertTo-Json -Depth 5|Set-Content -LiteralPath (Join-Path $OutputDirectory 'build-evidence.json') -Encoding UTF8
