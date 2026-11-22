use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

extern "system" {
    fn HeapCreate(options: u32, initial: usize, maximum: usize) -> *mut c_void;
    fn HeapAlloc(heap: *mut c_void, flags: u32, bytes: usize) -> *mut c_void;
    fn HeapReAlloc(heap: *mut c_void, flags: u32, mem: *mut c_void, bytes: usize) -> *mut c_void;
    fn HeapFree(heap: *mut c_void, flags: u32, mem: *mut c_void) -> i32;
    fn GetProcessHeap() -> *mut c_void;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
    fn CreateMutexW(attrs: *mut c_void, owner: i32, name: *const u16) -> *mut c_void;
    fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    fn ReleaseMutex(h: *mut c_void) -> i32;
    fn GetCurrentProcessId() -> u32;
}

const HEAP_ZERO_MEMORY: u32 = 0x8;
const MIN_ALIGN: usize = 16;

pub struct PrivateHeap;

static HEAP: AtomicUsize = AtomicUsize::new(0);

fn heap() -> *mut c_void {
    let h = HEAP.load(Ordering::Acquire);
    if h != 0 {
        return h as *mut c_void;
    }
    let name: [u16; 20] = {
        let s = b"TOMBOX_PRIVATE_HEAP\0";
        let mut w = [0u16; 20];
        let mut i = 0;
        while i < 20 {
            w[i] = s[i] as u16;
            i += 1;
        }
        w
    };
    let mut v = [0u16; 4];
    let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), v.as_mut_ptr(), 4) };
    let off = n == 1 && v[0] == b'0' as u16;
    let created = if off {
        unsafe { GetProcessHeap() }
    } else {
        let m = unsafe { CreateMutexW(core::ptr::null_mut(), 0, MUTEX_NAME.as_ptr()) };
        if !m.is_null() {
            unsafe { WaitForSingleObject(m, 0xffff_ffff) };
        }
        let shared = read_shared();
        let h = if shared != 0 {
            shared as *mut c_void
        } else {
            let h = unsafe { HeapCreate(0, 0, 0) };
            if !h.is_null() {
                write_shared(h as usize);
            }
            h
        };
        if !m.is_null() {
            unsafe { ReleaseMutex(m) };
        }
        h
    };
    let created = if created.is_null() { unsafe { GetProcessHeap() } } else { created };
    match HEAP.compare_exchange(0, created as usize, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => created,
        Err(existing) => existing as *mut c_void,
    }
}

const fn wide<const N: usize>(s: &[u8; N]) -> [u16; N] {
    let mut w = [0u16; N];
    let mut i = 0;
    while i < N {
        w[i] = s[i] as u16;
        i += 1;
    }
    w
}

static MUTEX_NAME: [u16; 33] = wide(b"Local\\TomBoxPrivateHeapInit_v1__\0");
static SHARED_VAR: [u16; 27] = wide(b"TOMBOX_PRIVATE_HEAP_HANDLE\0");

fn hex_digits(x: usize, out: &mut [u16], mut n: usize) -> usize {
    let mut started = false;
    for i in (0..16).rev() {
        let d = (x >> (i * 4)) & 0xf;
        if d != 0 || started || i == 0 {
            started = true;
            out[n] = if d < 10 { 0x30 + d as u16 } else { 0x61 + d as u16 - 10 };
            n += 1;
        }
    }
    n
}

fn read_shared() -> usize {
    let mut v = [0u16; 48];
    let n = unsafe { GetEnvironmentVariableW(SHARED_VAR.as_ptr(), v.as_mut_ptr(), 48) } as usize;
    if n == 0 || n >= 48 {
        return 0;
    }
    let (mut h, mut pid, mut seen_sep) = (0usize, 0usize, false);
    for &c in &v[..n] {
        if c == 0x2d {
            seen_sep = true;
            continue;
        }
        let d = match c {
            0x30..=0x39 => c - 0x30,
            0x61..=0x66 => c - 0x61 + 10,
            _ => return 0,
        } as usize;
        if seen_sep {
            pid = (pid << 4) | d;
        } else {
            h = (h << 4) | d;
        }
    }
    if !seen_sep || pid != unsafe { GetCurrentProcessId() } as usize {
        return 0;
    }
    h
}

fn write_shared(h: usize) {
    let mut v = [0u16; 48];
    let mut n = hex_digits(h, &mut v, 0);
    v[n] = 0x2d;
    n = hex_digits(unsafe { GetCurrentProcessId() } as usize, &mut v, n + 1);
    v[n] = 0;
    unsafe { SetEnvironmentVariableW(SHARED_VAR.as_ptr(), v.as_ptr()) };
}

pub fn handle() -> *mut c_void {
    heap()
}

unsafe fn aligned_alloc(layout: Layout, flags: u32) -> *mut u8 {
    let h = heap();
    if layout.align() <= MIN_ALIGN {
        return unsafe { HeapAlloc(h, flags, layout.size()) } as *mut u8;
    }
    let total = layout.size() + layout.align();
    let raw = unsafe { HeapAlloc(h, flags, total) } as *mut u8;
    if raw.is_null() {
        return raw;
    }
    let off = layout.align() - (raw as usize & (layout.align() - 1));
    let p = unsafe { raw.add(off) };
    unsafe { (p as *mut *mut u8).sub(1).write(raw) };
    p
}

unsafe impl GlobalAlloc for PrivateHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { aligned_alloc(layout, 0) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { aligned_alloc(layout, HEAP_ZERO_MEMORY) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let base = if layout.align() <= MIN_ALIGN { ptr } else { unsafe { (ptr as *mut *mut u8).sub(1).read() } };
        unsafe { HeapFree(heap(), 0, base as *mut c_void) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if layout.align() <= MIN_ALIGN {
            return unsafe { HeapReAlloc(heap(), 0, ptr as *mut c_void, new_size) } as *mut u8;
        }
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        let n = unsafe { self.alloc(new_layout) };
        if !n.is_null() {
            unsafe { core::ptr::copy_nonoverlapping(ptr, n, layout.size().min(new_size)) };
            unsafe { self.dealloc(ptr, layout) };
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_heap_is_not_the_process_heap_and_honours_alignment() {
        let a = PrivateHeap;
        assert_ne!(handle(), unsafe { GetProcessHeap() });
        assert_eq!(read_shared(), handle() as usize);
        for align in [1usize, 8, 16, 64, 4096] {
            let l = Layout::from_size_align(100, align).unwrap();
            let p = unsafe { a.alloc_zeroed(l) };
            assert!(!p.is_null());
            assert_eq!(p as usize % align, 0);
            assert!((0..100).all(|i| unsafe { *p.add(i) } == 0));
            unsafe { p.write_bytes(7, 100) };
            let q = unsafe { a.realloc(p, l, 5000) };
            assert_eq!(q as usize % align, 0);
            assert!((0..100).all(|i| unsafe { *q.add(i) } == 7));
            unsafe { a.dealloc(q, Layout::from_size_align(5000, align).unwrap()) };
        }
    }
}
