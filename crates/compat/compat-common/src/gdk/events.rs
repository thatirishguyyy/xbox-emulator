use super::{next_token, E_INVALIDARG, S_OK};
use std::sync::Mutex;

#[derive(Clone, Copy)]
pub struct Reg {
    pub token: u64,
    pub owner: usize,
    pub queue: usize,
    pub ctx: usize,
    pub cb: usize,
}

pub struct Registry {
    regs: Mutex<Vec<Reg>>,
}

impl Registry {
    pub const fn new() -> Registry {
        Registry { regs: Mutex::new(Vec::new()) }
    }

    pub unsafe fn register(&self, owner: usize, queue: usize, ctx: *mut core::ffi::c_void, cb: usize, token: *mut u64) -> i32 {
        if token.is_null() || cb == 0 {
            return E_INVALIDARG;
        }
        let t = next_token();
        self.regs.lock().unwrap_or_else(|e| e.into_inner()).push(Reg { token: t, owner, queue, ctx: ctx as usize, cb });
        *token = t;
        S_OK
    }

    pub fn unregister(&self, owner: Option<usize>, token: u64) -> bool {
        let mut v = self.regs.lock().unwrap_or_else(|e| e.into_inner());
        let before = v.len();
        v.retain(|r| !(r.token == token && owner.map_or(true, |o| o == r.owner)));
        v.len() != before
    }

    pub fn all(&self) -> Vec<Reg> {
        self.regs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn len(&self) -> usize {
        self.regs.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, token: u64) -> bool {
        self.regs.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|r| r.token == token)
    }

    pub fn fire_token<F>(&'static self, token: u64, f: F) -> i32
    where
        F: FnOnce(&Reg) + Send + 'static,
    {
        let found = self.regs.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|r| r.token == token).copied();
        let Some(reg) = found else { return E_INVALIDARG };
        unsafe {
            super::xasync::submit(reg.queue, 1, Box::new(move |canceled| {
                if !canceled && self.contains(reg.token) {
                    f(&reg)
                }
            }))
        }
    }

    pub fn fire<F>(&self, owner: Option<usize>, f: F)
    where
        F: Fn(&Reg) + Send + Sync + Clone + 'static,
    {
        for r in self.all() {
            if owner.map_or(true, |o| o == r.owner) {
                let f = f.clone();
                let reg = r;
                unsafe {
                    super::xasync::submit(r.queue, 1, Box::new(move |canceled| {
                        if !canceled {
                            f(&reg)
                        }
                    }));
                }
            }
        }
    }
}
