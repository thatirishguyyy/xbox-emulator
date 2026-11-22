import struct, sys, os, collections, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fmts import ufmt, ALL
N = 65536; d = sys.argv[1]
def h2f(h): return struct.unpack('<e', struct.pack('<H', h))[0]
def rne(x):
    if math.isinf(x): return x
    f = math.floor(x); r = x - f
    return f + 1 if r > 0.5 or (r == 0.5 and f % 2) else f
def pack(h, bits, num, variant):
    mx = (1 << bits) - 1
    if num in ('uint', 'sint'):
        v = h - 0x10000 if (num == 'sint' and variant != 'zext' and h & 0x8000) else h
        if num == 'uint': return min(v, mx)
        lo, hi = -(1 << (bits - 1)), (1 << (bits - 1)) - 1
        return max(lo, min(hi, v)) & mx
    x = h2f(h)
    if num == 'float':
        if bits == 16: return h
        if bits == 32:
            sg, e, m = (h & 0x8000) << 16, (h >> 10) & 31, h & 0x3ff
            if e == 31: return sg | 0x7f800000 | (m << 13)
            if e == 0: return sg | struct.unpack('<I', struct.pack('<f', m * 2.0 ** -24))[0]
            return sg | ((e + 112) << 23) | (m << 13)
        return None
    if math.isnan(x): return 0
    if num == 'unorm': return int(rne(min(max(x, 0.0), 1.0) * mx))
    if num == 'snorm':
        hm = (1 << (bits - 1)) - 1
        p = min(max(x, -1.0), 1.0) * hm
        q = math.floor(abs(p) + 0.5)
        return (q if p >= 0 else -q) & mx
    if num == 'uscaled':
        return int(min(max(x, 0.0), mx)) if variant == 'trunc' else int(min(max(rne(x), 0), mx))
    if num == 'sscaled':
        lo, hi = -(1 << (bits - 1)), (1 << (bits - 1)) - 1
        if math.isinf(x): v = hi if x > 0 else lo
        else: v = math.trunc(x) if variant == 'trunc' else rne(x)
        return int(max(lo, min(hi, v))) & mx
inp = open(f"{d}/st.in", 'rb').read()
for f in ALL:
    u = ufmt(f); fields = u['fields']; nb = u['bytes']; nc = len(fields); num = u['num']
    out = open(f"{d}/st_{f}.out", 'rb').read()
    variants = ['sext', 'zext'] if num == 'sint' else (['rne', 'trunc'] if 'scaled' in num else ['std'])
    res = {}
    for var in variants:
        bad = 0; ex = []; unsup = False
        for r, ncomp in enumerate([1, 2, 3, 4]):
            base = r * N * nb
            for i in range(N):
                hs = struct.unpack_from('<4H', inp, i * 8)
                got = int.from_bytes(out[base + i * nb: base + (i + 1) * nb], 'little')
                want = 0
                for k in range(nc):
                    off, bits = fields[k]
                    p = pack(hs[k] if k < ncomp else 0, bits, num, var)
                    if p is None: unsup = True; break
                    want = (want & ~(((1 << bits) - 1) << off)) | (p << off)
                if unsup: break
                if got != want:
                    bad += 1
                    if len(ex) < 3: ex.append((ncomp, i, [hex(h) for h in hs[:ncomp]], hex(got), hex(want)))
            if unsup: break
        res[var] = 'n/a' if unsup else (bad, ex)
    line = f"fmt {f:2} {num:8} {str([b for _, b in fields]):18} " + " ".join(f"{v}={res[v] if res[v]=='n/a' else res[v][0]}" for v in variants)
    print(line)
    for v in variants:
        if res[v] != 'n/a' and res[v][0]: print("      ", v, res[v][1])
