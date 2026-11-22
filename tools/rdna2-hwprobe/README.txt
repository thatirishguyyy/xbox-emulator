rdna2-hwprobe
=============

Runs hand-written RDNA2 kernels on a real AMD GPU and records exactly what the hardware returns.
It settles questions the public ISA documentation leaves open. Currently it probes
tbuffer_{load,store}_format_d16_* (and 32-bit partial stores) for every unified buffer format.
The translator's MTBUF d16 lowering (crates/tombox-xshader/src/hlsl/mtbuf.rs) and its test
model (tests/exec_mem/mtbuf.rs, hardware table tests/exec_mem/mtbuf_hw.rs) follow these results.

No game or console files are involved: the kernels are generated from scratch and the inputs are
synthetic.


Requirements
------------

- Linux with an RDNA2 GPU on the amdgpu driver (/dev/kfd). Recorded on the gfx1036 iGPU of a
  Ryzen 5 9600X; any RDNA2 part should do with GFX=gfx10xx.
- ROCm HIP runtime and headers, plus LLVM 18. On Ubuntu 24.04 / Mint 22:
    sudo apt install rocminfo libamdhip64-dev clang-18 lld-18
  rocminfo should list the GPU agent (e.g. gfx1036).


Run
---

    tools/rdna2-hwprobe/run.sh /tmp/probe                     builds runner + kernels, runs, checks both models
    python3 tools/rdna2-hwprobe/analyze_loads.py /tmp/probe   candidate narrowing rules, layout
    python3 tools/rdna2-hwprobe/nan_rule.py /tmp/probe        fp32 -> fp16 NaN rule
    python3 tools/rdna2-hwprobe/vectors.py /tmp/probe         regenerate the tests/exec_mem/mtbuf_hw.rs body

GFX selects the target (default gfx1036).

Each format gets one load kernel and one store kernel, run on 65536 threads.

- Inputs: every 8- and 16-bit component value appears in every component position. 32-bit
  components get edge values (rounding ties, the fp16 overflow boundary, subnormals, NaNs, int
  extremes) and then random values.
- Loads: registers are preset to sentinels 0xdeadbeef / 0xcafef00d, so preserved halves
  are visible.
- Stores: the output starts filled with 0xAA, so partial writes are visible.

"partial_store32.py gen <dir>", then build and run as run.sh does, followed by
"partial_store32.py show <dir>", does the same check for the 32-bit store forms.


Files
-----

run.sh              Builds runner.cpp and the kernels from gen.py, runs them, then runs model.py and
                    analyze_stores.py. Usage: run.sh <outdir>
runner.cpp          Loads <dir>/k.co and runs each kernel in <dir>/manifest.txt on 65536 threads
                    (256 per group) with args (in, out, N). The output buffer is pre-filled with
                    0xAA. Usage: runner <dir>
gen.py              Generates the probe: one load and one store kernel per buffer format, plus
                    inputs. Each thread loads two dwords, i.e. four packed 16-bit components, and
                    each component is exhaustive over 16 bits. Stores go to the out buffer in
                    regions of N*b bytes each. The buffer descriptor's word 3 (0x31016FAC) is
                    dst_sel xyzw, format 32_FLOAT (overridden by MTBUF), resource_level 1,
                    oob_select raw. Usage: gen.py <outdir> [fmt,fmt,...]
fmts.py             Unified buffer formats (GFX10 numbering), mirroring ufmt() in
                    crates/tombox-xshader/src/hlsl/mtbuf.rs.
model.py            Reference model for d16 loads from raw element bits (what the translator
                    implements), checked against every recorded element. Usage: model.py <outdir>
analyze_stores.py   Compares d16 store results (whole element, missing components 0) with the
                    store model, per format. Usage: analyze_stores.py <outdir>
analyze_loads.py    Compares every d16 load result with the hardware's own 32-bit result for the
                    same element, narrowed by candidate rules (fp32 -> fp16 nearest-even vs toward
                    zero; int truncate vs clamp), and checks the register layout and preserved
                    halves. Usage: analyze_loads.py <outdir>
nan_rule.py         Which fp32 -> fp16 NaN rule d16 loads follow. Usage: nan_rule.py <outdir>
partial_store32.py  32-bit (non-d16) partial stores: whether tbuffer_store_format_x/xy/xyz on a
                    wider format writes the whole element. Every thread stores
                    (0.25, 0.5, 0.75, 1.0) as fp32. Usage: partial_store32.py gen <outdir> |
                    show <outdir>
vectors.py          Prints the hardware vector table for tests/exec_mem/mtbuf_hw.rs: per format a
                    few elements, plus those where naive models (fp32 round-to-nearest-even, int
                    truncation) disagree with the hardware, and d16 stores of +-0.5, NaN, max
                    finite and -0. Usage: vectors.py <outdir>


Results (gfx1036)
-----------------

These results come from 77 formats x 65536 elements. model.py and analyze_stores.py report 0
mismatches. The Rust model in the tests matched all 5.0M load and 14.4M store elements.

d16 loads
- Two components per VGPR: c0/c1 in vdata lo/hi, c2/c3 in vdata+1.
- _x and _xyz leave the upper half of their last VGPR unchanged.
- Components the format lacks read 0, w included. The usual w = 1 comes from the descriptor's
  dst_sel, and these kernels use an identity swizzle.
- unorm / snorm / uscaled / sscaled: the exact value is rounded once to the nearest-even fp16,
  saturating at +-65504. It is not rounded through fp32: unorm16 65519 gives 0x3bff, where
  fp32 would give 0x3c00.
- 32-bit float: rounded toward zero. Finite overflow gives +-65504 and infinity stays infinity.
- NaN: sign | 0x7c00 | ((mant >> 13) or 1). The payload is truncated and not quieted.
- 32-bit int: clamped to 16 bits, never truncated.
- 11-/10-bit float: exact expansion.

d16 stores
- The fp16 input is widened exactly; a NaN keeps its payload.
- unorm: round_ne(saturate(x) * max).
- snorm: halves round away from zero (2-bit field: +-0.5 gives +-1).
- uint / sint: the 16-bit input is zero- or sign-extended, then clamped.
- uscaled / sscaled / 11-/10-bit float stores follow none of the simple rules tried; they are
  not lowered.

Partial stores (d16 and 32-bit): storing fewer components than the format has writes the whole
element, with the missing components as 0. For example, tbuffer_store_format_x of 0.25 to
8_8_8_8_UNORM writes 40 00 00 00.
