# Verify that closing to tray keeps the daemon and mihomo alive.
#
# Sequence: send WM_CLOSE to the main window (exactly what the system X does),
# then watch the process table for a while.

param(
    [string]$ProcessName = "ClashZhengl",
    [int]$ObserveSec = 45
)

function Snapshot {
    $names = @("ClashZhengl", "clash-zhengl-daemon", "mihomo")
    $rows = @()
    foreach ($n in $names) {
        $p = Get-Process -Name $n -ErrorAction SilentlyContinue
        if ($p) { $rows += "$n=$($p.Id)" } else { $rows += "$n=GONE" }
    }
    return ($rows -join "  ")
}

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Closer2 {
  [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }

  public static IntPtr FindMain(uint pid) {
    IntPtr best = IntPtr.Zero; int bestArea = 0;
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) {
        RECT r; GetWindowRect(h, out r);
        int a = (r.R - r.L) * (r.B - r.T);
        if (a > bestArea) { bestArea = a; best = h; }
      }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
"@

$p = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { Write-Error "not running"; exit 2 }

$hwnd = [Closer2]::FindMain([uint32]$p.Id)
Write-Host "hwnd=$hwnd"
Write-Host ("BEFORE      " + (Snapshot))

[Closer2]::SendMessageW($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep -Seconds 3

Write-Host ("AFTER CLOSE " + (Snapshot))
Write-Host "window visible = $([Closer2]::IsWindowVisible($hwnd))"

for ($i = 1; $i -le $ObserveSec; $i += 15) {
    Start-Sleep -Seconds 15
    Write-Host ("+{0,3}s       {1}" -f ($i + 3), (Snapshot))
}

Write-Host "final window visible = $([Closer2]::IsWindowVisible($hwnd))"
