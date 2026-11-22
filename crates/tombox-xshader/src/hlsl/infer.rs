use crate::meta::{PsvResource, RootParam, RootParamKind, RootRange, RootSig};
use crate::rdna2::{disassemble, Enc, Insn, Opnd};
use crate::{ShaderExtract, Stage};
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn sg(o: Option<&Opnd>) -> Option<u8> {
    match o {
        Some(Opnd::Sgpr(n)) => Some(*n),
        _ => None,
    }
}

fn determine_user_base(ex: &ShaderExtract, insns: &[Insn]) -> usize {
    if ex.stage == Stage::Mesh {
        16
    } else if ex.stage == Stage::Compute {
        4
    } else if ex.stage == Stage::Geometry {
        22
    } else {
        let chained = insns.iter().take(16).find(|i| i.mnemonic == "s_swappc_b64").and_then(|i| match i.dst.first() {
            Some(Opnd::Sgpr(n)) if *n >= 8 => Some(*n as usize + 7),
            _ => None,
        });
        chained.unwrap_or(8)
    }
}

fn match_resource(resources: &[PsvResource], used: &mut BTreeSet<usize>, range_type: u32, default_reg: u32) -> (u32, u32) {
    for (i, res) in resources.iter().enumerate() {
        if used.contains(&i) {
            continue;
        }
        let matches = match range_type {
            0 => (3..=5).contains(&res.res_type),
            1 => (6..=8).contains(&res.res_type),
            2 => res.res_type == 2,
            3 => res.res_type == 1,
            _ => false,
        };
        if matches {
            used.insert(i);
            return (res.lower, res.space);
        }
    }
    (default_reg, 0)
}

fn match_descriptor(resources: &[PsvResource], used: &mut BTreeSet<usize>, kind: u32, default_reg: u32) -> (u32, u32) {
    let range_type = match kind {
        2 => 2,
        3 => 0,
        _ => 1,
    };
    match_resource(resources, used, range_type, default_reg)
}

