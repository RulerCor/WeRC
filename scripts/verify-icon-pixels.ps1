$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
if (-not ('IconPixelVerifier' -as [type])) {
    $drawingReferences = @([System.Drawing.Bitmap].Assembly.Location)
    if ($PSVersionTable.PSEdition -eq 'Core') {
        # PowerShell 7 uses .NET reference assemblies and Drawing.Common;
        # Windows PowerShell 5.1 uses the framework's System.Drawing assembly.
        foreach ($name in @('System.Runtime.dll','System.Runtime.InteropServices.dll','System.Drawing.Primitives.dll')) {
            $drawingReferences += Join-Path $PSHOME ('ref/' + $name)
        }
        foreach ($dependency in [System.Drawing.Bitmap].Assembly.GetReferencedAssemblies()) {
            if ($dependency.Name.StartsWith('System.Private.Windows.')) {
                $drawingReferences += [Reflection.Assembly]::Load($dependency).Location
            }
        }
    }
    Add-Type -ReferencedAssemblies $drawingReferences -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class IconPixelVerifier {
    public static long Verify(string originalPath, string outputPath, int[] preserved) {
        using (var source = new Bitmap(originalPath))
        using (var output = new Bitmap(outputPath)) {
            int side = Math.Max(source.Width, source.Height);
            if (output.Width != side || output.Height != side)
                throw new Exception("Unexpected output canvas size.");
            int x = (side - source.Width) / 2;
            int y = (side - source.Height) / 2;
            var a = source.LockBits(new Rectangle(0, 0, source.Width, source.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            var b = output.LockBits(new Rectangle(0, 0, side, side), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            try {
                var sourceRow = new byte[source.Width * 4];
                var outputRow = new byte[side * 4];
                for (int row = 0; row < side; row++) {
                    Marshal.Copy(IntPtr.Add(b.Scan0, row * b.Stride), outputRow, 0, outputRow.Length);
                    if (row >= y && row < y + source.Height) {
                        Marshal.Copy(IntPtr.Add(a.Scan0, (row - y) * a.Stride), sourceRow, 0, sourceRow.Length);
                        for (int index = 0; index < sourceRow.Length; index++) {
                            if (preserved != null && (row < preserved[1] || row >= preserved[3] || x + index / 4 < preserved[0] || x + index / 4 >= preserved[2])) continue;
                            if (sourceRow[index] != outputRow[x * 4 + index])
                                throw new Exception("Image pixel mismatch at " + (index / 4) + "," + (row - y));
                        }
                    }
                    for (int column = 0; column < side; column++) {
                        bool padding = row < y || row >= y + source.Height || column < x || column >= x + source.Width;
                        if (preserved == null && padding && outputRow[column * 4 + 3] != 0)
                            throw new Exception("Padding must remain transparent.");
                    }
                }
            } finally {
                source.UnlockBits(a);
                output.UnlockBits(b);
            }
            return preserved == null ? (long)source.Width * source.Height : (long)(preserved[2] - preserved[0]) * (preserved[3] - preserved[1]);
        }
    }
}
'@
}
$project = Split-Path $PSScriptRoot -Parent
$original = Join-Path $project 'assets/app-icon-original.jpg'
$output = Join-Path $project 'assets/app-icon.png'
$manifestPath = Join-Path $project 'assets/app-icon-branding.json'
$preserved = $null
if (Test-Path -LiteralPath $manifestPath) {
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ((Get-FileHash -LiteralPath $original -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.source_sha256) { throw 'Original portrait hash differs from the branding manifest.' }
    if ((Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.output_sha256) { throw 'Branded icon hash differs from the branding manifest.' }
    $preserved = [int[]]$manifest.preserved_rectangle
}
$pixels = [IconPixelVerifier]::Verify($original, $output, $preserved)
Write-Output ("Icon verified: {0} original portrait pixels preserved; source and branded asset hashes checked." -f $pixels)
