import struct, sys, os
from fractions import Fraction as F
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fmts import ufmt, ALL
def rne_f16(fr, sat):
    if fr == 0: return 0
    s = 0x8000 if fr < 0 else 0; a = -fr if fr < 0 else fr
    e = a.numerator.bit_length() - a.denominator.bit_length()
    if F(2) ** e > a: e -= 1
    e = max(e, -14)
    q = a / F(2) ** (e - 10); qi = q.numerator // q.denominator; rem = q - qi
    if rem > F(1, 2) or (rem == F(1, 2) and qi & 1): qi += 1
    if qi >= 2048: qi >>= 1; e += 1
    if e > 15: return s | (0x7bff if sat else 0x7c00)
    if qi < 1024: return s | qi
    return s | ((e + 15) << 10) | (qi - 1024)
def rtz_f32_f16(u):
    s = (u >> 16) & 0x8000; e = (u >> 23) & 0xff; m = u & 0x7fffff
    if e == 255: return s | 0x7c00 | (((m >> 13) or 1) if m else 0)
    if e >= 143: return s | 0x7bff
    if e >= 113: return s | ((e - 112) << 10) | (m >> 13)
    sh = 126 - e
    return s | (((m | 0x800000) >> sh) if (e > 0 and sh < 32) else 0)
def small_float_f16(v, mb):
    return (((v >> mb) & 31) << 10) | ((v & ((1 << mb) - 1)) << (10 - mb))
def d16(raw, bits, num):
    mx = (1 << bits) - 1
    sv = raw - (1 << bits) if raw >> (bits - 1) else raw
    if num == 'unorm': return rne_f16(F(raw, mx), True)
    if num == 'snorm': return rne_f16(max(F(sv, mx >> 1), F(-1)), True)
    if num == 'uscaled': return rne_f16(F(raw), True)
    if num == 'sscaled': return rne_f16(F(sv), True)
    if num == 'uint': return min(raw, 0xffff)
    if num == 'sint': return max(-32768, min(32767, sv)) & 0xffff
    if bits == 32: return rtz_f32_f16(raw)
    if bits == 16: return raw
    return small_float_f16(raw, 6 if bits == 11 else 5)
if __name__ == "__main__":
    N = 65536; d = sys.argv[1]; total_bad = 0
    for f in ALL:
        u = ufmt(f); fields = u['fields']; nb = u['bytes']; nc = len(fields)
        inp = open(f"{d}/ld_{f}.in", 'rb').read(); out = open(f"{d}/ld_{f}.out", 'rb').read()
        bad = 0; ex = []; cache = {}
        for i in range(N):
            el = int.from_bytes(inp[i * nb:(i + 1) * nb], 'little')
            w = struct.unpack_from('<6I', out, i * 64)
            halves = [w[4] & 0xffff, w[4] >> 16, w[5] & 0xffff, w[5] >> 16]
            for k in range(4):
                if k < nc:
                    off, bits = fields[k]; raw = (el >> off) & ((1 << bits) - 1)
                    key = (raw, bits)
                    if key not in cache: cache[key] = d16(raw, bits, u['num'])
                    want = cache[key]
                else:
                    want = 0
                if halves[k] != want:
                    bad += 1
                    if len(ex) < 3: ex.append((i, k, hex(halves[k]), hex(want)))
        total_bad += bad
        if bad: print(f"fmt {f} {u['num']} {[b for _, b in fields]}: {bad} mismatches {ex}")
    print("total mismatches over all formats:", total_bad)
