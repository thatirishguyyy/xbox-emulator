use super::*;
use crate::file::{self, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY, INVALID_FILE_ATTRIBUTES};
use crate::paths::{full_path_name, path_type, PATH_APPLICATION, PATH_CONSOLE};
use super::xfer::host_wide;

pub const DELTA_RECEIVE_TIMEOUT: u32 = 120000;
pub const DELTA_ENTRY_SIZE: usize = 32;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const INVALID_HANDLE: isize = -1;

#[repr(C)]
#[derive(Clone, Copy)]
struct FindData {
    attributes: u32,
    creation: [u32; 2],
    access: [u32; 2],
    write: [u32; 2],
    size_high: u32,
    size_low: u32,
    reserved0: u32,
    reserved1: u32,
    file_name: [u16; 260],
    alternate: [u16; 14],
    file_type: u32,
    creator_type: u32,
    finder_flags: u16,
}

#[repr(C)]
#[derive(Default)]
struct ByHandleInfo {
    attributes: u32,
    creation: [u32; 2],
    access: [u32; 2],
    write: [u32; 2],
    serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

#[allow(clashing_extern_declarations)]
#[link(name = "kernel32")]
extern "system" {
    fn FindFirstFileExW(name: *const u16, level: u32, data: *mut FindData, op: u32, filter: *mut c_void, flags: u32) -> isize;
    fn FindNextFileW(h: isize, data: *mut FindData) -> i32;
    fn FindClose(h: isize) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: isize) -> isize;
    fn GetFileInformationByHandle(h: isize, info: *mut ByHandleInfo) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn Sleep(ms: u32);
}

fn last_hr() -> HRESULT {
    hr_from_win32(unsafe { GetLastError() })
}

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

fn md5_k() -> [u32; 64] {
    let mut k = [0u32; 64];
    for (i, v) in k.iter_mut().enumerate() {
        *v = ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32;
    }
    k
}

pub fn md5(data: &[u8]) -> [u8; 16] {
    let k = md5_k();
    let mut st = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_le_bytes());
    for block in msg.chunks(64) {
        let w: Vec<u32> = block.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (st[0], st[1], st[2], st[3]);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let t = d;
            d = c;
            c = b;
            b = b.wrapping_add(a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(w[g]).rotate_left(S[i]));
            a = t;
        }
        st[0] = st[0].wrapping_add(a);
        st[1] = st[1].wrapping_add(b);
        st[2] = st[2].wrapping_add(c);
        st[3] = st[3].wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in st.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

fn lower_unit(u: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&u) {
        return u + 32;
    }
    if u < 0x80 {
        return u;
    }
    match char::from_u32(u as u32) {
        Some(c) => {
            let mut it = c.to_lowercase();
            match (it.next(), it.next()) {
                (Some(x), None) if (x as u32) < 0x10000 => x as u16,
                _ => u,
            }
        }
        None => u,
    }
}

pub fn normalize_key(s: &[u16]) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::with_capacity(s.len());
    for &u in s {
        if u == b'\\' as u16 && out.last() == Some(&(b'\\' as u16)) {
            continue;
        }
        out.push(lower_unit(u));
    }
    out
}

