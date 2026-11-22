param(
  [string[]]$Crates = @(),
  [switch]$All,
  [ValidateSet('dev-fast', 'release')][string]$Profile = 'dev-fast',
  [string]$TargetDir = '',
  [string]$ReleaseDir = 'D:\TomBox\target\release',
  [string]$Root = 'D:\TomBox',
  [int]$Jobs = 6,
  [switch]$Lld,
  [switch]$NoBuild,
  [switch]$NoStage,
  [switch]$DryRun,
  [string[]]$Overlays = @(
    'D:\xbox-pc-port\emulator\run\Mount\FortniteGame\Binaries\XSX',
    'D:\xbox-pc-port\emulator\run-xdefiant\Mount',
    'D:\xbox-pc-port\emulator\run-minecraft\Mount',
    'D:\xbox-pc-port\emulator\run-xone\Trackmania\Mount',
    'D:\xbox-pc-port\emulator\run-xone\Terraria\Mount'
  ),
  [string[]]$Protected = @('D:\xbox-pc-port\games'),
  [string[]]$TitleProcs = @('GbxGame', 'Rogue_scarlett_rtx', 'Terraria', 'FortniteClient*', 'Minecraft*'),
  [string]$Log = (Join-Path $env:TEMP 'tombox-quick-build.log')
)

$ErrorActionPreference = 'Stop'
. (Join-Path $Root 'stage_compat.ps1')

function Say([string]$m, [string]$c = 'Gray') { Write-Host $m -ForegroundColor $c }

if (-not $TargetDir) {
  $TargetDir = if ($Profile -eq 'release') { 'D:\TomBox\target' } elseif ($Lld) { 'D:\TomBox\target-fast-lld' } else { 'D:\TomBox\target-fast' }
}
$OutDir = Join-Path $TargetDir $Profile

$sw = [Diagnostics.Stopwatch]::StartNew()
Push-Location $Root
try { $metaJson = cmd /c 'cargo metadata --format-version 1 --offline 2>nul' } finally { Pop-Location }
if ($LASTEXITCODE -ne 0 -or -not $metaJson) { throw 'cargo metadata failed (run cargo metadata in the workspace to see why)' }
Add-Type -AssemblyName System.Web.Extensions
$ser = New-Object System.Web.Script.Serialization.JavaScriptSerializer
$ser.MaxJsonLength = [int]::MaxValue
$meta = $ser.DeserializeObject(($metaJson -join "`n"))
$members = @{}
foreach ($m in $meta['workspace_members']) { $members[$m] = $true }

$pkgs = @{}
foreach ($p in $meta['packages']) {
  if ($null -ne $p['source']) { continue }
  $dir = Split-Path $p['manifest_path'] -Parent
  $lib = ''
  foreach ($t in $p['targets']) { if (@($t['crate_types']) -contains 'cdylib') { $lib = $t['name'] -replace '-', '_' } }
  $deps = @()
  foreach ($d in $p['dependencies']) { if ($d['path'] -and $d['kind'] -ne 'dev') { $deps += $d['name'] } }
  $pkgs[$p['name']] = [pscustomobject]@{ Name = $p['name']; Dir = $dir; Lib = $lib; Deps = @($deps | Sort-Object -Unique); Member = $members.ContainsKey($p['id']) }
}

$rdeps = @{}
foreach ($p in $pkgs.Values) { foreach ($d in $p.Deps) { if (-not $rdeps.ContainsKey($d)) { $rdeps[$d] = @() }; $rdeps[$d] += $p.Name } }

function Get-Dependents([string]$name) {
  $seen = @{ $name = $true }
  $q = New-Object System.Collections.Queue
  $q.Enqueue($name)
  while ($q.Count) {
    $n = $q.Dequeue()
    if ($rdeps.ContainsKey($n)) { foreach ($r in $rdeps[$n]) { if (-not $seen.ContainsKey($r)) { $seen[$r] = $true; $q.Enqueue($r) } } }
  }
  return @($seen.Keys)
}

$newestCache = @{}
function Get-NewestSource([string]$name) {
  if ($newestCache.ContainsKey($name)) { return $newestCache[$name] }
  $p = $pkgs[$name]
  $t = [datetime]::MinValue
  $skip = '\\(target[^\\]*|tests|examples|benches|\.git)(\\|$)'
  foreach ($f in [IO.Directory]::EnumerateFiles($p.Dir, '*', [IO.SearchOption]::AllDirectories)) {
    $rel = $f.Substring($p.Dir.Length)
    if ($rel -match $skip) { continue }
    $w = [IO.File]::GetLastWriteTimeUtc($f)
    if ($w -gt $t) { $t = $w }
  }
  $newestCache[$name] = $t
  return $t
}

