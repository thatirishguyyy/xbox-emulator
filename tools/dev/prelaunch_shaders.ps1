function Invoke-ShaderPrelaunch {
  param(
    [Parameter(Mandatory = $true)][string]$ExeStem,
    [string]$Name = '',
    [string]$ExeDir = '',
    [string]$Profile = 'release',
    [string]$Target = 'D:\xbox-pc-port\emulator\target\release',
    [string]$FastTarget = 'D:\TomBox\target-fast\dev-fast',
    [string]$LogDir = $env:TEMP,
    [switch]$NoUi
  )
  $ErrorActionPreference = 'Continue'
  if (-not $Name) { $Name = $ExeStem }
  $cache = if ($env:TOMBOX_SHADER_CACHE_DIR) { $env:TOMBOX_SHADER_CACHE_DIR } else { 'D:\xbox-pc-port\emulator\input\shader_cache' }
  $env:TOMBOX_SHADER_CACHE_DIR = $cache
  if (-not $env:TOMBOX_SHADER_DUMP) { $env:TOMBOX_SHADER_DUMP = Join-Path $cache 'dump' }
  Write-Host "  shader cache $cache; new shaders dumped to $($env:TOMBOX_SHADER_DUMP)\$ExeStem" -ForegroundColor DarkGray
  if ($env:TOMBOX_PRELAUNCH -eq '0') { Write-Host '  shader prelaunch off (TOMBOX_PRELAUNCH=0); shaders convert at runtime' -ForegroundColor DarkGray; return }
  $cands = @()
  if ($Profile -eq 'dev-fast') { $cands += (Join-Path $FastTarget 'xshader_prelaunch.exe') }
  $cands += (Join-Path $Target 'xshader_prelaunch.exe')
  $cands += 'D:\TomBox\target\release\xshader_prelaunch.exe'
  $cands += (Join-Path $FastTarget 'xshader_prelaunch.exe')
  $exe = $cands | Where-Object { Test-Path $_ } | Select-Object -First 1
  if (-not $exe) { Write-Host '  shader prelaunch: xshader_prelaunch.exe not built (cargo build -p tombox-xshader --bin xshader_prelaunch); shaders convert at runtime' -ForegroundColor Yellow; return }
  $dll = if ($ExeDir) { Join-Path $ExeDir 'd3d12_xs.dll' } else { Join-Path (Split-Path $exe -Parent) 'd3d12_xs.dll' }
  if ((Test-Path $dll) -and ((Get-Item $exe).LastWriteTime -lt (Get-Item $dll).LastWriteTime.AddMinutes(-2))) {
    Write-Host "  shader prelaunch: $exe is older than $dll; rebuild it (cargo build -p tombox-xshader --bin xshader_prelaunch) or its keys may not match the runtime translator" -ForegroundColor Yellow
  }
  $oldDxc = $env:TOMBOX_DXC_DIR
  if ($ExeDir -and (Test-Path (Join-Path $ExeDir 'dxcompiler.dll')) -and (Test-Path (Join-Path $ExeDir 'dxil.dll'))) { $env:TOMBOX_DXC_DIR = $ExeDir }
  elseif (-not $oldDxc) {
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $kit = Get-ChildItem $kits -Directory -EA 0 | ForEach-Object { Join-Path $_.FullName 'x64' } | Where-Object { (Test-Path (Join-Path $_ 'dxcompiler.dll')) -and (Test-Path (Join-Path $_ 'dxil.dll')) } | Sort-Object -Descending | Select-Object -First 1
    if ($kit) { $env:TOMBOX_DXC_DIR = $kit }
  }
  $a = @('--title', $ExeStem, '--name', $Name, '--yes')
  if ($NoUi -or $env:TOMBOX_PRELAUNCH_UI -eq '0') { $a += '--no-ui' }
  $sum = Join-Path $LogDir 'shader_prelaunch.txt'
  $a += @('--summary', $sum)
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $code = -1
  try {
    $out = & $exe @a 2>&1
    $code = $LASTEXITCODE
    foreach ($l in $out) { Write-Host "  $l" -ForegroundColor DarkGray }
  } catch {
    Write-Host "  shader prelaunch failed to run: $_; shaders convert at runtime" -ForegroundColor Yellow
  } finally {
    if ($null -eq $oldDxc) { Remove-Item Env:TOMBOX_DXC_DIR -EA 0 } else { $env:TOMBOX_DXC_DIR = $oldDxc }
  }
  $why = switch ($code) { 0 { 'done' } 3 { 'skipped by user; remaining shaders convert at runtime' } 2 { 'setup problem; shaders convert at runtime' } default { "exit $code; shaders convert at runtime" } }
  Write-Host ("  shader prelaunch {0} in {1:N1}s (summary {2})" -f $why, $sw.Elapsed.TotalSeconds, $sum) -ForegroundColor DarkGray
}
