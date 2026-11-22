use tombox_xshader::hlsl::{bl_alias_space, BL_ALIASES};

const PARAM_TABLE: u32 = 0;
const RANGE_SRV: u32 = 0;
const RANGE_UAV: u32 = 1;

fn rd(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| format!("RTS0 read at {o:#x} out of bounds ({} bytes)", b.len()))
}

fn wr(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn add_aliases(body: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let version = rd(body, 0)?;
    let rs = match version {
        1 => 20,
        2 | 3 => 24,
        v => return Err(format!("RTS0 version {v} is not supported")),
    };
    let (np, po) = (rd(body, 4)? as usize, rd(body, 8)? as usize);
    let mut out = body.to_vec();
    let mut added = 0;
    for i in 0..np {
        let at = po + i * 12;
        if rd(body, at)? != PARAM_TABLE {
            continue;
        }
        let payload = rd(body, at + 8)? as usize;
        let (nr, ro) = (rd(body, payload)? as usize, rd(body, payload + 4)? as usize);
        let mut ranges: Vec<Vec<u8>> = Vec::with_capacity(nr);
        let mut extra: Vec<Vec<u8>> = Vec::new();
        let mut next = 0u32;
        for r in 0..nr {
            let ra = ro + r * rs;
            let d = body.get(ra..ra + rs).ok_or("RTS0 range out of bounds")?.to_vec();
            let (ty, num, space, off) = (rd(&d, 0)?, rd(&d, 4)?, rd(&d, 12)?, rd(&d, rs - 4)?);
            let start = if off == u32::MAX { next } else { off };
            next = start.saturating_add(num);
            if (ty == RANGE_SRV || ty == RANGE_UAV) && space < 100_000 && start != u32::MAX {
                for k in 1..=BL_ALIASES {
                    let mut a = d.clone();
                    wr(&mut a, 12, bl_alias_space(k, space));
                    wr(&mut a, rs - 4, start);
                    extra.push(a);
                }
            }
            ranges.push(d);
        }
        if extra.is_empty() {
            continue;
        }
        let new_ro = out.len();
        for r in ranges.iter().chain(extra.iter()) {
            out.extend_from_slice(r);
        }
        added += extra.len();
        wr(&mut out, payload, (ranges.len() + extra.len()) as u32);
        wr(&mut out, payload + 4, new_ro as u32);
    }
    Ok((added > 0).then_some(out))
}

pub fn replace_rts0(blob: &[u8], body: &[u8]) -> Result<Vec<u8>, String> {
    if blob.get(0..4) != Some(b"DXBC") {
        return Ok(body.to_vec());
    }
    let n = rd(blob, 28)? as usize;
    let mut chunks: Vec<([u8; 4], Vec<u8>)> = Vec::with_capacity(n);
    for i in 0..n {
        let o = rd(blob, 32 + i * 4)? as usize;
        let size = rd(blob, o + 4)? as usize;
        let cc: [u8; 4] = blob.get(o..o + 4).ok_or("DXBC chunk out of bounds")?.try_into().unwrap();
        let data = if &cc == b"RTS0" { body.to_vec() } else { blob.get(o + 8..o + 8 + size).ok_or("DXBC chunk out of bounds")?.to_vec() };
        chunks.push((cc, data));
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"DXBC");
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&blob[20..24]);
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    let mut o = 32 + n * 4;
    for (_, d) in &chunks {
        out.extend_from_slice(&(o as u32).to_le_bytes());
        o += 8 + d.len();
    }
    for (cc, d) in &chunks {
        out.extend_from_slice(cc);
        out.extend_from_slice(&(d.len() as u32).to_le_bytes());
        out.extend_from_slice(d);
    }
    let total = out.len() as u32;
    wr(&mut out, 24, total);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbounded_srv_ranges_get_alias_spaces_at_the_same_offsets() {
        let rs = 24;
        let mut b = vec![0u8; 24 + 12 + 8 + 2 * rs];
        let w = |b: &mut Vec<u8>, o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        w(&mut b, 0, 2);
        w(&mut b, 4, 1);
        w(&mut b, 8, 24);
        w(&mut b, 32, 36);
        w(&mut b, 36, 2);
        w(&mut b, 40, 44);
        for (k, r) in [[1u32, 4, 0, 0, 0, 0], [0, u32::MAX, 0, 5, 0, u32::MAX]].iter().enumerate() {
            for (i, v) in r.iter().enumerate() {
                w(&mut b, 44 + k * rs + i * 4, *v);
            }
        }
        let o = add_aliases(&b).unwrap().unwrap();
        let p = tombox_xshader::meta::parse_rts0(&o).unwrap();
        let tombox_xshader::meta::RootParamKind::Table(r) = &p.params[0].kind else { panic!() };
        assert_eq!(r.len(), 2 + 2 * BL_ALIASES as usize);
        assert_eq!((r[1].offset_in_table, r[1].space), (u32::MAX, 5));
        for k in 1..=BL_ALIASES {
            let a = &r[1 + k as usize];
            assert_eq!((a.offset_in_table, a.space, a.num, a.range_type), (0, bl_alias_space(k, 0), 4, 1));
            let a = &r[1 + (BL_ALIASES + k) as usize];
            assert_eq!((a.offset_in_table, a.space, a.num, a.range_type), (4, bl_alias_space(k, 5), u32::MAX, 0));
        }
        let mut no = b.clone();
        w(&mut no, 44, 2);
        w(&mut no, 44 + rs, 3);
        assert!(add_aliases(&no).unwrap().is_none());
    }

    #[test]
    fn dxbc_container_is_rebuilt_around_the_new_rts0() {
        let mut blob = Vec::new();
        blob.extend_from_slice(b"DXBC");
        blob.extend_from_slice(&[7u8; 16]);
        blob.extend_from_slice(&1u32.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());
        blob.extend_from_slice(&2u32.to_le_bytes());
        blob.extend_from_slice(&40u32.to_le_bytes());
        blob.extend_from_slice(&52u32.to_le_bytes());
        blob.extend_from_slice(b"RTS0");
        blob.extend_from_slice(&4u32.to_le_bytes());
        blob.extend_from_slice(&[1, 2, 3, 4]);
        blob.extend_from_slice(b"XTRA");
        blob.extend_from_slice(&2u32.to_le_bytes());
        blob.extend_from_slice(&[9, 9]);
        let o = replace_rts0(&blob, &[5u8; 12]).unwrap();
        let c = tombox_xshader::Dxbc::parse(&o).unwrap();
        assert_eq!(c.chunk(b"RTS0").unwrap().size, 12);
        assert_eq!(c.chunk(b"XTRA").unwrap().size, 2);
        assert_eq!(rd(&o, 24).unwrap() as usize, o.len());
    }
}
