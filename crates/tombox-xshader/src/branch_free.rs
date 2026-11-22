use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    U,
    I,
    F,
    B,
    D,
}

const PURE_FN: &[&str] = &[
    "asuint", "asint", "asfloat", "asdouble", "abs", "min", "max", "clamp", "firstbithigh", "firstbitlow", "countbits", "reversebits", "f16tof32", "f32tof16",
    "floor", "ceil", "round", "frac", "trunc", "isinf", "isnan", "isfinite", "mad", "saturate", "sign", "sqrt", "rsqrt", "rcp", "log", "log2", "log10", "exp",
    "exp2", "pow", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "fmod", "lerp", "step", "dot", "ldexp", "select", "and", "or", "uint", "int", "float",
    "bool", "double", "uint2", "uint3", "uint4", "int2", "int3", "int4", "float2", "float3", "float4", "bool2", "bool3", "bool4", "asuint16", "asint16",
    "asfloat16", "half", "float16_t", "uint16_t", "int16_t", "tb_sel", "tb_and", "tb_or",
];

const ASSIGN: &[&str] = &["=", "+=", "-=", "*=", "/=", "%=", "|=", "&=", "^=", "<<=", ">>=", "++", "--"];

pub const PRELUDE: &str = "#if defined(__HLSL_VERSION) && __HLSL_VERSION >= 2021\n#define tb_sel(c, a, b) select((bool)(c), (a), (b))\n#define tb_and(a, b) and((a), (b))\n#define tb_or(a, b) or((a), (b))\n#else\n#define tb_sel(c, a, b) ((c) ? (a) : (b))\n#define tb_and(a, b) ((a) && (b))\n#define tb_or(a, b) ((a) || (b))\n#endif\n";

fn lex(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let st = i;
        if c.is_ascii_whitespace() {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
        } else if c.is_ascii_digit() || (c == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()) {
            while i < b.len() {
                let d = b[i];
                if d.is_ascii_alphanumeric() || d == b'.' || d == b'_' {
                    i += 1;
                } else if (d == b'+' || d == b'-') && matches!(b[i - 1], b'e' | b'E') && !s[st..i].starts_with("0x") && !s[st..i].starts_with("0X") {
                    i += 1;
                } else {
                    break;
                }
            }
        } else {
            let three = s.get(i..i + 3).unwrap_or("");
            let two = s.get(i..i + 2).unwrap_or("");
            if three == "<<=" || three == ">>=" {
                i += 3;
            } else if ["&&", "||", "==", "!=", "<=", ">=", "<<", ">>", "+=", "-=", "*=", "/=", "%=", "|=", "&=", "^=", "++", "--", "::"].contains(&two) {
                i += 2;
            } else {
                i += s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            }
        }
        out.push(s[st..i].to_string());
    }
    out
}

fn is_float_lit(t: &str) -> bool {
    let t = t.trim();
    t.starts_with(|c: char| c.is_ascii_digit() || c == '.') && !t.starts_with("0x") && !t.starts_with("0X") && (t.contains('.') || t.contains('e') || t.contains('E')) && !t.ends_with(['h', 'H', 'l', 'L']) && t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-' | 'f' | 'F'))
}

