import struct, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fmts import ufmt, ALL
from analyze_loads import f32_to_f16
N = 65536; d = sys.argv[1]
ld, st = [], []
inp_st = open(f"{d}/st.in", 'rb').read()
for f in ALL:
    u = ufmt(f); nb = u['bytes']; num = u['num']; nc = len(u['fields'])
    inp = open(f"{d}/ld_{f}.in", 'rb').read(); out = open(f"{d}/ld_{f}.out", 'rb').read()
    picked = []
    for i in range(N):
        w = struct.unpack_from('<10I', out, i * 64); hw32 = w[6:10]
        halves = [w[4] & 0xffff, w[4] >> 16, w[5] & 0xffff, w[5] >> 16]
        naive = [(min(v, 0xffff) if False else v & 0xffff) if num in ('uint', 'sint') else f32_to_f16(v, 'rne') for v in hw32]
        if any(halves[k] != naive[k] for k in range(nc)): picked.append(i)
        if len(picked) >= 4: break
    for i in [1, 255] + picked:
        if i in [p[1] for p in ld if p[0] == f]: continue
        el = inp[i * nb:(i + 1) * nb] + b'\0' * (16 - nb)
        w = struct.unpack_from('<6I', out, i * 64)
        ld.append((f, i, struct.unpack('<4I', el), (w[4], w[5])))
    if num in ('uscaled', 'sscaled') or (num == 'float' and u['fields'][0][1] in (10, 11)): continue
    so = open(f"{d}/st_{f}.out", 'rb').read()
    cands = [0x3800, 0xb800, 0x7c01, 0x7bff, 0x8000]
    for c in cands:
        i = c
        hs = struct.unpack_from('<4H', inp_st, i * 8)
        for r in ((0, 3) if c == 0x3800 and nc > 1 else (3,)):
            st.append((f, r + 1, hs, so[r * N * nb + i * nb: r * N * nb + (i + 1) * nb] + b'\0' * (16 - nb)))
print(f"pub const HW_D16_LOADS: [(u32, [u32; 4], [u32; 2]); {len(ld)}] = [")
for f, i, el, regs in ld: print(f"    ({f}, [{', '.join(f'{x:#x}' for x in el)}], [{regs[0]:#x}, {regs[1]:#x}]),")
print("];")
print(f"pub const HW_D16_STORES: [(u32, u32, [u32; 4], [u32; 4]); {len(st)}] = [")
for f, n, hs, el in st:
    print(f"    ({f}, {n}, [{', '.join(f'{x:#x}' for x in hs)}], [{', '.join(f'{x:#x}' for x in struct.unpack('<4I', el))}]),")
print("];")