pub fn hash_key(s: &[u16]) -> [u8; 16] {
    let key = normalize_key(s);
    let bytes: Vec<u8> = key.iter().flat_map(|u| u.to_le_bytes()).collect();
    md5(&bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeltaEntry {
    pub hash: [u8; 16],
    pub time: u64,
    pub less: u32,
    pub greater: u32,
}

impl DeltaEntry {
    pub fn from_bytes(b: &[u8]) -> Self {
        let mut hash = [0u8; 16];
        hash.copy_from_slice(&b[..16]);
        DeltaEntry {
            hash,
            time: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            less: u32::from_le_bytes(b[24..28].try_into().unwrap()),
            greater: u32::from_le_bytes(b[28..32].try_into().unwrap()),
        }
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[..16].copy_from_slice(&self.hash);
        b[16..24].copy_from_slice(&self.time.to_le_bytes());
        b[24..28].copy_from_slice(&self.less.to_le_bytes());
        b[28..32].copy_from_slice(&self.greater.to_le_bytes());
        b
    }
}

pub fn lookup(table: &[DeltaEntry], hash: &[u8; 16]) -> Option<usize> {
    let mut n = 0usize;
    let mut steps = 0usize;
    loop {
        let e = table.get(n)?;
        steps += 1;
        if steps > table.len() {
            return None;
        }
        match e.hash.cmp(hash) {
            core::cmp::Ordering::Less => n = e.less as usize,
            core::cmp::Ordering::Greater => n = e.greater as usize,
            core::cmp::Ordering::Equal => {
                let g = e.greater as usize;
                if g == 0 || table.get(g).map_or(true, |x| x.hash != *hash) {
                    return Some(n);
                }
                return None;
            }
        }
        if n == 0 {
            return None;
        }
    }
}

pub struct DeltaResult {
    pub bits: Vec<u64>,
}

impl DeltaResult {
    pub fn new(count: u32) -> Self {
        let q = if count == 0 { 0 } else { count as usize / 64 + 1 };
        DeltaResult { bits: vec![u64::MAX; q] }
    }

    pub fn clear(&mut self, i: usize) {
        if let Some(w) = self.bits.get_mut(i >> 6) {
            *w &= !(1u64 << (i & 63));
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.bits.iter().flat_map(|w| w.to_le_bytes()).collect()
    }
}

pub struct ScanStats {
    pub files_deleted: u32,
    pub dirs_deleted: u32,
}

fn join(cur: &str, name: &str) -> String {
    if cur.ends_with('\\') {
        format!("{cur}{name}")
    } else {
        format!("{cur}\\{name}")
    }
}

fn name_of(d: &FindData) -> String {
    let n = d.file_name.iter().position(|&c| c == 0).unwrap_or(d.file_name.len());
    String::from_utf16_lossy(&d.file_name[..n])
}

fn probe_symlink(child: &str, d: &mut FindData) -> HRESULT {
    let a = file::get_attributes_raw(child);
    let flags = if a & FILE_ATTRIBUTE_DIRECTORY != 0 { 0x0200_0010 } else { 0x80 };
    let w = match host_wide(child) {
        Ok(w) => w,
        Err(e) => return e,
    };
    let h = unsafe { CreateFileW(w.as_ptr(), 0, 1, null_mut(), 3, flags, 0) };
    if h == INVALID_HANDLE {
        return last_hr();
    }
    let mut bh = ByHandleInfo::default();
    let ok = unsafe { GetFileInformationByHandle(h, &mut bh) };
    let hr = if ok == 0 { last_hr() } else { S_OK };
    unsafe { CloseHandle(h) };
    if ok != 0 {
        d.attributes = bh.attributes;
        d.creation = bh.creation;
        d.access = bh.access;
        d.write = bh.write;
        d.size_high = bh.size_high;
        d.size_low = bh.size_low;
    }
    hr
}

fn find_first(pattern: &str, d: &mut FindData) -> Result<Option<isize>, HRESULT> {
    let w = host_wide(pattern);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let h = match &w {
            Ok(w) => unsafe { FindFirstFileExW(w.as_ptr(), 1, d, 0, null_mut(), 2) },
            Err(e) => {
                unsafe { SetLastError((*e as u32) & 0xFFFF) };
                INVALID_HANDLE
            }
        };
        if h != INVALID_HANDLE {
            return Ok(Some(h));
        }
        let e = unsafe { GetLastError() };
        match e {
            2 | 5 => return Ok(None),
            6 | 0x40 if attempt < 4 => unsafe { Sleep(100) },
            _ => return Err(hr_from_win32(e)),
        }
    }
}

pub fn enumerate_delta_root(root: &str, table: &[DeltaEntry], result: &mut DeltaResult, remove_orphans: bool, timer: &mut Timer, stats: &mut ScanStats) -> HRESULT {
    stats.files_deleted = 0;
    stats.dirs_deleted = 0;
    let full = match full_path_name(root) {
        Ok(f) => f,
        Err(e) => return e,
    };
    if file::get_attributes_raw(&full) == INVALID_FILE_ATTRIBUTES {
        return last_hr();
    }
    let root_len = root.encode_utf16().count();
    let mut stack: VecDeque<String> = VecDeque::new();
    stack.push_back(full);
    let mut del_dirs: Vec<String> = Vec::new();
    while let Some(cur) = stack.pop_front() {
        let pattern = join(&cur, "*");
        let mut d: FindData = unsafe { core::mem::zeroed() };
        let h = match find_first(&pattern, &mut d) {
            Ok(Some(h)) => h,
            Ok(None) => continue,
            Err(e) => return e,
        };
        timer.mark("FileScan");
        let hr = 'dir: loop {
            let name = name_of(&d);
            let skip = d.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && (name == "." || name == "..");
            if !skip {
                let child = join(&cur, &name);
                if d.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    let hr = probe_symlink(&child, &mut d);
                    if hr < 0 {
                        break 'dir hr;
                    }
                }
                if d.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    stack.push_front(child.clone());
                }
                timer.mark("HashPrep");
                let units: Vec<u16> = child.encode_utf16().collect();
                let h = hash_key(&units[root_len.min(units.len())..]);
                timer.mark("Hash");
                let found = lookup(table, &h);
                timer.mark("HashFind");
                match found {
                    Some(i) => {
                        let t = table[i].time;
                        if t == 0 || t == (((d.write[1] as u64) << 32) | d.write[0] as u64) {
                            result.clear(i);
                        }
                    }
                    None if remove_orphans => {
                        if d.attributes & FILE_ATTRIBUTE_READONLY != 0 {
                            let hr = file::set_file_attributes(&child, d.attributes & !FILE_ATTRIBUTE_READONLY);
                            if hr < 0 {
                                break 'dir hr;
                            }
                        }
                        if d.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                            del_dirs.push(child);
                        } else {
                            let ok = match host_wide(&child) {
                                Ok(w) => (unsafe { DeleteFileW(w.as_ptr()) }) != 0,
                                Err(e) => break 'dir e,
                            };
                            if !ok {
                                break 'dir last_hr();
                            }
                            stats.files_deleted += 1;
                        }
                    }
                    None => {}
                }
            }
            if unsafe { FindNextFileW(h, &mut d) } == 0 {
                break S_OK;
            }
            timer.mark("FileScan");
        };
        unsafe { FindClose(h) };
        if hr < 0 {
            return hr;
        }
    }
    let mut hr;
    loop {
        hr = S_OK;
        let mut any = false;
        let mut keep = Vec::new();
        for dir in del_dirs.drain(..) {
            let ok = match host_wide(&dir) {
                Ok(w) => (unsafe { RemoveDirectoryW(w.as_ptr()) }) != 0,
                Err(_) => false,
            };
            if ok {
                any = true;
                stats.dirs_deleted += 1;
            } else {
                hr = last_hr();
                keep.push(dir);
            }
        }
        del_dirs = keep;
        if !any {
            break;
        }
    }
    timer.mark("DeleteWait");
    hr
}

