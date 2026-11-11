# Captures one process's main window to a PNG, whatever is in front of it: PrintWindow with the
# DWM full-content flag renders the window from its own surface, not from the screen.
param([string]$Process = "rc-gui", [string]$Out = "$PSScriptRoot\window.png")
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
}
"@
$proc = Get-Process $Process -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { "no window for $Process"; exit 1 }
$hwnd = $proc.MainWindowHandle
if ([Win]::IsIconic($hwnd)) { "the window is minimized; nothing to render"; exit 1 }
$rect = New-Object Win+RECT
[void][Win]::GetWindowRect($hwnd, [ref]$rect)
$w = $rect.Right - $rect.Left; $h = $rect.Bottom - $rect.Top
$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# 2 = PW_RENDERFULLCONTENT: the composed surface, covered or not
$ok = [Win]::PrintWindow($hwnd, $hdc, 2)
$g.ReleaseHdc($hdc)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"saved $Out (${w}x${h}, printwindow=$ok)"
