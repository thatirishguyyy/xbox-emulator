[CmdletBinding(PositionalBinding = $false)]
param(
  [Parameter(Position = 0, ValueFromRemainingArguments = $true)][string[]]$Set = @(),
  [string[]]$Unset = @(),
  [switch]$Clear,
  [switch]$Keys,
  [string]$Path = ''
)

$ErrorActionPreference = 'Stop'
if (-not $Path) { $Path = if ($env:TOMBOX_LIVE_CFG) { $env:TOMBOX_LIVE_CFG } else { Join-Path $env:TEMP 'tombox\live.cfg' } }

if ($Keys) {
  $src = 'D:\TomBox\crates'
  $rx = [regex]'(?:Sw|Armed|Text)::new\(\s*"(TOMBOX_[A-Z0-9_]+)",\s*"([a-z0-9_]+)"(?:,\s*(true|false))?|live::(?:flag|number|value|hashes)\(\s*"(TOMBOX_[A-Z0-9_]+)",\s*"([a-z0-9_]+)"|\.poll\("([a-z0-9_]+)"\)'
  $rows = @{}
  foreach ($f in Get-ChildItem $src -Recurse -Filter *.rs -File | Where-Object { $_.Name -notmatch '_tests\.rs$' -and $_.FullName -notmatch '\\tests\\' }) {
    $t = [IO.File]::ReadAllText($f.FullName)
    foreach ($m in $rx.Matches($t)) {
      $env = if ($m.Groups[1].Success) { $m.Groups[1].Value } elseif ($m.Groups[4].Success) { $m.Groups[4].Value } else { '' }
      $key = if ($m.Groups[2].Success) { $m.Groups[2].Value } elseif ($m.Groups[5].Success) { $m.Groups[5].Value } else { $m.Groups[6].Value }
      if ($key -match '^livetest') { continue }
      $kind = if ($m.Groups[6].Success) { 'trigger (change value or <key>_seq)' } elseif ($t.Substring([Math]::Max(0, $m.Index - 200), [Math]::Min(200, $m.Index)) -match 'OnceLock|get_or_init|Once\b') { 'read once at start' } else { 'live' }
      $crate = ($f.FullName.Substring($src.Length + 1) -split '\\')[-3]
      $rows["$key"] = [pscustomobject]@{ Key = $key; Env = $env; Kind = $kind; Where = "$crate/$($f.Name)" }
    }
  }
  $rows.Values | Sort-Object Kind, Key | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
  $all = @(Get-ChildItem $src -Recurse -Filter *.rs -File | Select-String -Pattern '"(TOMBOX_[A-Z0-9_]+)"' -AllMatches | ForEach-Object { $_.Matches } | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
  Write-Host ("{0} TOMBOX_* names appear in the sources; only the keys above are read through live.cfg, the rest are plain environment variables read at start" -f $all.Count)
  return
}

$lines = New-Object System.Collections.Generic.List[string]
if (-not $Clear -and (Test-Path -LiteralPath $Path)) { foreach ($l in [IO.File]::ReadAllLines($Path)) { $lines.Add($l) } }

function Get-KeyOf([string]$l) {
  $t = $l.Trim()
  if (-not $t -or $t.StartsWith('#') -or $t.StartsWith(';') -or $t -notmatch '=') { return $null }
  $k = ($t -split '=', 2)[0].Trim().ToLower()
  if ($k.StartsWith('tombox_')) { $k = $k.Substring(7) }
  return $k
}

$changed = $Clear.IsPresent
foreach ($u in $Unset) {
  $k = $u.Trim().ToLower() -replace '^tombox_', ''
  for ($i = $lines.Count - 1; $i -ge 0; $i--) { if ((Get-KeyOf $lines[$i]) -eq $k) { $lines.RemoveAt($i); $changed = $true } }
}
foreach ($s in $Set) {
  if ($s -notmatch '=') { throw "expected key=value, got '$s'" }
  $k, $v = $s -split '=', 2
  $k = $k.Trim().ToLower() -replace '^tombox_', ''
  $done = $false
  for ($i = 0; $i -lt $lines.Count; $i++) {
    if ((Get-KeyOf $lines[$i]) -eq $k) {
      if (-not $done) { $lines[$i] = "$k = $($v.Trim())"; $done = $true } else { $lines.RemoveAt($i); $i-- }
    }
  }
  if (-not $done) { $lines.Add("$k = $($v.Trim())") }
  $changed = $true
}
if ($changed) {
  New-Item -ItemType Directory -Force -Path (Split-Path $Path -Parent) | Out-Null
  $tmp = "$Path.tmp$PID"
  [IO.File]::WriteAllText($tmp, (($lines -join "`r`n") + $(if ($lines.Count) { "`r`n" } else { '' })), (New-Object Text.UTF8Encoding $false))
  Move-Item -LiteralPath $tmp -Destination $Path -Force
}
Write-Host "$Path$(if ($changed) { ' (written; running titles pick it up within 250 ms)' })" -ForegroundColor Cyan
foreach ($l in $lines) { Write-Host "  $l" }
