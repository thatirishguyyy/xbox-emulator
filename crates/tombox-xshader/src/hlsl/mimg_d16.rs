use super::{f, r, Emitter};
use crate::rdna2::{Enc, Insn, Opnd};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Lane {
    Float,
    Uint,
    Sint,
}

pub(super) fn mimg_d16(ins: &Insn) -> bool {
    ins.enc == Enc::Mimg
        && !ins.mnemonic.starts_with("image_bvh")
        && !ins.mnemonic.starts_with("image_atomic")
        && ins.mnemonic != "image_get_resinfo"
        && ins.raw.get(1).is_some_and(|w| (w >> 31) & 1 != 0)
}

pub(super) fn mimg_data_regs(ins: &Insn) -> u32 {
    let c = if ins.mnemonic.starts_with("image_gather") { 4 } else { (ins.aux.mask & 0xf).count_ones() };
    if mimg_d16(ins) {
        c.div_ceil(2)
    } else {
        c
    }
}

pub(super) fn half_to_lane(reg: &str, hi: bool, lane: Lane) -> String {
    let h = if hi { format!("({reg} >> 16u)") } else { format!("({reg} & 0xffffu)") };
    match lane {
        Lane::Float => format!("f16tof32({h})"),
        Lane::Uint => h,
        Lane::Sint => if hi { format!("(asint({reg}) >> 16)") } else { format!("(asint({reg} << 16u) >> 16)") },
    }
}

pub(super) fn lane_to_half(bits: &str, lane: Lane) -> String {
    match lane {
        Lane::Float => format!("tb_f2h(asfloat({bits}))"),
        Lane::Uint | Lane::Sint => format!("(({bits}) & 0xffffu)"),
    }
}

pub(super) fn pack_halves(vals: &[String], lane: Lane) -> Vec<String> {
    vals.chunks(2)
        .map(|p| match p {
            [lo, hi] => format!("({} | ({} << 16u))", lane_to_half(lo, lane), lane_to_half(hi, lane)),
            [lo] => lane_to_half(lo, lane),
            _ => unreachable!(),
        })
        .collect()
}

impl<'a> Emitter<'a> {
    pub(super) fn mimg_store_lanes(&mut self, ins: &Insn, vd: u8, lane: Lane, fill: &str) -> Result<Vec<String>, String> {
        let d16 = mimg_d16(ins);
        let mut out = Vec::with_capacity(4);
        let mut n = 0u8;
        for ch in 0..4 {
            if (ins.aux.mask >> ch) & 1 == 0 {
                out.push(fill.to_string());
                continue;
            }
            let o = Opnd::Vgpr(vd + if d16 { n / 2 } else { n });
            self.touch(&o);
            out.push(if d16 {
                half_to_lane(&r(&o)?, n & 1 != 0, lane)
            } else if lane == Lane::Float {
                f(&o)?
            } else {
                r(&o)?
            });
            n += 1;
        }
        Ok(out)
    }

    pub(super) fn mimg_write_lanes(&mut self, ins: &Insn, vd: u8, vals: Vec<String>, lane: Lane) -> Result<(), String> {
        let regs = if mimg_d16(ins) { pack_halves(&vals, lane) } else { vals };
        for (n, e) in regs.iter().enumerate() {
            self.wv(&Opnd::Vgpr(vd + n as u8), e)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_pack_low_then_high() {
        let v = pack_halves(&["a".into(), "b".into(), "c".into()], Lane::Float);
        assert_eq!(v, vec!["(tb_f2h(asfloat(a)) | (tb_f2h(asfloat(b)) << 16u))".to_string(), "tb_f2h(asfloat(c))".to_string()]);
        let u = pack_halves(&["x".into(), "y".into()], Lane::Uint);
        assert_eq!(u, vec!["(((x) & 0xffffu) | (((y) & 0xffffu) << 16u))".to_string()]);
    }

    #[test]
    fn halves_unpack_by_lane_kind() {
        assert_eq!(half_to_lane("v4", false, Lane::Float), "f16tof32((v4 & 0xffffu))");
        assert_eq!(half_to_lane("v4", true, Lane::Float), "f16tof32((v4 >> 16u))");
        assert_eq!(half_to_lane("v4", true, Lane::Uint), "(v4 >> 16u)");
        assert_eq!(half_to_lane("v4", false, Lane::Sint), "(asint(v4 << 16u) >> 16)");
        assert_eq!(half_to_lane("v4", true, Lane::Sint), "(asint(v4) >> 16)");
    }
}
