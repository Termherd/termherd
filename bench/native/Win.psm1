# Window capture and process counters for windows.ps1; never sends input.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr dc);
  [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleBitmap(IntPtr dc, int w, int h);
  [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr dc, IntPtr o);
  [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr dc);
  [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr o);
  [DllImport("gdi32.dll")] static extern uint GetPixel(IntPtr dc, int x, int y);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  // Renders the client area into a new HBITMAP; the caller frees it.
  public static IntPtr Grab(IntPtr h) {
    RECT r; GetClientRect(h, out r);
    int w = Math.Max(1, r.R - r.L), ht = Math.Max(1, r.B - r.T);
    IntPtr sdc = GetDC(IntPtr.Zero), mdc = CreateCompatibleDC(sdc);
    IntPtr bmp = CreateCompatibleBitmap(sdc, w, ht), old = SelectObject(mdc, bmp);
    PrintWindow(h, mdc, 3);  // PW_CLIENTONLY | PW_RENDERFULLCONTENT
    SelectObject(mdc, old); DeleteDC(mdc); ReleaseDC(IntPtr.Zero, sdc);
    return bmp;
  }
  public static uint Pixel(IntPtr h, int x, int y) {
    IntPtr bmp = Grab(h), sdc = GetDC(IntPtr.Zero), mdc = CreateCompatibleDC(sdc);
    IntPtr old = SelectObject(mdc, bmp);
    uint px = GetPixel(mdc, x, y);
    SelectObject(mdc, old); DeleteDC(mdc); ReleaseDC(IntPtr.Zero, sdc); DeleteObject(bmp);
    return px;
  }
}
'@
Add-Type -AssemblyName System.Drawing

function Save-Window([IntPtr]$Hwnd, [string]$Path) {
  $hb = [W]::Grab($Hwnd)
  $bmp = [System.Drawing.Image]::FromHbitmap($hb)
  $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
}
# The process and all its descendants.
function Get-Tree([int]$RootPid) {
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId
  $ids = [System.Collections.Generic.List[int]]::new(); $ids.Add($RootPid)
  $i = 0
  while ($i -lt $ids.Count) {
    $all | Where-Object ParentProcessId -eq $ids[$i] | ForEach-Object { $ids.Add([int]$_.ProcessId) }
    $i++
  }
  $ids
}
# Working set is resident but trimmed at will; private bytes are committed.
function Get-Mem([int]$RootPid) {
  $app = Get-Process -Id $RootPid
  $tree = Get-Tree $RootPid | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }
  [pscustomobject]@{
    app_ws_mb    = [math]::Round($app.WorkingSet64 / 1MB, 1)
    app_priv_mb  = [math]::Round($app.PrivateMemorySize64 / 1MB, 1)
    tree_ws_mb   = [math]::Round(($tree | Measure-Object WorkingSet64 -Sum).Sum / 1MB, 1)
    tree_priv_mb = [math]::Round(($tree | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 1)
  }
}
function Get-CpuSec([int[]]$Pids) {
  ($Pids | ForEach-Object { (Get-Process -Id $_ -ErrorAction SilentlyContinue).TotalProcessorTime.TotalSeconds } | Measure-Object -Sum).Sum
}
Export-ModuleMember -Function Save-Window, Get-Tree, Get-Mem, Get-CpuSec
