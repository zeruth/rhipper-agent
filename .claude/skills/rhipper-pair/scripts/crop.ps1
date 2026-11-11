# Crops a region of a PNG to another PNG, for a closer look.
param([string]$In, [string]$Out, [int]$X, [int]$Y, [int]$W, [int]$H)
Add-Type -AssemblyName System.Drawing
$src = [System.Drawing.Image]::FromFile($In)
$rect = New-Object System.Drawing.Rectangle $X, $Y, $W, $H
$bmp = New-Object System.Drawing.Bitmap $W, $H
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.DrawImage($src, (New-Object System.Drawing.Rectangle 0, 0, $W, $H), $rect, [System.Drawing.GraphicsUnit]::Pixel)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose(); $src.Dispose()
"saved $Out"
