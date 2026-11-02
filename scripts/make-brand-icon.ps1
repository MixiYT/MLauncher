param([string]$Output = '')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
if ([string]::IsNullOrWhiteSpace($Output)) {
    $Output = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) '..\assets\icon-official-source.png'
}
# Mirrors assets/icon-official.svg; no fonts or external renderers are required.
$bitmap = New-Object System.Drawing.Bitmap(1024, 1024)
$g = [System.Drawing.Graphics]::FromImage($bitmap)
$g.Clear([System.Drawing.Color]::Transparent)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
$g.ScaleTransform(2, 2)
$tile = New-Object System.Drawing.Drawing2D.GraphicsPath
$tile.AddArc(16, 16, 144, 144, 180, 90)
$tile.AddArc(352, 16, 144, 144, 270, 90)
$tile.AddArc(352, 352, 144, 144, 0, 90)
$tile.AddArc(16, 352, 144, 144, 90, 90)
$tile.CloseFigure()
$g.SetClip($tile)
foreach ($band in @(@('#382c0a',16,480), @('#236c2c',16,60), @('#2d8e35',76,64), @('#54a751',140,64))) {
    $brush = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml($band[0]))
    $g.FillRectangle($brush, 16, [int]$band[1], 480, [int]$band[2])
    $brush.Dispose()
}
$g.ResetClip()
$fill = New-Object System.Drawing.SolidBrush([System.Drawing.ColorTranslator]::FromHtml('#d4edbe'))
$stroke = New-Object System.Drawing.Pen([System.Drawing.ColorTranslator]::FromHtml('#294722'), 3)
$stroke.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
$letters = @(
    @(84,374,84,148,124,148,186,254,248,148,288,148,288,374,246,374,246,224,202,302,170,302,126,224,126,374),
    @(320,148,362,148,362,332,428,332,428,374,320,374)
)
foreach ($coordinates in $letters) {
    $points = New-Object 'System.Collections.Generic.List[System.Drawing.PointF]'
    for ($i = 0; $i -lt $coordinates.Count; $i += 2) {
        $points.Add((New-Object System.Drawing.PointF([float]$coordinates[$i], [float]$coordinates[$i + 1])))
    }
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddPolygon($points.ToArray())
    $g.FillPath($fill, $path)
    $g.DrawPath($stroke, $path)
    $path.Dispose()
}
$bitmap.Save([System.IO.Path]::GetFullPath($Output), [System.Drawing.Imaging.ImageFormat]::Png)
$stroke.Dispose(); $fill.Dispose(); $tile.Dispose(); $g.Dispose(); $bitmap.Dispose()
Write-Host "Wrote $Output"
