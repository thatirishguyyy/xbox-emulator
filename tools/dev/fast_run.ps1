param(
  [ValidateSet('trackmania', 'terraria', 'fortnite', 'minecraft', 'xdefiant')][string]$Title = 'trackmania',
  [string]$Scene = '',
  [string]$Target = 'D:\TomBox\target-fast\dev-fast',
  [int]$Seconds = 0,
  [hashtable]$Env = @{},
  [string]$PadScript = 'default',
  [string]$GameArgs = 'default',
  [string]$MapFile = 'none',
  [int]$AfterScene = -1,
  [int]$CaptureEvery = 15,
  [string]$Id = '',
  [string]$Out = '',
  [int]$LockWaitMinutes = 45,
  [switch]$NoWaitUser,
  [string]$Seed = 'default',
  [switch]$ListScenes
)

$ErrorActionPreference = 'Stop'
$Root = 'D:\TomBox'
$Dev = Join-Path $Root 'tools\dev'
$Runs = Join-Path $Dev 'batch_runs'
$TestRoot = 'D:\xbox-pc-port\emulator\run-xone-test'
$Screens = Join-Path $Dev 'screens'

function Scr([string]$Name, [string]$File, [int]$X, [int]$Y, [int]$W, [int]$H, [int]$Hi = 215, [int]$CapH = 1080) {
  return @{ Name = $Name; Ref = (Join-Path $Screens $File); X = $X; Y = $Y; W = $W; H = $H; CapW = 1920; CapH = $CapH; Hi = $Hi; Min = 0.8 }
}

$tmDrive = "/init challenge 'Training - 01'/+12:press a;+3:press a;+2:down rt+a;+10:stick l 0.5 0 800;+6:stick l -0.5 0 800"
$tmToTraining = "0:connect;#start+1:press a;?#offline+1:press a;#play+2:press a;#solo+2:press down;+1.5:press left;+1.5:press a;+4:press a;+3:press a"
$AllSpecs = @{
  trackmania = [pscustomobject]@{
    RunTitle = 'Trackmania'; Proc = 'GbxGame'; Drive = 'R:'; Main = 'training01_drive'; Seed = ''
    PadScreens = @(
      (Scr 'start' 'trackmania_start.png' 840 795 250 60),
      (Scr 'offline' 'trackmania_play_offline.png' 760 828 400 50),
      (Scr 'play' 'trackmania_menu_play.png' 250 262 140 50),
      (Scr 'solo' 'trackmania_solo_training.png' 195 630 205 42 180))
    Scenes = [ordered]@{
      boot = @{ Pad = '0:connect'; Rx = '\[Game\] main menu'; Screen = ''; Timeout = 60; After = 3 }
      menu = @{ Pad = "0:connect;#start+1:press a;?#offline+1:press a"; Rx = ''; Screen = 'play'; Timeout = 90; After = 3 }
      training01 = @{ Pad = $tmToTraining; Rx = "init challenge 'Training - 01'"; Screen = ''; Timeout = 120; After = 8 }
      training01_drive = @{ Pad = "$tmToTraining;$tmDrive"; Rx = "init challenge 'Training - 01'"; Screen = ''; Motion = 'down rt'; Timeout = 150; After = 2 }
    }
    Milestones = @(
      @{ Name = 'main menu (engine)'; Src = 'ods'; Rx = [regex]'\[Game\] main menu' },
      @{ Name = 'login request'; Src = 'compat'; Rx = [regex]'/v3/profiles/sessions' },
      @{ Name = 'map load'; Src = 'ods'; Rx = [regex]'init challenge' })
    GameLog = (Join-Path $TestRoot 'Trackmania\gdkroot\pls\700107ED\log.txt')
  }
  terraria = [pscustomobject]@{
    RunTitle = 'Terraria'; Proc = 'Terraria'; Drive = 'Q:'; Main = 'world'; Seed = (Join-Path $TestRoot 'seeds\Terraria')
    PadScreens = @(
      (Scr 'title' 'terraria_menu_version.png' 12 1034 256 36),
      (Scr 'selplayer' 'terraria_select_player.png' 860 195 200 40),
      (Scr 'selworld' 'terraria_select_world.png' 860 195 200 40),
      (Scr 'inworld' 'terraria_world_hud.png' 830 1018 260 28))
    Scenes = [ordered]@{
      menu = @{ Pad = '0:connect'; Rx = ''; Screen = 'title'; Timeout = 60; After = 2 }
      world = @{ Pad = '0:connect;#title+0.3:press a;#selplayer+0.3:press a;#selworld+0.3:press a'; Rx = ''; Screen = 'inworld'; Timeout = 90; After = 3 }
    }
    Milestones = @(@{ Name = 'save read'; Src = 'compat'; Rx = [regex]'XGameSave' })
    GameLog = ''
  }
  fortnite = [pscustomobject]@{
    RunTitle = 'Fortnite'; Proc = 'FortniteClient-XSX-Shipping'; Drive = 'K:'; Main = 'lobby'; Seed = ''
    PadScreens = @((Scr 'start' 'fortnite_press_start.png' 800 745 370 60), (Scr 'safezone' 'fortnite_safe_zone.png' 620 470 680 55))
    Scenes = [ordered]@{
      lobby = @{ Pad = '0:connect;/Setting user focus to widget Splash_Screen/+2:press a;?#safezone+1:press a;/to \[UI\.State\.Startup\.SubgameSelect/+4:press a'; Rx = 'to \[UI\.State\.Athena\.Frontend'; Screen = ''; Timeout = 240; After = 5 }
    }
    Milestones = @(
      @{ Name = 'login'; Src = 'ods'; Rx = [regex]'HandleLoginRequestComplete.*Result=2' },
      @{ Name = 'hotfix check'; Src = 'ods'; Rx = [regex]'CheckComplete UpdateSuccess' },
      @{ Name = 'game-mode select'; Src = 'ods'; Rx = [regex]'UI\.State\.Startup\.SubgameSelect\]' })
    GameLog = ''
  }
  minecraft = [pscustomobject]@{
    RunTitle = 'Minecraft'; Proc = 'Minecraft.Windows'; Drive = 'N:'; Main = 'menu'; Seed = ''
    PadScreens = @(
      (Scr 'welcome' 'minecraft_welcome.png' 675 340 260 32 200 1049))
    Scenes = [ordered]@{
      welcome = @{ Pad = '0:connect'; Rx = ''; Screen = 'welcome'; Timeout = 120; After = 1 }
      menu = @{ Pad = '0:connect;#welcome+1:press a'; Rx = ''; Screen = ''; ArmAfter = 'press a'; NotScreen = 'welcome'; Timeout = 150; After = 3 }
    }
    Milestones = @(
      @{ Name = 'graphics context'; Src = 'ods'; Rx = [regex]'graphics context was gained' },
      @{ Name = 'accessibility query'; Src = 'compat'; Rx = [regex]'XAccessibility' })
    GameLog = ''
  }
  xdefiant = [pscustomobject]@{
    RunTitle = 'XDefiant'; Proc = 'Rogue_scarlett_rtx'; Drive = 'Y:'; Main = 'menu'; Seed = ''
    PadScreens = @()
    Scenes = [ordered]@{
      menu = @{ Pad = '0:connect'; Rx = ''; Screen = ''; Timeout = 240; After = 5 }
    }
    Milestones = @()
    GameLog = ''
  }
}
if ($ListScenes) {
  foreach ($k in $AllSpecs.Keys) { "{0}: {1} (main {2})" -f $k, ($AllSpecs[$k].Scenes.Keys -join ', '), $AllSpecs[$k].Main }
  return
}
$Spec = $AllSpecs[$Title]
if ($Seed -ne 'default') { $Spec.Seed = $Seed }
if (-not $Scene -or $Scene -eq 'main') { $Scene = $Spec.Main }
if (-not $Spec.Scenes.Contains($Scene)) { throw "scene '$Scene' is not defined for $Title (scenes: $($Spec.Scenes.Keys -join ', '))" }
$Sc = $Spec.Scenes[$Scene]
if ($Seconds -le 0) { $Seconds = $Sc.Timeout }
if ($AfterScene -lt 0) { $AfterScene = $Sc.After }
if ($PadScript -eq 'default') { $PadScript = $Sc.Pad }
if ($GameArgs -eq 'default') { $GameArgs = '' }
$SceneRx = if ($Sc.Rx) { [regex]$Sc.Rx } else { $null }
$SceneScreen = if ($Sc.Screen) { $Spec.PadScreens | Where-Object { $_.Name -eq $Sc.Screen } | Select-Object -First 1 } else { $null }
if ($Sc.Screen -and -not $SceneScreen) { throw "scene screen '$($Sc.Screen)' has no reference for $Title" }
$MotionCmd = if ($Sc.Contains('Motion')) { [string]$Sc.Motion } else { '' }
$ArmCmd = if ($Sc.Contains('ArmAfter')) { [string]$Sc.ArmAfter } else { '' }
$NotScreen = if ($Sc.Contains('NotScreen')) { $Spec.PadScreens | Where-Object { $_.Name -eq $Sc.NotScreen } | Select-Object -First 1 } else { $null }
if ($Sc.Contains('NotScreen') -and -not $NotScreen) { throw "scene NotScreen '$($Sc.NotScreen)' has no reference for $Title" }

