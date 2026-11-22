use super::bits::{key_decode, key_overflows};
use super::{dec, enc, Dcc, Stop, E_INVALIDARG};
use std::sync::Mutex;

pub(crate) const FILLS: usize = 4;

pub(crate) const UNSET: [(usize, usize); 4] = [(0x120, 0x344), (0x3a4, 0x3ac), (0x3dc, 0x3e0), (0x414, 0x418)];

pub(crate) struct Scratch {
    pub(crate) b: [u8; 64],
    pub(crate) unknown: u64,
}

pub(crate) static SCRATCH: Mutex<Scratch> = Mutex::new(Scratch { b: [0; 64], unknown: 0 });

pub(crate) fn fill(f: usize, i: usize) -> u8 {
    match f {
        0 => 0x00,
        1 => 0xff,
        _ => {
            let mut x = (i as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (f as u64).wrapping_mul(0xd6e8_feb8_6659_fd93);
            x ^= x >> 29;
            x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
            (x >> 32) as u8
        }
    }
}

pub(crate) fn fresh(f: usize, sc: &Scratch) -> Dcc {
    let mut d = Dcc::new();
    for (a, b) in UNSET {
        for i in a..b {
            d.o[i] = fill(f, i);
        }
    }
    for s in 0..3 {
        for i in 0..256 {
            d.s[s][i] = fill(f, 0x1000 + 0x100 * s + i);
        }
    }
    d.sc = sc.b;
    for i in 0..64 {
        if sc.unknown >> i & 1 != 0 {
            d.sc[i] = fill(f, 0x2000 + i);
        }
    }
    d
}

fn commit(sc: &mut Scratch, runs: &[Dcc], poisoned: bool) {
    let mut unk = 0u64;
    for i in 0..64 {
        if poisoned || runs.iter().any(|r| r.sc[i] != runs[0].sc[i]) {
            unk |= 1 << i;
        }
    }
    sc.b = runs[0].sc;
    sc.unknown = unk;
}

pub(crate) enum Outcome {
    Done,
    Stopped { block: usize, why: String },
}

fn why(s: Stop) -> String {
    match s {
        Stop::Fault(t) => format!("the console faults here ({t}), so it never returns"),
        Stop::Undef(t) => format!("console behaviour is not fixed by the inputs here ({t})"),
    }
}

pub(crate) fn setup_runs(format: i32, tile_mode: i32, params: [u32; 2], sc: &Scratch) -> Result<Vec<Dcc>, i32> {
    let mut runs = Vec::with_capacity(FILLS);
    for f in 0..FILLS {
        let mut d = fresh(f, sc);
        let hr = d.setup(format, tile_mode, params[0], params[1]);
        if hr != 0 {
            return Err(hr);
        }
        runs.push(d);
    }
    Ok(runs)
}

pub(crate) struct Blocks {
    pub(crate) out: Outcome,
    pub(crate) poisoned: bool,
}

fn first(stop: &mut Option<(usize, String)>, block: usize, why: String) {
    if stop.is_none() {
        *stop = Some((block, why));
    }
}

fn outcome(stop: Option<(usize, String)>) -> Outcome {
    match stop {
        None => Outcome::Done,
        Some((block, why)) => Outcome::Stopped { block, why },
    }
}

const DIVERGED: &str = "the output takes bytes the console never wrote this call (uninitialised heap surfaces or codec object stack)";

pub(crate) fn decode_all(runs: &mut [Dcc], src: &[u8], meta: &[u8], dst: &mut [u8], notes: &mut Vec<String>) -> Blocks {
    let mut stop = None;
    let mut live = vec![true; runs.len()];
    for i in 0..meta.len() {
        for (k, d) in runs.iter_mut().enumerate() {
            if !live[k] {
                continue;
            }
            d.s[1].copy_from_slice(&src[i * 256..i * 256 + 256]);
            d.p(0x34, meta[i] as u32);
            d.msg.clear();
            if let Err(e) = dec::run(d) {
                live[k] = false;
                first(&mut stop, i, why(e));
            }
        }
        if live.iter().any(|l| !l) {
            break;
        }
        if runs.iter().any(|r| r.s[0] != runs[0].s[0]) {
            first(&mut stop, i, DIVERGED.into());
        }
        if stop.is_none() {
            notes.extend(runs[0].msg.drain(..));
            dst[i * 256..i * 256 + 256].copy_from_slice(&runs[0].s[0]);
        }
    }
    Blocks { poisoned: live.iter().any(|l| !l), out: outcome(stop) }
}

pub(crate) fn decode_blocks(runs: &mut [Dcc], src: &[u8], meta: &[u8], dst: &mut [u8], notes: &mut Vec<String>) -> Outcome {
    decode_all(runs, src, meta, dst, notes).out
}

pub(crate) fn block_size(d: &Dcc) -> Result<u32, Stop> {
    let key = d.g(0x34);
    if key_overflows(key) {
        return Err(Stop::Undef("the export's size key decode writes past its stack buffer into the codec object"));
    }
    let k = key_decode(key);
    let mask = d.g(0x38);
    let mut acc = 0u32;
    for i in 0..4 {
        if mask >> i & 1 != 0 {
            acc |= k[4 + i];
        }
    }
    Ok(acc.count_ones() << 5)
}

pub(crate) const OVERRUN: &str = "the packed stream overruns the console's 256-byte heap output surface; whether the console then crashes depends on what the heap holds next to it";

pub(crate) fn encode_all(runs: &mut [Dcc], src: &[u8], dst: &mut [u8], meta: &mut [u8], mut sizes: Option<&mut [u32]>) -> Blocks {
    let mut stop = None;
    let mut live = vec![true; runs.len()];
    for i in 0..meta.len() {
        let mut outs = Vec::with_capacity(runs.len());
        for (k, d) in runs.iter_mut().enumerate() {
            if !live[k] {
                continue;
            }
            d.s[0].copy_from_slice(&src[i * 256..i * 256 + 256]);
            d.msg.clear();
            if let Err(e) = enc::run(d) {
                live[k] = false;
                first(&mut stop, i, why(e));
                continue;
            }
            if !d.spill.is_empty() {
                first(&mut stop, i, why(Stop::Undef(OVERRUN)));
            }
            match block_size(d) {
                Ok(s) => outs.push((d.s[1], d.o[0x34], s)),
                Err(e) => {
                    live[k] = false;
                    first(&mut stop, i, why(e));
                }
            }
        }
        if live.iter().any(|l| !l) {
            break;
        }
        if outs.iter().any(|o| *o != outs[0]) {
            first(&mut stop, i, DIVERGED.into());
        }
        if stop.is_none() {
            let (b, m, s) = outs[0];
            dst[i * 256..i * 256 + 256].copy_from_slice(&b);
            meta[i] = m;
            if let Some(z) = sizes.as_deref_mut() {
                z[i] = s;
            }
        }
    }
    Blocks { poisoned: live.iter().any(|l| !l), out: outcome(stop) }
}

pub(crate) fn encode_blocks(runs: &mut [Dcc], src: &[u8], dst: &mut [u8], meta: &mut [u8], sizes: Option<&mut [u32]>) -> Outcome {
    encode_all(runs, src, dst, meta, sizes).out
}

pub(crate) struct Call<'a> {
    pub(crate) format: i32,
    pub(crate) tile_mode: i32,
    pub(crate) params: [u32; 2],
    pub(crate) name: &'a str,
}

