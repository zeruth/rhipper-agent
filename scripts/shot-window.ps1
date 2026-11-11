param([string]$Process = 'rc-gui', [string]$Out = 'window.png')
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Shot {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT r, int size);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
[Shot]::SetProcessDPIAware() | Out-Null
$p = Get-Process $Process -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $p) { throw "no window for $Process" }
$h = $p.MainWindowHandle
$r = New-Object Shot+RECT
# the frame without the shadow (DWMWA_EXTENDED_FRAME_BOUNDS = 9)
if ([Shot]::DwmGetWindowAttribute($h, 9, [ref]$r, 16) -ne 0) { [Shot]::GetWindowRect($h, [ref]$r) | Out-Null }
$full = New-Object Shot+RECT
[Shot]::GetWindowRect($h, [ref]$full) | Out-Null
$w = $full.R - $full.L; $hh = $full.B - $full.T
$bmp = New-Object System.Drawing.Bitmap $w, $hh
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[Shot]::PrintWindow($h, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
$g.ReleaseHdc($hdc)
$g.Dispose()
# crop the invisible border to the visible frame
$crop = New-Object System.Drawing.Rectangle ($r.L - $full.L), ($r.T - $full.T), ($r.R - $r.L), ($r.B - $r.T)
$cut = $bmp.Clone($crop, $bmp.PixelFormat)
$cut.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$cut.Dispose(); $bmp.Dispose()
"$Out $($crop.Width)x$($crop.Height)"
