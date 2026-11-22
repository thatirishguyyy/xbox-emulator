pub const MODE_LINEAR_ALIGNED: u32 = 8;
pub const MODE_1D_THIN: u32 = 13;
pub const MODE_2D_THIN: u32 = 14;
pub const MODE_LINEAR_GENERAL: u32 = 31;
pub const MAX_MIPS: usize = 15;
pub const MIP_STRIDE: usize = 0x60;
pub const PLANE_STRIDE: usize = 0x20 + MAX_MIPS * MIP_STRIDE;
pub const PLANE_PITCH: usize = PLANE_STRIDE + 8;
pub const MAX_PLANES: usize = 4;
pub const DIMENSION_OFFSET: usize = 0x20 + MAX_PLANES * PLANE_STRIDE + 0x18;
pub const LAYOUT_BYTES: usize = DIMENSION_OFFSET + 8;

const TILE: [[u32; 32]; 2] = [
    [0x00800150, 0x00800950, 0x00801150, 0x00801950, 0x00802950, 0x00800148, 0x00801158, 0x00802958, 0x00000144, 0x00000148, 0x02000150, 0x06000154, 0x06000158, 0x00400148, 0x02400150, 0x02400170, 0x06400154, 0x06400158, 0x0640016c, 0x0100014c, 0x0100015c, 0x01000174, 0x01000164, 0x01000168, 0x0100017c, 0x01000160, 0x01000178, 0x00c00148, 0x02c00150, 0x06c00154, 0x06c00158, 0x00000000],
    [0x00800310, 0x00800b10, 0x00801310, 0x00801b10, 0x00802b10, 0x00800308, 0x00801318, 0x00802b18, 0x00000304, 0x00000308, 0x02000310, 0x06000314, 0x06000318, 0x00400308, 0x02400310, 0x02400330, 0x06400314, 0x06400318, 0x0640032c, 0x0100030c, 0x0100031c, 0x01000334, 0x01000324, 0x01000328, 0x0100033c, 0x01000320, 0x01000338, 0x00c00308, 0x02c00310, 0x06c00314, 0x06c00318, 0x00000000],
];
const MACRO: [[u8; 16]; 2] = [[0xa8, 0x94, 0x90, 0x90, 0x90, 0x90, 0x40, 0x00, 0xed, 0xe9, 0xe8, 0xe4, 0xd0, 0x90, 0x40, 0x00], [0xe8, 0xd8, 0xd4, 0xd4, 0xd0, 0x90, 0x40, 0x00, 0xec, 0xe8, 0xd4, 0xd0, 0x80, 0x40, 0x00, 0x00]];
const ROWS: [[u16; 8]; 122] = [
    [0, 1, 1, 1, 0, 0, 0, 0],
    [128, 1, 1, 1, 0, 1, 0, 0],
    [128, 1, 1, 1, 0, 2, 0, 0],
    [128, 1, 1, 1, 0, 3, 0, 0],
    [128, 1, 1, 1, 0, 4, 0, 0],
    [96, 1, 1, 1, 0, 5, 0, 0],
    [96, 1, 1, 1, 0, 6, 0, 0],
    [96, 1, 1, 1, 0, 7, 0, 0],
    [96, 1, 1, 1, 0, 8, 0, 0],
    [64, 1, 1, 1, 0, 9, 0, 0],
    [64, 1, 1, 1, 0, 10, 0, 0],
    [64, 1, 1, 1, 0, 11, 0, 0],
    [64, 1, 1, 1, 0, 12, 0, 0],
    [64, 1, 1, 1, 0, 13, 0, 0],
    [64, 1, 1, 1, 0, 14, 0, 0],
    [64, 1, 1, 1, 0, 15, 0, 0],
    [64, 1, 1, 1, 0, 16, 0, 0],
    [64, 1, 1, 1, 0, 17, 0, 0],
    [64, 1, 1, 1, 0, 18, 0, 0],
    [64, 1, 1, 1, 1, 39, 60, 0],
    [64, 1, 1, 1, 1, 41, 192, 0],
    [64, 1, 1, 1, 1, 41, 0, 0],
    [64, 1, 1, 1, 1, 0, 192, 0],
    [32, 1, 1, 1, 0, 23, 0, 0],
    [32, 1, 1, 1, 0, 24, 0, 0],
    [32, 1, 1, 1, 0, 25, 0, 0],
    [32, 1, 1, 1, 0, 26, 0, 0],
    [32, 1, 1, 1, 0, 27, 0, 0],
    [32, 1, 1, 1, 0, 28, 0, 0],
    [32, 1, 1, 1, 0, 29, 0, 0],
    [32, 1, 1, 1, 0, 30, 0, 0],
    [32, 1, 1, 1, 0, 31, 0, 0],
    [32, 1, 1, 1, 0, 32, 0, 0],
    [32, 1, 1, 1, 0, 33, 0, 0],
    [32, 1, 1, 1, 0, 34, 0, 0],
    [32, 1, 1, 1, 0, 35, 0, 0],
    [32, 1, 1, 1, 0, 36, 0, 0],
    [32, 1, 1, 1, 0, 37, 0, 0],
    [32, 1, 1, 1, 0, 38, 0, 0],
    [32, 1, 1, 1, 0, 39, 0, 0],
    [32, 1, 1, 1, 0, 41, 0, 0],
    [32, 1, 1, 1, 0, 41, 0, 0],
    [32, 1, 1, 1, 0, 42, 0, 0],
    [32, 1, 1, 1, 0, 43, 0, 0],
    [32, 1, 1, 1, 1, 44, 60, 0],
    [32, 1, 1, 1, 1, 46, 192, 0],
    [32, 1, 1, 1, 1, 46, 0, 0],
    [32, 1, 1, 1, 1, 0, 192, 0],
    [16, 1, 1, 1, 0, 48, 0, 0],
    [16, 1, 1, 1, 0, 49, 0, 0],
    [16, 1, 1, 1, 0, 50, 0, 0],
    [16, 1, 1, 1, 0, 51, 0, 0],
    [16, 1, 1, 1, 0, 52, 0, 0],
    [16, 1, 1, 1, 0, 53, 0, 0],
    [16, 1, 1, 1, 0, 54, 0, 0],
    [16, 1, 1, 1, 0, 56, 0, 0],
    [16, 1, 1, 1, 0, 56, 0, 0],
    [16, 1, 1, 1, 0, 57, 0, 0],
    [16, 1, 1, 1, 0, 58, 0, 0],
    [16, 1, 1, 1, 0, 59, 0, 0],
    [8, 1, 1, 1, 0, 60, 0, 0],
    [8, 1, 1, 1, 0, 61, 0, 0],
    [8, 1, 1, 1, 0, 62, 0, 0],
    [8, 1, 1, 1, 0, 63, 0, 0],
    [8, 1, 1, 1, 0, 64, 0, 0],
    [8, 1, 1, 1, 0, 65, 0, 0],
    [1, 1, 8, 1, 0, 66, 0, 0],
    [32, 1, 1, 1, 0, 67, 0, 0],
    [16, 1, 2, 1, 0, 68, 0, 4],
    [16, 1, 2, 1, 0, 69, 0, 4],
    [4, 1, 4, 4, 0, 70, 0, 8],
    [4, 1, 4, 4, 0, 71, 0, 8],
    [4, 1, 4, 4, 0, 72, 0, 8],
    [8, 1, 4, 4, 0, 73, 0, 16],
    [8, 1, 4, 4, 0, 74, 0, 16],
    [8, 1, 4, 4, 0, 75, 0, 16],
    [8, 1, 4, 4, 0, 76, 0, 16],
    [8, 1, 4, 4, 0, 77, 0, 16],
    [8, 1, 4, 4, 0, 78, 0, 16],
    [4, 1, 4, 4, 0, 79, 0, 8],
    [4, 1, 4, 4, 0, 80, 0, 8],
    [4, 1, 4, 4, 0, 81, 0, 8],
    [8, 1, 4, 4, 0, 82, 0, 16],
    [8, 1, 4, 4, 0, 83, 0, 16],
    [8, 1, 4, 4, 0, 84, 0, 16],
    [16, 1, 1, 1, 0, 85, 0, 0],
    [16, 1, 1, 1, 0, 86, 0, 0],
    [32, 1, 1, 1, 0, 87, 0, 0],
    [32, 1, 1, 1, 0, 88, 0, 0],
    [32, 1, 1, 1, 0, 89, 0, 0],
    [32, 1, 1, 1, 0, 90, 0, 0],
    [32, 1, 1, 1, 0, 91, 0, 0],
    [32, 1, 1, 1, 0, 92, 0, 0],
    [32, 1, 1, 1, 0, 93, 0, 0],
    [8, 1, 4, 4, 0, 94, 0, 16],
    [8, 1, 4, 4, 0, 95, 0, 16],
    [8, 1, 4, 4, 0, 96, 0, 16],
    [8, 1, 4, 4, 0, 97, 0, 16],
    [8, 1, 4, 4, 0, 98, 0, 16],
    [8, 1, 4, 4, 0, 99, 0, 16],
    [32, 1, 1, 1, 0, 100, 0, 0],
    [32, 1, 1, 1, 0, 101, 0, 0],
    [64, 1, 1, 1, 0, 102, 0, 0],
    [8, 1, 1, 1, 2, 61, 49, 0],
    [16, 1, 1, 1, 2, 56, 35, 0],
    [0, 1, 1, 1, 2, 56, 35, 0],
    [0, 1, 1, 1, 2, 61, 49, 0],
    [16, 1, 2, 1, 0, 107, 0, 0],
    [0, 1, 1, 1, 0, 108, 0, 0],
    [0, 1, 1, 1, 0, 109, 0, 0],
    [0, 1, 1, 1, 4, 61, 49, 0],
    [8, 1, 1, 1, 0, 111, 0, 0],
    [8, 1, 1, 1, 0, 112, 0, 0],
    [8, 1, 1, 1, 0, 113, 0, 0],
    [16, 1, 1, 1, 0, 114, 0, 0],
    [16, 1, 1, 1, 0, 115, 0, 0],
    [32, 1, 1, 1, 0, 116, 0, 0],
    [32, 1, 1, 1, 0, 117, 0, 0],
    [16, 1, 1, 1, 1, 56, 192, 0],
    [16, 1, 1, 1, 1, 56, 0, 0],
    [8, 1, 1, 1, 1, 0, 192, 0],
    [0, 1, 1, 1, 0, 0, 0, 0],
];

