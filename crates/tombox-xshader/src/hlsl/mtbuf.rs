use super::{r, Emitter, Res, BINDLESS};
use crate::rdna2::{Insn, Opnd};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Num {
    Unorm,
    Snorm,
    Uscaled,
    Sscaled,
    Uint,
    Sint,
    Float,
}

pub(super) struct Ufmt {
    pub fields: &'static [(u32, u32)],
    pub bytes: u32,
    pub num: Num,
}

const L8: &[(u32, u32)] = &[(0, 8)];
const L16: &[(u32, u32)] = &[(0, 16)];
const L8_8: &[(u32, u32)] = &[(0, 8), (8, 8)];
const L32: &[(u32, u32)] = &[(0, 32)];
const L16_16: &[(u32, u32)] = &[(0, 16), (16, 16)];
const L10_11_11: &[(u32, u32)] = &[(0, 11), (11, 11), (22, 10)];
const L11_11_10: &[(u32, u32)] = &[(0, 10), (10, 11), (21, 11)];
const L10_10_10_2: &[(u32, u32)] = &[(0, 2), (2, 10), (12, 10), (22, 10)];
const L2_10_10_10: &[(u32, u32)] = &[(0, 10), (10, 10), (20, 10), (30, 2)];
const L8_8_8_8: &[(u32, u32)] = &[(0, 8), (8, 8), (16, 8), (24, 8)];
const L32_32: &[(u32, u32)] = &[(0, 32), (32, 32)];
const L16_16_16_16: &[(u32, u32)] = &[(0, 16), (16, 16), (32, 16), (48, 16)];
const L32_32_32: &[(u32, u32)] = &[(0, 32), (32, 32), (64, 32)];
const L32_32_32_32: &[(u32, u32)] = &[(0, 32), (32, 32), (64, 32), (96, 32)];

const N6: [Num; 6] = [Num::Unorm, Num::Snorm, Num::Uscaled, Num::Sscaled, Num::Uint, Num::Sint];
const N7: [Num; 7] = [Num::Unorm, Num::Snorm, Num::Uscaled, Num::Sscaled, Num::Uint, Num::Sint, Num::Float];
const N3: [Num; 3] = [Num::Uint, Num::Sint, Num::Float];

pub(super) fn ufmt(f: u32) -> Option<Ufmt> {
    let groups: [(u32, &'static [(u32, u32)], u32, &[Num]); 14] = [
        (1, L8, 1, &N6),
        (7, L16, 2, &N7),
        (14, L8_8, 2, &N6),
        (20, L32, 4, &N3),
        (23, L16_16, 4, &N7),
        (30, L10_11_11, 4, &N7),
        (37, L11_11_10, 4, &N7),
        (44, L10_10_10_2, 4, &N6),
        (50, L2_10_10_10, 4, &N6),
        (56, L8_8_8_8, 4, &N6),
        (62, L32_32, 8, &N3),
        (65, L16_16_16_16, 8, &N7),
        (72, L32_32_32, 12, &N3),
        (75, L32_32_32_32, 16, &N3),
    ];
    groups.iter().find_map(|&(first, fields, bytes, nums)| {
        (f >= first && f < first + nums.len() as u32).then(|| Ufmt { fields, bytes, num: nums[(f - first) as usize] })
    })
}

fn comps(m: &str) -> Option<u32> {
    Some(match m.rsplit('_').next()? {
        "x" => 1,
        "xy" => 2,
        "xyz" => 3,
        "xyzw" => 4,
        _ => return None,
    })
}

fn mask(bits: u32) -> u32 {
    if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 }
}

fn sext(v: &str, bits: u32) -> String {
    if bits >= 32 { format!("asint({v})") } else { format!("(asint(({v}) << {}u) >> {}u)", 32 - bits, 32 - bits) }
}

fn small_float(v: &str, mb: u32) -> String {
    let den = 127 - 14 - mb;
    format!(
        "((((({v}) >> {mb}u) & 31u) == 0u) ? asuint((float)(({v}) & {m:#x}u) * asfloat({:#x}u)) : (((({v}) >> {mb}u) & 31u) == 31u) ? (0x7f800000u | ((({v}) & {m:#x}u) << {sh}u)) : (((((({v}) >> {mb}u) & 31u) + 112u) << 23) | ((({v}) & {m:#x}u) << {sh}u)))",
        den << 23,
        m = mask(mb),
        sh = 23 - mb
    )
}

