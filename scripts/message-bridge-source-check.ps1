# Follow the actual adapter include graph; research/fixture files are not runtime.
# Every literal Weixin RVA in runtime code must pass through exact-build resolution.
function Test-WeRCMessageBridgeAddresses([string]$AdapterDirectory) {
    $adapterRoot=(Resolve-Path -LiteralPath $AdapterDirectory).Path
    $pending=[System.Collections.Generic.Stack[string]]::new()
    $visited=[System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
    $pending.Push((Join-Path $adapterRoot 'bridge.cpp'))
    while($pending.Count) {
        $sourcePath=$pending.Pop()
        if(-not $visited.Add($sourcePath)){continue}
        $sourceText=Get-Content -LiteralPath $sourcePath -Raw -Encoding UTF8
        if($sourceText -match '\bcore_base\s*\+\s*0[xX][0-9a-fA-F]+') {
            throw ('Unresolved literal core address in runtime source: '+$sourcePath)
        }
        foreach($include in [regex]::Matches($sourceText,'(?m)^\s*#include\s+"([^"]+)"')) {
            $includePath=[IO.Path]::GetFullPath((Join-Path (Split-Path $sourcePath -Parent) $include.Groups[1].Value))
            if($includePath.StartsWith($adapterRoot+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) {
                $pending.Push($includePath)
            }
        }
    }
    return @{runtime_source_files=$visited.Count;unresolved_literal_core_addresses=0}
}
