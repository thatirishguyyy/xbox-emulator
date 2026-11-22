#!/bin/sh
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${1:?usage: run.sh <outdir>}
GFX=${GFX:-gfx1036}
mkdir -p "$OUT"
g++ -O2 -D__HIP_PLATFORM_AMD__ "$HERE/runner.cpp" -o "$OUT/runner" -lamdhip64
python3 -I "$HERE/gen.py" "$OUT"
clang-18 -x assembler -target amdgcn-amd-amdhsa -mcpu="$GFX" -c "$OUT/k.s" -o "$OUT/k.o"
ld.lld-18 -shared "$OUT/k.o" -o "$OUT/k.co"
"$OUT/runner" "$OUT"
python3 -I "$HERE/model.py" "$OUT"
python3 -I "$HERE/analyze_stores.py" "$OUT"
