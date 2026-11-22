use super::xasync::{self, Payload};
use super::events::Registry;
use super::{cstr, log, setting, wstr, Guid, Iface, This, E_INVALIDARG, S_OK};
use core::ffi::c_void;

pub const CLSID: Guid = Guid::parse("37e56907-2f10-41e8-b72f-36edb185331a");

static OP_PORT: &[u8] = b"XNetworkingQueryPreferredLocalUdpMultiplayerPortAsync\0";
static OP_SECURITY: &[u8] = b"XNetworkingQuerySecurityInformationForUrlAsync\0";
static OP_SECURITY16: &[u8] = b"XNetworkingQuerySecurityInformationForUrlUtf16Async\0";

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NlHint {
    pub level: u32,
    pub cost: u32,
    pub approaching: u8,
    pub over: u8,
    pub roaming: u8,
}

#[link(name = "iphlpapi")]
extern "system" {
    fn GetNetworkConnectivityHint(hint: *mut NlHint) -> u32;
}

pub fn system_hint() -> Option<NlHint> {
    let mut h = NlHint::default();
    if unsafe { GetNetworkConnectivityHint(&mut h) } == 0 { Some(h) } else { None }
}

pub fn connectivity_level() -> u32 {
    match setting("connectivity").as_deref() {
        Some("none") => 1,
        Some("local") => 2,
        Some("internet") => 3,
        Some("constrained") => 4,
        _ => match system_hint().map(|h| h.level) {
            Some(l @ 1..=4) => l,
            Some(5) => 2,
            _ => 3,
        },
    }
}

pub fn udp_port() -> u16 {
    setting("udp_port").and_then(|p| p.parse().ok()).unwrap_or(3074)
}

fn security_payload() -> Payload {
    let mut b = vec![0u8; 24];
    b[0..4].copy_from_slice(&(0x0000_0800u32 | 0x0000_2000).to_le_bytes());
    Payload::data(b)
}

unsafe extern "system" fn port_async(_t: This, block: *mut c_void) -> i32 {
    xasync::run(block, OP_PORT, Payload::value(udp_port()))
}

unsafe extern "system" fn port_result(_t: This, block: *mut c_void, out: *mut u16) -> i32 {
    xasync::value(block, OP_PORT, out)
}

unsafe extern "system" fn security_async(_t: This, url: *const u8, block: *mut c_void) -> i32 {
    log("XNetworkingQuerySecurityInformationForUrlAsync", &format!("'{}' -> TLS1.2/1.3, no pinning", cstr(url).unwrap_or_default()));
    xasync::run(block, OP_SECURITY, security_payload())
}

unsafe extern "system" fn security16_async(_t: This, url: *const u16, block: *mut c_void) -> i32 {
    log("XNetworkingQuerySecurityInformationForUrlUtf16Async", &format!("'{}' -> TLS1.2/1.3, no pinning", wstr(url).unwrap_or_default()));
    xasync::run(block, OP_SECURITY16, security_payload())
}

unsafe extern "system" fn result_size(_t: This, block: *mut c_void, size: *mut usize) -> i32 {
    xasync::size(block, size)
}

unsafe fn security_result_impl(op: &'static [u8], block: *mut c_void, size: usize, used: *mut usize, buf: *mut u8, ptr: *mut *mut u8) -> i32 {
    if !ptr.is_null() {
        *ptr = core::ptr::null_mut();
    }
    let mut n = 0usize;
    let hr = xasync::bytes(block, op, size, buf as *mut c_void, &mut n);
    if !used.is_null() {
        *used = n;
    }
    if hr >= 0 && !ptr.is_null() {
        *ptr = buf;
    }
    hr
}

pub unsafe extern "system" fn security_result(_t: This, block: *mut c_void, size: usize, used: *mut usize, buf: *mut u8, ptr: *mut *mut u8) -> i32 {
    security_result_impl(OP_SECURITY, block, size, used, buf, ptr)
}

pub unsafe extern "system" fn security16_result(_t: This, block: *mut c_void, size: usize, used: *mut usize, buf: *mut u8, ptr: *mut *mut u8) -> i32 {
    security_result_impl(OP_SECURITY16, block, size, used, buf, ptr)
}

#[repr(C)]
struct CertContext {
    encoding: u32,
    encoded: *const u8,
    len: u32,
    info: *const c_void,
    store: *mut c_void,
}

#[repr(C)]
struct ChainPara {
    size: u32,
    usage_type: u32,
    usage_count: u32,
    usage: *const *const u8,
}

#[repr(C)]
struct SslPolicy {
    size: u32,
    auth_type: u32,
    checks: u32,
    server: *const u16,
}

#[repr(C)]
struct PolicyPara {
    size: u32,
    flags: u32,
    extra: *const c_void,
}

