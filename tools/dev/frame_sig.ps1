if (-not ('TbSig' -as [type])) { Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class TbSig {
    static byte[] Pixels(Bitmap b, out int stride) {
        var d = b.LockBits(new Rectangle(0, 0, b.Width, b.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        stride = d.Stride;
        var buf = new byte[d.Stride * b.Height];
        Marshal.Copy(d.Scan0, buf, 0, buf.Length);
        b.UnlockBits(d);
        return buf;
    }

    static int Class(byte[] p, int o, int hi, int lo) {
        int bl = p[o], g = p[o + 1], r = p[o + 2];
        int mn = Math.Min(r, Math.Min(g, bl)), mx = Math.Max(r, Math.Max(g, bl));
        if (mn >= hi) return 1;
        if (mx <= lo) return 2;
        return 0;
    }

    public static void Crop(string src, string dst, int x, int y, int w, int h) {
        using (var b = new Bitmap(src))
        using (var c = b.Clone(new Rectangle(x, y, w, h), PixelFormat.Format32bppArgb)) c.Save(dst, ImageFormat.Png);
    }

    public static double[] Score(string capture, string reference, int x, int y, int w, int h, int capW, int capH, int hi, int lo) {
        using (var c = new Bitmap(capture))
        using (var r = new Bitmap(reference)) {
            if (c.Width != capW || c.Height != capH || r.Width != w || r.Height != h) return new double[] { -1, 0, 0, 0 };
            int cs, rs;
            var cp = Pixels(c, out cs);
            var rp = Pixels(r, out rs);
            int light = 0, lightHit = 0, dark = 0, darkHit = 0, other = 0, otherLight = 0;
            for (int j = 0; j < h; j++) {
                for (int i = 0; i < w; i++) {
                    int rc = Class(rp, j * rs + i * 4, hi, lo);
                    int cc = Class(cp, (y + j) * cs + (x + i) * 4, hi, lo);
                    if (rc == 1) { light++; if (cc == 1) lightHit++; }
                    else if (rc == 2) { dark++; if (cc == 2) darkHit++; }
                    else { other++; if (cc == 1) otherLight++; }
                }
            }
            double l = light > 0 ? (double)lightHit / light : (dark > 0 ? 1 : 0);
            double d = dark > 0 ? (double)darkHit / dark : (light > 0 ? 1 : 0);
            double f = other > 0 ? (double)otherLight / other : 0;
            return new double[] { Math.Min(l, d) - f, l, d, f };
        }
    }
}
'@ }

function Test-FrameScreens($res, [string]$Png, [double]$El) {
  $t = $res.Title
  if (-not $Png -or -not $t.PSObject.Properties['Screens'] -or -not $t.Screens) { return }
  if (-not $res.Contains('FrameMs')) { $res['FrameMs'] = [ordered]@{} }
  foreach ($s in $t.Screens) {
    if ($res.FrameMs.Contains($s.Milestone) -and $res.Live.Contains($s.Milestone)) { continue }
    if (-not (Test-Path -LiteralPath $s.Ref)) { continue }
    $v = [TbSig]::Score($Png, $s.Ref, $s.X, $s.Y, $s.W, $s.H, $s.CapW, $s.CapH, 215, 60)
    if ($v[0] -lt $s.Min) { continue }
    $leaf = Split-Path $Png -Leaf
    $res.FrameMs[$s.Milestone] = "${leaf} ($($s.Label) signature score $([Math]::Round($v[0], 2)))"
    if (-not $res.Live.Contains($s.Milestone)) { $res.Live[$s.Milestone] = [Math]::Round($El, 1) }
    Say ("  +{0:N0}s milestone: {1} ({2} on screen, score {3:N2})" -f $El, $s.Milestone, $s.Label, $v[0]) 'Green'
  }
}

function Test-PadScreen($t, [string]$Name, [uint32[]]$Pids, [string]$Png) {
  if (-not $t.PSObject.Properties['PadScreens']) { return $null }
  $s = $t.PadScreens | Where-Object { $_.Name -eq $Name } | Select-Object -First 1
  if (-not $s -or -not (Test-Path -LiteralPath $s.Ref)) { return $null }
  try { $c = [TbFrame]::Capture($Pids, $Png) } catch { return $null }
  if (-not $c -or -not $c.Drawn) { return $null }
  $hi = if ($s.Contains('Hi')) { $s.Hi } else { 215 }
  $v = [TbSig]::Score($Png, $s.Ref, $s.X, $s.Y, $s.W, $s.H, $s.CapW, $s.CapH, $hi, 60)
  return $v[0]
}

function Merge-FrameMilestones($res) {
  if (-not $res.Contains('FrameMs') -or -not $res.Analysis) { return }
  foreach ($k in @($res.FrameMs.Keys)) { if (-not $res.Analysis.Milestones.Contains($k)) { $res.Analysis.Milestones[$k] = $res.FrameMs[$k] } }
}
