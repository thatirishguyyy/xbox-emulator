use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Durango {
    pub hash: u64,
    pub code_start: usize,
    pub code_end: usize,
    pub prefix: Vec<u8>,
    pub code: Vec<u8>,
    pub data: Vec<u8>,
    pub tail: Vec<u8>,
}

const PROGRAM_PREFIX: usize = 0x28;

fn u32_at(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| format!("durango: u32 read at {o:#x} out of bounds (len {})", b.len()))
}

fn u64_at(b: &[u8], o: usize) -> Result<u64, String> {
    Ok(u32_at(b, o)? as u64 | (u32_at(b, o + 4)? as u64) << 32)
}

pub fn is_durango(b: &[u8]) -> bool {
    b.len() >= 0x40
        && u64_at(b, 0).map(|s| s as usize == b.len()).unwrap_or(false)
        && u32_at(b, 0x18).ok() == Some(5)
        && u64_at(b, 0x28).map(|c| c >= 0x40 && (c as usize) < b.len()).unwrap_or(false)
}

fn exports_position(blob: &[u8]) -> bool {
    parse(blob).map(|d| decode(&d.code).iter().any(|i| i.enc == Enc::Exp && (12..16).contains(&((i.words[0] >> 4) & 0x3f)))).unwrap_or(false)
}

