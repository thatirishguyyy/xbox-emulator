import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fmts import ufmt
from gen import kernel, kd, meta, N
FMTS = [14, 56, 60, 65, 69, 75, 44, 22, 62]
def body(f):
    b = ufmt(f)['bytes']
    lines = ["  v_lshlrev_b32 v6, 4, v1", "  v_add_co_u32 v7, vcc_lo, s4, v6", "  v_add_co_ci_u32_e32 v8, vcc_lo, s5, v31, vcc_lo",
             "  global_load_dwordx4 v[10:13], v[7:8], off", "  s_waitcnt vmcnt(0)",
             "  s_mov_b32 s12, s6", "  s_and_b32 s13, s7, 0xffff", f"  v_mul_u32_u24 v2, {b}, v1"]
    for r, (op, regs) in enumerate([("x", "v10"), ("xy", "v[10:11]"), ("xyz", "v[10:12]"), ("xyzw", "v[10:13]")]):
        lines += [f"  s_mov_b32 s20, {r * N * b:#x}", f"  tbuffer_store_format_{op} {regs}, v2, s[12:15], s20 format:{f} offen"]
    return "\n".join(lines)
if __name__ == "__main__":
    mode, out = sys.argv[1], sys.argv[2]
    if mode == "gen":
        os.makedirs(out, exist_ok=True)
        src = [f'.amdgcn_target "amdgcn-amd-amdhsa--{os.environ.get("GFX", "gfx1036")}"', '.amdhsa_code_object_version 4']
        names = []
        for f in FMTS:
            n = f"s32_{f}"; names.append(n); src += [kernel(n, body(f)), kd(n)]
        src.append(meta(names)); open(f"{out}/k.s", "w").write("\n".join(src))
        open(f"{out}/s32.in", "wb").write(struct.pack('<4f', 0.25, 0.5, 0.75, 1.0) * N)
        with open(f"{out}/manifest.txt", "w") as m:
            for f in FMTS: m.write(f"s32_{f} s32.in {4 * N * ufmt(f)['bytes']} s32_{f}.out\n")
    else:
        for f in FMTS:
            u = ufmt(f); b = u["bytes"]; d = open(f"{out}/s32_{f}.out", "rb").read()
            print(f"fmt {f:2} {u['num']:6} {b:2}B", " | ".join(f"{op}: {d[r*N*b:r*N*b+b].hex()}" for r, op in enumerate(["x", "xy", "xyz", "xyzw"])))