pub(crate) fn decode(c: &Call, src: &[u8], meta: &[u8], dst: &mut [u8]) -> i32 {
    let mut sc = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    let mut runs = match setup_runs(c.format, c.tile_mode, c.params, &sc) {
        Ok(r) => r,
        Err(hr) => return hr,
    };
    let mut notes = Vec::new();
    let r = decode_all(&mut runs, src, meta, dst, &mut notes);
    commit(&mut sc, &runs, r.poisoned);
    let out = r.out;
    drop(sc);
    for n in notes.iter().take(4) {
        compat_common::record(crate::M, c.name, &format!("console codec reports: {n}; it carries on and returns S_OK, as native does"));
    }
    finish(c, out)
}

pub(crate) fn encode(c: &Call, src: &[u8], dst: &mut [u8], meta: &mut [u8], sizes: Option<&mut [u32]>) -> i32 {
    let mut sc = SCRATCH.lock().unwrap_or_else(|e| e.into_inner());
    let mut runs = match setup_runs(c.format, c.tile_mode, c.params, &sc) {
        Ok(r) => r,
        Err(hr) => return hr,
    };
    let r = encode_all(&mut runs, src, dst, meta, sizes);
    commit(&mut sc, &runs, r.poisoned);
    let out = r.out;
    drop(sc);
    finish(c, out)
}

fn finish(c: &Call, out: Outcome) -> i32 {
    match out {
        Outcome::Done => 0,
        Outcome::Stopped { block, why } => {
            compat_common::record(crate::M, c.name, &format!("block {block} format {} tile mode {} params {:#x},{:#x}: {why}; native rejects and leaves this and later blocks unwritten", c.format, c.tile_mode, c.params[0], c.params[1]));
            E_INVALIDARG
        }
    }
}