const PIPES: [u64; 2] = [4, 8];
const ROW_BYTES: u64 = 2048;
const PIPE_INTERLEAVE: u64 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub bpe: u32,
    pub block: u32,
}

pub fn format(dxgi: u32) -> Option<Format> {
    let f = |bpe, block| Some(Format { bpe, block });
    match dxgi {
        1..=4 => f(16, 1),
        5..=8 => f(12, 1),
        9..=22 => f(8, 1),
        23..=47 => f(4, 1),
        48..=59 => f(2, 1),
        60..=65 => f(1, 1),
        67 => f(4, 1),
        70..=72 | 79..=81 => f(8, 4),
        73..=78 | 82..=84 | 94..=99 => f(16, 4),
        85 | 86 => f(2, 1),
        87..=93 => f(4, 1),
        115 => f(2, 1),
        116 | 117 => f(4, 1),
        119 => f(4, 1),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
struct Fi {
    bpe: u64,
    padw: u64,
    padh: u64,
    a: u64,
    b: u64,
    c: u64,
    px: u64,
    e: u64,
    bc: bool,
}

enum Kind {
    Plain(Fi),
    Planar(u32, u32, u16),
    Zero,
}

#[allow(clippy::too_many_arguments)]
fn plain_kind(bpe: u64, padw: u64, padh: u64, a: u64, b: u64, c: u64, px: u64, e: u64, bc: bool) -> Option<Kind> {
    Some(Kind::Plain(Fi { bpe, padw, padh, a, b, c, px, e, bc }))
}

fn fmt_info(dxgi: u32, dim: u32, plane: bool) -> Option<Kind> {
    if dxgi == 192 {
        return plain_kind(1, 1, 1, 1, 1, 1, 1, 1, false);
    }
    let r = ROWS.get(dxgi as usize)?;
    let [bits, a, b, c, planar, f4, f8, f13] = r.map(|v| v as u64);
    if planar != 0 && f8 != 0 && dim != 4 && !plane {
        return Some(Kind::Planar(f4 as u32, f8 as u32, planar as u16));
    }
    match bits {
        0 => Some(Kind::Zero),
        96 => None,
        _ if c == 4 => {
            if dim == 2 && f13 >= 8 {
                return None;
            }
            plain_kind(bits * 2, 4, 4, 1, 4, 4, 4, bits * 2, true)
        }
        _ if b == 8 => plain_kind(1, 8, 1, 1, 8, 1, 8, 1, false),
        _ => plain_kind(bits / 8, 1, 1, a, b, c, 1, bits * b * c / 8, false),
    }
}

fn al(x: u64, a: u64) -> u64 {
    x.div_ceil(a) * a
}

fn p2(x: u64) -> u64 {
    x.max(1).next_power_of_two()
}

fn log2(x: u64) -> u32 {
    63 - x.max(1).leading_zeros()
}

#[derive(Clone, Copy, Debug)]
struct Tile {
    idx: u32,
    micro: u32,
    thick: u64,
    macro_tiled: bool,
    prt: bool,
    pipes: u64,
    banks: u64,
    mw: u64,
    mh: u64,
    base_align: u64,
}

fn thickness(arr: u32) -> u64 {
    match arr {
        3 | 7 | 9 | 10 | 13 | 15 => 4,
        8 | 14 => 8,
        _ => 1,
    }
}

fn tile_info(hw: usize, idx: u32, bpe: u64, samples: u64) -> Tile {
    let reg = TILE[hw][idx as usize];
    let arr = (reg >> 2) & 15;
    let micro = (reg >> 22) & 7;
    let split_reg = 64u64 << ((reg >> 11) & 7);
    let ssplit = 1u64 << ((reg >> 25) & 3);
    let thick = thickness(arr);
    let macro_tiled = arr >= 4;
    let prt = matches!(arr, 5 | 6 | 9 | 10 | 11 | 15);
    let pipes = PIPES[hw];
    let mut t = Tile { idx, micro, thick, macro_tiled, prt, pipes, banks: 0, mw: 0, mh: 0, base_align: 0 };
    if macro_tiled {
        let bytes1x = bpe * 64 * thick;
        let split = if micro == 2 { split_reg } else { 256u64.max(ssplit * bytes1x) };
        let splitc = ROW_BYTES.min(split);
        let tb = 64u64.max(splitc.min(samples * bytes1x));
        let mi = log2(tb / 64) as usize + if prt { 8 } else { 0 };
        let m = MACRO[hw][mi] as u64;
        let (mut bw, mut bh, mut asp, banks) = (1u64 << (m & 3), 1u64 << ((m >> 2) & 3), 1u64 << ((m >> 4) & 3), 2u64 << ((m >> 6) & 3));
        let tsize = splitc.min(bpe * 64 * thick * samples);
        let bha = 1u64.max(PIPE_INTERLEAVE / (tsize * bw));
        if bh % bha != 0 {
            bh = bh.max(bha);
        }
        if samples == 1 {
            asp = asp.max(1u64.max(PIPE_INTERLEAVE / (tsize * pipes * bw)));
        }
        while bw > 1 && tsize * bw * bh > ROW_BYTES {
            bw >>= 1;
        }
        while bh > bha && tsize * bw * bh > ROW_BYTES {
            bh >>= 1;
        }
        t.banks = banks;
        t.mw = 8 * bw * pipes * asp;
        t.mh = 8 * bh * banks / asp;
        t.base_align = pipes * bw * banks * bh * tsize;
    }
    t
}

pub const KIND_LINEAR_GENERAL: u8 = 0;
pub const KIND_LINEAR_ALIGNED: u8 = 1;
pub const KIND_1D: u8 = 2;
pub const KIND_2D: u8 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mip {
    pub size: u64,
    pub offset: u64,
    pub slice: u64,
    pub pitch_pixels: u32,
    pub pitch_bytes: u32,
    pub align: u32,
    pub padded_w: u32,
    pub padded_h: u32,
    pub padded_d: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub samples: u32,
    pub mode: u32,
    pub kind: u8,
    pub thick: u32,
    pub q48: u64,
    pub q50: u64,
    pub macro_w: u32,
    pub macro_h: u32,
}

impl Mip {
    pub fn is_tiled(&self) -> bool {
        self.kind == KIND_1D || self.kind == KIND_2D
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plane {
    pub size: u64,
    pub offset: u64,
    pub align: u64,
    pub bpe: u32,
    pub mips: Vec<Mip>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: u64,
    pub align: u64,
    pub fmt: Format,
    pub dxgi: u32,
    pub dimension: u32,
    pub mips: Vec<Mip>,
    pub array: u32,
    pub is_3d: bool,
    pub planes: Vec<Option<Plane>>,
    pub zero: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Desc {
    pub dimension: u32,
    pub width: u64,
    pub height: u32,
    pub depth_or_array: u32,
    pub mips: u32,
    pub dxgi: u32,
    pub samples: u32,
    pub mode: u32,
    pub pitch: u32,
}

fn hw_index() -> usize {
    crate::HARDWARE_FLAG.load(core::sync::atomic::Ordering::Relaxed) as usize
}

fn remap(mode: u32) -> u32 {
    match mode & 31 {
        11 => 16,
        12 => 17,
        15 => 14,
        18 => 17,
        27 => 13,
        28 => 14,
        29 => 16,
        30 => 17,
        m => m,
    }
}

fn full_chain(w: u64, h: u64, z: u64, is3d: bool) -> u32 {
    let mut full = 1u32;
    while (w >> full) != 0 || (h >> full) != 0 || (is3d && (z >> full) != 0) {
        full += 1;
    }
    full
}

fn plain(d: &Desc, fi: Fi, hw: usize) -> Option<Plane> {
    let bpe = fi.bpe;
    let mode = remap(d.mode);
    let arr = if mode == 31 { 0 } else { (TILE[hw][mode as usize] >> 2) & 15 };
    if fi.bc && arr <= 1 {
        return None;
    }
    if d.width == 0 || d.height == 0 || d.depth_or_array == 0 || d.width > 16384 || d.height > 16384 {
        return None;
    }
    let is3d = d.dimension == 4;
    let (w, h, da) = (d.width, d.height as u64, d.depth_or_array as u64);
    let full = full_chain(w, h, da, is3d);
    let samples = d.samples.max(1) as u64;
    let n = if samples > 1 {
        1
    } else if d.mips == 0 {
        full
    } else if d.mips & 0xf == 0 {
        return None;
    } else {
        d.mips & 0xf
    };
    if n as usize > MAX_MIPS {
        return None;
    }
    let pow2 = (if d.mips != 0 { d.mips } else { full }) != 1;
    let pq = if d.pitch != 0 { ((d.pitch as u64 - 1) & 0x3fff) + 1 } else { 0 };
    let mut disp0 = false;
    let mut mips = Vec::with_capacity(n as usize);
    let (mut acc, mut balign, mut prevp) = (0u64, 0u64, 0u64);
    let mut frozen: Option<u64> = None;
    for m in 0..n as u64 {
        let wsrc = if pq != 0 { (pq >> m).max(1) } else { (w >> m).max(1) };
        let (wm, hm) = if pow2 { (p2(wsrc), p2((h >> m).max(1))) } else { (wsrc, h) };
        let zm = if is3d { (da >> m).max(1) } else { da };
        let mut pz = if pow2 { p2(zm) } else { zm };
        let mut ew = wm.div_ceil(fi.padw);
        let eh = hm.div_ceil(fi.padh);
        let uw = ((w >> m).max(1) * fi.a).div_ceil(fi.b);
        let uh = (h >> m).max(1).div_ceil(fi.c);
        if m > 0 && (pq == 0 || disp0) {
            ew = ew.max(prevp / 2);
        }
        let mut rep = mode;
        let mut banks = 0u64;
        let mut pipes = 1u64;
        let (mut mw, mut mh) = (0u64, 0u64);
        let (pw, ph, a, thick, kind);
        if mode == 31 {
            (pw, ph, a, thick, kind) = (ew, eh, bpe, 1, KIND_LINEAR_GENERAL);
        } else if arr == 1 {
            let st = 8u64.max(64 / bpe);
            let mut x = al(ew, st);
            while (x * eh * bpe) % 256 != 0 || (x * eh) % 64 != 0 {
                x += st;
            }
            (pw, ph, a, thick, kind) = (x, eh, 256, 1, KIND_LINEAR_ALIGNED);
        } else {
            let t = tile_info(hw, mode, bpe, samples);
            let mut th = t.thick;
            let mut k = if t.macro_tiled { KIND_2D } else { KIND_1D };
            if t.macro_tiled && m > 0 && (ew < t.mw || eh < t.mh) {
                k = KIND_1D;
                rep = match t.micro {
                    2 => 5,
                    1 => 13,
                    _ => t.idx,
                };
                if th == 8 {
                    th = 4;
                }
            }
            if k == KIND_2D {
                let pa = if t.micro == 0 && m == 0 { t.mw * fi.e * t.banks / (2 * t.pipes) / fi.padh } else { t.mw };
                if t.micro == 0 && m == 0 {
                    disp0 = true;
                }
                (pw, ph, a) = (al(ew, pa), al(eh, t.mh), t.base_align);
                if !t.prt {
                    banks = t.banks;
                    pipes = t.pipes;
                }
                (mw, mh) = (t.mw, t.mh);
            } else {
                let mut pa = 8u64;
                if t.micro == 0 && !t.macro_tiled && m == 0 {
                    pa = pa.max(64u64.max(256 / fi.e) / fi.px);
                }
                let mut x = al(ew, pa);
                let y = al(eh, 8);
                while (x * y * bpe * th) % 256 != 0 {
                    x += 8;
                }
                (pw, ph, a) = (x, y, 256);
            }
            pz = al(pz, th);
            (thick, kind) = (th, k);
        }
        let slice = pw * ph * bpe * samples * thick;
        let size = slice * (pz / thick);
        acc = al(acc, a);
        let offset = if mode == 31 {
            let o = frozen.unwrap_or(acc & !0xff);
            if frozen.is_none() && size < 256 {
                frozen = Some(o);
            }
            o
        } else {
            acc
        };
        let (q48, q50) = if banks != 0 {
            let sh = 8 + log2(pipes);
            ((banks - 1) << sh, (banks / 2 - 1) << sh)
        } else {
            (0, 0)
        };
        mips.push(Mip {
            size,
            offset,
            slice,
            pitch_pixels: (pw * fi.px) as u32,
            pitch_bytes: (pw * bpe) as u32,
            align: a as u32,
            padded_w: pw as u32,
            padded_h: ph as u32,
            padded_d: pz as u32,
            width: uw as u32,
            height: uh as u32,
            depth: zm as u32,
            samples: samples as u32,
            mode: rep,
            kind,
            thick: thick as u32,
            q48,
            q50,
            macro_w: mw as u32,
            macro_h: mh as u32,
        });
        acc += size;
        balign = balign.max(a);
        prevp = pw;
    }
    Some(Plane { size: acc, offset: 0, align: balign, bpe: bpe as u32, mips })
}

fn empty(d: &Desc, zero: bool) -> Layout {
    Layout { size: 0, align: 0, fmt: Format { bpe: 0, block: 1 }, dxgi: d.dxgi, dimension: d.dimension, mips: Vec::new(), array: d.depth_or_array, is_3d: d.dimension == 4, planes: Vec::new(), zero }
}

pub fn is_planar(dxgi: u32) -> bool {
    ROWS.get(dxgi as usize).is_some_and(|r| r[4] != 0 && r[6] != 0)
}

pub fn compute(d: &Desc) -> Option<Layout> {
    compute_with(d, false)
}

pub fn depth_parent_format(dxgi: u32) -> u32 {
    match dxgi {
        22 => 20,
        47 => 45,
        120 => 118,
        other => other,
    }
}

pub fn is_depth_stencil_planar(dxgi: u32) -> bool {
    let fmt = depth_parent_format(dxgi);
    ROWS.get(fmt as usize).is_some_and(|r| r[4] == 1 && r[5] != 0 && r[6] != 0)
}

pub fn compute_depth_msaa(d: &Desc) -> Option<Layout> {
    if d.samples <= 1 || d.dimension != 3 || d.mips != 1 || !is_depth_stencil_planar(d.dxgi) {
        return None;
    }
    let mut base = *d;
    base.dxgi = depth_parent_format(d.dxgi);
    let mut l = compute_with(&base, true)?;
    l.dxgi = d.dxgi;
    Some(l)
}

fn compute_with(d: &Desc, msaa_planes: bool) -> Option<Layout> {
    let hw = hw_index();
    if d.pitch != 0 && (d.pitch as u64) < d.width {
        return None;
    }
    match fmt_info(d.dxgi, d.dimension, false)? {
        Kind::Zero => Some(empty(d, true)),
        Kind::Plain(fi) => {
            let p = plain(d, fi, hw)?;
            let mut l = empty(d, false);
            l.size = p.size;
            l.align = p.align;
            l.fmt = Format { bpe: p.bpe, block: fi.padw as u32 };
            l.mips = p.mips;
            Some(l)
        }
        Kind::Planar(f4, f8, kind) => {
            if d.samples > 1 && !msaa_planes {
                return None;
            }
            let video = kind != 1 && ROWS[d.dxgi as usize][0] != 0 && d.dimension != 2;
            let mut base = *d;
            if video && base.mips == 0 {
                base.mips = full_chain(d.width, d.height as u64, 1, false);
            }
            let mut planes: Vec<Option<Plane>> = Vec::new();
            let (mut acc, mut align) = (0u64, 0u64);
            let mut block = 1u32;
            for (i, pf) in [f4, f8].into_iter().enumerate() {
                if pf == 0 {
                    planes.push(None);
                    continue;
                }
                let mut pd = base;
                pd.dxgi = pf;
                if video {
                    pd.width = al(d.width, 256);
                    if i == 1 {
                        pd.width /= 2;
                        pd.height = (d.height / 2).max(1);
                    }
                }
                let Kind::Plain(fi) = fmt_info(pf, d.dimension, true)? else { return None };
                let mut p = plain(&pd, fi, hw)?;
                if planes.iter().all(|p| p.is_none()) {
                    block = fi.padw as u32;
                }
                acc = al(acc, p.align);
                p.offset = acc;
                acc += p.size;
                align = align.max(p.align);
                planes.push(Some(p));
            }
            let mut l = empty(d, false);
            l.size = acc;
            l.align = align;
            if let Some(p) = planes.iter().flatten().next() {
                l.fmt = Format { bpe: p.bpe, block };
                l.mips = p.mips.clone();
            }
            l.planes = planes;
            Some(l)
        }
    }
}

fn storage_offset(l: &Layout, mip: usize) -> u64 {
    let mut acc = 0u64;
    for (i, m) in l.mips.iter().enumerate() {
        acc = al(acc, m.align as u64);
        if i == mip {
            return acc;
        }
        acc += m.size;
    }
    acc
}

pub fn element_offset(l: &Layout, mip: usize, x: u64, y: u64, z: u64) -> Option<u64> {
    let m = l.mips.get(mip)?;
    if x >= m.padded_w as u64 || y >= m.padded_h as u64 || z >= m.padded_d as u64 {
        return None;
    }
    let bpe = l.fmt.bpe as u64;
    let thick = m.thick.max(1) as u64;
    let base = storage_offset(l, mip) + (z / thick) * m.slice;
    let zl = z % thick;
    let pw = m.padded_w as u64;
    let within = match m.kind {
        KIND_LINEAR_GENERAL | KIND_LINEAR_ALIGNED => (y * pw + x) * bpe,
        KIND_1D => {
            let tile = (y / 8) * (pw / 8) + x / 8;
            tile * 64 * thick * bpe + (zl * 64 + zorder(x, y)) * bpe
        }
        _ => {
            let (mw, mh) = (m.macro_w.max(8) as u64, m.macro_h.max(8) as u64);
            let macro_index = (y / mh) * (pw / mw) + x / mw;
            let (lx, ly) = (x % mw, y % mh);
            let micro = (ly / 8) * (mw / 8) + lx / 8;
            macro_index * mw * mh * thick * bpe + (micro * 64 * thick + zl * 64 + zorder(x, y)) * bpe
        }
    };
    Some(base + within)
}

fn zorder(x: u64, y: u64) -> u64 {
    (x & 1) | ((y & 1) << 1) | ((x & 2) << 1) | ((y & 2) << 2) | ((x & 4) << 2) | ((y & 4) << 3)
}

fn put32(o: &mut [u8], at: usize, v: u32) {
    o[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(o: &mut [u8], at: usize, v: u64) {
    o[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn write_plane(out: &mut [u8], pb: usize, p: (u64, u64, u64, u32), mips: &[Mip]) {
    put64(out, pb, p.0);
    put64(out, pb + 8, p.1);
    put64(out, pb + 0x10, p.2);
    put32(out, pb + 0x18, p.3);
    for (i, m) in mips.iter().enumerate() {
        let b = pb + 0x20 + i * MIP_STRIDE;
        put64(out, b, m.size);
        put64(out, b + 8, m.offset);
        put64(out, b + 0x10, m.slice);
        let words = [m.pitch_pixels, m.pitch_bytes, m.align, m.padded_w, m.padded_h, m.padded_d, m.width, m.height, m.depth, m.samples, m.mode];
        for (j, v) in words.iter().enumerate() {
            put32(out, b + 0x18 + j * 4, *v);
        }
        put64(out, b + 0x48, m.q48);
        put64(out, b + 0x50, m.q50);
        put32(out, b + 0x58, m.thick);
    }
}

pub fn write_layout(l: &Layout, out: &mut [u8]) {
    let n = LAYOUT_BYTES.min(out.len());
    out[..n].fill(0);
    if out.len() < LAYOUT_BYTES {
        return;
    }
    put32(out, DIMENSION_OFFSET, l.dimension);
    if l.zero {
        put32(out, 0x10, 1);
        put32(out, 0x14, 1);
        put32(out, 0x18, 1);
        return;
    }
    put64(out, 0, l.size);
    put64(out, 8, l.align);
    if !l.planes.is_empty() {
        let nm = l.planes.iter().flatten().map(|p| p.mips.len()).max().unwrap_or(0);
        put32(out, 0x10, nm as u32);
        put32(out, 0x14, 2);
        put32(out, 0x18, 5);
        for (i, p) in l.planes.iter().enumerate() {
            if let Some(p) = p {
                write_plane(out, 0x20 + i * PLANE_PITCH, (p.size, p.offset, p.align, p.bpe), &p.mips);
            }
        }
        put32(out, 0x20 + PLANE_STRIDE, 6);
        return;
    }
    put32(out, 0x10, l.mips.len() as u32);
    put32(out, 0x14, 1);
    put32(out, 0x18, 1);
    write_plane(out, 0x20, (l.size, 0, l.align, l.fmt.bpe), &l.mips);
}

pub fn mip_offset(l: &Layout, mip: usize) -> Option<u64> {
    l.mips.get(mip).map(|_| storage_offset(l, mip))
}