function Get-ClosureNewest([string]$name) {
  $seen = @{}
  $stack = New-Object System.Collections.Stack
  $stack.Push($name)
  $t = [datetime]::MinValue
  $why = ''
  while ($stack.Count) {
    $n = $stack.Pop()
    if ($seen.ContainsKey($n) -or -not $pkgs.ContainsKey($n)) { continue }
    $seen[$n] = $true
    $w = Get-NewestSource $n
    if ($w -gt $t) { $t = $w; $why = $n }
    foreach ($d in $pkgs[$n].Deps) { $stack.Push($d) }
  }
  return [pscustomobject]@{ Time = $t; Crate = $why }
}

function Get-BuiltTime([string]$lib) {
  $best = [datetime]::MinValue
  foreach ($d in @($OutDir, $ReleaseDir) | Select-Object -Unique) {
    $f = Join-Path $d "$lib.dll"
    if (Test-Path -LiteralPath $f) { $w = [IO.File]::GetLastWriteTimeUtc($f); if ($w -gt $best) { $best = $w } }
  }
  return $best
}

$dllPkgs = @($pkgs.Values | Where-Object { $_.Lib -and $_.Member })
$build = [ordered]@{}
if ($All) {
  foreach ($p in $dllPkgs) { $build[$p.Name] = 'all DLL crates' }
} elseif ($Crates.Count) {
  foreach ($c in ($Crates | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })) {
    if (-not $pkgs.ContainsKey($c)) { throw "unknown crate '$c' (package names as in Cargo.toml, e.g. d3d12_xs, tombox-xshader)" }
    if (-not $pkgs[$c].Member) { $build[$c] = 'requested' }
    if ($pkgs[$c].Member -and -not $pkgs[$c].Lib) { $build[$c] = 'requested' }
    foreach ($d in Get-Dependents $c) {
      if ($pkgs[$d].Lib -and $pkgs[$d].Member -and -not $build.Contains($d)) { $build[$d] = if ($d -eq $c) { 'requested' } else { "depends on $c" } }
    }
  }
  foreach ($k in @($build.Keys)) { if (-not $pkgs[$k].Member) { $build.Remove($k) } }
} else {
  foreach ($p in $dllPkgs) {
    $src = Get-ClosureNewest $p.Name
    $dll = Get-BuiltTime $p.Lib
    if ($src.Time -gt $dll) {
      $build[$p.Name] = if ($dll -eq [datetime]::MinValue) { 'never built' } else { "$($src.Crate) changed $([int]($src.Time - $dll).TotalMinutes) min after the newest $($p.Lib).dll" }
    }
  }
}
Say ("quick: {0} crate(s) selected in {1:N1}s ({2}; target {3})" -f $build.Count, $sw.Elapsed.TotalSeconds, $Profile, $OutDir) 'Cyan'
foreach ($k in $build.Keys) { Say ("  {0,-28} {1}" -f $k, $build[$k]) 'DarkGray' }
if (-not $build.Count) { Say '  nothing changed since the newest built DLLs; nothing to do' 'Green'; exit 0 }