if (-not $Id) { $Id = "pid$PID" }
$Stamp = Get-Date -Format 'yyyyMMdd_HHmmss'
if (-not $Out) { $Out = Join-Path $Runs ("fast_{0}_{1}_{2}" -f $Stamp, $Title, ($Id -replace '[^A-Za-z0-9_-]', '')) }
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$Live = Join-Path $Out 'live'
$CapDir = Join-Path $Out 'capture'
$PadDir = Join-Path $Out 'pad'
foreach ($d in $Live, $CapDir, $PadDir) { New-Item -ItemType Directory -Force -Path $d | Out-Null }
$Transcript = Join-Path $Out 'fast_run.txt'

function Say([string]$m, [string]$c = 'Gray') {
  $line = "[{0:HH:mm:ss}] {1}" -f (Get-Date), $m
  Write-Host $line -ForegroundColor $c
  Add-Content -LiteralPath $Transcript -Value $line -Encoding utf8
}

$bt = [IO.File]::ReadAllText((Join-Path $Dev 'batch_test.ps1'))
foreach ($ty in 'TbAudio', 'TbFrame2') {
  if ($ty -as [type]) { continue }
  $m = [regex]::Match($bt, "(?s)if \(-not \('$ty' -as \[type\]\)\) \{ Add-Type.*?\n'@ \}")
  if (-not $m.Success) { throw "could not find the $ty type block in batch_test.ps1" }
  Invoke-Expression $m.Value
}
. (Join-Path $Dev 'frame_sig.ps1')
if (-not ('TbDiff' -as [type])) { Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
public static class TbDiff {
    public static double Changed(string a, string b) {
        using (var x = new Bitmap(a))
        using (var y = new Bitmap(b)) {
            if (x.Width != y.Width || x.Height != y.Height) return 1.0;
            int n = 0, hit = 0;
            for (int j = 0; j < x.Height; j += 6) {
                for (int i = 0; i < x.Width; i += 6) {
                    Color p = x.GetPixel(i, j), q = y.GetPixel(i, j);
                    int d = Math.Abs(p.R - q.R) + Math.Abs(p.G - q.G) + Math.Abs(p.B - q.B);
                    n++;
                    if (d > 48) hit++;
                }
            }
            return n > 0 ? (double)hit / n : 0;
        }
    }
}
'@ }

if ((Test-Path -LiteralPath (Join-Path $Target 'dev-fast')) -and -not (Test-Path -LiteralPath (Join-Path $Target 'd3d12_xs.dll'))) { $Target = Join-Path $Target 'dev-fast' }
if (-not (Test-Path -LiteralPath (Join-Path $Target 'd3d12_xs.dll'))) { throw "no d3d12_xs.dll in -Target $Target (build first: tools\dev\quick.ps1 -NoStage)" }
$isRelease = (Split-Path $Target -Leaf) -eq 'release'

$userPlaying = Join-Path $Runs 'USER_PLAYING'
$lockSlots = @((Join-Path $Runs 'GAME_LOCK'), (Join-Path $Runs 'GAME_LOCK_2'))
$overlayLock = Join-Path $Runs ("TEST_OVERLAY_{0}.lock" -f $Title)
$held = New-Object System.Collections.Generic.List[string]
$lockBody = "id=$Id pid=$PID title=$Title out=$Out started=$(Get-Date -Format s)"

function Test-UserPlaying {
  if (-not (Test-Path -LiteralPath $userPlaying)) { return $false }
  $txt = try { Get-Content -LiteralPath $userPlaying -Raw -ErrorAction Stop } catch { '' }
  $pids = @([regex]::Matches([string]$txt, '(?:script|game)_pid=(\d+)') | ForEach-Object { [int]$_.Groups[1].Value } | Where-Object { $_ -gt 0 })
  if (-not $pids.Count) { return $true }
  return @($pids | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }).Count -gt 0
}