pub struct DeltaRequest {
    pub file_count: u32,
    pub collision_table_size: u32,
    pub root: Bstr,
    pub remove_orphans: bool,
}

unsafe fn read_request(body: *mut c_void) -> Result<DeltaRequest, HRESULT> {
    let chk = |hr: HRESULT| if hr < 0 { Err(hr) } else { Ok(()) };
    let mut r = DeltaRequest { file_count: 0, collision_table_size: 0, root: Bstr::null(), remove_orphans: false };
    chk(get_u32(body, "FileCount", &mut r.file_count))?;
    chk(get_u32(body, "CollisionTableSize", &mut r.collision_table_size))?;
    chk(get_bstr(body, "DstFileName", &mut r.root))?;
    let mut d = 0f64;
    chk(get_number(body, "RemoveOrphans", &mut d))?;
    r.remove_orphans = d != 0.0;
    Ok(r)
}

unsafe fn receive_table(session: Session, request: *mut c_void, file_count: u32) -> Result<Vec<u8>, HRESULT> {
    let cap = DELTA_ENTRY_SIZE * file_count as usize;
    let mut table: Vec<u8> = Vec::with_capacity(cap);
    let mut need = file_count.wrapping_shl(5);
    let (p, n) = binary_body(request)?;
    let push = |table: &mut Vec<u8>, p: *mut u8, n: u32| -> Result<(), HRESULT> {
        if table.len() + n as usize > cap {
            return Err(E_INVALIDARG);
        }
        table.extend_from_slice(binary_slice(p, n));
        Ok(())
    };
    push(&mut table, p, n)?;
    need = need.wrapping_sub(n);
    while need != 0 {
        let m = session.receive_more(DELTA_RECEIVE_TIMEOUT)?;
        let (p, n) = binary_body(m.0)?;
        push(&mut table, p, n)?;
        need = need.wrapping_sub(n);
    }
    Ok(table)
}

