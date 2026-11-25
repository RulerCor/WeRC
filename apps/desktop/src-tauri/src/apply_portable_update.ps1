param([int]$PreviousPid, [string]$Program, [string]$DataDirectory)
$ErrorActionPreference = 'Stop'
$payload = Join-Path $PSScriptRoot 'payload'
$rollback = Join-Path $PSScriptRoot 'previous'
function Launch-WeRC {
    $tail = if ($DataDirectory.EndsWith('\')) { '\' } else { '' }
    Start-Process -FilePath (Join-Path $Program 'werc-desktop.exe') -WorkingDirectory $Program -ArgumentList @('--data-directory', ('"' + $DataDirectory + $tail + '"')) -PassThru
}
try {
    $previous = Get-Process -Id $PreviousPid -ErrorAction SilentlyContinue
    if ($previous -and -not $previous.WaitForExit(30000)) { throw 'WeRC did not exit' }
    try { [IO.Directory]::Move($Program, $rollback) } catch { $null = Launch-WeRC; throw }
    try {
        [IO.Directory]::Move($payload, $Program)
        $updated = Launch-WeRC
        if ($updated.WaitForExit(5000)) { throw 'Updated WeRC exited during startup' }
    } catch {
        if ([IO.Directory]::Exists($Program)) { [IO.Directory]::Move($Program, $payload) }
        [IO.Directory]::Move($rollback, $Program)
        $null = Launch-WeRC
        throw
    }
    $backupName = (Split-Path -Leaf $Program) + '.previous'
    $backup = Join-Path (Split-Path -Parent $Program) $backupName
    if (-not [IO.Directory]::Exists($backup)) { [IO.Directory]::Move($rollback, $backup) }
} catch {
    [IO.File]::WriteAllText((Join-Path $PSScriptRoot 'update-error.txt'), $_.ToString())
}