fn unpack(v: &str, bits: u32, num: Num) -> String {
    let mx = mask(bits);
    match num {
        Num::Unorm => format!("asuint((float)((double)({v}) / {}.0L))", mx),
        Num::Snorm => format!("asuint(max((float)((double){} / {}.0L), -1.0))", sext(v, bits), mask(bits - 1)),
        Num::Uscaled => format!("asuint((float)({v}))"),
        Num::Sscaled => format!("asuint((float){})", sext(v, bits)),
        Num::Uint => v.to_string(),
        Num::Sint => format!("asuint({})", sext(v, bits)),
        Num::Float => match bits {
            32 => v.to_string(),
            16 => format!("asuint(f16tof32({v}))"),
            11 => small_float(v, 6),
            10 => small_float(v, 5),
            _ => unreachable!(),
        },
    }
}

fn pack(v: &str, bits: u32, num: Num) -> Result<String, String> {
    let mx = mask(bits);
    Ok(match num {
        Num::Unorm => format!("((uint)round(saturate(asfloat({v})) * {mx}.0))"),
        Num::Snorm => {
            let a = format!("(abs(clamp(asfloat({v}), -1.0, 1.0)) * {}.0)", mask(bits - 1));
            let q = format!("((int)(floor({a}) + ((({a}) - floor({a})) >= 0.5 ? 1.0 : 0.0)))");
            format!("(((({v}) & 0x7fffffffu) > 0x7f800000u) ? 0u : (asuint((({v}) >> 31) != 0u ? -{q} : {q}) & {mx:#x}u))")
        }
        Num::Uint if bits == 32 => v.to_string(),
        Num::Uint => format!("min({v}, {mx:#x}u)"),
        Num::Sint if bits == 32 => v.to_string(),
        Num::Sint => format!("(asuint(clamp(asint({v}), {}, {})) & {mx:#x}u)", -(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1),
        Num::Float if bits == 32 => v.to_string(),
        Num::Float if bits == 16 => format!("(f32tof16(asfloat({v})) & 0xffffu)"),
        other => return Err(format!("tbuffer store to a {bits}-bit {other:?} component is not lowered")),
    })
}

fn unpack_d16(v: &str, bits: u32, num: Num) -> Result<String, String> {
    Ok(match (num, bits) {
        (Num::Float, 16) => v.to_string(),
        (Num::Float, 11) => format!("(({v}) << 4u)"),
        (Num::Float, 10) => format!("(({v}) << 5u)"),
        (Num::Uint, b) if b <= 16 => v.to_string(),
        (Num::Sint, b) if b <= 16 => format!("(asuint({}) & 0xffffu)", sext(v, b)),
        (Num::Uscaled, b) if b <= 11 => format!("(f32tof16((float)({v})) & 0xffffu)"),
        (Num::Sscaled, b) if b <= 11 => format!("(f32tof16((float){}) & 0xffffu)", sext(v, b)),
        (Num::Unorm | Num::Snorm, _) => return Err(format!("{bits}-bit {num:?} -> f16 rounding is not specified by the RDNA2 ISA")),
        _ => return Err(format!("a {bits}-bit {num:?} component does not fit 16 bits and the RDNA2 ISA does not specify the conversion")),
    })
}

fn pack_d16(h: &str, bits: u32, num: Num) -> Result<String, String> {
    match num {
        Num::Float if bits == 16 => Ok(h.to_string()),
        Num::Unorm | Num::Snorm | Num::Float => pack(&format!("asuint(f16tof32({h}))"), bits, num),
        Num::Sint => pack(&format!("asuint({})", sext(h, 16)), bits, num),
        _ => pack(h, bits, num),
    }
}

impl<'a> Emitter<'a> {
    pub(super) fn mtbuf_res(&mut self, ins: &Insn, slot: u32, table: u8, off: u32) -> Result<Res, String> {
        let fmt = ufmt(ins.aux.imm2 as u32).ok_or_else(|| format!("mtbuf at {:#x}: invalid buffer format {}", ins.offset, ins.aux.imm2))?;
        let writes = ins.mnemonic.starts_with("tbuffer_store") || self.uav_force.contains(&slot);
        let kind = self.kind_at(table, off, writes)?;
        let is_uav = match kind {
            Some(0) => false,
            Some(1) => true,
            other => return Err(format!("mtbuf at {:#x}: buffer in table of kind {other:?}", ins.offset)),
        };
        if slot & BINDLESS != 0 {
            if is_uav { self.bl_uav.insert(slot); } else { self.bl_srv.insert(slot); }
        }
        let end = (ins.aux.imm as u32 + fmt.bytes).div_ceil(4);
        let e = self.sbuf.entry((is_uav, slot)).or_insert(0);
        *e = (*e).max(end);
        Ok(Res::Buf(slot, is_uav))
    }

    pub(super) fn emit_mtbuf(&mut self, idx: usize, ins: &Insn) -> Result<(), String> {
        let d16 = ins.mnemonic.contains("_d16_");
        let fmt = ufmt(ins.aux.imm2 as u32).ok_or_else(|| format!("mtbuf: invalid buffer format {}", ins.aux.imm2))?;
        let n = comps(&ins.mnemonic).ok_or("mtbuf: bad op")?;
        let store = ins.mnemonic.starts_with("tbuffer_store");
        let (slot, is_uav) = match self.res.get(&idx).and_then(|v| v.first()) {
            Some(Res::Buf(s, u)) => (*s, *u),
            _ => return Err("mtbuf: unresolved buffer".into()),
        };
        if store && !is_uav {
            return Err("tbuffer_store to a non-UAV table".into());
        }
        let (offen, idxen) = (ins.aux.mask & 1 != 0, ins.aux.mask & 2 != 0);
        let a0 = ins.src[0];
        let a1 = match a0 {
            Opnd::Vgpr(v) => Opnd::Vgpr(v + 1),
            other => other,
        };
        let soff = ins.src[2];
        self.touch(&a0);
        self.touch(&soff);
        let index = if idxen { r(&a0)? } else { "0u".to_string() };
        let voff = match (idxen, offen) {
            (_, false) => None,
            (false, true) => Some(r(&a0)?),
            (true, true) => {
                self.touch(&a1);
                Some(r(&a1)?)
            }
        };
        let imm = ins.aux.imm as u32;
        let konst = voff.is_none() && matches!(soff, Opnd::ConstInt(0));
        let mut byte = format!("{imm:#x}u");
        if let Some(vo) = &voff {
            byte = format!("({vo}) + {byte}");
        }
        if !matches!(soff, Opnd::ConstInt(0)) {
            byte = format!("{byte} + {}", r(&soff)?);
        }
        let vd = match ins.dst[0] {
            Opnd::Vgpr(v) => v,
            _ => return Err("mtbuf: bad vdata".into()),
        };
        let nm = if is_uav { format!("ubuf{slot}") } else { format!("sbuf{slot}") };
        let nwords = self.sbuf_words((is_uav, slot));
        let el = |w: String| format!("{nm}[{index} + ({w}) / {nwords}u].w[({w}) % {nwords}u]");
        let t = self.tmp;
        self.tmp += 1;
        self.line(&format!("uint t{t}b = {byte}; uint t{t}w = t{t}b >> 2; uint t{t}s = (t{t}b & 3u) * 8u;"));
        let nc = fmt.fields.len() as u32;
        let ndw = fmt.bytes.div_ceil(4);
        if !store {
            for k in 0..ndw {
                let lo = el(format!("t{t}w + {k}u"));
                if konst && imm % 4 == 0 {
                    self.line(&format!("uint t{t}_{k} = {lo};"));
                } else {
                    let hi = el(format!("t{t}w + {}u", k + 1));
                    self.line(&format!("uint t{t}_{k} = ({lo} >> t{t}s) | (({hi} << (31u - t{t}s)) << 1u);"));
                }
            }
            let one = match (matches!(fmt.num, Num::Uint | Num::Sint), d16) {
                (true, _) => "1u",
                (false, false) => "0x3f800000u",
                (false, true) => "0x3c00u",
            };
            for k in 0..n {
                let e = if k < nc {
                    let (off, bits) = fmt.fields[k as usize];
                    let raw = if bits == 32 {
                        format!("t{t}_{}", off / 32)
                    } else {
                        format!("((t{t}_{} >> {}u) & {:#x}u)", off / 32, off % 32, mask(bits))
                    };
                    let c = format!("t{t}c{k}");
                    let v = if d16 { unpack_d16(&c, bits, fmt.num).map_err(|e| format!("{}: {e}", ins.mnemonic))? } else { unpack(&c, bits, fmt.num) };
                    self.line(&format!("uint {c} = {raw};"));
                    v
                } else if k == 3 {
                    one.to_string()
                } else {
                    "0u".to_string()
                };
                self.line(&format!("uint t{t}v{k} = {e};"));
            }
            if d16 {
                for j in 0..n.div_ceil(2) {
                    let o = Opnd::Vgpr(vd + j as u8);
                    let e = if 2 * j + 1 < n {
                        format!("(t{t}v{} | (t{t}v{} << 16u))", 2 * j, 2 * j + 1)
                    } else {
                        self.touch(&o);
                        format!("(({} & 0xffff0000u) | t{t}v{})", r(&o)?, 2 * j)
                    };
                    self.wv(&o, &e)?;
                }
                return Ok(());
            }
            for k in 0..n {
                self.wv(&Opnd::Vgpr(vd + k as u8), &format!("t{t}v{k}"))?;
            }
            return Ok(());
        }
        let w = n.min(nc);
        let len = fmt.bytes;
        let mut words = vec![String::new(); ndw as usize];
        for k in 0..w {
            let (off, bits) = fmt.fields[k as usize];
            let p = if d16 {
                let o = Opnd::Vgpr(vd + (k / 2) as u8);
                self.touch(&o);
                let h = if k % 2 == 0 { format!("({} & 0xffffu)", r(&o)?) } else { format!("({} >> 16u)", r(&o)?) };
                pack_d16(&h, bits, fmt.num)?
            } else {
                let o = Opnd::Vgpr(vd + k as u8);
                self.touch(&o);
                pack(&r(&o)?, bits, fmt.num)?
            };
            let part = if off % 32 == 0 { format!("({p})") } else { format!("(({p}) << {}u)", off % 32) };
            let slot_w = &mut words[(off / 32) as usize];
            if slot_w.is_empty() {
                *slot_w = part;
            } else {
                *slot_w = format!("{slot_w} | {part}");
            }
        }
        let init: Vec<String> = words.iter().map(|s| if s.is_empty() { "0u".to_string() } else { s.clone() }).collect();
        let full = len / 4;
        let mut body = format!("uint t{t}p[{}] = {{ {} }}; uint t{t}j0 = 0u; ", ndw.max(1), init.join(", "));
        if full > 0 {
            let direct: String = (0..full).map(|k| format!("{} = t{t}p[{k}]; ", el(format!("t{t}w + {k}u")))).collect();
            body += &format!("if (t{t}s == 0u) {{ {direct}t{t}j0 = {}u; }} ", full * 4);
        }
        let ew = el(format!("t{t}x"));
        body += &format!(
            "[allow_uav_condition] for (uint t{t}j = t{t}j0; t{t}j < {len}u; t{t}j++) {{ uint t{t}y = t{t}b + t{t}j; uint t{t}x = t{t}y >> 2; uint t{t}q = (t{t}y & 3u) * 8u; uint t{t}d = (t{t}p[t{t}j >> 2] >> ((t{t}j & 3u) * 8u)) & 0xffu; uint t{t}o = {ew}; [allow_uav_condition] for (uint t{t}i = 0u; t{t}i < 4096u; t{t}i++) {{ uint t{t}r; InterlockedCompareExchange({ew}, t{t}o, (t{t}o & ~(0xffu << t{t}q)) | (t{t}d << t{t}q), t{t}r); if (t{t}r == t{t}o) break; t{t}o = t{t}r; }} }}"
        );
        self.line(&format!("if (exec_lo & 1u) {{ {body}}}"));
        Ok(())
    }
}
