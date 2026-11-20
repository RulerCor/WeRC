param([Parameter(Mandatory = $true)][string]$Source, [string]$PythonPath = 'python.exe')
$ErrorActionPreference = 'Stop'
$assets = Join-Path (Split-Path $PSScriptRoot -Parent) 'assets'
$original = Join-Path $assets 'app-icon-original.jpg'
if (Test-Path $original) {
    if ((Get-FileHash $Source -Algorithm SHA256).Hash -ne (Get-FileHash $original -Algorithm SHA256).Hash) {
        throw 'An icon source already exists and differs. Review it before replacing it.'
    }
} else {
    Copy-Item -LiteralPath $Source -Destination $original
}
# The reusable branding recipe preserves the supplied portrait and writes a
# manifest checked below. Python needs Pillow; the app itself does not use it.
& $PythonPath (Join-Path $PSScriptRoot 'brand-icon.py')
if ($LASTEXITCODE -ne 0) { throw 'Icon branding failed (Python with Pillow is required).' }
& (Join-Path $PSScriptRoot 'verify-icon-pixels.ps1')
