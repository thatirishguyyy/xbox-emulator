use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

pub const DISPLAY_MODE_1: u32 = 0x0001_0007;
pub const DISPLAY_MODE_2: u32 = 0x0002_0007;
pub const PROXY_APP: u32 = 0x0064_003f;
pub const CHANNEL_OPEN: u32 = 0x0000_0005;
pub const WINDOW_MESSAGING: u32 = 0x0000_0001;
pub const AUDIO_HELLO: u32 = 0x0008;

#[derive(Default)]
struct Stream {
    buf: Vec<u8>,
    prev_in: usize,
    prev_out: usize,
    opened: Vec<u16>,
    window_pids: Vec<u32>,
}

fn streams() -> MutexGuard<'static, HashMap<u64, Stream>> {
    static S: OnceLock<Mutex<HashMap<u64, Stream>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn pad(prev: usize) -> usize {
    prev.wrapping_neg() & 7
}

pub fn frame(prev: usize, kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8; pad(prev)];
    v.extend_from_slice(&kind.to_le_bytes());
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

pub fn parse(buf: &[u8], mut prev: usize) -> (Vec<(u32, Vec<u8>)>, usize, usize) {
    let mut at = 0;
    let mut out = Vec::new();
    loop {
        let p = pad(prev);
        if buf.len() < at + p + 8 {
            break;
        }
        let h = at + p;
        let kind = u32::from_le_bytes(buf[h..h + 4].try_into().unwrap());
        let len = u32::from_le_bytes(buf[h + 4..h + 8].try_into().unwrap()) as usize;
        if buf.len() < h + 8 + len {
            break;
        }
        out.push((kind, buf[h + 8..h + 8 + len].to_vec()));
        at = h + 8 + len;
        prev = len;
    }
    (out, at, prev)
}

pub fn opened(id: u64) -> Vec<u16> {
    streams().get(&id).map(|s| s.opened.clone()).unwrap_or_default()
}

pub fn window_pids(id: u64) -> Vec<u32> {
    streams().get(&id).map(|s| s.window_pids.clone()).unwrap_or_default()
}

pub fn push(id: u64, kind: u32, payload: &[u8]) -> i32 {
    let bytes = {
        let mut s = streams();
        let st = s.entry(id).or_default();
        let b = frame(st.prev_out, kind, payload);
        st.prev_out = payload.len();
        b
    };
    crate::xvcmgmt::host_send(id, &bytes)
}

pub fn push_input(id: u64, records: &[(u32, u64, u64)]) -> i32 {
    let mut p = Vec::with_capacity(records.len() * 24);
    for &(msg, w, l) in records {
        p.extend_from_slice(&msg.to_le_bytes());
        p.extend_from_slice(&[0; 4]);
        p.extend_from_slice(&w.to_le_bytes());
        p.extend_from_slice(&l.to_le_bytes());
    }
    push(id, WINDOW_MESSAGING, &p)
}

pub fn forget(id: u64) {
    streams().remove(&id);
}

pub fn on_bytes(id: u64, data: &[u8]) -> Vec<u8> {
    let mut s = streams();
    let st = s.entry(id).or_default();
    st.buf.extend_from_slice(data);
    let (msgs, used, prev) = parse(&st.buf, st.prev_in);
    st.buf.drain(..used);
    st.prev_in = prev;
    let mut out = Vec::new();
    for (kind, payload) in msgs {
        if kind & 0xffff == AUDIO_HELLO && payload.is_empty() {
            let ack = kind & 0xffff_0000;
            crate::crossvm::set_hello((kind >> 16) as u16);
            compat_common::record_raw("gamecore-host", "audio", &format!("XVMAudioService hello {kind:#010x}; host ack frame {ack:#010x}"));
            out.extend(frame(st.prev_out, ack, &[]));
            st.prev_out = 0;
            continue;
        }
        match crate::gcreply::reply(kind, &payload) {
            Some(r) => {
                compat_common::record_raw("gamecore-host", "reply", &format!("request {kind:#010x} ({} bytes) -> reply {} bytes", payload.len(), r.len()));
                out.extend(frame(st.prev_out, kind, &r));
                st.prev_out = r.len();
            }
            None if kind == CHANNEL_OPEN && payload.len() == 6 => {
                let class = u16::from_le_bytes([payload[4], payload[5]]);
                compat_common::record_raw("gamecore-host", "open", &format!("client opened message class {class:#x}; the client does not wait, no reply"));
                st.opened.push(class);
            }
            None if kind == WINDOW_MESSAGING && payload.len() == 4 => {
                let pid = u32::from_le_bytes(payload[..4].try_into().unwrap());
                compat_common::record_raw("gamecore-host", "window-messaging", &format!("client registered process {pid} for host input records; no host input is pushed"));
                st.window_pids.push(pid);
            }
            None => compat_common::record_raw("gamecore-host", "unanswered", &format!("request {kind:#010x} ({} bytes) has no host handler; no reply", payload.len())),
        }
    }
    out
}
