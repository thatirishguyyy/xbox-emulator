use compat_common::record;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const DECODE: &str = "float tb_snorm10(float u) { int s = (int)round(saturate(u) * 1023.0); if (s >= 512) s -= 1024; return max((float)s / 511.0, -1.0); }\n";

type Key = (u64, Vec<String>);
static VARIANTS: Mutex<Option<HashMap<Key, Option<Arc<Vec<u8>>>>>> = Mutex::new(None);

fn comps(ty: &str) -> usize {
    ty.chars().last().and_then(|c| c.to_digit(10)).map_or(1, |d| d as usize)
}

pub fn edit_hlsl(h: &str, sems: &[String]) -> Result<String, String> {
    let mut decodes = String::new();
    for sem in sems {
        let line = h
            .lines()
            .find(|l| l.trim_end().trim_end_matches(';').rsplit(':').next().is_some_and(|s| s.trim().eq_ignore_ascii_case(sem)) && l.trim_start().split_whitespace().nth(1).is_some_and(|v| v.starts_with('i')))
            .ok_or_else(|| format!("VS has no input with semantic {sem}"))?;
        let mut w = line.split_whitespace().filter(|t| !matches!(*t, "nointerpolation" | "linear" | "centroid" | "noperspective" | "precise"));
        let (ty, var) = (w.next().unwrap_or(""), w.next().unwrap_or(""));
        if !ty.starts_with("float") {
            return Err(format!("VS input {var} ({sem}) is {ty}, not float"));
        }
        for (k, c) in ["x", "y", "z"].iter().enumerate().take(comps(ty).min(3)) {
            let f = if comps(ty) == 1 { format!("i.{var}") } else { format!("i.{var}.{c}") };
            decodes.push_str(&format!("    {f} = tb_snorm10({f});\n"));
            let _ = k;
        }
    }
    let at = h.find("SOut main(SIn i) {\n").ok_or("VS has no `SOut main(SIn i)` entry")?;
    let body = at + "SOut main(SIn i) {\n".len();
    let decl = h[..at].rfind("struct SIn").ok_or("VS has no SIn struct")?;
    Ok(format!("{}{DECODE}{}{decodes}{}", &h[..decl], &h[decl..body], &h[body..]))
}

pub unsafe fn variant(p: *const u8, n: usize, sems: &[String]) -> Option<Arc<Vec<u8>>> {
    if p.is_null() || n == 0 || sems.is_empty() {
        return None;
    }
    let blob = unsafe { core::slice::from_raw_parts(p, n) };
    let hash = crate::shader_xlat::fnv1a(blob);
    let key = (hash, sems.to_vec());
    if let Some(v) = VARIANTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&key).cloned()) {
        return v;
    }
    let edit = |h: &str| edit_hlsl(h, sems);
    let r = match crate::shader_xlat::translate_edited(blob, hash, &[0; 8], &edit) {
        Ok(v) => {
            record("d3d12_xs", "vs_snorm", &format!("VS {hash:016x}: inputs {sems:?} decoded from R10G10B10_SNORM_A2_UNORM in the shader"));
            Some(Arc::new(v))
        }
        Err(e) => {
            record("d3d12_xs", "vs_snorm", &format!("VS {hash:016x}: SNORM decode variant for {sems:?} failed ({e}); inputs stay UNORM"));
            None
        }
    };
    VARIANTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key, r.clone());
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snorm_inputs_are_decoded_at_entry() {
        let h = "struct SIn {\n    float3 i0 : POSITION0;\n    float4 i1 : NORMAL0;\n};\nstruct SOut {\n    float4 o0 : SV_Position0;\n};\nSOut main(SIn i) {\n    SOut o = (SOut)0;\n    return o;\n}\n";
        let o = edit_hlsl(h, &["NORMAL0".to_string()]).unwrap();
        assert!(o.contains("i.i1.x = tb_snorm10(i.i1.x);"), "{o}");
        assert!(o.contains("i.i1.z = tb_snorm10(i.i1.z);"), "{o}");
        assert!(!o.contains("i.i1.w"), "{o}");
        assert!(o.find("float tb_snorm10").unwrap() < o.find("struct SIn").unwrap());
        assert!(edit_hlsl(h, &["TANGENT0".to_string()]).is_err());
    }

    #[test]
    fn decode_matches_snorm10() {
        let f = |s: i32| { let u = ((s + 1024) % 1024) as f32 / 1023.0; let r = (u * 1023.0).round() as i32; let r = if r >= 512 { r - 1024 } else { r }; (r as f32 / 511.0).max(-1.0) };
        assert_eq!(f(511), 1.0);
        assert_eq!(f(-511), -1.0);
        assert_eq!(f(-512), -1.0);
        assert_eq!(f(0), 0.0);
    }
}
