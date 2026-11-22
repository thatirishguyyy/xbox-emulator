$CompatStatuses = @('Nothing', 'Boots', 'Menus', 'Ingame', 'Playable')
$CompatMenuMilestones = @{ fortnite = @('game-mode select', 'lobby', 'ready-up ticket', 'session assigned', 'session joined'); generic = @('menu / lobby') }
$CompatGameMilestones = @{ fortnite = @('match start'); generic = @('match start') }
$CompatIssueOrder = @('Crashes and hangs', 'Missing exports / unimplemented calls', 'Shader translation failures', 'D3D12 debug layer / DRED / device removal', 'Compat errors', 'Title errors')

function ConvertTo-PublicText([string]$s) {
  if ($null -eq $s) { return '' }
  $o = $s
  foreach ($p in @(@($env:TEMP, '%TEMP%'), @($env:LOCALAPPDATA, '%LOCALAPPDATA%'), @($env:USERPROFILE, '%USERPROFILE%'))) {
    if ($p[0]) { $o = $o -replace [regex]::Escape($p[0]), $p[1] }
  }
  $o = $o -replace '(?i)[A-Z]:\\Users\\[^\\\s"''`|]+', '%USERPROFILE%'
  $o = $o -replace '(?i)/[a-z]/Users/[^/\s"''`|]+', '%USERPROFILE%'
  foreach ($w in @($env:USERNAME, $env:COMPUTERNAME)) { if ($w -and $w.Length -ge 3) { $o = [regex]::Replace($o, "(?i)\b$([regex]::Escape($w))\b", '<redacted>') } }
  return $o
}

function Get-BuildStamp([string]$Root) {
  $head = (& git -C $Root rev-parse --short HEAD 2>$null)
  $dirty = @(& git -C $Root status --porcelain --untracked-files=no 2>$null).Count
  $dll = Join-Path $Root 'target\release\d3d12_xs.dll'
  $built = if (Test-Path $dll) { (Get-Item $dll).LastWriteTimeUtc.ToString('yyyy-MM-ddTHH:mm:ssZ') } else { '' }
  [pscustomobject]@{ Commit = "$head"; Dirty = $dirty; Built = $built; Stamp = ("{0}{1} built {2}" -f $head, $(if ($dirty) { "+$dirty modified" } else { '' }), $built) }
}