function Clear-StaleLock([string]$Path) {
  $txt = try { Get-Content -LiteralPath $Path -Raw -ErrorAction Stop } catch { return }
  $m = [regex]::Match([string]$txt, '(?<![_a-z])pid=(\d+)')
  if (-not $m.Success) { return }
  $owner = [int]$m.Groups[1].Value
  if ($owner -le 0 -or $owner -eq $PID) { return }
  if (Get-Process -Id $owner -ErrorAction SilentlyContinue) { return }
  $age = try { ((Get-Date) - (Get-Item -LiteralPath $Path -ErrorAction Stop).LastWriteTime).TotalSeconds } catch { 0 }
  if ($age -lt 10) { return }
  Say "  stale lock $Path (owner pid $owner dead): taking over" 'Yellow'
  Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
}

function Get-Lock([string]$Path) {
  if (Test-Path -LiteralPath $Path) { Clear-StaleLock $Path }
  try { $fs = [IO.File]::Open($Path, 'CreateNew', 'Write', 'None'); $b = [Text.Encoding]::UTF8.GetBytes($lockBody); $fs.Write($b, 0, $b.Length); $fs.Close(); $held.Add($Path); return $true } catch { return $false }
}

function Remove-MyLocks {
  foreach ($l in @($held)) {
    $txt = try { Get-Content -LiteralPath $l -Raw -ErrorAction Stop } catch { '' }
    if ($txt -and $txt.StartsWith("id=$Id pid=$PID ")) { Remove-Item -LiteralPath $l -Force -ErrorAction SilentlyContinue }
  }
  $held.Clear()
}

function Get-Tree([int]$RootPid) {
  $all = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId)
  $set = New-Object System.Collections.Generic.List[int]
  $set.Add($RootPid)
  $grew = $true
  while ($grew) {
    $grew = $false
    foreach ($p in $all) { if ($set.Contains([int]$p.ParentProcessId) -and -not $set.Contains([int]$p.ProcessId)) { $set.Add([int]$p.ProcessId); $grew = $true } }
  }
  return $set
}

function Read-New($st) {
  if (-not (Test-Path -LiteralPath $st.Path)) { return @() }
  $fs = [IO.File]::Open($st.Path, 'Open', 'Read', 'ReadWrite, Delete')
  try {
    if ($fs.Length -lt $st.Pos) { $st.Pos = 0; $st.Rest = '' }
    $len = [int][Math]::Min($fs.Length - $st.Pos, 64MB)
    if ($len -le 0) { return @() }
    [void]$fs.Seek($st.Pos, 'Begin')
    $buf = New-Object byte[] $len
    $n = $fs.Read($buf, 0, $len)
    $st.Pos += $n
  } finally { $fs.Close() }
  $parts = ($st.Rest + [Text.Encoding]::UTF8.GetString($buf, 0, $n)) -split "`n"
  $st.Rest = $parts[-1]
  if ($parts.Count -lt 2) { return @() }
  return $parts[0..($parts.Count - 2)]
}