pub fn infer_root_signature(ex: &ShaderExtract) -> Result<RootSig, String> {
    let insns = disassemble(&ex.isa);
    let base = determine_user_base(ex, &insns);
    let mut origin: HashMap<u8, u8> = HashMap::new();
    let mut loaded_desc: HashMap<u8, (u8, u32)> = HashMap::new();
    let mut root_descs: BTreeMap<u8, u32> = BTreeMap::new();
    let mut table_ranges: BTreeMap<u8, BTreeSet<(u32, u32)>> = BTreeMap::new();

    for ins in &insns {
        match ins.enc {
            Enc::Sop1 if ins.mnemonic == "s_mov_b32" => {
                if let (Some(d), Some(s)) = (sg(ins.dst.first()), sg(ins.src.first())) {
                    let root = origin.get(&s).copied().or_else(|| (s as usize >= base).then_some(s));
                    if let Some(r) = root {
                        origin.insert(d, r);
                    } else {
                        origin.remove(&d);
                    }
                    if let Some(&desc) = loaded_desc.get(&s) {
                        loaded_desc.insert(d, desc);
                    } else {
                        loaded_desc.remove(&d);
                    }
                }
            }
            Enc::Sop1 if ins.mnemonic == "s_mov_b64" => {
                if let (Some(d), Some(s)) = (sg(ins.dst.first()), sg(ins.src.first())) {
                    for k in 0..2u8 {
                        let src_reg = s + k;
                        let dst_reg = d + k;
                        let root = origin.get(&src_reg).copied().or_else(|| (src_reg as usize >= base).then_some(src_reg));
                        if let Some(r) = root {
                            origin.insert(dst_reg, r);
                        } else {
                            origin.remove(&dst_reg);
                        }
                        if let Some(&desc) = loaded_desc.get(&src_reg) {
                            loaded_desc.insert(dst_reg, desc);
                        } else {
                            loaded_desc.remove(&dst_reg);
                        }
                    }
                }
            }
            Enc::Smem if ins.op <= 4 => {
                let b = sg(ins.src.first());
                let root = b.and_then(|r| origin.get(&r).copied().or_else(|| (r as usize >= base).then_some(r)));
                if let Some(table_root) = root {
                    let off = ins.aux.imm as u32;
                    if let Some(d) = sg(ins.dst.first()) {
                        let n = 1u32 << ins.op;
                        for k in 0..n as u8 {
                            loaded_desc.insert(d + k, (table_root, off));
                            origin.remove(&(d + k));
                        }
                    }
                } else if let Some(d) = sg(ins.dst.first()) {
                    let n = 1u32 << ins.op;
                    for k in 0..n as u8 {
                        loaded_desc.remove(&(d + k));
                        origin.remove(&(d + k));
                    }
                }
            }
            Enc::Smem if (8..=12).contains(&ins.op) => {
                if let Some(b) = sg(ins.src.first()) {
                    if let Some(&(table_root, off)) = loaded_desc.get(&b) {
                        table_ranges.entry(table_root).or_default().insert((2, off / 32));
                    } else {
                        let root = origin.get(&b).copied().or_else(|| (b as usize >= base).then_some(b));
                        if let Some(root_sgpr) = root {
                            root_descs.insert(root_sgpr, 2);
                        }
                    }
                }
                for d in &ins.dst {
                    if let Some(r) = sg(Some(d)) {
                        loaded_desc.remove(&r);
                        origin.remove(&r);
                    }
                }
            }
            Enc::Mubuf | Enc::Mtbuf => {
                if let Some(rs) = ins.src.get(1).and_then(|o| sg(Some(o))) {
                    if let Some(&(table_root, off)) = loaded_desc.get(&rs) {
                        let writes = ins.mnemonic.contains("store") || ins.mnemonic.contains("atomic");
                        let range_type = if writes { 1 } else { 0 };
                        table_ranges.entry(table_root).or_default().insert((range_type, off / 32));
                    }
                }
            }
            Enc::Mimg => {
                let n = ins.src.len();
                if let Some(rs) = ins.src.get(n.saturating_sub(2)).and_then(|o| sg(Some(o))) {
                    if let Some(&(table_root, off)) = loaded_desc.get(&rs) {
                        let writes = ins.mnemonic.contains("store") || ins.mnemonic.contains("atomic");
                        let range_type = if writes { 1 } else { 0 };
                        table_ranges.entry(table_root).or_default().insert((range_type, off / 32));
                    }
                }
                if ins.mnemonic.starts_with("image_sample") || ins.mnemonic.starts_with("image_gather") || ins.mnemonic == "image_get_lod" {
                    if let Some(ss) = ins.src.get(n.saturating_sub(1)).and_then(|o| sg(Some(o))) {
                        if let Some(&(table_root, off)) = loaded_desc.get(&ss) {
                            table_ranges.entry(table_root).or_default().insert((3, off / 16));
                        }
                    }
                }
            }
            _ => {
                for d in &ins.dst {
                    if let Some(r) = sg(Some(d)) {
                        loaded_desc.remove(&r);
                        origin.remove(&r);
                    }
                }
            }
        }
    }

    if root_descs.is_empty() && table_ranges.is_empty() {
        return Ok(RootSig {
            version: 2,
            flags: 0,
            params: Vec::new(),
            num_static_samplers: 0,
            static_samplers: Vec::new(),
        });
    }

    let max_desc = root_descs.keys().copied().map(|r| r.saturating_add(1));
    let max_table = table_ranges.keys().copied();
    let max_sgpr = max_desc.chain(max_table).max().unwrap_or(base as u8);

    let mut used_res: BTreeSet<usize> = BTreeSet::new();
    let mut params = Vec::new();
    let mut s = base as u8;
    while s <= max_sgpr {
        if let Some(&kind) = root_descs.get(&s) {
            let default_reg = (s as u32).saturating_sub(base as u32);
            let (reg, space) = match_descriptor(&ex.resources, &mut used_res, kind, default_reg);
            params.push(RootParam {
                visibility: 0,
                kind: RootParamKind::Descriptor { kind, register: reg, space },
            });
            s = s.saturating_add(2);
        } else if let Some(ranges_set) = table_ranges.get(&s) {
            let mut ranges = Vec::with_capacity(ranges_set.len());
            for &(range_type, off_table) in ranges_set {
                let (reg, space) = match_resource(&ex.resources, &mut used_res, range_type, off_table);
                ranges.push(RootRange {
                    range_type,
                    num: 1,
                    base_register: reg,
                    space,
                    offset_in_table: off_table,
                });
            }
            ranges.sort_by_key(|r| r.offset_in_table);
            params.push(RootParam {
                visibility: 0,
                kind: RootParamKind::Table(ranges),
            });
            s = s.saturating_add(1);
        } else {
            params.push(RootParam {
                visibility: 0,
                kind: RootParamKind::Table(Vec::new()),
            });
            s = s.saturating_add(1);
        }
    }

    Ok(RootSig {
        version: 2,
        flags: 0,
        params,
        num_static_samplers: 0,
        static_samplers: Vec::new(),
    })
}
