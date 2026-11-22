import struct, sys, os, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fmts import ufmt, ALL
N = 65536; d = sys.argv[1]
rules = {
 'zero->1':  lambda s, m: s | 0x7c00 | ((m >> 13) or 1),
 'sticky':   lambda s, m: s | 0x7c00 | (m >> 13) | (1 if m & 0x1fff else 0),
 'quiet':    lambda s, m: s | 0x7e00 | (m >> 13),
}
bad = collections.Counter(); seen = 0; exs = collections.defaultdict(list)
for f in ALL:
    u = ufmt(f)
    if u['num'] != 'float': continue
    out = open(f"{d}/ld_{f}.out", 'rb').read()
    for i in range(N):
        w = struct.unpack_from('<10I', out, i * 64); hw32 = w[6:10]
        halves = [w[4] & 0xffff, w[4] >> 16, w[5] & 0xffff, w[5] >> 16]
        for c in range(4):
            v = hw32[c]
            if (v >> 23) & 0xff == 255 and v & 0x7fffff:
                seen += 1
                for k, r in rules.items():
                    if r((v >> 16) & 0x8000, v & 0x7fffff) != halves[c]:
                        bad[k] += 1
                        if len(exs[k]) < 3: exs[k].append((f, hex(v), hex(halves[c])))
print("NaN components seen:", seen)
for k in rules: print(f"  {k:8} mismatches={bad[k]} {exs[k]}")