#[repr(C)]
struct PolicyStatus {
    size: u32,
    error: u32,
    chain_index: i32,
    element_index: i32,
    extra: *mut c_void,
}

#[link(name = "crypt32")]
extern "system" {
    fn CertGetCertificateChain(engine: *mut c_void, cert: *const CertContext, time: *const c_void, store: *mut c_void, para: *const ChainPara, flags: u32, reserved: *mut c_void, out: *mut *const c_void) -> i32;
    fn CertVerifyCertificateChainPolicy(oid: usize, chain: *const c_void, para: *const PolicyPara, status: *mut PolicyStatus) -> i32;
    fn CertFreeCertificateChain(chain: *const c_void);
    fn CertFreeCertificateContext(cert: *const CertContext) -> i32;
    fn CertGetCertificateContextProperty(cert: *const CertContext, id: u32, data: *mut c_void, len: *mut u32) -> i32;
}

unsafe fn thumbprint(cert: *const CertContext) -> Vec<u8> {
    let mut b = vec![0u8; 20];
    let mut n = 20u32;
    if CertGetCertificateContextProperty(cert, 3, b.as_mut_ptr() as *mut c_void, &mut n) == 0 {
        return Vec::new();
    }
    b.truncate(n as usize);
    b
}

unsafe fn chain_certs(chain: *const c_void) -> Vec<*const CertContext> {
    let mut v = Vec::new();
    if chain.is_null() {
        return v;
    }
    let count = *(chain as *const u8).add(16).cast::<u32>();
    let simple = *(chain as *const u8).add(24).cast::<*const *const u8>();
    if count == 0 || simple.is_null() {
        return v;
    }
    let s = *simple;
    let n = *s.add(20).cast::<u32>();
    let elems = *s.add(24).cast::<*const *const u8>();
    for i in 0..n as usize {
        let e = *elems.add(i);
        v.push(*e.add(8).cast::<*const CertContext>());
    }
    v
}

pub unsafe fn verify_pins(info: *const u8, certs: &[Vec<u8>]) -> bool {
    if info.is_null() {
        return true;
    }
    let count = core::ptr::read_unaligned(info.add(8) as *const usize);
    let list = core::ptr::read_unaligned(info.add(16) as *const *const u8);
    if count == 0 || list.is_null() {
        return true;
    }
    for i in 0..count {
        let t = list.add(i * 24);
        let kind = core::ptr::read_unaligned(t as *const u32);
        let n = core::ptr::read_unaligned(t.add(8) as *const usize);
        let p = core::ptr::read_unaligned(t.add(16) as *const *const u8);
        if p.is_null() || n == 0 {
            continue;
        }
        let want = core::slice::from_raw_parts(p, n);
        let pos = match kind {
            0 => certs.first(),
            1 => certs.get(1),
            _ => certs.last(),
        };
        if pos.map(|c| c.as_slice() == want).unwrap_or(false) || (kind > 2 && certs.iter().any(|c| c.as_slice() == want)) {
            return true;
        }
    }
    false
}

unsafe extern "system" fn verify_certificate(_t: This, request: *mut c_void, info: *const u8) -> i32 {
    if request.is_null() {
        return E_INVALIDARG;
    }
    if let Some(host) = crate::net_guard::in_process_request(request) {
        log("XNetworkingVerifyServerCertificate", &format!("'{host}' is served by the in-process online backend; no TLS hop to verify"));
        return S_OK;
    }
    let mut cert: *const CertContext = core::ptr::null();
    let mut n = 8u32;
    if super::webauth::WinHttpQueryOption(request, 78, &mut cert as *mut *const CertContext as *mut c_void, &mut n) == 0 || cert.is_null() {
        let hr = super::win::last_error_hr();
        log("XNetworkingVerifyServerCertificate", &format!("no server certificate on request: {hr:#010x}"));
        return hr;
    }
    let mut url = vec![0u16; 2048];
    let mut ul = (url.len() * 2) as u32;
    let host = if super::webauth::WinHttpQueryOption(request, 34, url.as_mut_ptr() as *mut c_void, &mut ul) != 0 {
        let u = String::from_utf16_lossy(&url[..ul as usize / 2]);
        u.split("://").nth(1).unwrap_or("").split(['/', ':', '?']).next().unwrap_or("").to_string()
    } else {
        String::new()
    };
    let para = ChainPara { size: core::mem::size_of::<ChainPara>() as u32, usage_type: 0, usage_count: 0, usage: core::ptr::null() };
    let mut chain: *const c_void = core::ptr::null();
    let ok = CertGetCertificateChain(core::ptr::null_mut(), cert, core::ptr::null(), (*cert).store, &para, 0, core::ptr::null_mut(), &mut chain) != 0;
    let mut hr = if ok { S_OK } else { super::win::last_error_hr() };
    if ok {
        let w = super::win::wide(&host);
        let ssl = SslPolicy { size: core::mem::size_of::<SslPolicy>() as u32, auth_type: 2, checks: 0, server: if host.is_empty() { core::ptr::null() } else { w.as_ptr() } };
        let pp = PolicyPara { size: core::mem::size_of::<PolicyPara>() as u32, flags: 0, extra: &ssl as *const SslPolicy as *const c_void };
        let mut st = PolicyStatus { size: core::mem::size_of::<PolicyStatus>() as u32, error: 0, chain_index: -1, element_index: -1, extra: core::ptr::null_mut() };
        if CertVerifyCertificateChainPolicy(4, chain, &pp, &mut st) == 0 {
            hr = super::win::last_error_hr();
        } else if st.error != 0 {
            hr = st.error as i32;
        } else {
            let prints: Vec<Vec<u8>> = chain_certs(chain).into_iter().map(|c| thumbprint(c)).collect();
            if !verify_pins(info, &prints) {
                hr = 0x800B_0109u32 as i32;
            }
        }
        CertFreeCertificateChain(chain);
    }
    CertFreeCertificateContext(cert);
    if hr < 0 {
        log("XNetworkingVerifyServerCertificate", &format!("'{host}' rejected hr={hr:#010x}"));
    }
    hr
}

