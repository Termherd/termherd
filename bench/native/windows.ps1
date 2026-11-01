# Native S1 and S0 (idle, no session), beside the user's own instance through
# a private home and TMP. Needs a build whose lock follows TMP; an older one
# exits with code 2. No S2-S5: termherd ignores posted input.
param(
  [Parameter(Mandatory)][string]$Exe,         # termherd.exe to measure
  [int]$Reps = 3,
  [switch]$RealHistory,                       # copy ~/.claude/projects into the private home
  [string]$Out = (Join-Path $PSScriptRoot "..\results\native")
)
Import-Module (Join-Path $PSScriptRoot "Win.psm1") -Force
$iso = Join-Path $env:TEMP "termherd-bench-iso"
New-Item -ItemType Directory -Force "$iso\tmp", "$iso\home\.claude\projects", $Out | Out-Null
if ($RealHistory) { Copy-Item -Recurse -Force "$env:USERPROFILE\.claude\projects\*" "$iso\home\.claude\projects\" }

$rows = foreach ($r in 1..$Reps) {
  $psi = [Diagnostics.ProcessStartInfo]::new($Exe)
  $psi.UseShellExecute = $false; $psi.RedirectStandardError = $true; $psi.RedirectStandardOutput = $true
  $psi.Environment["TMP"] = "$iso\tmp"; $psi.Environment["TEMP"] = "$iso\tmp"
  $psi.Environment["USERPROFILE"] = "$iso\home"
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $p = [Diagnostics.Process]::Start($psi)
  while ($true) {                                 # S1: a sized window, first non-black pixel
    $p.Refresh()
    if ($p.HasExited) { throw "termherd exited with code $($p.ExitCode) (lock not isolated?)" }
    if ($p.MainWindowHandle -ne 0) {
      $rc = New-Object W+RECT; [W]::GetClientRect($p.MainWindowHandle, [ref]$rc) | Out-Null
      if (($rc.R - $rc.L) -ge 1000 -and [W]::Pixel($p.MainWindowHandle, 700, 200) -ne 0) { break }
    }
    Start-Sleep -Milliseconds 10
  }
  $s1 = $sw.Elapsed.TotalSeconds
  Save-Window $p.MainWindowHandle "$Out\s1-rep$r.png"
  $c0 = Get-CpuSec (Get-Tree $p.Id); $t0 = $sw.Elapsed.TotalSeconds
  Start-Sleep 30                                  # S0: idle, no session
  $c1 = Get-CpuSec (Get-Tree $p.Id); $t1 = $sw.Elapsed.TotalSeconds
  $m = Get-Mem $p.Id
  [pscustomobject]@{
    rep = $r; s1_startup_s = [math]::Round($s1, 3)
    s0_app_ws_mb = $m.app_ws_mb; s0_app_priv_mb = $m.app_priv_mb
    s0_tree_ws_mb = $m.tree_ws_mb; s0_tree_priv_mb = $m.tree_priv_mb
    s0_cpu_pct_of_core = [math]::Round(100 * ($c1 - $c0) / ($t1 - $t0), 2)
    logical_cores = [Environment]::ProcessorCount
    date_utc = (Get-Date).ToUniversalTime().ToString("s") + "Z"
    exe = $Exe; real_history = [bool]$RealHistory
  }
  Stop-Process -Id $p.Id -Force                   # only the process this script started
  Start-Sleep 3
}
$rows | Format-Table rep, s1_startup_s, s0_app_ws_mb, s0_app_priv_mb, s0_cpu_pct_of_core -AutoSize | Out-String -Width 200
$rows | ConvertTo-Json | Set-Content "$Out\native.json"
