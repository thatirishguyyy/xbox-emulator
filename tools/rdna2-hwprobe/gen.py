import struct, random, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fmts import ufmt, ALL
N = 65536
W3 = 0x31016FAC
SPEC32 = [0,1,2,0x7fff,0x8000,0xffff,0x10000,0x7fffffff,0x80000000,0xffffffff,0xfffe0000,0x18000,0xffff7fff,0x0001ffff,
          0xfffeffff,0x00008001,0x7fff8000]
def f2u(x): return struct.unpack('<I', struct.pack('<f', x))[0]
SPECF = [0.0,-0.0,1.0,-1.0,65504.0,65505.0,65519.0,65519.99,65520.0,65536.0,1e10,-65520.0,float('inf'),float('-inf'),
         2.0**-24,2.0**-25,3*2.0**-26,2.0**-25*1.0000001,2.0**-14,2.0**-15,1+2.0**-11,1+3*2.0**-11,1+2.0**-11+2.0**-20,
         1-2.0**-12,0.1,1/3,2.0**-30,2.0**-140]
SPECF_RAW = [0x7fc00000,0x7f800001,0xffc00001,0x7fffffff,0x7fa00000,0x00000001,0x807fffff]
def comp32(i, c, rnd):
    specials = SPEC32 + [f2u(x) for x in SPECF] + SPECF_RAW
    if i < len(specials): return specials[(i + c) % len(specials)]
    if i % 2 == 0: return rnd.getrandbits(32)
    e = rnd.randint(-27, 16); m = rnd.getrandbits(23); s = rnd.getrandbits(1)
    return (s << 31) | (((e + 127) & 0xff) << 23) | m
def load_input(f):
    u = ufmt(f); rnd = random.Random(1000 + f); out = bytearray()
    for i in range(N):
        val = 0
        for c, (off, bits) in enumerate(u['fields']):
            v = comp32(i, c, rnd) if bits == 32 else (i + c * 0x3d17) & ((1 << bits) - 1)
            val |= v << off
        out += val.to_bytes(u['bytes'], 'little')
    return bytes(out)
def store_input():
    out = bytearray()
    for i in range(N):
        cs = [(i + c * 0x3d17) & 0xffff for c in range(4)]
        out += struct.pack('<HHHH', *cs)
    return bytes(out)
def kernel(name, body):
    return f"""
.text
.globl {name}
.p2align 8
.type {name},@function
{name}:
  s_load_dwordx4 s[4:7], s[0:1], 0x0
  v_mov_b32 v31, 0
  s_waitcnt lgkmcnt(0)
  s_lshl_b32 s9, s2, 8
  v_add_nc_u32 v1, s9, v0
  s_mov_b32 s12, s4
  s_and_b32 s13, s5, 0xffff
  s_mov_b32 s14, -1
  s_mov_b32 s15, {W3:#x}
  v_lshlrev_b32 v3, 6, v1
  v_add_co_u32 v4, vcc_lo, s6, v3
  v_add_co_ci_u32_e32 v5, vcc_lo, s7, v31, vcc_lo
{body}
  s_waitcnt vmcnt(0)
  s_endpgm
"""
def ld_body(f):
    b = ufmt(f)['bytes']
    S = lambda regs, vals: "\n".join(f"  v_mov_b32 v{r}, {v:#x}" for r, v in zip(regs, vals))
    return f"""  v_mul_u32_u24 v2, {b}, v1
{S([10,11,12,13,14,15,16,17,18,19],[0xdeadbeef,0xdeadbeef,0xdeadbeef,0xcafef00d,0xdeadbeef,0xcafef00d,0xdeadbeef,0xdeadbeef,0xdeadbeef,0xdeadbeef])}
  tbuffer_load_format_d16_x v10, v2, s[12:15], 0 format:{f} offen
  tbuffer_load_format_d16_xy v11, v2, s[12:15], 0 format:{f} offen
  tbuffer_load_format_d16_xyz v[12:13], v2, s[12:15], 0 format:{f} offen
  tbuffer_load_format_d16_xyzw v[14:15], v2, s[12:15], 0 format:{f} offen
  tbuffer_load_format_xyzw v[16:19], v2, s[12:15], 0 format:{f} offen
  s_waitcnt vmcnt(0)
  global_store_dwordx4 v[4:5], v[10:13], off
  global_store_dwordx4 v[4:5], v[14:17], off offset:16
  global_store_dwordx2 v[4:5], v[18:19], off offset:32"""
