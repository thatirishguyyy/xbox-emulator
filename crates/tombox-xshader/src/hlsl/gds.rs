use super::{mesh, r, Emitter};
use crate::gds_abi::{tb_gds_byte_offset, TB_GDS_HLSL_DECL, TB_GDS_TITLE_DWORDS, TB_GWS_COUNTER_MASK, TB_GWS_RESOURCES, TB_GWS_TABLE_DWORD};
use crate::rdna2::{Enc, Insn};
use crate::Stage;

pub(super) fn is_gds(ins: &Insn) -> bool {
    ins.raw.first().is_some_and(|w| (w >> 17) & 1 != 0) || ins.mnemonic.starts_with("ds_gws_") || ins.mnemonic == "ds_ordered_count"
}

pub(super) fn gds_header(body: &str) -> Option<&'static str> {
    body.contains("tb_gds.").then_some(TB_GDS_HLSL_DECL)
}

impl Emitter<'_> {
    fn emit_gws_counter(&mut self, ins: &Insn) -> Result<(), String> {
        let name = ins.mnemonic.as_str();
        let [_, x, y, z] = self.ex.isa_preamble;
        let has_control_flow = self.insns.iter().enumerate().any(|(i, op)| {
            if op.mnemonic == "s_endpgm" {
                return i + 1 != self.insns.len();
            }
            (op.enc == Enc::Sopp && !matches!(op.mnemonic.as_str(), "s_nop" | "s_waitcnt"))
                || matches!(op.mnemonic.as_str(), "s_setpc_b64" | "s_swappc_b64")
        });
        let threads = x.saturating_mul(y).saturating_mul(z);
        if self.ex.stage != Stage::Compute || !(1..=1024).contains(&threads) || z > 64
            || mesh::wave_size(&self.insns) != 32 || has_control_flow
        {
            return Err(format!("{name}: the GWS counter model requires a straight-line wave32 compute group"));
        }
        if ins.raw[0] & (1 << 17) == 0 || ins.aux.imm & !0x3f != 0 {
            return Err(format!("{name}: invalid GWS resource encoding"));
        }
        let t = self.tmp;
        self.tmp += 1;
        let offset = ins.aux.imm;
        let resource_mask = TB_GWS_RESOURCES - 1;
        let table = tb_gds_byte_offset(TB_GWS_TABLE_DWORD);
        self.line(&format!("uint t{t}id = gtid.x + {x}u * (gtid.y + {y}u * gtid.z); uint t{t}wave = t{t}id >> 5u;"));
        self.line(&format!("if ((t{t}id & 31u) == 0u) tb_gws_counter_first[t{t}wave] = 0xffffffffu;"));
        self.line("GroupMemoryBarrierWithGroupSync();");
        self.line(&format!("if ((exec_lo & 1u) != 0u) InterlockedMin(tb_gws_counter_first[t{t}wave], t{t}id);"));
        self.line("GroupMemoryBarrierWithGroupSync();");
        self.line(&format!("if (t{t}id == tb_gws_counter_first[t{t}wave]) {{ uint t{t}a = {table}u + 4u * (((m0 >> 16) + {offset}u) & {resource_mask}u);"));
        let value = if name == "ds_gws_sema_v" {
            "1u".to_string()
        } else {
            let data = &ins.src[1];
            self.touch(data);
            format!("({}) & {TB_GWS_COUNTER_MASK}u", r(data)?)
        };
        let atomic = if name == "ds_gws_init" { "InterlockedExchange" } else { "InterlockedAdd" };
        self.line(&format!("uint t{t}old; tb_gds.{atomic}(t{t}a, {value}, t{t}old);"));
        self.line("}");
        self.line("AllMemoryBarrierWithGroupSync();");
        Ok(())
    }

    pub(super) fn emit_gds(&mut self, ins: &Insn) -> Result<(), String> {
        let m = ins.mnemonic.clone();
        if matches!(m.as_str(), "ds_gws_init" | "ds_gws_sema_v" | "ds_gws_sema_br") {
            return self.emit_gws_counter(ins);
        }
        if m.starts_with("ds_gws_") {
            return Err(format!("{m}: global wave sync counts hardware waves across thread groups; no lowering in the one-lane-per-thread model"));
        }
        if m == "ds_ordered_count" {
            return Err("ds_ordered_count: dispatch-ordered GDS append is not lowered".into());
        }
        let off = ins.aux.imm as u32 & 0xffff;
        let limit = TB_GDS_TITLE_DWORDS * 4;
        let t = self.tmp;
        self.tmp += 1;
        let d = ins.dst.first().copied();
        let (a, v0, v1) = (ins.src[0], ins.src[1], ins.src[2]);
        let counter = matches!(m.as_str(), "ds_append" | "ds_consume");
        if counter && mesh::wave_size(&self.insns) == 64 {
            return Err(format!("{m}: a wave64 shader spans two host waves, so count_bits(exec) and the shared pre-op value cannot be formed in one host wave"));
        }
        let rel = if counter {
            format!("{off:#x}u")
        } else {
            self.touch(&a);
            format!("({}) + {off:#x}u", r(&a)?)
        };
        self.line(&format!(
            "uint t{t}r = {rel}; uint t{t}a = ((m0 >> 16) + t{t}r) & ~3u; bool t{t}k = (exec_lo & 1u) != 0u && t{t}r < (m0 & 0xffffu) && t{t}a < {limit}u; uint t{t} = 0u;"
        ));
        let rtn = m.contains("_rtn") || counter || m == "ds_read_b32";
        let op = m.replace("_rtn", "");
        self.touch(&v0);
        self.touch(&v1);
        let (x0, x1) = (r(&v0)?, r(&v1)?);
        let body = match op.as_str() {
            "ds_append" | "ds_consume" => {
                self.uses_wave = true;
                let n = if op == "ds_append" { format!("t{t}n") } else { format!("0u - t{t}n") };
                self.line(&format!("uint4 t{t}b = WaveActiveBallot((exec_lo & 1u) != 0u); uint t{t}h = WaveGetLaneIndex() >> 5; uint t{t}m = (t{t}h == 0u) ? t{t}b.x : ((t{t}h == 1u) ? t{t}b.y : ((t{t}h == 2u) ? t{t}b.z : t{t}b.w)); uint t{t}n = countbits(t{t}m); uint t{t}l = (t{t}h << 5) + firstbitlow(t{t}m);"));
                format!("if (WaveGetLaneIndex() == t{t}l) {{ tb_gds.InterlockedAdd(t{t}a, {n}, t{t}); }}")
            }
            "ds_read_b32" => format!("t{t} = tb_gds.Load(t{t}a);"),
            "ds_write_b32" => format!("tb_gds.Store(t{t}a, {x0});"),
            "ds_add_u32" => format!("tb_gds.InterlockedAdd(t{t}a, {x0}, t{t});"),
            "ds_sub_u32" => format!("tb_gds.InterlockedAdd(t{t}a, 0u - ({x0}), t{t});"),
            "ds_min_u32" => format!("tb_gds.InterlockedMin(t{t}a, {x0}, t{t});"),
            "ds_max_u32" => format!("tb_gds.InterlockedMax(t{t}a, {x0}, t{t});"),
            "ds_min_i32" => format!("int t{t}s; tb_gds.InterlockedMin(t{t}a, asint({x0}), t{t}s); t{t} = asuint(t{t}s);"),
            "ds_max_i32" => format!("int t{t}s; tb_gds.InterlockedMax(t{t}a, asint({x0}), t{t}s); t{t} = asuint(t{t}s);"),
            "ds_and_b32" => format!("tb_gds.InterlockedAnd(t{t}a, {x0}, t{t});"),
            "ds_or_b32" => format!("tb_gds.InterlockedOr(t{t}a, {x0}, t{t});"),
            "ds_xor_b32" => format!("tb_gds.InterlockedXor(t{t}a, {x0}, t{t});"),
            "ds_wrxchg_b32" => format!("tb_gds.InterlockedExchange(t{t}a, {x0}, t{t});"),
            "ds_cmpst_b32" => {
                format!("tb_gds.InterlockedCompareExchange(t{t}a, {x0}, {x1}, t{t});")
            }
            "ds_rsub_u32" | "ds_inc_u32" | "ds_dec_u32" => {
                let x = &x0;
                let c = format!("t{t}c");
                let new = match op.as_str() {
                    "ds_rsub_u32" => format!("({x}) - {c}"),
                    "ds_inc_u32" => format!("(({c} >= ({x})) ? 0u : ({c} + 1u))"),
                    _ => format!("((({c} == 0u) || ({c} > ({x}))) ? ({x}) : ({c} - 1u))"),
                };
                format!(
                    "uint {c} = tb_gds.Load(t{t}a); [allow_uav_condition] for (uint t{t}i = 0u; t{t}i < 4096u; t{t}i++) {{ uint t{t}p; tb_gds.InterlockedCompareExchange(t{t}a, {c}, {new}, t{t}p); if (t{t}p == {c}) break; {c} = t{t}p; }} t{t} = {c};"
                )
            }
            other => return Err(format!("{other} with GDS=1 is not lowered")),
        };
        self.line(&format!("if (t{t}k) {{ {body} }}"));
        if counter {
            self.line(&format!("t{t} = WaveReadLaneAt(t{t}, t{t}l);"));
        }
        if rtn {
            let d = d.ok_or("gds: no vdst")?;
            self.wv(&d, &format!("t{t}"))?;
        }
        Ok(())
    }
}