pub unsafe fn calculate_incremental_changes(session: Session, request: *mut c_void, response: *mut c_void) -> HRESULT {
    let mut timer = Timer::new();
    let mut rb = Com::null();
    XbtpCreateJsonObject(rb.out());
    let mut stats = ScanStats { files_deleted: 0, dirs_deleted: 0 };
    let hr = 'reply: {
        let mut body = Com::null();
        let hr = deserialize_json_body(request, &mut body);
        if hr < 0 {
            break 'reply hr;
        }
        let req = match read_request(body.0) {
            Ok(r) => r,
            Err(e) => break 'reply e,
        };
        let units = bstr_units(&req.root);
        let (ty, rel) = match path_type(&units) {
            Ok(v) => v,
            Err(e) => break 'reply e,
        };
        if ty == PATH_APPLICATION {
            break 'reply HR_FILE_NOT_FOUND;
        }
        let root_units = if ty == PATH_CONSOLE { &units[rel..] } else { &units[..] };
        let root = String::from_utf16_lossy(root_units).trim_end_matches('\\').to_string();
        timer.mark("Cpu");
        let raw = match receive_table(session, request, req.file_count) {
            Ok(t) => t,
            Err(e) => break 'reply e,
        };
        let mut collisions: Vec<u8> = Vec::with_capacity(req.collision_table_size as usize);
        while collisions.len() < req.collision_table_size as usize {
            let m = match session.receive_more(DELTA_RECEIVE_TIMEOUT) {
                Ok(m) => m,
                Err(e) => break 'reply e,
            };
            let (p, n) = match binary_body(m.0) {
                Ok(v) => v,
                Err(e) => break 'reply e,
            };
            if n == 0 || collisions.len() + n as usize > req.collision_table_size as usize {
                break 'reply E_INVALIDARG;
            }
            collisions.extend_from_slice(binary_slice(p, n));
        }
        timer.mark("Network");
        let table: Vec<DeltaEntry> = raw.chunks_exact(DELTA_ENTRY_SIZE).map(DeltaEntry::from_bytes).collect();
        let mut result = DeltaResult::new(req.file_count);
        let hr = enumerate_delta_root(&root, &table, &mut result, req.remove_orphans, &mut timer, &mut stats);
        if hr < 0 {
            break 'reply hr;
        }
        let bytes = result.bytes();
        (vt::<MessageVtbl>(response).set_binary_body)(response, bytes.len() as u32, bytes.as_ptr())
    };
    timer.merge(rb.0);
    set_i32(rb.0, "Result", hr);
    set_u32(rb.0, "FilesDeleted", stats.files_deleted);
    set_u32(rb.0, "DirectoriesDeleted", stats.dirs_deleted);
    serialize_response_object(rb.0, response)
}