def st_body(f):
    b = ufmt(f)['bytes']
    lines = [f"  v_lshlrev_b32 v6, 3, v1",
             "  v_add_co_u32 v7, vcc_lo, s4, v6", "  v_add_co_ci_u32_e32 v8, vcc_lo, s5, v31, vcc_lo",
             "  global_load_dwordx2 v[10:11], v[7:8], off", "  s_waitcnt vmcnt(0)",
             "  s_mov_b32 s12, s6", "  s_and_b32 s13, s7, 0xffff",
             f"  v_mul_u32_u24 v2, {b}, v1"]
    for r, (op, regs) in enumerate([("d16_x","v10"),("d16_xy","v10"),("d16_xyz","v[10:11]"),("d16_xyzw","v[10:11]")]):
        lines.append(f"  s_mov_b32 s20, {r * N * b:#x}")
        lines.append(f"  tbuffer_store_format_{op} {regs}, v2, s[12:15], s20 format:{f} offen")
    return "\n".join(lines)
def meta(names):
    ks = "\n".join(f"""  - .name: {n}
    .symbol: {n}.kd
    .kernarg_segment_size: 24
    .kernarg_segment_align: 8
    .group_segment_fixed_size: 0
    .private_segment_fixed_size: 0
    .wavefront_size: 32
    .sgpr_count: 32
    .vgpr_count: 32
    .max_flat_workgroup_size: 256
    .args:
      - {{ .size: 8, .offset: 0, .value_kind: global_buffer, .address_space: global }}
      - {{ .size: 8, .offset: 8, .value_kind: global_buffer, .address_space: global }}
      - {{ .size: 8, .offset: 16, .value_kind: by_value }}""" for n in names)
    return f".amdgpu_metadata\n---\namdhsa.version: [ 1, 1 ]\namdhsa.kernels:\n{ks}\n...\n.end_amdgpu_metadata\n"
def kd(name):
    return f"""
.rodata
.p2align 6
.amdhsa_kernel {name}
  .amdhsa_user_sgpr_kernarg_segment_ptr 1
  .amdhsa_system_sgpr_workgroup_id_x 1
  .amdhsa_next_free_vgpr 32
  .amdhsa_next_free_sgpr 32
  .amdhsa_wavefront_size32 1
  .amdhsa_kernarg_size 24
.end_amdhsa_kernel
"""
if __name__ == "__main__":
    out = sys.argv[1]; fl = [int(x) for x in sys.argv[2].split(',')] if len(sys.argv) > 2 else ALL
    os.makedirs(out, exist_ok=True)
    src = [f'.amdgcn_target "amdgcn-amd-amdhsa--{os.environ.get("GFX", "gfx1036")}"', '.amdhsa_code_object_version 4']
    names = []
    for f in fl:
        for kind, body in (("ld", ld_body(f)), ("st", st_body(f))):
            n = f"{kind}_{f}"; names.append(n); src.append(kernel(n, body)); src.append(kd(n))
        open(f"{out}/ld_{f}.in", "wb").write(load_input(f))
    open(f"{out}/st.in", "wb").write(store_input())
    src.append(meta(names))
    open(f"{out}/k.s", "w").write("\n".join(src))
    with open(f"{out}/manifest.txt", "w") as m:
        for f in fl:
            b = ufmt(f)['bytes']
            m.write(f"ld_{f} ld_{f}.in {N*64} ld_{f}.out\n")
            m.write(f"st_{f} st.in {4*N*b} st_{f}.out\n")
