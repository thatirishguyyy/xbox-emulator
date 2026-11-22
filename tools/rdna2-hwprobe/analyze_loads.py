import struct, sys, os, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fmts import ufmt, ALL
N = 65536
def f32_to_f16(u, mode):
    s = (u >> 16) & 0x8000; e = (u >> 23) & 0xff; m = u & 0x7fffff
    if e == 255:
        if m == 0: return s | 0x7c00
        return s | 0x7c00 | 0x200 | (m >> 13)
    if e == 0 and m == 0: return s
    if e == 0: mant, exp = m, -149
    else: mant, exp = m | 0x800000, e - 150
    import math
    bl = mant.bit_length() + exp
    e16 = max(bl - 1, -14)
    shift = exp - (e16 - 10)
    if shift >= 0: q, rem, half = mant << shift, 0, 1
    else:
        q = mant >> -shift; rem = mant & ((1 << -shift) - 1); half = 1 << (-shift - 1)
    if mode == 'rne':
        if rem > half or (rem == half and (q & 1)): q += 1
    if q >= 2048: q >>= 1; e16 += 1
    if e16 > 15:
        return s | (0x7c00 if mode == 'rne' else 0x7bff)
    if q < 1024: return s | q
    return s | ((e16 + 15) << 10) | (q - 1024)
def narrow_int(v, num, mode):
    if mode == 'trunc': return v & 0xffff
    if num == 'uint': return min(v, 0xffff)
    sv = v - (1 << 32) if v >> 31 else v
    return max(-32768, min(32767, sv)) & 0xffff
def main():
    res = collections.OrderedDict()
    d = sys.argv[1]
    for f in ALL:
        u = ufmt(f); nc = len(u['fields']); num = u['num']; isint = num in ('uint', 'sint')
        out = open(f"{d}/ld_{f}.out", 'rb').read()
        models = ['trunc', 'clamp'] if isint else ['rne', 'rtz']
        bad = {m: 0 for m in models}; ex = {m: [] for m in models}
        layout_bad = 0; layout_ex = []
        for i in range(N):
            w = struct.unpack_from('<10I', out, i * 64)
            r_x, r_xy, r_xyz0, r_xyz1, r_xyzw0, r_xyzw1 = w[0:6]; hw32 = w[6:10]
            halves = [r_xyzw0 & 0xffff, r_xyzw0 >> 16, r_xyzw1 & 0xffff, r_xyzw1 >> 16]
            exp_layout = [(0xdead0000 | halves[0]), (halves[1] << 16) | halves[0], (halves[1] << 16) | halves[0], 0xcafe0000 | halves[2]]
            if [r_x, r_xy, r_xyz0, r_xyz1] != exp_layout:
                layout_bad += 1
                if len(layout_ex) < 3: layout_ex.append((i, [hex(x) for x in (r_x, r_xy, r_xyz0, r_xyz1, r_xyzw0, r_xyzw1)]))
            for c in range(4):
                v = hw32[c]
                for m in models:
                    want = narrow_int(v, num, m) if isint else f32_to_f16(v, m)
                    if halves[c] != want:
                        bad[m] += 1
                        if len(ex[m]) < 4: ex[m].append((i, c, hex(v), hex(halves[c]), hex(want)))
        res[f] = (num, u['fields'], bad, ex, layout_bad, layout_ex)
        print(f"fmt {f:2} {num:8} {str([b for _, b in u['fields']]):18} layout_bad={layout_bad} " + " ".join(f"{m}={bad[m]}" for m in models))
        for m in models:
            if 0 < bad[m] and bad[m] < 4 * N:
                print("     ", m, ex[m])
        if layout_bad: print("      layout", layout_ex)

if __name__ == '__main__':
    main()