pub fn parse(b: &[u8]) -> Result<Durango, String> {
    if !is_durango(b) {
        return Err(format!("not a Durango shader blob (first words {:x?})", b.get(0..0x24)));
    }
    let code_start = u64_at(b, 0x28)? as usize;
    let code_end = u64_at(b, 0x30)? as usize;
    if code_start >= code_end || code_end > b.len() || (code_end - code_start) % 4 != 0 {
        return Err(format!("durango: bad code range {code_start:#x}..{code_end:#x} in {} bytes", b.len()));
    }
    let prog_start = code_start + PROGRAM_PREFIX;
    let prog_len = u32_at(b, 0x1dc)? as usize;
    let prog_end = prog_start + prog_len;
    if prog_len == 0 || prog_len % 4 != 0 || prog_end > code_end || u32_at(b, prog_end - 4)? != 0xbf81_0000 {
        return Err(format!("durango: program {prog_start:#x}+{prog_len:#x} (header 0x1dc) does not end in s_endpgm inside code {code_start:#x}..{code_end:#x}"));
    }
    Ok(Durango {
        hash: u64_at(b, 8)?,
        code_start,
        code_end,
        prefix: b[code_start..prog_start].to_vec(),
        code: b[prog_start..prog_end].to_vec(),
        data: b[prog_end..code_end].to_vec(),
        tail: b[code_end..].to_vec(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enc {
    Sop2,
    Sopk,
    Sop1,
    Sopc,
    Sopp,
    Smrd,
    Vop2,
    Vop1,
    Vopc,
    Vop3,
    Vintrp,
    Ds,
    Mubuf,
    Mtbuf,
    Mimg,
    Flat,
    Exp,
    Unknown,
}

pub fn classify(w: u32) -> Enc {
    match w >> 23 {
        0x17f => return Enc::Sopp,
        0x17d => return Enc::Sop1,
        0x17e => return Enc::Sopc,
        _ => {}
    }
    if w >> 28 == 0b1011 {
        return Enc::Sopk;
    }
    if w >> 30 == 0b10 {
        return Enc::Sop2;
    }
    if w >> 31 == 0 {
        return match w >> 25 {
            0b0111111 => Enc::Vop1,
            0b0111110 => Enc::Vopc,
            _ => Enc::Vop2,
        };
    }
    if w >> 27 == 0b11000 {
        return Enc::Smrd;
    }
    match w >> 26 {
        0b110100 => Enc::Vop3,
        0b110010 => Enc::Vintrp,
        0b110110 => Enc::Ds,
        0b110111 => Enc::Flat,
        0b111000 => Enc::Mubuf,
        0b111010 => Enc::Mtbuf,
        0b111100 => Enc::Mimg,
        0b111110 => Enc::Exp,
        _ => Enc::Unknown,
    }
}

fn length(enc: Enc, w: u32) -> usize {
    match enc {
        Enc::Sop2 | Enc::Sopc => 1 + ((w & 0xff) == 255 || ((w >> 8) & 0xff) == 255) as usize,
        Enc::Sop1 => 1 + ((w & 0xff) == 255) as usize,
        Enc::Sopk => 1 + (((w >> 23) & 0x1f) == 0x15) as usize,
        Enc::Sopp | Enc::Vintrp | Enc::Unknown => 1,
        Enc::Smrd => 1 + ((w >> 8) & 1 == 0 && (w & 0xff) == 255) as usize,
        Enc::Vop2 => 1 + ((w & 0x1ff) == 255 || matches!((w >> 25) & 0x3f, 0x20 | 0x21)) as usize,
        Enc::Vop1 | Enc::Vopc => 1 + ((w & 0x1ff) == 255) as usize,
        Enc::Vop3 | Enc::Ds | Enc::Mubuf | Enc::Mtbuf | Enc::Mimg | Enc::Flat | Enc::Exp => 2,
    }
}

#[derive(Debug, Clone)]
pub struct Insn {
    pub offset: usize,
    pub enc: Enc,
    pub words: Vec<u32>,
}

pub fn decode(code: &[u8]) -> Vec<Insn> {
    let w: Vec<u32> = code.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < w.len() {
        let enc = classify(w[i]);
        let n = length(enc, w[i]).min(w.len() - i);
        out.push(Insn { offset: i * 4, enc, words: w[i..i + n].to_vec() });
        i += n;
    }
    out
}

const VOP3_PREFIX: u32 = 0b110101 << 26;
const SGPR_VCC: u32 = 106;
const SOFFSET_NULL: u32 = 0x7d;

enum Vop2Map {
    Same(u32),
    Vop3(u32),
    Vop3Swap(u32),
    Vop3Carry(u32),
}

fn vop2_map(op: u32) -> Result<Vop2Map, String> {
    use Vop2Map::*;
    Ok(match op {
        0x00 => Same(0x01),
        0x01 => Vop3(0x360),
        0x02 => Vop3(0x361),
        0x03..=0x0c | 0x0f..=0x14 | 0x16 | 0x18 | 0x1a..=0x1d | 0x1f..=0x21 | 0x28..=0x2a | 0x2f => Same(op),
        0x15 => Vop3Swap(0x116),
        0x17 => Vop3Swap(0x118),
        0x19 => Vop3Swap(0x11a),
        0x1e => Vop3(0x363),
        0x22 => Vop3(0x364),
        0x23 => Vop3(0x365),
        0x24 => Vop3(0x366),
        0x25 => Vop3Carry(0x30f),
        0x26 => Vop3Carry(0x310),
        0x27 => Vop3Carry(0x319),
        0x2b => Vop3(0x362),
        0x2d => Vop3(0x368),
        0x2e => Vop3(0x369),
        0x30 => Vop3(0x36a),
        0x31 => Vop3(0x36b),
        0x0d => return Err("v_min_legacy_f32 (gfx7 VOP2 0x0d) has no RDNA2 equivalent yet".into()),
        0x0e => return Err("v_max_legacy_f32 (gfx7 VOP2 0x0e) has no RDNA2 equivalent yet".into()),
        0x2c => return Err("v_cvt_pkaccum_u8_f32 (gfx7 VOP2 0x2c) has no RDNA2 equivalent yet".into()),
        _ => return Err(format!("gfx7 VOP2 opcode {op:#x} is not defined")),
    })
}

fn vop1_map(op: u32) -> Result<u32, String> {
    match op {
        0x00..=0x08 | 0x0a..=0x1a | 0x20..=0x25 | 0x27 | 0x2a | 0x2b | 0x2e | 0x2f | 0x31 | 0x33..=0x44 => Ok(op),
        0x09 => Err("v_mov_fed_b32 (gfx7 VOP1 0x09) has no RDNA2 equivalent".into()),
        0x26 | 0x28 | 0x29 | 0x2c | 0x2d | 0x30 | 0x32 | 0x45 | 0x46 => {
            Err(format!("gfx7 VOP1 {op:#x} (clamp/legacy transcendental) has no RDNA2 equivalent yet"))
        }
        _ => Err(format!("gfx7 VOP1 opcode {op:#x} is not defined")),
    }
}

fn vopc_map(op: u32) -> Result<u32, String> {
    match op {
        0x00..=0x3f => Ok(op),
        0x40..=0x7f => Ok(op - 0x40),
        0x80..=0x88 | 0x90..=0x98 | 0xa0..=0xa8 | 0xb0..=0xb8 | 0xc0..=0xc7 | 0xd0..=0xd7 | 0xe0..=0xe7 | 0xf0..=0xf7 => Ok(op),
        _ => Err(format!("gfx7 VOPC opcode {op:#x} is not defined")),
    }
}

fn vop3_only_map(op: u32) -> Result<(u32, bool), String> {
    match op {
        0x140..=0x160 | 0x164..=0x177 => Ok((op, false)),
        0x161 => Ok((0x2ff, true)),
        0x162 => Ok((0x300, true)),
        0x163 => Ok((0x301, true)),
        _ => Err(format!("gfx7 VOP3 opcode {op:#x} is not defined")),
    }
}

fn vop3(op: u32, vdst: u32, abs: u32, clamp: u32, s: [u32; 3], omod: u32, neg: u32) -> [u32; 2] {
    [
        VOP3_PREFIX | (op & 0x3ff) << 16 | (clamp & 1) << 15 | (abs & 7) << 8 | (vdst & 0xff),
        (s[0] & 0x1ff) | (s[1] & 0x1ff) << 9 | (s[2] & 0x1ff) << 18 | (omod & 3) << 27 | (neg & 7) << 29,
    ]
}

fn vop3b(op: u32, vdst: u32, sdst: u32, clamp: u32, s: [u32; 3], omod: u32, neg: u32) -> [u32; 2] {
    [
        VOP3_PREFIX | (op & 0x3ff) << 16 | (clamp & 1) << 15 | (sdst & 0x7f) << 8 | (vdst & 0xff),
        (s[0] & 0x1ff) | (s[1] & 0x1ff) << 9 | (s[2] & 0x1ff) << 18 | (omod & 3) << 27 | (neg & 7) << 29,
    ]
}

fn swap01(abs: u32, neg: u32) -> (u32, u32) {
    let sw = |m: u32| (m & 4) | (m & 1) << 1 | (m >> 1) & 1;
    (sw(abs), sw(neg))
}

fn smrd_op(op: u32) -> Result<u32, String> {
    match op {
        0x00..=0x04 | 0x08..=0x0c => Ok(op),
        0x1d => Ok(0x24),
        0x1e => Ok(0x20),
        _ => Err(format!("gfx7 SMRD opcode {op:#x} is not supported")),
    }
}

fn is_branch(w: u32) -> bool {
    classify(w) == Enc::Sopp && matches!((w >> 16) & 0x7f, 0x02 | 0x04..=0x09)
}

fn convert(ins: &Insn) -> Result<Vec<u32>, String> {
    let w = ins.words[0];
    let at = ins.offset;
    let lit = ins.words.get(1).copied();
    let err = |e: String| format!("gfx7 at {at:#x} ({w:#010x}): {e}");
    Ok(match ins.enc {
        Enc::Sop2 => match (w >> 23) & 0x7f {
            0x00..=0x0b | 0x0e..=0x2a | 0x2c => ins.words.clone(),
            op => return Err(err(format!("SOP2 opcode {op:#x} is not supported"))),
        },
        Enc::Sopk => match (w >> 23) & 0x1f {
            0x00..=0x10 | 0x12 | 0x13 | 0x15 => ins.words.clone(),
            op => return Err(err(format!("SOPK opcode {op:#x} is not supported"))),
        },
        Enc::Sop1 => match (w >> 8) & 0xff {
            0x03..=0x21 | 0x24..=0x31 | 0x34 => ins.words.clone(),
            op => return Err(err(format!("SOP1 opcode {op:#x} is not supported"))),
        },
        Enc::Sopc => match (w >> 16) & 0x7f {
            0x00..=0x0f => ins.words.clone(),
            op => return Err(err(format!("SOPC opcode {op:#x} is not supported"))),
        },
        Enc::Sopp => match (w >> 16) & 0x7f {
            0x00..=0x15 => ins.words.clone(),
            op => return Err(err(format!("SOPP opcode {op:#x} is not supported"))),
        },
        Enc::Smrd => {
            let op = smrd_op((w >> 22) & 0x1f).map_err(err)?;
            let sdst = (w >> 15) & 0x7f;
            let sbase = (w >> 9) & 0x3f;
            let imm = (w >> 8) & 1;
            let off = w & 0xff;
            let (offset, soffset) = if imm == 1 {
                (off * 4, SOFFSET_NULL)
            } else if off == 255 {
                let l = lit.ok_or_else(|| err("SMRD literal offset missing".into()))?;
                let bytes = l.checked_mul(4).filter(|b| *b < 1 << 20).ok_or_else(|| err(format!("SMRD literal offset {l:#x} does not fit RDNA2 SMEM")))?;
                (bytes, SOFFSET_NULL)
            } else if off <= 105 || off == SGPR_VCC || off == SGPR_VCC + 1 || off == 124 {
                (0, off)
            } else {
                return Err(err(format!("SMRD offset register code {off} is not supported")));
            };
            vec![(0b111101 << 26) | op << 18 | sdst << 6 | sbase, offset | soffset << 25]
        }
        Enc::Vop2 => {
            let op = (w >> 25) & 0x3f;
            let vdst = (w >> 17) & 0xff;
            let vsrc1 = if matches!(op, 0x01 | 0x02) { (w >> 9) & 0xff } else { 256 + ((w >> 9) & 0xff) };
            let src0 = w & 0x1ff;
            match vop2_map(op).map_err(err)? {
                Vop2Map::Same(n) => {
                    let mut v = vec![(w & !(0x3f << 25)) | n << 25];
                    v.extend_from_slice(&ins.words[1..]);
                    v
                }
                Vop2Map::Vop3(n) => {
                    let mut v = vop3(n, vdst, 0, 0, [src0, vsrc1, 0], 0, 0).to_vec();
                    v.extend(lit);
                    v
                }
                Vop2Map::Vop3Swap(n) => {
                    let mut v = vop3(n, vdst, 0, 0, [vsrc1, src0, 0], 0, 0).to_vec();
                    v.extend(lit);
                    v
                }
                Vop2Map::Vop3Carry(n) => {
                    let mut v = vop3b(n, vdst, SGPR_VCC, 0, [src0, vsrc1, 0], 0, 0).to_vec();
                    v.extend(lit);
                    v
                }
            }
        }
        Enc::Vop1 => {
            let op = vop1_map((w >> 9) & 0xff).map_err(err)?;
            let mut v = vec![(w & !(0xff << 9)) | op << 9];
            v.extend_from_slice(&ins.words[1..]);
            v
        }
        Enc::Vopc => {
            let op = vopc_map((w >> 17) & 0xff).map_err(err)?;
            let mut v = vec![(w & !(0xff << 17)) | op << 17];
            v.extend_from_slice(&ins.words[1..]);
            v
        }
        Enc::Vop3 => {
            let d1 = lit.unwrap_or(0);
            let op = (w >> 17) & 0x1ff;
            let vdst = w & 0xff;
            let abs = (w >> 8) & 7;
            let clamp = (w >> 11) & 1;
            let sdst = (w >> 8) & 0x7f;
            let s = [d1 & 0x1ff, (d1 >> 9) & 0x1ff, (d1 >> 18) & 0x1ff];
            let omod = (d1 >> 27) & 3;
            let neg = (d1 >> 29) & 7;
            match op {
                0x000..=0x0ff => vop3(vopc_map(op).map_err(err)?, vdst, abs, clamp, s, omod, neg).to_vec(),
                0x100..=0x13f => match op - 0x100 {
                    0x25 | 0x26 | 0x27 => {
                        let n = [0x30f, 0x310, 0x319][(op - 0x125) as usize];
                        vop3b(n, vdst, sdst, 0, s, omod, neg).to_vec()
                    }
                    0x28..=0x2a => vop3b(op, vdst, sdst, 0, s, omod, neg).to_vec(),
                    o => match vop2_map(o).map_err(err)? {
                        Vop2Map::Same(n) => vop3(0x100 + n, vdst, abs, clamp, s, omod, neg).to_vec(),
                        Vop2Map::Vop3(n) => vop3(n, vdst, abs, clamp, s, omod, neg).to_vec(),
                        Vop2Map::Vop3Swap(n) => {
                            let (a, ng) = swap01(abs, neg);
                            vop3(n, vdst, a, clamp, [s[1], s[0], s[2]], omod, ng).to_vec()
                        }
                        Vop2Map::Vop3Carry(n) => vop3b(n, vdst, sdst, 0, s, omod, neg).to_vec(),
                    },
                },
                0x16d | 0x16e | 0x176 | 0x177 => vop3b(op, vdst, sdst, 0, s, omod, neg).to_vec(),
                0x140..=0x17f => {
                    let (n, sw) = vop3_only_map(op).map_err(err)?;
                    if sw {
                        let (a, ng) = swap01(abs, neg);
                        vop3(n, vdst, a, clamp, [s[1], s[0], s[2]], omod, ng).to_vec()
                    } else {
                        vop3(n, vdst, abs, clamp, s, omod, neg).to_vec()
                    }
                }
                0x180..=0x1ff => vop3(0x180 + vop1_map(op - 0x180).map_err(err)?, vdst, abs, clamp, s, omod, neg).to_vec(),
                _ => return Err(err(format!("VOP3 opcode {op:#x} is not defined"))),
            }
        }
        Enc::Vintrp | Enc::Ds | Enc::Exp => ins.words.clone(),
        Enc::Mubuf => {
            if (w >> 15) & 1 != 0 {
                return addr64(ins).map_err(err);
            }
            ins.words.clone()
        }
        Enc::Mimg => {
            let da = (w >> 14) & 1;
            let dim = if da != 0 { 5 } else { 1 };
            vec![(w & !(1 << 14)) | dim << 3, ins.words[1]]
        }
        Enc::Mtbuf => return Err(err("MTBUF (typed buffer) is not supported by the RDNA2 backend yet".into())),
        Enc::Flat => return Err(err("FLAT is not supported yet".into())),
        Enc::Unknown => return Err(err("unknown gfx7 encoding".into())),
    })
}

pub const ADDR64_LO: u8 = 254;
pub const ADDR64_HI: u8 = 255;
pub const ADDR64_CARRY: u8 = 104;

fn addr64(ins: &Insn) -> Result<Vec<u32>, String> {
    let (w0, w1) = (ins.words[0], ins.words.get(1).copied().unwrap_or(0));
    let op = (w0 >> 18) & 0x7f;
    if (w0 >> 12) & 3 != 0 {
        return Err(format!("MUBUF addr64 op {op:#x} with offen/idxen set"));
    }
    if (w0 >> 16) & 1 != 0 {
        return Err(format!("MUBUF addr64 op {op:#x} with LDS=1"));
    }
    if (w1 >> 23) & 1 != 0 {
        return Err(format!("MUBUF addr64 op {op:#x} with TFE=1"));
    }
    if !matches!(op, 0x08..=0x0f | 0x18 | 0x1a | 0x1c..=0x1f) {
        return Err(format!("MUBUF addr64 op {op:#x} is not a plain load/store"));
    }
    let off = w0 & 0xfff;
    let glc = (w0 >> 14) & 1;
    let slc = (w1 >> 22) & 1;
    let vaddr = w1 & 0xff;
    let vdata = (w1 >> 8) & 0xff;
    let rsrc = ((w1 >> 16) & 0x1f) * 4;
    let soff = (w1 >> 24) & 0xff;
    let (lo, hi, cy) = (ADDR64_LO as u32, ADDR64_HI as u32, ADDR64_CARRY as u32);
    let mut v = vec![0x7e00_0000 | hi << 17 | 0x01 << 9 | (rsrc + 1), 0x1b << 25 | hi << 17 | hi << 9 | 0xff, 0xffff];
    v.extend(vop3b(0x30f, lo, cy, 0, [rsrc, 256 + vaddr, 0], 0, 0));
    v.extend(vop3b(0x128, hi, cy, 0, [256 + hi, 256 + vaddr + 1, cy], 0, 0));
    let mut add = |src: u32, lit: Option<u32>| {
        v.extend(vop3b(0x30f, lo, cy, 0, [src, 256 + lo, 0], 0, 0));
        if let Some(l) = lit {
            v.push(l);
        }
        v.extend(vop3b(0x128, hi, cy, 0, [0x80, 256 + hi, cy], 0, 0));
    };
    if soff != 0x80 {
        add(soff, None);
    }
    let inst_off = if off > 2047 {
        add(0xff, Some(off));
        0
    } else {
        off
    };
    v.push(0b110111 << 26 | op << 18 | slc << 17 | glc << 16 | 2 << 14 | inst_off);
    let data_or_dst = if op >= 0x18 { vdata << 8 } else { vdata << 24 };
    v.push(lo | data_or_dst | 0x7f << 16);
    Ok(v)
}

#[derive(Debug, Clone, Default)]
pub struct Lowered {
    pub isa: Vec<u8>,
    pub code_len: usize,
    pub notes: Vec<String>,
}

fn sopp_op(ins: &Insn) -> Option<u32> {
    (ins.enc == Enc::Sopp).then(|| (ins.words[0] >> 16) & 0x7f)
}

fn sop1_op(ins: &Insn) -> Option<u32> {
    (ins.enc == Enc::Sop1).then(|| (ins.words[0] >> 8) & 0xff)
}

fn reachable(insns: &[Insn]) -> Result<Vec<bool>, String> {
    let at: BTreeMap<usize, usize> = insns.iter().enumerate().map(|(k, i)| (i.offset / 4, k)).collect();
    let mut live = vec![false; insns.len()];
    let mut todo = vec![0usize];
    while let Some(k) = todo.pop() {
        if k >= insns.len() || live[k] {
            continue;
        }
        live[k] = true;
        let ins = &insns[k];
        let w = ins.words[0];
        match (sopp_op(ins), sop1_op(ins)) {
            (Some(0x01), _) | (_, Some(0x20)) => continue,
            (Some(op @ (0x02 | 0x04..=0x09)), _) => {
                let target = (ins.offset / 4) as i64 + 1 + (w & 0xffff) as i16 as i64;
                let t = usize::try_from(target).ok().and_then(|t| at.get(&t).copied()).ok_or_else(|| {
                    format!("gfx7 branch at {:#x} targets dword {target}, which is not an instruction start", ins.offset)
                })?;
                todo.push(t);
                if op != 0x02 {
                    todo.push(k + 1);
                }
            }
            _ => todo.push(k + 1),
        }
    }
    Ok(live)
}

fn sop2(ins: &Insn) -> Option<(u32, u32, u32, u32)> {
    let w = ins.words[0];
    (ins.enc == Enc::Sop2).then(|| ((w >> 23) & 0x7f, (w >> 16) & 0x7f, w & 0xff, (w >> 8) & 0xff))
}

fn pc_add(insns: &[Insn], k: usize) -> Result<u32, String> {
    let g = &insns[k];
    let d = (g.words[0] >> 16) & 0x7f;
    let bad = || format!("gfx7 s_getpc_b64 at {:#x}: the PC in s[{d}:{}] is not consumed by s_add_u32 s{d}, s{d}, literal; s_addc_u32 s{}, s{}, 0", g.offset, d + 1, d + 1, d + 1);
    let (add, addc) = (insns.get(k + 1).ok_or_else(bad)?, insns.get(k + 2).ok_or_else(bad)?);
    let lit = match sop2(add) {
        Some((0, sd, a, b)) if sd == d && add.words.len() == 2 && ((a == d && b == 0xff) || (a == 0xff && b == d)) => add.words[1],
        _ => return Err(bad()),
    };
    match sop2(addc) {
        Some((4, sd, a, b)) if sd == d + 1 && addc.words.len() == 1 && ((a == d + 1 && b == 0x80) || (a == 0x80 && b == d + 1)) => Ok(lit),
        _ => Err(bad()),
    }
}

pub fn to_rdna2(code: &[u8]) -> Result<Lowered, String> {
    lower(code, &[])
}

pub fn lower(code: &[u8], data: &[u8]) -> Result<Lowered, String> {
    let all = decode(code);
    let live = reachable(&all)?;
    let n = live.iter().rposition(|&r| r).map_or(0, |k| k + 1);
    let insns = &all[..n];
    let code_end = all.get(n).map_or(code.len(), |i| i.offset);
    let mut tail = code[code_end..].to_vec();
    tail.extend_from_slice(data);
    let mut out: Vec<Vec<u32>> = Vec::with_capacity(insns.len());
    let mut notes = Vec::new();
    let mut new_at: BTreeMap<usize, usize> = BTreeMap::new();
    let mut pcs = Vec::new();
    let mut expanded: Vec<std::ops::Range<usize>> = Vec::new();
    let mut pos = 0usize;
    for (k, ins) in insns.iter().enumerate() {
        match sop1_op(ins) {
            Some(0x1f) => pcs.push((k, pc_add(insns, k)?)),
            Some(0x20) if k + 1 != n || n == 0 => {
                return Err(format!("gfx7 at {:#x}: s_setpc_b64 jumps to a computed address inside the program", ins.offset));
            }
            Some(0x21) => notes.push(format!("swappc at {:#x}: fetch-shader call, vertex attributes come from the input layout", ins.offset)),
            _ => {}
        }
        if ins.enc == Enc::Mimg && (ins.words[0] >> 18) & 0x7f != 0x0e {
            notes.push(format!("mimg at {:#x}: dim taken from DA ({})", ins.offset, if (ins.words[0] >> 14) & 1 != 0 { "2D array" } else { "2D" }));
        }
        new_at.insert(ins.offset / 4, pos);
        let v = convert(ins)?;
        if ins.enc == Enc::Mubuf && (ins.words[0] >> 15) & 1 != 0 {
            expanded.push(pos..pos + v.len());
        }
        pos += v.len();
        out.push(v);
    }
    new_at.insert(code_end / 4, pos);
    let code_len = pos * 4;
    for (k, ins) in insns.iter().enumerate() {
        let w = ins.words[0];
        if !is_branch(w) {
            continue;
        }
        let old_pc = ins.offset / 4;
        let target = old_pc as i64 + 1 + (w & 0xffff) as i16 as i64;
        let new_target = *new_at.get(&(target as usize)).filter(|_| target >= 0).ok_or_else(|| {
            format!("gfx7 branch at {:#x} targets dword {target}, which is not an instruction start", ins.offset)
        })?;
        let rel = new_target as i64 - (new_at[&old_pc] as i64 + 1);
        let rel = i16::try_from(rel).map_err(|_| format!("gfx7 branch at {:#x}: relocated offset {rel} overflows", ins.offset))?;
        out[k][0] = (w & 0xffff_0000) | rel as u16 as u32;
    }
    for (k, lit) in pcs {
        let g = &insns[k];
        let target = (g.offset + 4) as u64 + lit as u64;
        let new_target = if target >= code_end as u64 {
            let t = target - code_end as u64;
            if t >= tail.len() as u64 {
                return Err(format!("gfx7 s_getpc_b64 at {:#x}: target {target:#x} lies past the code section ({:#x} bytes)", g.offset, code_end + tail.len()));
            }
            code_len as u64 + t
        } else if target % 4 == 0 && insns.iter().any(|i| i.offset as u64 == target) {
            new_at[&(target as usize / 4)] as u64 * 4
        } else {
            return Err(format!("gfx7 s_getpc_b64 at {:#x}: target {target:#x} is inside an instruction", g.offset));
        };
        let new_pc = (new_at[&(g.offset / 4)] as u64 + 1) * 4;
        let rel = u32::try_from(new_target.checked_sub(new_pc).ok_or_else(|| format!("gfx7 s_getpc_b64 at {:#x}: relocated target precedes the PC", g.offset))?)
            .map_err(|_| format!("gfx7 s_getpc_b64 at {:#x}: relocated offset overflows", g.offset))?;
        out[k + 1][1] = rel;
        notes.push(format!("getpc at {:#x}: pc+{lit:#x} -> lowered pc+{rel:#x}", g.offset));
    }
    let mut isa: Vec<u8> = out.into_iter().flatten().flat_map(|d| d.to_le_bytes()).collect();
    if !expanded.is_empty() {
        use crate::rdna2::Opnd;
        for i in crate::rdna2::disassemble(&isa[..code_len]) {
            if expanded.iter().any(|r| r.contains(&(i.offset / 4))) {
                continue;
            }
            let clash = i.src.iter().chain(i.dst.iter()).any(|o| matches!(o, Opnd::Vgpr(ADDR64_LO | ADDR64_HI) | Opnd::Sgpr(ADDR64_CARRY)));
            if clash {
                return Err(format!("gfx7 MUBUF addr64 lowering needs v{ADDR64_LO}, v{ADDR64_HI} and s{ADDR64_CARRY}, but {} at lowered {:#x} uses them", i.mnemonic, i.offset));
            }
        }
        notes.push(format!("{} MUBUF addr64 access(es) lowered to GLOBAL through v{ADDR64_LO}:v{ADDR64_HI}", expanded.len()));
    }
    isa.extend_from_slice(&tail);
    Ok(Lowered { isa, code_len, notes })
}

pub fn find_durango_all(xdat: &[u8]) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    let mut i = 8;
    while i + 2 < xdat.len() {
        if xdat[i] == 0x78 && matches!(xdat[i + 1], 0x01 | 0x9c | 0xda) {
            if let Ok(out) = miniz_oxide::inflate::decompress_to_vec_zlib(&xdat[i..]) {
                if is_durango(&out) {
                    found.push(out);
                }
            }
        }
        i += 1;
    }
    found
}

pub fn find_durango(xdat: &[u8]) -> Option<Vec<u8>> {
    let all = find_durango_all(xdat);
    if all.len() > 1 {
        if let Some(k) = all.iter().position(|b| exports_position(b)) {
            return all.into_iter().nth(k);
        }
    }
    all.into_iter().next()
}

fn thread_group(d: &Durango, stage: crate::Stage) -> [u32; 4] {
    if stage != crate::Stage::Compute {
        return [0; 4];
    }
    let w = |o: usize| u32_at(&d.prefix, o).unwrap_or(0);
    [0, w(0x18), w(0x1c), w(0x20)]
}

pub fn extract_durango(
    dxbc: &[u8],
    container: &crate::Dxbc,
    stage: crate::Stage,
    shader_model: (u32, u32),
    bitcode_stripped: bool,
    blob: &[u8],
) -> Result<crate::ShaderExtract, String> {
    let d = parse(blob)?;
    let lowered = lower(&d.code, &d.data)?;
    let body = |cc: &[u8; 4]| container.chunk(cc).map(|c| &dxbc[c.offset..c.offset + c.size]);
    let sig = |a: &[u8; 4], b: &[u8; 4]| body(a).or_else(|| body(b)).and_then(|x| crate::meta::parse_signature(x).ok()).unwrap_or_default();
    Ok(crate::ShaderExtract {
        stage,
        shader_model,
        bitcode_stripped,
        fssm: blob.to_vec(),
        entry: None,
        isa: lowered.isa[..lowered.code_len].to_vec(),
        isa_offset_in_fssm: d.code_start,
        isa_preamble: thread_group(&d, stage),
        isa_data: lowered.isa,
        has_input_sig: container.chunk(b"ISG1").is_some() || container.chunk(b"ISGN").is_some(),
        has_output_sig: container.chunk(b"OSG1").is_some() || container.chunk(b"OSGN").is_some(),
        has_psv0: container.chunk(b"PSV0").is_some(),
        has_root_sig: container.chunk(b"RTS0").is_some(),
        input_sig: sig(b"ISG1", b"ISGN"),
        output_sig: sig(b"OSG1", b"OSGN"),
        resources: body(b"PSV0").and_then(|b| crate::meta::parse_psv0(b).ok()).unwrap_or_default(),
        gs_info: body(b"PSV0").and_then(crate::meta::parse_psv0_gs),
        root_sig: body(b"RTS0").and_then(|b| crate::meta::parse_rts0(b).ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn dumps() -> Vec<(String, Vec<u8>)> {
        let dir = std::env::var_os("TOMBOX_SHADER_DUMPS").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("tombox_shaders"));
        let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
        let mut v = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("dxbc") {
                continue;
            }
            let Ok(b) = std::fs::read(&p) else { continue };
            let Ok(c) = crate::Dxbc::parse(&b) else { continue };
            let Some(d) = c.chunk(b"DXIL") else { continue };
            let body = &b[d.offset..d.offset + d.size];
            if let Some(blob) = body.get(24..).and_then(find_durango) {
                v.push((p.file_name().unwrap().to_string_lossy().into_owned(), blob));
            }
        }
        v.sort();
        v
    }

    #[test]
    fn mubuf_addr64_load_becomes_a_global_load_of_base_plus_vaddr64() {
        let l = to_rdna2(&bytes(&[0xe0388000, 0x80020402, 0xbf810000])).unwrap();
        let d = crate::rdna2::disassemble(&l.isa[..l.code_len]);
        assert_eq!(d.iter().map(|i| i.mnemonic.as_str()).collect::<Vec<_>>(), ["v_mov_b32", "v_and_b32", "v_add_co_u32", "v_add_co_ci_u32", "flat_load_dwordx4", "s_endpgm"]);
        use crate::rdna2::Opnd;
        assert_eq!(d[0].src[0], Opnd::Sgpr(9));
        assert_eq!(d[2].src[..2], [Opnd::Sgpr(8), Opnd::Vgpr(2)]);
        assert_eq!(d[3].src[1], Opnd::Vgpr(3));
        let g = &d[4];
        assert_eq!(g.aux.mask, 2);
        assert_eq!(g.aux.imm, 0);
        assert_eq!(g.src[0], Opnd::Vgpr(ADDR64_LO));
        assert_eq!(g.src[2], Opnd::Null);
        assert_eq!(g.dst[0], Opnd::Vgpr(4));
    }

    #[test]
    fn mubuf_addr64_store_folds_soffset_and_a_large_offset() {
        let l = to_rdna2(&bytes(&[0xe0708bb8, 0x05030602, 0xbf810000])).unwrap();
        let d = crate::rdna2::disassemble(&l.isa[..l.code_len]);
        let m: Vec<&str> = d.iter().map(|i| i.mnemonic.as_str()).collect();
        assert_eq!(m, ["v_mov_b32", "v_and_b32", "v_add_co_u32", "v_add_co_ci_u32", "v_add_co_u32", "v_add_co_ci_u32", "v_add_co_u32", "v_add_co_ci_u32", "flat_store_dword", "s_endpgm"]);
        use crate::rdna2::Opnd;
        assert_eq!(d[4].src[0], Opnd::Sgpr(5));
        assert_eq!(d[8].aux.imm, 0);
        assert_eq!(d[8].src[1], Opnd::Vgpr(6));
        assert!(d[6].raw.contains(&0xbb8), "{:x?}", d[6].raw);
    }

    #[test]
    fn mubuf_addr64_refuses_when_the_title_uses_the_temporaries() {
        let e = to_rdna2(&bytes(&[0x7ffc0280, 0xe0388000, 0x80020402, 0xbf810000])).unwrap_err();
        assert!(e.contains("v254"), "{e}");
    }

    #[test]
    fn branch_relocation_and_smrd() {
        let code: Vec<u32> = vec![
            0xc0000300,
            0xbf820001,
            0x4a000080,
            0xbf810000,
        ];
        let bytes: Vec<u8> = code.iter().flat_map(|d| d.to_le_bytes()).collect();
        let l = to_rdna2(&bytes).unwrap();
        let w: Vec<u32> = l.isa.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        assert_eq!(w[0] >> 26, 0b111101);
        assert_eq!(w[1] & 0x1f_ffff, 0);
        assert_eq!(w[1] >> 25, SOFFSET_NULL);
        assert_eq!(w[2], 0xbf820002);
        assert_eq!(w[3] >> 26, 0b110101);
        assert_eq!(w[5], 0xbf810000);
        let d = crate::rdna2::disassemble(&l.isa);
        assert_eq!(d[0].mnemonic, "s_load_dword");
        assert_eq!(d[2].mnemonic, "v_add_co_u32");
        assert_eq!(d.last().unwrap().mnemonic, "s_endpgm");
    }

    fn words(b: &[u8]) -> Vec<u32> {
        b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    fn bytes(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|d| d.to_le_bytes()).collect()
    }

    #[test]
    fn getpc_relocates_into_embedded_data() {
        let code = bytes(&[0xc0000300, 0xbe821f00, 0x8002ff02, 0x14, 0x82038003, 0xc0020300, 0xbf810000, 0x0000000e, 0x3f000000, 0xbf810000]);
        let l = lower(&code, &[0x11, 0, 0, 0]).unwrap();
        let w = words(&l.isa);
        assert_eq!(l.code_len, 0x24);
        assert_eq!(w[2], 0xbe821f00);
        assert_eq!(w[4], 0x18);
        assert_eq!(&w[9..], &[0x0000000e, 0x3f000000, 0xbf810000, 0x11]);
        let pc = (2 + 1) * 4;
        assert_eq!(w[(pc + w[4] as usize) / 4], 0x0000000e);
        let d = crate::rdna2::disassemble(&l.isa[..l.code_len]);
        assert_eq!(d.iter().map(|i| i.mnemonic.as_str()).collect::<Vec<_>>(), ["s_load_dword", "s_getpc_b64", "s_add_u32", "s_addc_u32", "s_load_dword", "s_endpgm"]);
    }

    #[test]
    fn getpc_into_data_block_after_program() {
        let code = bytes(&[0xbe821f00, 0x8002ff02, 0x18, 0x82038003, 0xc0020300, 0xbf810000]);
        let l = lower(&code, &[7, 0, 0, 0, 9, 0, 0, 0]).unwrap();
        let w = words(&l.isa);
        assert_eq!(l.code_len, 0x1c);
        assert_eq!(w[2], 0x1c);
        assert_eq!(w[(4 + w[2] as usize) / 4], 9);
    }

    #[test]
    fn getpc_relocates_code_label() {
        let code = bytes(&[0xbe821f00, 0x8002ff02, 0x0c, 0x82038003, 0x4a000080, 0xbf810000]);
        let l = lower(&code, &[]).unwrap();
        let w = words(&l.isa);
        assert_eq!(w[2], 0x0c);
        let code = bytes(&[0xbe821f00, 0x8002ff02, 0x10, 0x82038003, 0x4a000080, 0xbf810000]);
        assert_eq!(words(&lower(&code, &[]).unwrap().isa)[2], 0x14);
        let code = bytes(&[0xbe821f00, 0x8002ff02, 0x04, 0x82038003, 0x4a000080, 0xbf810000]);
        assert!(lower(&code, &[]).unwrap_err().contains("inside an instruction"));
    }

    #[test]
    fn getpc_unrecognised_forms_are_errors() {
        let code = bytes(&[0xbe821f00, 0xc0020300, 0xbf810000]);
        assert!(lower(&code, &[]).unwrap_err().contains("not consumed by s_add_u32"));
        let code = bytes(&[0xbe821f00, 0x8002ff02, 0x40, 0x82038003, 0xbf810000]);
        assert!(lower(&code, &[]).unwrap_err().contains("past the code section"));
        let code = bytes(&[0xbf840001, 0xbe802000, 0xbf810000]);
        assert!(lower(&code, &[]).unwrap_err().contains("computed address"));
    }

    #[test]
    fn swappc_fetch_call_passes_through() {
        let code = bytes(&[0xbe802100, 0xbe840303, 0xbe850380, 0xbf810000]);
        let l = lower(&code, &[]).unwrap();
        assert_eq!(words(&l.isa), [0xbe802100, 0xbe840303, 0xbe850380, 0xbf810000]);
        let d = crate::rdna2::disassemble(&l.isa);
        assert_eq!(d[0].mnemonic, "s_swappc_b64");
    }

    #[test]
    fn unreachable_tail_after_branch_is_kept_verbatim() {
        let code = bytes(&[0xbf820001, 0xbf800000, 0xbf810000]);
        assert_eq!(lower(&code, &[]).unwrap().code_len, 12);
        let code = bytes(&[0xbf810000, 0x0000000e, 0xbf810000]);
        let l = lower(&code, &[]).unwrap();
        assert_eq!(l.code_len, 4);
        assert_eq!(words(&l.isa), [0xbf810000, 0x0000000e, 0xbf810000]);
    }

    #[test]
    fn show_one() {
        let Ok(want) = std::env::var("TOMBOX_GCN_SHOW") else { return };
        for (name, blob) in dumps() {
            if !name.starts_with(&want) {
                continue;
            }
            let d = parse(&blob).unwrap();
            let words: Vec<u32> = blob.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            for (k, ch) in words[..d.code_start / 4].chunks(8).enumerate() {
                eprintln!("hdr {:04x}: {}", k * 32, ch.iter().map(|w| format!("{w:08x}")).collect::<Vec<_>>().join(" "));
            }
            for (k, ch) in words[d.code_end / 4..].chunks(8).enumerate() {
                eprintln!("tail {:04x}: {}", d.code_end + k * 32, ch.iter().map(|w| format!("{w:08x}")).collect::<Vec<_>>().join(" "));
            }
            let l = to_rdna2(&d.code).unwrap();
            for i in crate::rdna2::disassemble(&l.isa) {
                eprintln!("{:4x}  {:24} {} <- {}", i.offset, i.mnemonic, i.dst.iter().map(crate::rdna2::opnd_str).collect::<Vec<_>>().join(","), i.src.iter().map(crate::rdna2::opnd_str).collect::<Vec<_>>().join(","));
            }
        }
    }

    #[test]
    fn dumped_durango_shaders() {
        let all = dumps();
        if all.is_empty() {
            eprintln!("no Durango dumps in %TEMP%\\tombox_shaders; skipped");
            return;
        }
        let mut ok = 0;
        let mut getpcs = 0;
        let mut errs: BTreeMap<String, usize> = BTreeMap::new();
        let mut unnamed: BTreeMap<String, usize> = BTreeMap::new();
        for (name, blob) in &all {
            let d = parse(blob).unwrap_or_else(|e| panic!("{name}: {e}"));
            let insns = decode(&d.code);
            assert_eq!(insns.iter().map(|i| i.words.len() * 4).sum::<usize>(), d.code.len(), "{name}: decode does not land on the program end");
            assert_eq!(insns.last().map(|i| i.words[0]), Some(0xbf81_0000), "{name}: program does not end in s_endpgm");
            if let Ok(l) = lower(&d.code, &d.data) {
                let orig: Vec<u8> = d.code.iter().chain(d.data.iter()).copied().collect();
                let ow = words(&orig);
                let lw = words(&l.isa);
                let olds: Vec<&Insn> = insns.iter().filter(|i| i.enc == Enc::Sop1 && (i.words[0] >> 8) & 0xff == 0x1f).collect();
                let news: Vec<usize> = crate::rdna2::disassemble(&l.isa[..l.code_len]).iter().filter(|i| i.mnemonic == "s_getpc_b64").map(|i| i.offset).collect();
                for (o, n) in olds.iter().zip(news.iter()) {
                    let ot = o.offset + 4 + ow[o.offset / 4 + 2] as usize;
                    let nt = n + 4 + lw[n / 4 + 2] as usize;
                    assert_eq!(ow[ot / 4], lw[nt / 4], "{name}: getpc at {:#x} relocated target differs", o.offset);
                    getpcs += 1;
                }
            }
            match to_rdna2(&d.code) {
                Ok(l) => {
                    ok += 1;
                    for i in crate::rdna2::disassemble(&l.isa) {
                        if i.mnemonic.contains('<') {
                            *unnamed.entry(i.mnemonic.clone()).or_default() += 1;
                        }
                    }
                }
                Err(e) => {
                    let key = e.split(": ").skip(1).collect::<Vec<_>>().join(": ");
                    *errs.entry(key).or_default() += 1;
                }
            }
        }
        eprintln!("durango: {ok}/{} lowered to RDNA2, {getpcs} getpc targets verified", all.len());
        for (e, n) in &errs {
            eprintln!("  {n:4}  {e}");
        }
        for (m, n) in &unnamed {
            eprintln!("  unnamed {n:4}  {m}");
        }
        let mut herr: BTreeMap<String, usize> = BTreeMap::new();
        let mut hok = 0;
        let dir = std::env::var_os("TOMBOX_SHADER_DUMPS").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("tombox_shaders"));
        for (name, _) in &all {
            let b = std::fs::read(dir.join(name)).unwrap();
            match crate::extract(&b).and_then(|ex| crate::hlsl::translate(&ex)) {
                Ok(_) => hok += 1,
                Err(e) => {
                    let k: String = e.chars().filter(|c| !c.is_ascii_hexdigit() || c.is_alphabetic()).take(90).collect();
                    *herr.entry(k).or_default() += 1;
                }
            }
        }
        eprintln!("durango -> hlsl: {hok}/{} translated", all.len());
        for (e, n) in &herr {
            eprintln!("  hlsl {n:4}  {e}");
        }
    }
}