$failed = @()
$started = Get-Date
if (-not $NoBuild -and -not $DryRun) {
  $env:CARGO_TARGET_DIR = $TargetDir
  if ($Lld) { $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = 'rust-lld.exe' } else { Remove-Item Env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER -ErrorAction SilentlyContinue }
  $pargs = ($build.Keys | ForEach-Object { "-p $_" }) -join ' '
  $prof = if ($Profile -eq 'release') { '--release' } else { "--profile $Profile" }
  $bw = [Diagnostics.Stopwatch]::StartNew()
  Push-Location $Root
  try {
    cmd /c "cargo build $prof --keep-going -j $Jobs --message-format=short $pargs > `"$Log`" 2>&1"
    $code = $LASTEXITCODE
  } finally { Pop-Location }
  $lines = Get-Content $Log
  $failed = @($lines | Select-String -Pattern 'could not compile `([^`]+)`' | ForEach-Object { $_.Matches[0].Groups[1].Value } | Sort-Object -Unique)
  Say ("cargo exit {0} after {1:N1}s; log {2}" -f $code, $bw.Elapsed.TotalSeconds, $Log) $(if ($code -eq 0) { 'Green' } else { 'Red' })
  foreach ($e in ($lines | Where-Object { $_ -match '(^|: )error(\[E\d+\])?:' -and $_ -notmatch 'could not compile' } | Select-Object -First 12)) { Say "  $e" 'Red' }
  if ($lines | Select-String -SimpleMatch 'is not allowed in .drectve') { Say '  rust-lld cannot link crates whose build.rs passes a lib.exe .exp file; build those without -Lld' 'Yellow' }
}

if ($NoStage) { if ($failed.Count) { exit 1 }; exit 0 }

$running = @(Get-Process -Name $TitleProcs -ErrorAction SilentlyContinue)
if ($running.Count) { Say ("running titles: {0}" -f (($running | ForEach-Object { "$($_.ProcessName) pid $($_.Id)" }) -join ', ')) 'Yellow' }

$rows = @()
foreach ($k in $build.Keys) {
  $p = $pkgs[$k]
  if (-not $p.Lib) { continue }
  if ($failed -contains $k) { $rows += [pscustomobject]@{ Dll = "$($p.Lib).dll"; Overlay = '-'; Action = 'skip'; Why = 'build failed' }; continue }
  $from = Join-Path $OutDir "$($p.Lib).dll"
  if (-not (Test-Path -LiteralPath $from)) { $rows += [pscustomobject]@{ Dll = "$($p.Lib).dll"; Overlay = '-'; Action = 'missing'; Why = "$from not built" }; continue }
  if (-not $NoBuild -and -not $DryRun -and (Get-Item -LiteralPath $from).LastWriteTime -lt $started.AddSeconds(-2)) {
    $why = 'up to date (cargo relinked nothing)'
  } else { $why = '' }
  $key = Get-NormalDllKey $p.Lib
  foreach ($o in $Overlays) {
    if (-not (Test-Path -LiteralPath $o)) { continue }
    $hits = @(Get-ChildItem -LiteralPath $o -Filter *.dll -File -ErrorAction SilentlyContinue | Where-Object { (Get-NormalDllKey $_.Name) -eq $key })
    foreach ($h in $hits) {
      $short = Split-Path (Split-Path $o -Parent) -Leaf
      if ($h.LinkType -eq 'HardLink') { $rows += [pscustomobject]@{ Dll = $h.Name; Overlay = $o; Action = 'skip'; Why = 'hard link into the title dump (the title ships its own copy)' }; continue }
      try { Assert-OutsideDump $o $Protected } catch { $rows += [pscustomobject]@{ Dll = $h.Name; Overlay = $o; Action = 'skip'; Why = $_.Exception.Message }; continue }
      if ([IO.File]::GetLastWriteTimeUtc($h.FullName) -eq [IO.File]::GetLastWriteTimeUtc($from) -and $h.Length -eq (Get-Item -LiteralPath $from).Length) {
        $rows += [pscustomobject]@{ Dll = $h.Name; Overlay = $o; Action = 'same'; Why = 'already this build' }; continue
      }
      $locked = $false
      try { $fs = [IO.File]::Open($h.FullName, 'Open', 'ReadWrite', 'None'); $fs.Close() } catch { $locked = $true }
      if ($locked) {
        $who = ($running | ForEach-Object { "$($_.ProcessName) pid $($_.Id)" }) -join ', '
        $rows += [pscustomobject]@{ Dll = $h.Name; Overlay = $o; Action = 'LOCKED'; Why = "in use$(if ($who) { " (running: $who)" }); stop the title first, not replaced" }
        continue
      }
      if (-not $DryRun) { Copy-IntoOverlay $from $o $h.Name $Protected }
      $rows += [pscustomobject]@{ Dll = $h.Name; Overlay = $o; Action = $(if ($DryRun) { 'would' } else { 'staged' }); Why = $why }
    }
  }
}
foreach ($r in $rows) {
  $c = switch ($r.Action) { 'staged' { 'Green' } 'LOCKED' { 'Red' } 'missing' { 'Red' } default { 'DarkGray' } }
  Say ("  {0,-7} {1,-28} {2} {3}" -f $r.Action, $r.Dll, $r.Overlay, $r.Why) $c
}
$nStaged = @($rows | Where-Object { $_.Action -eq 'staged' }).Count
$nLocked = @($rows | Where-Object { $_.Action -eq 'LOCKED' }).Count
Say ("quick done in {0:N1}s: {1} built, {2} failed, {3} copies staged, {4} locked" -f $sw.Elapsed.TotalSeconds, ($build.Count - $failed.Count), $failed.Count, $nStaged, $nLocked) $(if ($failed.Count -or $nLocked) { 'Yellow' } else { 'Green' })
if ($Profile -ne 'release') { Say "  launch with the run script's -Profile $Profile (or batch_test.ps1 -Profile $Profile) so it stages these instead of target\release" 'DarkGray' }
if ($failed.Count) { exit 1 }
if ($env:TOMBOX_FILL_PACK -ne '0') { & (Join-Path $PSScriptRoot 'fill_pack.ps1') -TargetDir $TargetDir -Profile $Profile }
if ($nLocked) { exit 1 }
exit 0