function Reset-TestGdkRoot([string]$Dir, [string]$Seed) {
  if (-not $Dir.StartsWith($TestRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw "refusing to wipe $Dir (outside $TestRoot)" }
  if (Test-Path -LiteralPath $Dir) { Remove-Item -LiteralPath $Dir -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $Dir | Out-Null
  $social = Join-Path $env:LOCALAPPDATA 'XboxEmulator\social'
  if (Test-Path -LiteralPath $social) { Copy-Item -LiteralPath $social -Destination (Join-Path $Dir 'social') -Recurse }
  if ($Seed -and (Test-Path -LiteralPath $Seed)) {
    foreach ($c in Get-ChildItem -LiteralPath $Seed -Force) { Copy-Item -LiteralPath $c.FullName -Destination $Dir -Recurse -Force }
    return @(Get-ChildItem -LiteralPath $Seed -Recurse -File).Count
  }
  return 0
}

$frames = New-Object System.Collections.Generic.List[string]
function Save-Frame([uint32[]]$Pids, [double]$El, [string]$Tag) {
  $png = Join-Path $Out ("frame_{0:000.0}s{1}.png" -f $El, $Tag)
  try { $s = [TbFrame2]::Grab($Pids, $png) } catch { Say "  frame +$([int]$El)s failed: $($_.Exception.Message)" 'Yellow'; return $null }
  if (-not $s) { return $null }
  if ($s.Verdict -in 'hung', 'no present', 'capture error') { Say ("  +{0:N1}s frame: {1} ({2})" -f $El, $s.Verdict, $s.Method) 'DarkGray'; return $null }
  $frames.Add(("{0} [{1}, mean {2:N0}]" -f $png, $s.Verdict, $s.Mean))
  Say ("  +{0:N1}s frame {1}: {2} mean {3:N1} std {4:N1}" -f $El, (Split-Path $png -Leaf), $s.Verdict, $s.Mean, $s.Std) $(if ($s.Drawn) { 'Green' } else { 'Yellow' })
  return [pscustomobject]@{ Png = $png; Drawn = [bool]$s.Drawn; Verdict = $s.Verdict }
}

function Get-Probe([uint32[]]$Pids, [string]$Png) {
  try { $s = [TbFrame2]::Grab($Pids, $Png) } catch { return $null }
  if (-not $s -or $s.Verdict -in 'hung', 'no present', 'capture error') { return $null }
  return [pscustomobject]@{ Png = $Png; Drawn = [bool]$s.Drawn; Verdict = $s.Verdict }
}

function Get-ScreenScore($Def, [string]$Png) {
  if (-not $Def -or -not (Test-Path -LiteralPath $Def.Ref)) { return $null }
  try { return [TbSig]::Score($Png, $Def.Ref, $Def.X, $Def.Y, $Def.W, $Def.H, $Def.CapW, $Def.CapH, $Def.Hi, 60)[0] } catch { return $null }
}

$padQueue = New-Object System.Collections.ArrayList
foreach ($e in ($PadScript -split ';')) {
  if ($e -match '^\s*(\d+(?:\.\d+)?)\s*:\s*(.+?)\s*$') { [void]$padQueue.Add([pscustomobject]@{ At = [double]$Matches[1]; Cmd = $Matches[2]; Rx = $null; Screen = ''; Delay = 0.0; Prev = $false }) }
  elseif ($e -match '^\s*/(.+?)/\+?(\d+(?:\.\d+)?)\s*:\s*(.+?)\s*$') { [void]$padQueue.Add([pscustomobject]@{ At = $null; Cmd = $Matches[3]; Rx = [regex]$Matches[1]; Screen = ''; Delay = [double]$Matches[2]; Prev = $false }) }
  elseif ($e -match '^\s*(\?)?#([a-z0-9_]+)\+?(\d+(?:\.\d+)?)\s*:\s*(.+?)\s*$') { [void]$padQueue.Add([pscustomobject]@{ At = $null; Cmd = $Matches[4]; Rx = $null; Screen = $Matches[2]; Delay = [double]$Matches[3]; Prev = $false; Opt = [bool]$Matches[1] }) }
  elseif ($e -match '^\s*\+(\d+(?:\.\d+)?)\s*:\s*(.+?)\s*$') { [void]$padQueue.Add([pscustomobject]@{ At = $null; Cmd = $Matches[2]; Rx = $null; Screen = ''; Delay = [double]$Matches[1]; Prev = $true }) }
  elseif ($e.Trim()) { throw "PadScript entry not understood: '$e' (use 'sec:cmd', '/regex/+sec:cmd', '#screen+sec:cmd' or '+sec:cmd')" }
}

Say "fast_run $Title scene=$Scene target=$Target seconds=$Seconds after=$AfterScene id=$Id -> $Out" 'Cyan'
$need = @()
if ($SceneRx) { $need += "log /$SceneRx/" }
if ($SceneScreen) { $need += "screen #$($SceneScreen.Name)" }
$need += 'drawn frame'
if ($MotionCmd) { $need += "motion after pad '$MotionCmd'" }
if ($ArmCmd) { $need[-1] = $need[-1] + " (checked only after pad '$ArmCmd')" }
if ($NotScreen) { $need += "screen #$($NotScreen.Name) gone" }
Say ("  scene '{0}' is reached when: {1}" -f $Scene, ($need -join ' + ')) 'DarkGray'
$waitStart = Get-Date
$said = @{}
$sceneAt = $null
$sceneOdsMs = $null
$verdict = ''
$elapsed = 0.0
$ms = [ordered]@{}
try {
  while ($true) {
    if (-not $NoWaitUser -and (Test-UserPlaying)) {
      if (-not $said['user']) { Say '  waiting: USER_PLAYING (the user is playing)' 'Yellow'; $said['user'] = 1 }
      Start-Sleep -Seconds 5; continue
    }
    if (-not $held.Contains($overlayLock)) {
      if (-not (Get-Lock $overlayLock)) {
        if (-not $said['ov']) { Say "  waiting: test overlay of $Title busy ($((Get-Content -LiteralPath $overlayLock -Raw -ErrorAction SilentlyContinue)))" 'Yellow'; $said['ov'] = 1 }
        if (((Get-Date) - $waitStart).TotalMinutes -gt $LockWaitMinutes) { throw "gave up after $LockWaitMinutes min waiting for $overlayLock" }
        Start-Sleep -Seconds 5; continue
      }
    }
    $slot = $null
    foreach ($l in $lockSlots) { if (Get-Lock $l) { $slot = $l; break } }
    if ($slot) { break }
    if (-not $said['slot']) { Say ('  waiting: 2 game runs active (' + (($lockSlots | ForEach-Object { Get-Content -LiteralPath $_ -Raw -ErrorAction SilentlyContinue }) -join ' | ') + ')') 'Yellow'; $said['slot'] = 1 }
    if (((Get-Date) - $waitStart).TotalMinutes -gt $LockWaitMinutes) { throw "gave up after $LockWaitMinutes min waiting for a GAME_LOCK slot" }
    Start-Sleep -Seconds 5
  }
  Say ("  locks: {0} (waited {1:N0}s)" -f ($held -join ', '), ((Get-Date) - $waitStart).TotalSeconds) 'DarkGray'

  $envKeys = @('TOMBOX_HEADLESS', 'TOMBOX_TEST_RUN', 'TOMBOX_HOST_PAD', 'TOMBOX_HOST_KBM', 'TOMBOX_PRELAUNCH_UI', 'TOMBOX_CAPTURE_DIR', 'TOMBOX_PAD_DIR', 'TOMBOX_GDK_ROOT', 'TOMBOX_ALLOW_NETWORK', 'TOMBOX_NET_LOCAL_PORT', 'TOMBOX_TAKEOVER_FILE', 'TOMBOX_USER_PLAYING_FILE') + @($Env.Keys)
  $oldEnv = @{}
  foreach ($k in $envKeys) { $oldEnv[$k] = [Environment]::GetEnvironmentVariable($k) }
  Remove-Item Env:TOMBOX_ALLOW_NETWORK, Env:TOMBOX_NET_LOCAL_PORT, Env:TOMBOX_TAKEOVER_FILE, Env:TOMBOX_USER_PLAYING_FILE -ErrorAction SilentlyContinue
  $env:TOMBOX_HEADLESS = '1'
  $env:TOMBOX_TEST_RUN = '1'
  $env:TOMBOX_HOST_PAD = '0'
  $env:TOMBOX_HOST_KBM = '0'
  $env:TOMBOX_PRELAUNCH_UI = '0'
  $env:TOMBOX_CAPTURE_DIR = $CapDir
  $env:TOMBOX_PAD_DIR = $PadDir
  [TbFrame2]::CaptureDir = $CapDir
  if ($Title -ne 'trackmania') {
    $tg = Join-Path $TestRoot "$($Spec.RunTitle)\gdkroot"
    $nSeed = Reset-TestGdkRoot $tg $Spec.Seed
    $env:TOMBOX_GDK_ROOT = $tg
    Say ("  test save root {0} wiped{1}" -f $tg, $(if ($nSeed) { " and seeded with $nSeed files from $($Spec.Seed)" } else { '' })) 'DarkGray'
  }
  foreach ($k in $Env.Keys) { [Environment]::SetEnvironmentVariable($k, [string]$Env[$k]) }

  $profileArgs = if ($isRelease) { @('-Profile', 'release', '-Target', $Target) } else { @('-Profile', 'dev-fast', '-FastTarget', $Target) }
  $common = @('-NoShaderUi', '-LogDir', $Live, '-Seconds', "$($Seconds + 120)")
  switch ($Title) {
    'trackmania' {
      $ra = @('-NoProfile', '-File', (Join-Path $Root 'run_xone.ps1'), '-Title', 'Trackmania') + $common + $profileArgs
      if ($isRelease) { $ra += @('-XoneTarget', $Target) }
      $ra += @('-TestBoot', 'TrainingFirst', '-TestBootFile', $(if ($MapFile) { "`"$MapFile`"" } else { 'none' }))
    }
    'terraria' {
      $ra = @('-NoProfile', '-File', (Join-Path $Root 'run_xone.ps1'), '-Title', 'Terraria') + $common + $profileArgs
      if ($isRelease) { $ra += @('-XoneTarget', $Target) }
      $ra += @('-OverlayRoot', $TestRoot, '-Drive', $Spec.Drive)
    }
    'fortnite' {
      $ra = @('-NoProfile', '-File', (Join-Path $Root 'run_fortnite.ps1')) + $common + $profileArgs
      $ra += @('-Overlay', (Join-Path $TestRoot 'Fortnite'), '-Drive', $Spec.Drive, '-Match', 'Off')
    }
    'minecraft' {
      $ra = @('-NoProfile', '-File', (Join-Path $Root 'run_minecraft.ps1')) + $common + $profileArgs
      $ra += @('-Overlay', (Join-Path $TestRoot 'Minecraft'), '-Drive', $Spec.Drive)
    }
    'xdefiant' {
      $ra = @('-NoProfile', '-File', (Join-Path $Root 'run_xdefiant.ps1')) + $common + $profileArgs
      $ra += @('-Overlay', (Join-Path $TestRoot 'XDefiant'), '-Drive', $Spec.Drive)
    }
  }
  if ($GameArgs) { $ra += @('-GameArgs', "`"$GameArgs`"") }
  Say ("  launch: powershell {0}" -f ($ra -join ' ')) 'DarkGray'
  if ($Env.Count) { Say ("  env: {0}" -f (($Env.Keys | ForEach-Object { "$_=$($Env[$_])" }) -join ' ')) 'DarkGray' }

  $start = Get-Date
  $ps = Start-Process -FilePath 'powershell.exe' -ArgumentList $ra -RedirectStandardOutput (Join-Path $Out 'launcher.txt') -RedirectStandardError (Join-Path $Out 'launcher.err.txt') -WindowStyle Hidden -PassThru
  foreach ($k in $oldEnv.Keys) { [Environment]::SetEnvironmentVariable($k, $oldEnv[$k]) }
  [TbFrame2]::CaptureDir = $CapDir

  $streams = @(
    [pscustomobject]@{ Src = 'ods'; Path = (Join-Path $Live 'run.txt'); Pos = 0L; Rest = '' },
    [pscustomobject]@{ Src = 'trace'; Path = (Join-Path $Live 'run.err.txt'); Pos = 0L; Rest = '' },
    [pscustomobject]@{ Src = 'compat'; Path = (Join-Path $Live 'tombox_compat.log'); Pos = 0L; Rest = '' })
  $msDefs = @(
    @{ Name = 'game process'; Src = 'trace'; Rx = [regex]'\[trace\] launching' },
    @{ Name = 'first present'; Src = 'compat'; Rx = [regex]'PresentX presenting|\[perf\] fps=' }) + @($Spec.Milestones)
  if ($SceneRx) { $msDefs += @{ Name = 'scene log line'; Src = 'any'; Rx = $SceneRx } }
  $odsFirst = $null
  $logOk = -not $SceneRx
  $logOdsMs = $null
  $screenOk = -not $SceneScreen
  $motionOk = -not $MotionCmd
  $motionArmed = $false
  $probeArmed = -not $ArmCmd
  $motionPrev = $null
  $motionMax = 0.0
  $lastProbe = $null
  $nextProbe = 0.0
  $probeN = 0
  $padLines = New-Object System.Collections.Generic.List[string]
  $padLast = 0.0
  $nextShot = 5.0
  $nextGate = 0.0
  $muted = $false
  $tree = Get-Tree $ps.Id
  $nextTree = Get-Date
  $padT = [pscustomobject]@{ PadScreens = $Spec.PadScreens }
  while ($true) {
    Start-Sleep -Milliseconds 100
    $el = ((Get-Date) - $start).TotalSeconds
    if ((Get-Date) -ge $nextTree) { $tree = Get-Tree $ps.Id; $nextTree = (Get-Date).AddSeconds(1) }
    $pids = [uint32[]]@($tree)
    try { if ([TbAudio]::Sessions($pids, $true, $null) -gt 0 -and -not $muted) { $muted = $true; Say ("  +{0:N1}s audio muted" -f $el) 'DarkGray' } } catch {}
    foreach ($s in $streams) {
      foreach ($l in Read-New $s) {
        if ($s.Src -eq 'ods' -and -not $odsFirst -and $l -match '^\[ODS\] (\d{13}) ') { $odsFirst = [int64]$Matches[1] }
        foreach ($m in $msDefs) {
          if ($ms.Contains($m.Name) -or ($m.Src -ne 'any' -and $m.Src -ne $s.Src) -or -not $m.Rx.IsMatch($l)) { continue }
          $ms[$m.Name] = [Math]::Round($el, 1)
          Say ("  +{0:N1}s milestone: {1}" -f $el, $m.Name) 'Green'
          if ($m.Name -eq 'scene log line') {
            $logOk = $true
            if ($l -match '^\[ODS\] (\d{13}) ' -and $odsFirst) { $logOdsMs = [int64]$Matches[1] - $odsFirst }
          }
        }
        if ($s.Src -eq 'ods' -and $padLines.Count -lt 20000) { [void]$padLines.Add($l) }
        if ($s.Src -eq 'trace' -and $l -match 'firstChance=0|ACCESS_VIOLATION|EXIT_PROCESS|\[HANG\]') { Say ("  +{0:N1}s {1}" -f $el, $l.Trim()) 'Yellow' }
      }
    }
    if ($padQueue.Count -gt 0 -and $null -eq $padQueue[0].At -and $padQueue[0].Prev) { $padQueue[0].At = $padLast + $padQueue[0].Delay }
    if ($padQueue.Count -gt 0 -and $null -eq $padQueue[0].At -and $padQueue[0].Rx -and $padLines.Count) {
      $hit = -1
      for ($i = 0; $i -lt $padLines.Count; $i++) { if ($padQueue[0].Rx.IsMatch($padLines[$i])) { $hit = $i; break } }
      if ($hit -ge 0) { $padLines.RemoveRange(0, $hit + 1); $padQueue[0].At = $el + $padQueue[0].Delay; Say ("  +{0:N1}s pad gate /{1}/ matched" -f $el, $padQueue[0].Rx) 'DarkGray' } else { $padLines.Clear() }
    }
    if ($padQueue.Count -gt 0 -and $null -eq $padQueue[0].At -and $padQueue[0].Screen -and $el -ge $nextGate) {
      $nextGate = $el + 0.5
      $sv = Test-PadScreen $padT $padQueue[0].Screen $pids (Join-Path $Out 'pad_gate.png')
      $sd = $Spec.PadScreens | Where-Object { $_.Name -eq $padQueue[0].Screen } | Select-Object -First 1
      if ($null -ne $sv -and $sd -and $sv -ge $sd.Min) { $padQueue[0].At = $el + $padQueue[0].Delay; Say ("  +{0:N1}s pad gate #{1} on screen (score {2:N2})" -f $el, $padQueue[0].Screen, $sv) 'DarkGray' }
      elseif ($padQueue[0].PSObject.Properties['Opt'] -and $padQueue[0].Opt -and $padQueue.Count -gt 1 -and $padQueue[1].Screen) {
        $nv = Test-PadScreen $padT $padQueue[1].Screen $pids (Join-Path $Out 'pad_gate.png')
        $nd = $Spec.PadScreens | Where-Object { $_.Name -eq $padQueue[1].Screen } | Select-Object -First 1
        if ($null -ne $nv -and $nd -and $nv -ge $nd.Min) { Say ("  +{0:N1}s optional pad step #{1} skipped: #{2} already on screen" -f $el, $padQueue[0].Screen, $padQueue[1].Screen) 'DarkGray'; $padQueue.RemoveAt(0) }
      }
    }
    while ($padQueue.Count -gt 0 -and $null -ne $padQueue[0].At -and $el -ge $padQueue[0].At) {
      $pc = $padQueue[0]; $padQueue.RemoveAt(0); $padLast = $el
      $pn = '{0:D20}_fast' -f [DateTime]::UtcNow.Ticks
      [IO.File]::WriteAllText((Join-Path $PadDir "$pn.tmp"), "$($pc.Cmd)`n", (New-Object Text.UTF8Encoding $false))
      Move-Item -LiteralPath (Join-Path $PadDir "$pn.tmp") -Destination (Join-Path $PadDir "$pn.cmd")
      Say ("  +{0:N1}s pad: {1}" -f $el, $pc.Cmd) 'DarkGray'
      if ($MotionCmd -and -not $motionArmed -and $pc.Cmd -like "*$MotionCmd*") { $motionArmed = $true; $nextProbe = $el + 1.0; Say ("  +{0:N1}s motion check armed" -f $el) 'DarkGray' }
      if ($ArmCmd -and -not $probeArmed -and $pc.Cmd -like "*$ArmCmd*") { $probeArmed = $true; $nextProbe = $el + 1.0; Say ("  +{0:N1}s scene check armed" -f $el) 'DarkGray' }
    }
    if (-not $sceneAt -and $logOk -and $probeArmed -and $el -ge $nextProbe -and $ms.Contains('first present')) {
      $nextProbe = $el + 1.0
      $probeN++
      $pp = Join-Path $Out ("probe_{0}.png" -f ($probeN % 2))
      $pr = Get-Probe $pids $pp
      if ($pr -and $pr.Drawn) {
        if (-not $screenOk) {
          $sv = Get-ScreenScore $SceneScreen $pr.Png
          if ($null -ne $sv -and $sv -ge $SceneScreen.Min) { $screenOk = $true; Say ("  +{0:N1}s scene screen #{1} on screen (score {2:N2})" -f $el, $SceneScreen.Name, $sv) 'Green' }
        }
        if (-not $motionOk -and $motionArmed) {
          if ($motionPrev -and (Test-Path -LiteralPath $motionPrev)) {
            $dv = try { [TbDiff]::Changed($motionPrev, $pr.Png) } catch { 0.0 }
            if ($dv -gt $motionMax) { $motionMax = $dv }
            Say ("  +{0:N1}s motion: {1:P1} of the frame changed in 1 s" -f $el, $dv) 'DarkGray'
            if ($dv -ge 0.10) { $motionOk = $true }
          }
          $mp = Join-Path $Out ("motion_{0}.png" -f ($probeN % 3))
          Copy-Item -LiteralPath $pr.Png -Destination $mp -Force
          $motionPrev = $mp
        }
        $goneOk = $true
        if ($NotScreen) {
          $nv = Get-ScreenScore $NotScreen $pr.Png
          if ($null -ne $nv -and $nv -ge $NotScreen.Min) { $goneOk = $false; $lastProbe = "#$($NotScreen.Name) still on screen" }
        }
        if ($screenOk -and $motionOk -and $goneOk) {
          $sceneAt = $el
          if ($null -ne $logOdsMs) { $sceneOdsMs = $logOdsMs }
          $ms['scene'] = [Math]::Round($el, 1)
          Say ("  +{0:N1}s SCENE '{1}' REACHED" -f $el, $Scene) 'Green'
          $null = Save-Frame $pids $el '_scene'
        }
      }
      elseif ($pr) { $lastProbe = $pr.Verdict }
    }
    if ($el -ge $nextShot) { $null = Save-Frame $pids $el ''; $nextShot = $el + $CaptureEvery }
    $ps.Refresh()
    if ($ps.HasExited) { $verdict = 'exit'; break }
    if (-not (Get-Process -Name $Spec.Proc -ErrorAction SilentlyContinue | Where-Object { $tree.Contains($_.Id) }) -and $el -gt 30 -and $ms.Contains('game process')) { $tree = Get-Tree $ps.Id; if (-not (Get-Process -Name $Spec.Proc -ErrorAction SilentlyContinue | Where-Object { $tree.Contains($_.Id) })) { $verdict = 'exit'; break } }
    $padLeft = @($padQueue | Where-Object { -not ($_.PSObject.Properties['Opt'] -and $_.Opt) }).Count
    if ($sceneAt -and $el -ge $sceneAt + $AfterScene -and ($padLeft -eq 0 -or $MotionCmd)) { $null = Save-Frame $pids $el '_final'; $verdict = 'scene reached'; break }
    $limit = if ($ms.Contains('game process')) { $ms['game process'] + $Seconds } else { $Seconds + 180 }
    if ($el -ge $limit) { $null = Save-Frame $pids $el '_final'; $verdict = 'timeout'; break }
  }
  $elapsed = ((Get-Date) - $start).TotalSeconds
  $tree = Get-Tree $ps.Id
  foreach ($tp in @($tree | Sort-Object -Descending)) { try { Stop-Process -Id $tp -Force -ErrorAction Stop } catch {} }
  Start-Sleep -Seconds 2
  try { [void][TbAudio]::Restore() } catch {}
}
finally {
  Remove-MyLocks
}

foreach ($f in Get-ChildItem -LiteralPath $Live -File -ErrorAction SilentlyContinue) { Copy-Item -LiteralPath $f.FullName -Destination $Out -Force }
if ($Spec.GameLog -and (Test-Path -LiteralPath $Spec.GameLog)) { Copy-Item -LiteralPath $Spec.GameLog -Destination (Join-Path $Out 'game_log.txt') -Force }
$compat = Join-Path $Out 'tombox_compat.log'
$trace = Join-Path $Out 'run.err.txt'
$ods = Join-Path $Out 'run.txt'
$exit = ''
if (Test-Path -LiteralPath $trace) { $em = Select-String -LiteralPath $trace -Pattern '\[trace\] EXIT_PROCESS code=(-?\d+) \((0x[0-9a-fA-F]+)\)' | Select-Object -Last 1; if ($em) { $exit = $em.Matches[0].Groups[2].Value } }
$crash = $false
if (Test-Path -LiteralPath $trace) { $crash = [bool](Select-String -LiteralPath $trace -Pattern 'firstChance=0|\[HANG\]' -Quiet) }
$kind = if ($crash) { 'crash' } elseif ($verdict -eq 'exit') { "exit $exit" } else { $verdict }
$boot = if ($null -ne $sceneOdsMs) { "{0:N1}s wall from launch (scene log line at {1:N1}s game clock)" -f $sceneAt, ($sceneOdsMs / 1000.0) } elseif ($sceneAt) { "{0:N1}s wall from launch" -f $sceneAt } else { 'scene not reached' }
$why = @()
if (-not $sceneAt) {
  if (-not $logOk) { $why += "log line /$SceneRx/ never seen" }
  if (-not $screenOk) { $why += "screen #$($SceneScreen.Name) never matched" }
  if (-not $probeArmed) { $why += "pad '$ArmCmd' never sent" }
  if (-not $motionOk) { $why +=$(if ($motionArmed) { "no motion after '$MotionCmd' (max {0:P1} changed per s)" -f $motionMax } else { "pad '$MotionCmd' never sent" }) }
  if (-not $ms.Contains('first present')) { $why += 'title never presented' }
  if ($lastProbe) { $why += "last probe frame: $lastProbe" }
}

Say ''
Say ("RESULT: {0} after {1:N1}s; scene '{2}': {3}" -f $kind, $elapsed, $Scene, $boot) $(if ($sceneAt -and -not $crash) { 'Green' } else { 'Red' })
if ($why.Count) { Say ("NOT REACHED: {0}" -f ($why -join '; ')) 'Red' }
Say ("MILESTONES: {0}" -f (($ms.Keys | ForEach-Object { "$_ +$($ms[$_])s" }) -join ', '))
Say 'DRED:'
if (Test-Path -LiteralPath $compat) {
  $dred = @(Select-String -LiteralPath $compat -Pattern 'dred\.(removed|breadcrumb|page)|DEVICE_REMOVED|device removed|DEVICE_HUNG' | Select-Object -First 40)
  if ($dred.Count) { foreach ($d in $dred) { Say ('  ' + $d.Line.Substring(0, [Math]::Min(300, $d.Line.Length))) } } else { Say '  none' }
}
Say 'GAME ERRORS / DEBUG:'
$gl = Join-Path $Out 'game_log.txt'
$gsrc = if (Test-Path -LiteralPath $gl) { $gl } else { $ods }
if (Test-Path -LiteralPath $gsrc) {
  $ge = @(Get-Content -LiteralPath $gsrc | Where-Object { $_ -match '(?i)error|fail|warn|assert|exception|exhaust|init challenge|main menu|MenuResult|Connection|JSon|offline' -and $_ -notmatch '\[Hms\]' } | Select-Object -First 60)
  if ($ge.Count) { foreach ($g in $ge) { Say ('  ' + $g.Substring(0, [Math]::Min(300, $g.Length))) } } else { Say '  none' }
}
Say 'COMPAT ERRORS:'
if (Test-Path -LiteralPath $compat) {
  $ce = @(Select-String -LiteralPath $compat -Pattern '(?i)\bFAILED\b|\bpanic|ecl\.refuse|shader_xlat FAIL|untranslatable' | Group-Object { $n = ($_.Line -replace '^\d+ ', '' -replace '0x[0-9a-fA-F]+', '0x#' -replace '\d+', '#'); $n.Substring(0, [Math]::Min(160, $n.Length)) } | Sort-Object Count -Descending | Select-Object -First 15)
  if ($ce.Count) { foreach ($c in $ce) { Say ("  x{0} {1}" -f $c.Count, $c.Group[0].Line.Substring(0, [Math]::Min(260, $c.Group[0].Line.Length))) } } else { Say '  none' }
}
Say 'FRAMES:'
foreach ($f in $frames) { Say "  $f" }
Say "LOGS: $Out (tombox_compat.log, run.txt = ODS, run.err.txt = trace, game_log.txt, launcher.txt, fast_run.txt)"
$result = [ordered]@{
  title = $Title; scene = $Scene; reached = [bool]$sceneAt; scene_s = $(if ($sceneAt) { [Math]::Round($sceneAt, 1) } else { $null })
  scene_game_s = $(if ($null -ne $sceneOdsMs) { [Math]::Round($sceneOdsMs / 1000.0, 1) } else { $null }); result = $kind; elapsed_s = [Math]::Round($elapsed, 1)
  not_reached = ($why -join '; '); milestones = $ms; frames = @($frames); out = $Out; target = $Target; id = $Id
}
[IO.File]::WriteAllText((Join-Path $Out 'result.json'), ($result | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding $false))