unsafe extern "system" fn port_sync(_t: This, out: *mut u16) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    *out = udp_port();
    S_OK
}

static PORT_CHANGED: Registry = Registry::new();
static HINT_CHANGED: Registry = Registry::new();

unsafe extern "system" fn register_port(_t: This, queue: usize, ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    PORT_CHANGED.register(0, queue, ctx, cb, token)
}

unsafe extern "system" fn unregister_port(_t: This, token: u64, _wait: bool) -> bool {
    PORT_CHANGED.unregister(None, token)
}

pub fn notify_connectivity_changed() {
    HINT_CHANGED.fire(None, |r| unsafe { deliver_hint(r) });
}

pub unsafe extern "system" fn connectivity_hint(_t: This, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let sys = system_hint().unwrap_or(NlHint { level: 3, cost: 1, ..Default::default() });
    core::ptr::write_bytes(out, 0, 16);
    core::ptr::write_unaligned(out as *mut u32, connectivity_level());
    core::ptr::write_unaligned(out.add(4) as *mut u32, if sys.cost == 0 { 1 } else { sys.cost });
    core::ptr::write_unaligned(out.add(8) as *mut u32, 6);
    *out.add(12) = 1;
    *out.add(13) = sys.approaching;
    *out.add(14) = sys.over;
    *out.add(15) = sys.roaming;
    S_OK
}

unsafe fn deliver_hint(r: &super::events::Reg) {
    let mut hint = [0u8; 16];
    connectivity_hint(core::ptr::null(), hint.as_mut_ptr());
    let cb: unsafe extern "system" fn(*mut c_void, *const u8) = core::mem::transmute(r.cb);
    cb(r.ctx as *mut c_void, hint.as_ptr());
}

unsafe extern "system" fn register_hint(_t: This, queue: usize, ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    let hr = HINT_CHANGED.register(0, queue, ctx, cb, token);
    if hr < 0 {
        return hr;
    }
    let t = *token;
    let sub = HINT_CHANGED.fire_token(t, |r| unsafe {
        deliver_hint(r);
        log("XNetworkingRegisterConnectivityHintChanged", &format!("token={} initial hint level={} networkInitialized=1 delivered", r.token, connectivity_level()));
    });
    log("XNetworkingRegisterConnectivityHintChanged", &format!("queue={queue:#x} token={t} -> initial hint queued hr={sub:#010x}"));
    hr
}

unsafe extern "system" fn unregister_hint(_t: This, token: u64, _wait: bool) -> bool {
    HINT_CHANGED.unregister(None, token)
}

pub const IID_V2: Guid = Guid::parse("bf2346b2-39af-4658-b5ea-44713c7e83b3");

fn slots() -> Vec<(usize, usize)> {
    vec![
            (3, port_sync as usize),
            (4, port_async as usize),
            (5, port_result as usize),
            (6, register_port as usize),
            (7, unregister_port as usize),
            (8, security_async as usize),
            (9, result_size as usize),
            (10, security_result as usize),
            (11, security16_async as usize),
            (12, result_size as usize),
            (13, security16_result as usize),
            (14, verify_certificate as usize),
            (15, connectivity_hint as usize),
            (16, register_hint as usize),
            (17, unregister_hint as usize),
    ]
}

pub fn interfaces() -> Vec<Iface> {
    vec![
        Iface { clsid: CLSID, iid: CLSID, name: "XNetworking(37e56907)", slots: slots() },
        Iface { clsid: CLSID, iid: IID_V2, name: "XNetworking(bf2346b2)", slots: slots() },
    ]
}