fn is_ident(t: &str) -> bool {
    t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

fn ty_of(name: &str) -> Option<Ty> {
    match name {
        "uint" | "dword" => Some(Ty::U),
        "int" => Some(Ty::I),
        "float" => Some(Ty::F),
        "bool" => Some(Ty::B),
        "double" => Some(Ty::D),
        _ => None,
    }
}

struct Ctx {
    vars: HashMap<String, Option<Ty>>,
    helpers: HashMap<String, Option<Ty>>,
    pure_helpers: HashSet<String>,
}

fn next_sig(ts: &[String], i: usize) -> Option<usize> {
    (i + 1..ts.len()).find(|&j| !ts[j].trim().is_empty())
}

fn calls(ts: &[String]) -> Vec<String> {
    let mut v = Vec::new();
    for i in 0..ts.len() {
        if is_ident(&ts[i]) && !matches!(ts[i].as_str(), "return" | "if" | "for" | "while" | "switch" | "else" | "do" | "case") && next_sig(ts, i).map(|j| ts[j] == "(").unwrap_or(false) {
            let prev = (0..i).rev().find(|&j| !ts[j].trim().is_empty());
            if prev.map(|p| ts[p] == ".").unwrap_or(false) {
                v.push(format!(".{}", ts[i]));
            } else {
                v.push(ts[i].clone());
            }
        }
    }
    v
}

impl Ctx {
    fn new(src: &str) -> Ctx {
        let mut vars: HashMap<String, Option<Ty>> = HashMap::new();
        let mut helpers: HashMap<String, Option<Ty>> = HashMap::new();
        let mut bodies: Vec<(String, Vec<String>, bool)> = Vec::new();
        for line in src.split('\n') {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let ts = lex(line);
            let sig: Vec<usize> = (0..ts.len()).filter(|&k| !ts[k].trim().is_empty()).collect();
            for w in 0..sig.len() {
                let (a, b) = (sig[w], sig.get(w + 1).copied());
                let Some(b) = b else { break };
                let Some(t) = ty_of(&ts[a]) else { continue };
                if !is_ident(&ts[b]) || ty_of(&ts[b]).is_some() {
                    continue;
                }
                let after = sig.get(w + 2).map(|&k| ts[k].as_str()).unwrap_or("");
                let before = if w > 0 { ts[sig[w - 1]].as_str() } else { "" };
                if before == "(" && after == ")" {
                    continue;
                }
                if after == "(" {
                    if w == 0 && ts[b].starts_with("tb_") {
                        let e = helpers.entry(ts[b].clone()).or_insert(Some(t));
                        if *e != Some(t) {
                            *e = None;
                        }
                        let open = sig[w + 2];
                        let mut d = 0i32;
                        let mut close = None;
                        for k in open..ts.len() {
                            if ts[k] == "(" {
                                d += 1;
                            } else if ts[k] == ")" {
                                d -= 1;
                                if d == 0 {
                                    close = Some(k);
                                    break;
                                }
                            }
                        }
                        if let Some(c) = close {
                            let params = ts[open..c].concat();
                            let outp = params.contains("out ") || params.contains("inout ") || params.contains("out\t");
                            bodies.push((ts[b].clone(), ts[c + 1..].to_vec(), outp));
                        }
                    }
                    continue;
                }
                if matches!(after, "=" | ";" | "," | ")" | "[") {
                    let e = vars.entry(ts[b].clone()).or_insert(Some(t));
                    if *e != Some(t) {
                        *e = None;
                    }
                }
            }
            for stmt in depth_split(&ts, &[";", "{", "}"]) {
                let s: Vec<&String> = stmt.iter().filter(|t| !t.trim().is_empty()).collect();
                let mut k = 0;
                while k < s.len() && matches!(s[k].as_str(), "precise" | "static" | "const") {
                    k += 1;
                }
                let Some(t) = s.get(k).and_then(|x| ty_of(x)) else { continue };
                if s.get(k + 2).map(|x| x.as_str() == "(").unwrap_or(true) {
                    continue;
                }
                let parts = depth_split(&stmt, &[","]);
                for p in parts.iter().step_by(2).skip(1) {
                    let names: Vec<&String> = p.iter().filter(|t| !t.trim().is_empty()).collect();
                    if let (Some(n), Some(nx)) = (names.first(), names.get(1).map(|x| x.as_str()).or(Some(""))) {
                        if is_ident(n) && ty_of(n).is_none() && matches!(nx, "=" | "" | "[") {
                            let e = vars.entry((*n).clone()).or_insert(Some(t));
                            if *e != Some(t) {
                                *e = None;
                            }
                        }
                    }
                }
            }
        }
        let mut pure_helpers: HashSet<String> = HashSet::new();
        loop {
            let mut grew = false;
            for (name, body, outp) in &bodies {
                if *outp || pure_helpers.contains(name) {
                    continue;
                }
                if body.iter().any(|t| t == "[" || t == "groupshared" || t == "static") {
                    continue;
                }
                if calls(body).iter().all(|c| PURE_FN.contains(&c.as_str()) || pure_helpers.contains(c) || c == name) {
                    pure_helpers.insert(name.clone());
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let dup: HashSet<String> = bodies.iter().map(|b| b.0.clone()).fold((HashSet::new(), HashSet::new()), |(mut s, mut d), n| {
            if !s.insert(n.clone()) {
                d.insert(n);
            }
            (s, d)
        }).1;
        for n in dup {
            let all_pure = bodies.iter().filter(|b| b.0 == n).all(|b| !b.2 && !b.1.iter().any(|t| t == "[" || t == "groupshared" || t == "static") && calls(&b.1).iter().all(|c| PURE_FN.contains(&c.as_str()) || pure_helpers.contains(c) || *c == n));
            if !all_pure {
                pure_helpers.remove(&n);
            }
        }
        Ctx { vars, helpers, pure_helpers }
    }

    fn pure(&self, s: &str) -> bool {
        let ts = lex(s);
        if ts.iter().any(|t| t == "[" || t == "]" || ASSIGN.contains(&t.as_str()) || t == "ResourceDescriptorHeap" || t == "SamplerDescriptorHeap") {
            return false;
        }
        calls(&ts).iter().all(|c| PURE_FN.contains(&c.as_str()) || self.pure_helpers.contains(c))
    }

    fn classify(&self, s: &str) -> Option<Ty> {
        let ts: Vec<String> = lex(s).into_iter().filter(|t| !t.trim().is_empty()).collect();
        if ts.is_empty() {
            return None;
        }
        let bin = depth_split(&ts, &["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"]);
        if bin.len() >= 3 && !bin[0].is_empty() && bin.iter().step_by(2).all(|p| !p.is_empty()) {
            let shifty = bin.iter().skip(1).step_by(2).any(|o| matches!(o[0].as_str(), "&" | "|" | "^" | "<<" | ">>" | "%"));
            let mut acc: Option<Ty> = None;
            let mut lit = false;
            for p in bin.iter().step_by(2) {
                let t = p.concat();
                if is_float_lit(&t) {
                    lit = true;
                    continue;
                }
                let c = self.classify(&t)?;
                if acc.is_some() && acc != Some(c) {
                    return None;
                }
                acc = Some(c);
            }
            return match acc {
                Some(Ty::F) if !shifty => Some(Ty::F),
                Some(t @ (Ty::U | Ty::I)) if !lit => Some(t),
                _ => None,
            };
        }
        if ts[0] == "-" && ts.len() >= 2 {
            return match self.classify(&ts[1..].concat()) {
                Some(Ty::F) => Some(Ty::F),
                Some(Ty::I) => Some(Ty::I),
                _ => None,
            };
        }
        if ts.len() == 1 {
            let t = &ts[0];
            if t.starts_with(|c: char| c.is_ascii_digit()) {
                let l = t.to_ascii_lowercase();
                if l.ends_with('u') && !l.ends_with("lu") && !l.ends_with("ul") {
                    return Some(Ty::U);
                }
                return None;
            }
            if t == "true" || t == "false" {
                return Some(Ty::B);
            }
            return self.vars.get(t).copied().flatten();
        }
        let close_of = |open: usize| -> Option<usize> {
            let mut d = 0i32;
            for k in open..ts.len() {
                if ts[k] == "(" {
                    d += 1;
                } else if ts[k] == ")" {
                    d -= 1;
                    if d == 0 {
                        return Some(k);
                    }
                }
            }
            None
        };
        if ts[0] == "(" {
            let c = close_of(0)?;
            if c == ts.len() - 1 {
                return self.classify(&ts[1..c].concat());
            }
            if c == 2 {
                if let Some(t) = ty_of(&ts[1]) {
                    let rest = &ts[3..];
                    if rest.len() == 1 || (rest.first().map(|x| x == "(").unwrap_or(false) && close_of(3) == Some(ts.len() - 1)) {
                        return Some(t);
                    }
                }
            }
            return None;
        }
        if is_ident(&ts[0]) && ts[1] == "(" && close_of(1) == Some(ts.len() - 1) {
            return match ts[0].as_str() {
                "asuint" => Some(Ty::U),
                "asint" => Some(Ty::I),
                "asfloat" => Some(Ty::F),
                n if n.starts_with("tb_") => self.helpers.get(n).copied().flatten(),
                n => ty_of(n),
            };
        }
        None
    }
}

fn depth_split(ts: &[String], ops: &[&str]) -> Vec<Vec<String>> {
    let mut out = vec![Vec::new()];
    let mut d = 0i32;
    for t in ts {
        let is_op = ops.contains(&t.as_str());
        match t.as_str() {
            "(" | "[" => d += 1,
            ")" | "]" => d -= 1,
            "{" if !is_op => d += 1,
            "}" if !is_op => d -= 1,
            _ => {}
        }
        if d == 0 && is_op {
            out.push(vec![t.clone()]);
            out.push(Vec::new());
        } else {
            out.last_mut().unwrap().push(t.clone());
        }
    }
    out
}

fn trimmed(ts: &[String]) -> String {
    ts.concat().trim().to_string()
}

impl Ctx {
    fn expr(&self, ts: &[String]) -> Vec<String> {
        let ts = self.groups(ts);
        let mut d = 0i32;
        let mut q = None;
        for (i, t) in ts.iter().enumerate() {
            match t.as_str() {
                "(" | "[" | "{" => d += 1,
                ")" | "]" | "}" => d -= 1,
                "?" if d == 0 => {
                    q = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let Some(q) = q else { return self.logic(&ts) };
        let (mut d, mut nest, mut colon) = (0i32, 0i32, None);
        for i in q + 1..ts.len() {
            match ts[i].as_str() {
                "(" | "[" | "{" => d += 1,
                ")" | "]" | "}" => d -= 1,
                "?" if d == 0 => nest += 1,
                ":" if d == 0 => {
                    if nest == 0 {
                        colon = Some(i);
                        break;
                    }
                    nest -= 1;
                }
                _ => {}
            }
        }
        let Some(colon) = colon else { return ts };
        let cond = self.logic(&ts[..q]);
        let a = self.expr(&ts[q + 1..colon]);
        let b = self.expr(&ts[colon + 1..]);
        let (sc, sa, sb) = (trimmed(&cond), trimmed(&a), trimmed(&b));
        let lead: Vec<String> = ts[..q].iter().take_while(|t| t.trim().is_empty()).cloned().collect();
        if sa == "1u" && sb == "0u" {
            let mut v = lead;
            v.push(format!("(uint)((bool)({sc}))"));
            v.push(" ".into());
            return v;
        }
        if self.pure(&sc) && self.pure(&sa) && self.pure(&sb) {
            let ca = if is_float_lit(&sa) { self.classify(&sb).filter(|t| *t == Ty::F) } else { self.classify(&sa) };
            let cb = if is_float_lit(&sb) { self.classify(&sa).filter(|t| *t == Ty::F) } else { self.classify(&sb) };
            if let (Some(x), Some(y)) = (ca, cb) {
                if x == y {
                    let mut v = lead;
                    v.push(format!("tb_sel({sc}, {sa}, {sb})"));
                    v.push(" ".into());
                    return v;
                }
            }
        }
        let mut v = cond;
        v.push("?".into());
        v.extend(a);
        v.push(":".into());
        v.extend(b);
        v
    }

    fn logic(&self, ts: &[String]) -> Vec<String> {
        for (op, f) in [("||", "tb_or"), ("&&", "tb_and")] {
            let parts = depth_split(ts, &[op]);
            if parts.len() < 3 {
                continue;
            }
            let subs: Vec<Vec<String>> = parts.iter().step_by(2).map(|p| self.logic(p)).collect();
            let texts: Vec<String> = subs.iter().map(|s| trimmed(s)).collect();
            if texts.iter().all(|t| !t.is_empty() && self.pure(t)) {
                let lead: Vec<String> = ts.iter().take_while(|t| t.trim().is_empty()).cloned().collect();
                let mut acc = format!("(bool)({})", texts[0]);
                for t in &texts[1..] {
                    acc = format!("{f}({acc}, (bool)({t}))");
                }
                let mut v = lead;
                v.push(acc);
                v.push(" ".into());
                return v;
            }
            let pre = texts.iter().take_while(|t| !t.is_empty() && self.pure(t)).count();
            let mut v = Vec::new();
            let mut start = 0;
            if pre >= 2 {
                v.extend(ts.iter().take_while(|t| t.trim().is_empty()).cloned());
                let mut acc = format!("(bool)({})", texts[0]);
                for t in &texts[1..pre] {
                    acc = format!("{f}({acc}, (bool)({t}))");
                }
                v.push(acc);
                v.push(" ".into());
                start = pre;
            }
            for (i, s) in subs.into_iter().enumerate().skip(start) {
                if i > 0 {
                    v.push(op.into());
                }
                v.extend(s);
            }
            return v;
        }
        ts.to_vec()
    }

    fn groups(&self, ts: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < ts.len() {
            let t = &ts[i];
            if t == "(" || t == "[" {
                let mut d = 0i32;
                let mut j = i;
                while j < ts.len() {
                    match ts[j].as_str() {
                        "(" | "[" => d += 1,
                        ")" | "]" => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if j >= ts.len() {
                    out.extend(ts[i..].iter().cloned());
                    return out;
                }
                out.push(t.clone());
                for p in depth_split(&ts[i + 1..j], &[",", ";"]) {
                    if p.len() == 1 && (p[0] == "," || p[0] == ";") {
                        out.push(p[0].clone());
                    } else {
                        out.extend(self.assign(&p));
                    }
                }
                out.push(ts[j].clone());
                i = j + 1;
            } else {
                out.push(t.clone());
                i += 1;
            }
        }
        out
    }

    fn assign(&self, ts: &[String]) -> Vec<String> {
        let mut d = 0i32;
        for (i, t) in ts.iter().enumerate() {
            match t.as_str() {
                "(" | "[" | "{" => d += 1,
                ")" | "]" | "}" => d -= 1,
                x if d == 0 && ASSIGN.contains(&x) && x != "++" && x != "--" => {
                    let mut v = self.groups(&ts[..i]);
                    v.push(t.clone());
                    v.extend(self.expr(&ts[i + 1..]));
                    return v;
                }
                _ => {}
            }
        }
        if let Some(k) = ts.iter().position(|t| t == "return") {
            let mut v = self.groups(&ts[..k]);
            v.push(ts[k].clone());
            v.extend(self.expr(&ts[k + 1..]));
            return v;
        }
        let first = ts.iter().find(|t| !t.trim().is_empty()).map(|s| s.as_str()).unwrap_or("");
        if matches!(first, "case" | "default") {
            return ts.to_vec();
        }
        self.expr(ts)
    }

    fn line(&self, l: &str) -> String {
        if l.trim_start().starts_with('#') || !(l.contains('?') || l.contains("&&") || l.contains("||")) {
            return l.to_string();
        }
        let ts = lex(l);
        let mut bal = 0i32;
        for t in &ts {
            match t.as_str() {
                "(" | "[" => bal += 1,
                ")" | "]" => bal -= 1,
                _ => {}
            }
            if bal < 0 {
                return l.to_string();
            }
        }
        if bal != 0 {
            return l.to_string();
        }
        let mut out = Vec::new();
        let mut cur: Vec<String> = Vec::new();
        let mut d = 0i32;
        for t in ts {
            match t.as_str() {
                "(" | "[" => d += 1,
                ")" | "]" => d -= 1,
                _ => {}
            }
            if d == 0 && matches!(t.as_str(), "{" | "}" | ";" | ",") {
                out.extend(self.assign(&cur));
                out.push(t);
                cur.clear();
                continue;
            }
            cur.push(t);
        }
        if cur.iter().any(|t| !t.trim().is_empty()) {
            out.extend(self.assign(&cur));
        } else {
            out.extend(cur);
        }
        out.concat()
    }
}

pub fn rewrite(src: &str) -> String {
    let ctx = Ctx::new(src);
    let mut out = String::with_capacity(src.len() + PRELUDE.len() + src.len() / 8);
    out.push_str(PRELUDE);
    let mut first = true;
    for l in src.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        out.push_str(&ctx.line(l));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(s: &str) -> String {
        rewrite(s)[PRELUDE.len()..].to_string()
    }

    #[test]
    fn bool_to_one_zero_becomes_a_cast() {
        let o = body("    uint scc; uint a; uint b;\n    scc = ((a | b) != 0u) ? 1u : 0u;");
        assert!(o.contains("scc = (uint)((bool)(((a | b) != 0u)))"), "{o}");
        assert!(!o.lines().nth(1).unwrap().contains('?'), "{o}");
    }

    #[test]
    fn pure_short_circuit_becomes_and_or() {
        let o = body("    uint x; uint y; uint e;\n    e = (((e & 1u) != 0u) && ((x < y) || (y < x))) ? 1u : 0u;");
        let l = o.lines().nth(1).unwrap();
        assert!(!l.contains("&&") && !l.contains("||") && !l.contains('?'), "{l}");
        assert!(l.contains("tb_and(") && l.contains("tb_or("), "{l}");
    }

    #[test]
    fn same_type_arms_select_mixed_types_keep_the_ternary() {
        let o = body("    uint v1; uint v2; int k; uint c;\n    v1 = (c != 0u) ? v2 : 0x5u;\n    v2 = (c != 0u) ? k : v2;\n    v2 = (c != 0u) ? 2 : v2;");
        let ls: Vec<&str> = o.lines().collect();
        assert!(ls[1].contains("tb_sel((c != 0u), v2, 0x5u)"), "{}", ls[1]);
        assert!(ls[2].contains('?'), "{}", ls[2]);
        assert!(ls[3].contains('?'), "{}", ls[3]);
    }

    #[test]
    fn memory_wave_and_indexing_keep_short_circuit() {
        let src = "    uint i; uint n; uint r[4]; uint a;\n    a = (i < n && r[i] != 0u) ? 1u : 0u;\n    a = (a != 0u) ? WaveReadLaneAt(a, 3u) : a;\n    a = (a != 0u && buf.Load(a) != 0u) ? 1u : 0u;";
        let o = body(src);
        let ls: Vec<&str> = o.lines().collect();
        assert!(ls[1].contains("&&"), "{}", ls[1]);
        assert!(ls[2].contains('?'), "{}", ls[2]);
        assert!(ls[3].contains("&&"), "{}", ls[3]);
    }

    #[test]
    fn helpers_are_rewritten_and_purity_follows_calls() {
        let src = "float tb_mull(float a, float b) { return (a == 0.0 || b == 0.0) ? 0.0 : a * b; }\nuint tb_rd(uint i) { return g[i]; }\n    uint q; uint z;\n    q = (z != 0u && tb_rd(z) != 0u) ? 1u : 0u;\n    q = (tb_mull(1.0, asfloat(z)) != 0.0 && z != 0u) ? 1u : 0u;";
        let o = body(src);
        let ls: Vec<&str> = o.lines().collect();
        assert!(ls[0].contains("tb_or(") && ls[0].contains("tb_sel(") && !ls[0].contains('?'), "{}", ls[0]);
        assert!(ls[3].contains("&&"), "{}", ls[3]);
        assert!(ls[4].contains("tb_and("), "{}", ls[4]);
    }

    #[test]
    fn declaration_lists_and_loops_are_split_correctly() {
        let src = "float tb_c(float x, float y) { float ax = abs(x), ay = abs(y); if (ax >= ay && ay >= 0.0) return x < 0.0 ? -x : x; for (uint k = 0u; k < 3u && ax > 1.0; k++) ax = ax * 0.5; return ax; }";
        let o = body(src);
        assert!(o.contains("float ax = abs(x), ay = abs(y);"), "{o}");
        assert!(o.contains("tb_and((bool)(ax >= ay), (bool)(ay >= 0.0))"), "{o}");
        assert!(o.contains("k++"), "{o}");
        assert!(o.contains("tb_sel(x < 0.0, -x, x)") && !o.contains('?'), "{o}");
        assert!(o.contains("tb_and((bool)(k < 3u), (bool)(ax > 1.0))"), "{o}");
    }

    #[test]
    fn float_arms_and_pure_prefixes() {
        let o = body("float tb_cs(float x, float z) { return z < 0.0 ? -x : x; }
float tb_m(float a, float b) { return (a == 0.0 || b == 0.0) ? 0.0 : a * b; }
    bool c; bool g; uint n;
    } while (c && g && ++n < 9u);
    n = c ? 2 : n;
    float w = c ? 0.5 : 1.5;");
        let ls: Vec<&str> = o.lines().collect();
        assert!(ls[0].contains("tb_sel(z < 0.0, -x, x)"), "{}", ls[0]);
        assert!(ls[1].contains("tb_sel(") && !ls[1].contains('?'), "{}", ls[1]);
        assert!(ls[3].contains("tb_and((bool)(c), (bool)(g))") && ls[3].contains("&& ++n"), "{}", ls[3]);
        assert!(ls[4].contains('?') && ls[5].contains('?'), "{o}");
    }

    #[test]
    fn lines_without_operators_and_unbalanced_lines_are_untouched() {
        let src = "#define TB_WV(e, d) (((exec_lo & 1u) != 0u) ? (uint)(e) : (d))\n    v0 = v1 + v2;\n    if ((a && b)\n";
        assert_eq!(body(src), src);
    }
}
