param(
  [string]$Root = 'D:\TomBox',
  [string]$TargetDir = 'D:\TomBox\target',
  [int]$Jobs = 6,
  [string]$Log = (Join-Path $env:TEMP 'tombox-release-build.log')
)

$ErrorActionPreference = 'Stop'
$env:CARGO_TARGET_DIR = $TargetDir
$sw = [Diagnostics.Stopwatch]::StartNew()
Push-Location $Root
try {
  cmd /c "cargo build --release --workspace --keep-going -j $Jobs --message-format=short > `"$Log`" 2>&1"
  $code = $LASTEXITCODE
} finally {
  Pop-Location
}
$lines = Get-Content $Log
$failed = @($lines | Select-String -Pattern 'could not compile `([^`]+)`' | ForEach-Object { $_.Matches[0].Groups[1].Value } | Sort-Object -Unique)
. (Join-Path $Root 'stage_compat.ps1')
$plan = @(Get-CompatDllPlan $Root '' | Where-Object { $_.Lib })
$release = Join-Path $TargetDir 'release'
$noDll = @($plan | Where-Object { -not (Test-Path (Join-Path $release $_.Built)) })
Write-Host ("cargo exit {0} after {1:N0}s; log {2}" -f $code, $sw.Elapsed.TotalSeconds, $Log)
Write-Host ("DLL crates: {0}, DLL present: {1}, failing crates: {2}" -f $plan.Count, ($plan.Count - $noDll.Count), $failed.Count)
if ($code -eq 0) { Write-Host '  all crates built' -ForegroundColor Green }
foreach ($f in $failed) {
  $first = $lines | Select-String -Pattern 'error(\[E\d+\])?:' | Where-Object { $_.Line -notmatch 'could not compile' } |
           Where-Object { $_.Line -match [regex]::Escape("crates\") } | ForEach-Object { $_.Line } |
           Where-Object { $_ -match "\\$([regex]::Escape($f))\\" } | Select-Object -First 1
  if (-not $first) {
    $idx = ($lines | Select-String -Pattern "could not compile ``$([regex]::Escape($f))``" | Select-Object -First 1).LineNumber
    $first = ($lines[0..([Math]::Max(0, $idx - 2))] | Select-String -Pattern '^error|: error' | Select-Object -Last 1).Line
  }
  Write-Host ("  FAIL {0}: {1}" -f $f, $first) -ForegroundColor Red
}
$other = @($lines | Select-String -Pattern '^error: (?!could not compile)' | ForEach-Object {
  $i = $_.LineNumber
  $why = ($lines[$i..([Math]::Min($lines.Count - 1, $i + 3))] | Where-Object { $_ -match '\S' -and $_ -notmatch '^Caused by' } | Select-Object -First 1)
  "$($_.Line.Substring(7)) ($($why.Trim()))"
})
foreach ($o in $other) { Write-Host ("  ERROR {0}" -f $o) -ForegroundColor Red }
foreach ($n in $noDll) { Write-Host ("  NO DLL {0} -> {1}" -f $n.Crate, $n.Built) -ForegroundColor Yellow }
if ($code -ne 0 -and -not ($failed.Count -or $other.Count)) { Write-Host '  cargo failed without a parsed error; read the log' -ForegroundColor Red }
if ($code -ne 0 -or $failed.Count -or $other.Count -or $noDll.Count) { exit 1 }
if ($env:TOMBOX_FILL_PACK -ne '0') { & (Join-Path $PSScriptRoot 'fill_pack.ps1') -TargetDir 'D:\TomBox\target' -Profile 'release' }
exit 0
