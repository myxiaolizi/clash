# Screenshot helper: capture a window with PrintWindow(PW_RENDERFULLCONTENT).
#
# In this session CopyFromScreen only captures the desktop wallpaper (the DWM
# composition layer is not reachable that way), so we ask the window itself to
# render into a bitmap via PrintWindow with flag 2.
#
# Usage:
#   powershell -File tools/shoot.ps1 -ProcessName ClashZhengl -Out shot.png

param(
    [string]$ProcessName = "ClashZhengl",
    [string]$Out = "shot.png",
    [int]$MinWidth = 500,
    [int]$MinHeight = 400,
    [int]$WaitMs = 1500
)

Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class WinShot
{
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint flags);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int max);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    public static List<IntPtr> EnumByPid(uint targetPid)
    {
        var found = new List<IntPtr>();
        EnumWindows((h, l) =>
        {
            uint pid;
            GetWindowThreadProcessId(h, out pid);
            if (pid == targetPid && IsWindowVisible(h))
                found.Add(h);
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static string Title(IntPtr h)
    {
        var sb = new StringBuilder(512);
        GetWindowTextW(h, sb, 512);
        return sb.ToString();
    }
}
"@

$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) {
    Write-Error "Process $ProcessName not running"
    exit 2
}

Write-Host "pid=$($proc.Id)"

$handles = [WinShot]::EnumByPid([uint32]$proc.Id)
if ($handles.Count -eq 0) {
    Write-Error "No visible window for this process"
    exit 3
}

# Pick the largest window and skip tiny tool windows.
$best = [IntPtr]::Zero
$bestArea = 0
$bestRect = New-Object WinShot+RECT

foreach ($h in $handles) {
    $r = New-Object WinShot+RECT
    if (-not [WinShot]::GetWindowRect($h, [ref]$r)) { continue }
    $w = $r.Right - $r.Left
    $hh = $r.Bottom - $r.Top
    if ($w -lt $MinWidth -or $hh -lt $MinHeight) { continue }
    $area = $w * $hh
    if ($area -gt $bestArea) {
        $bestArea = $area
        $best = $h
        $bestRect = $r
    }
}

if ($best -eq [IntPtr]::Zero) {
    Write-Error "No window larger than ${MinWidth}x${MinHeight}"
    exit 4
}

$w = $bestRect.Right - $bestRect.Left
$h2 = $bestRect.Bottom - $bestRect.Top
Write-Host "window '$([WinShot]::Title($best))' ${w}x${h2}"

[WinShot]::ShowWindow($best, 9) | Out-Null   # SW_RESTORE
[WinShot]::SetForegroundWindow($best) | Out-Null
Start-Sleep -Milliseconds $WaitMs

$bmp = New-Object System.Drawing.Bitmap($w, $h2)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()

# flags=2 means PW_RENDERFULLCONTENT; required for WinUI3 / DWM-composed windows.
$ok = [WinShot]::PrintWindow($best, $hdc, 2)

$g.ReleaseHdc($hdc)
$g.Dispose()

if (-not $ok) {
    Write-Warning "PrintWindow returned false; the image may be blank"
}

$dir = Split-Path -Parent $Out
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }

$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

Write-Host "saved $Out"
