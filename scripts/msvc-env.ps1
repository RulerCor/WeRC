function Restore-WeRCEnvironment([hashtable]$Values) {
    foreach ($name in $Values.Keys) {
        if ($null -eq $Values[$name]) {
            Remove-Item -LiteralPath ('Env:' + $name) -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable($name, $Values[$name], 'Process')
        }
    }
}

function Find-WeRCNativePerl([string]$PerlPath) {
    $previousLocale = [Environment]::GetEnvironmentVariable('LC_ALL', 'Process')
    try {
    $env:LC_ALL = 'C'
    $candidates = if ($PerlPath) { @($PerlPath) } else {
        @(Get-Command perl.exe -All -ErrorAction SilentlyContinue | ForEach-Object Source)
    }
    foreach ($candidate in $candidates) {
        # Resolve against PowerShell's location, not the process working directory.
        # Use exactly the same resolved file for invocation and the build PATH.
        $resolved = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($candidate)
        if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) { continue }
        $platform = & $resolved -e 'print $^O' 2>$null
        if ($LASTEXITCODE -eq 0 -and $platform -eq 'MSWin32') { return $resolved }
    }
    throw 'Native Windows Perl is required. Put it on PATH or pass -PerlPath. Git bundled Perl is not sufficient.'
    } finally {
        Restore-WeRCEnvironment @{ LC_ALL = $previousLocale }
    }
}

function Initialize-MsvcEnvironment {
    $ErrorActionPreference = 'Stop'
    [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
    $OutputEncoding = [Console]::OutputEncoding
    $vswhereCommand = Get-Command vswhere.exe -ErrorAction SilentlyContinue
    $vswhere = if ($vswhereCommand) { $vswhereCommand.Source } else {
        $installerRoot = [Environment]::GetFolderPath('ProgramFilesX86')
        Join-Path $installerRoot 'Microsoft Visual Studio/Installer/vswhere.exe'
    }
    if (-not (Test-Path -LiteralPath $vswhere)) { throw '缺少 Visual Studio Installer 的 vswhere；请安装 C++ 构建工具或将 vswhere 加入 PATH。' }
    $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $installation) { throw '缺少 MSVC C++ 构建组件。' }
    $developerShell = Join-Path $installation 'Common7\Tools\Launch-VsDevShell.ps1'
    $existingCompiler = Get-Command cl.exe -ErrorAction SilentlyContinue
    $existingLinker = Get-Command link.exe -ErrorAction SilentlyContinue
    $ready = $existingCompiler -and $existingLinker -and $env:INCLUDE -and $env:LIB -and
        $env:VSCMD_ARG_TGT_ARCH -eq 'x64' -and $env:VSCMD_ARG_HOST_ARCH -eq 'x64' -and
        $existingCompiler.Source.StartsWith($installation + '\', [StringComparison]::OrdinalIgnoreCase) -and
        $existingLinker.Source.StartsWith($installation + '\', [StringComparison]::OrdinalIgnoreCase)
    if (-not $ready) {
        # DevShell reads the native command shell's environment using the active
        # console encoding. Preserve Unicode caller paths across that round trip.
        $callerPaths = $env:PATH
        $callerEnvironment = @{}
        foreach ($name in @('CARGO_HOME','RUSTUP_HOME','CARGO_TARGET_DIR','TEMP','TMP','HOME','USERPROFILE','APPDATA','LOCALAPPDATA')) {
            $callerEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        }
        try {
            [Console]::OutputEncoding = [Text.Encoding]::GetEncoding([Globalization.CultureInfo]::CurrentCulture.TextInfo.OEMCodePage)
            & $developerShell -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
        } finally {
            Restore-WeRCEnvironment $callerEnvironment
            $env:PATH = $env:PATH + ';' + $callerPaths
            [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
            $OutputEncoding = [Console]::OutputEncoding
        }
    }
    $compiler = Get-Command cl.exe -ErrorAction SilentlyContinue
    $linker = Get-Command link.exe -ErrorAction SilentlyContinue
    if (-not $compiler -or -not $linker -or -not $linker.Source.StartsWith($installation, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'MSVC 开发环境初始化失败；不能使用 Git 的同名 link.exe。'
    }
    Write-Host ('MSVC compiler: ' + $compiler.Source)
    Write-Host ('MSVC linker: ' + $linker.Source)
    # Nested component builds share this environment. Repeated VS initialization
    # plus npm's PATH additions can exceed cmd.exe's environment value limit.
    $seenPaths = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $rustHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path ([Environment]::GetFolderPath('UserProfile')) '.cargo' }
    $rustBin = Join-Path $rustHome 'bin'
    $env:PATH = (($rustBin + ';' + $env:PATH).Split(';') | Where-Object {
        $_ -and $seenPaths.Add($_.TrimEnd('\'))
    }) -join ';'
}