function Get-CompatEntry($r, $Build, [string]$Root, [string]$RunStamp) {
  $t = $r.Title
  $a = $r.Analysis
  $ms = if ($a) { $a.Milestones } else { @{} }
  $order = @($MilestoneDefs[$t.Kind] | ForEach-Object { $_.Name })
  $far = ''
  foreach ($n in $order) { if ($ms.Contains($n)) { $far = $n } }
  $frames = if ($r.Frames) { [object[]]$r.Frames.ToArray() } else { [object[]]@() }
  $withWin = @($frames | Where-Object { $_.Png })
  $drawn = @($frames | Where-Object { $_.Drawn })
  $crashed = $r.Status -match '^crash'
  $fpsAvg = $null; $fpsMin = $null; $fpsMax = $null
  if ($a -and $a.Fps.Count) { $st = $a.Fps | Measure-Object -Minimum -Maximum -Average; $fpsAvg = [Math]::Round($st.Average, 1); $fpsMin = [Math]::Round($st.Minimum, 1); $fpsMax = [Math]::Round($st.Maximum, 1) }
  $p99 = if ($a -and $a.P99) { [double]$a.P99 } else { $null }
  $status = 'Nothing'
  $basisNote = ''
  if ($ms.Contains('first frame') -or $withWin.Count) { $status = 'Boots' }
  if (@($CompatMenuMilestones[$t.Kind] | Where-Object { $ms.Contains($_) }).Count) { $status = 'Menus' }
  if (@($CompatGameMilestones[$t.Kind] | Where-Object { $ms.Contains($_) }).Count) {
    $status = 'Ingame'
    if ($drawn.Count -and -not $crashed -and $fpsAvg -ge 30) { $basisNote = 'meets the automatic Playable checks (drawn, no crash, >=30 fps avg); Playable is only set after a manual review' }
  }
  $basis = if ($far) { "$far at $($ms[$far])" } elseif ($withWin.Count) { "game window captured at +$($withWin[0].T)s" } else { 'no first frame and no window' }
  if ($basisNote) { $basis += "; $basisNote" }
  $gfx = New-Object System.Collections.Generic.List[string]
  if (-not $withWin.Count) { $gfx.Add('no game window captured') }
  else {
    $verd = @($withWin | Group-Object Verdict | Sort-Object Count -Descending | ForEach-Object { "$($_.Name) x$($_.Count)" })
    $gfx.Add("captures: $($verd -join ', ')")
    if (-not $drawn.Count) { $gfx.Add('no captured frame shows real content (black or uniform)') }
    $means = @($withWin | Where-Object { $_.Drawn } | ForEach-Object { $_.Mean })
    for ($i = 1; $i -lt $means.Count; $i++) { if ([Math]::Abs($means[$i] - $means[$i - 1]) -gt 60) { $gfx.Add('large brightness swings between captures (possible flicker)'); break } }
  }
  $top = ''
  if ($a) {
    foreach ($c in $CompatIssueOrder) {
      $b = $a.Defects[$c]
      if ($b -and $b.Count) { $e = @($b.Values)[0]; $s = $e.Sample; if ($s.Length -gt 240) { $s = $s.Substring(0, 240) + '...' }; $top = "${c}: $s ($($e.Where))"; break }
    }
  }
  if ($crashed) { $top = "exit $($r.Exit); " + $top }
  if (-not $top -and -not $drawn.Count) { $top = 'frames not drawn' }
  $shot = ''
  $pick = if ($drawn.Count) { $drawn[-1] } elseif ($withWin.Count) { $withWin[-1] } else { $null }
  if ($pick) {
    $dst = Join-Path (Split-Path $r.Dir -Parent) "$($t.Slug).png"
    Copy-Item -LiteralPath $pick.Png $dst -Force
    $shot = ($dst.Substring($Root.Length).TrimStart('\') -replace '\\', '/')
  }
  [pscustomobject][ordered]@{
    run = $RunStamp
    date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    build = $Build.Stamp
    commit = $Build.Commit
    modified_files = $Build.Dirty
    status = $status
    status_basis = $basis
    furthest = $(if ($far) { $far } else { 'none' })
    milestones = @($order | Where-Object { $ms.Contains($_) })
    run_result = (ConvertTo-PublicText $r.Status)
    seconds = $r.Elapsed
    fps_avg = $fpsAvg; fps_min = $fpsMin; fps_max = $fpsMax; frametime_p99_ms = $p99
    meets_60fps = [bool]($fpsAvg -ge 60)
    frames_drawn = [bool]$drawn.Count
    captures = @($withWin | ForEach-Object { [pscustomobject][ordered]@{ t = $_.T; verdict = $_.Verdict; mean = $_.Mean; std = $_.Std; colours = $_.Distinct } })
    graphics_issues = @($gfx)
    top_issue = (ConvertTo-PublicText $top)
    screenshot = $shot
    d3d12_debug_layer = [bool]$DbgLayer
  }
}

function Update-CompatList([string]$Dir, $Entries, $Titles) {
  New-Item -ItemType Directory -Force -Path $Dir | Out-Null
  $jp = Join-Path $Dir 'compat.json'
  $db = [ordered]@{}
  if (Test-Path $jp) {
    $old = Get-Content $jp -Raw | ConvertFrom-Json
    foreach ($p in $old.titles.PSObject.Properties) { $db[$p.Name] = [ordered]@{ name = $p.Value.name; status = $p.Value.status; last = $p.Value.last; history = @($p.Value.history) } }
  }
  foreach ($k in $Entries.Keys) {
    $e = $Entries[$k]
    $name = ($Titles | Where-Object { $_.Slug -eq $k } | Select-Object -First 1).Name
    if (-not $db.Contains($k)) { $db[$k] = [ordered]@{ name = $name; status = ''; last = $null; history = @() } }
    $db[$k].name = $name
    $db[$k].status = $e.status
    $db[$k].last = $e
    $h = @($db[$k].history) + @($e)
    if ($h.Count -gt 100) { $h = $h[($h.Count - 100)..($h.Count - 1)] }
    $db[$k].history = $h
  }
  $out = [ordered]@{ schema = 1; updated = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'); statuses = $CompatStatuses; titles = $db }
  [IO.File]::WriteAllText($jp, (ConvertTo-PublicText ($out | ConvertTo-Json -Depth 12)), (New-Object System.Text.UTF8Encoding $false))
  Write-CompatMarkdown $Dir
}

function Write-CompatMarkdown([string]$Dir) {
  $j = Get-Content (Join-Path $Dir 'compat.json') -Raw | ConvertFrom-Json
  $L = New-Object System.Collections.Generic.List[string]
  $L.Add('# TomBox compatibility list')
  $L.Add('')
  $L.Add("Updated $($j.updated) by tools/dev/batch_test.ps1 after every batch run. Machine-readable copy: compat.json.")
  $L.Add('')
  $L.Add('| Status | Meaning |')
  $L.Add('|---|---|')
  $L.Add('| Nothing | does not start, or crashes before any window |')
  $L.Add('| Boots | window or first frame, but no menus |')
  $L.Add('| Menus | reaches menus or lobby, cannot get in-game |')
  $L.Add('| Ingame | reaches gameplay with major issues (crashes, broken graphics, under 30 fps) |')
  $L.Add('| Playable | the game loop works with only minor issues; the 60 fps target is noted separately |')
  $L.Add('')
  $L.Add('| Title | Status | Tested (UTC) | Build | Furthest point | FPS avg / p99 ms | 60 fps | Frames drawn | Graphics | Top issue | Screenshot |')
  $L.Add('|---|---|---|---|---|---|---|---|---|---|---|')
  foreach ($p in $j.titles.PSObject.Properties) {
    $e = $p.Value.last
    if (-not $e) { continue }
    $fps = if ($null -ne $e.fps_avg) { "$($e.fps_avg) / $($e.frametime_p99_ms)" } else { '-' }
    $ti = ($e.top_issue -replace '\|', '\|' -replace '`', "'"); if ($ti.Length -gt 200) { $ti = $ti.Substring(0, 200) + '...' }
    $shot = if ($e.screenshot) { "[png]($('../../../' + $e.screenshot))" } else { '-' }
    $L.Add("| $($p.Value.name) | **$($e.status)** | $($e.date) | $($e.build) | $($e.furthest) | $fps | $(if ($e.meets_60fps) { 'yes' } else { 'no' }) | $(if ($e.frames_drawn) { 'yes' } else { 'no' }) | $((@($e.graphics_issues) -join '; ') -replace '\|', '\|') | ``$ti`` | $shot |")
  }
  $L.Add('')
  $L.Add('## History')
  foreach ($p in $j.titles.PSObject.Properties) {
    $L.Add('')
    $L.Add("### $($p.Value.name)")
    $L.Add('')
    $L.Add('| Run | Status | Build | Furthest | FPS avg / p99 ms | Frames drawn | Debug layer |')
    $L.Add('|---|---|---|---|---|---|---|')
    $hist = @($p.Value.history)
    [array]::Reverse($hist)
    foreach ($h in ($hist | Select-Object -First 20)) {
      $fps = if ($null -ne $h.fps_avg) { "$($h.fps_avg) / $($h.frametime_p99_ms)" } else { '-' }
      $L.Add("| $($h.run) | $($h.status) | $($h.build) | $($h.furthest) | $fps | $(if ($h.frames_drawn) { 'yes' } else { 'no' }) | $(if ($h.d3d12_debug_layer) { 'on' } else { 'off' }) |")
    }
  }
  [IO.File]::WriteAllText((Join-Path $Dir 'COMPATIBILITY.md'), (ConvertTo-PublicText (($L -join "`n") + "`n")), (New-Object System.Text.UTF8Encoding $false))
}
