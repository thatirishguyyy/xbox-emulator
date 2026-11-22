param(
  [string]$Exe = '',
  [int]$Jobs = 0,
  [switch]$Wait,
  [string]$TargetDir = 'D:\TomBox\target',
  [string]$Profile = 'release'
)
if (-not $Exe) {
  $env:CARGO_TARGET_DIR = $TargetDir
  Push-Location 'D:\TomBox'
  $eap = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
  $o = & cargo build --profile $Profile -p tombox-xshader --bin xshader_corpus 2>&1
  $ErrorActionPreference = $eap
  $code = $LASTEXITCODE
  Pop-Location
  if ($code -ne 0) { Write-Host 'fill_pack: xshader_corpus build failed; skipped' -ForegroundColor Yellow; $o | Select-Object -Last 5 | ForEach-Object { Write-Host "  $_" }; exit 0 }
  $Exe = Join-Path (Join-Path $TargetDir $Profile) 'xshader_corpus.exe'
}
$cands = @($Exe, 'D:\TomBox\target\release\xshader_corpus.exe', 'D:\TomBox\target-fast\dev-fast\xshader_corpus.exe') | Where-Object { $_ -and (Test-Path $_) }
if (-not $cands) { Write-Host 'fill_pack: xshader_corpus.exe not built; skipped' -ForegroundColor Yellow; exit 0 }
$corpus = $cands[0]
$running = Get-Process xshader_corpus,xshader_prelaunch -ErrorAction SilentlyContinue
if ($running) { Write-Host "fill_pack: already running (pid $($running[0].Id)); skipped" -ForegroundColor DarkGray; exit 0 }
if ($Jobs -le 0) { $Jobs = [Math]::Max(1, [Math]::Floor([Environment]::ProcessorCount / 4)) }
$log = Join-Path $env:TEMP 'tombox-fill-pack.log'
$cargs = @('--all-titles', '--fill-pack', '--jobs', "$Jobs")
$p = Start-Process -FilePath $corpus -ArgumentList $cargs -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError "$log.err" -PassThru
try { $p.PriorityClass = 'BelowNormal' } catch {}
Write-Host ("fill_pack: {0} {1} started (pid {2}, below-normal priority); log {3}" -f $corpus, ($cargs -join ' '), $p.Id, $log) -ForegroundColor DarkGray
if ($Wait) { $p.WaitForExit(); Write-Host "fill_pack: exit $($p.ExitCode)" }
exit 0
