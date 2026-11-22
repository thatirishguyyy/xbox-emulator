pub(crate) mod addr;
pub(crate) mod bits;
pub(crate) mod dec;
pub(crate) mod enc;
pub(crate) mod run;
mod tables;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod oracle_dec;
#[cfg(test)]
mod oracle_enc;
#[cfg(test)]
mod verify;

use tables::FORMATS;

pub(crate) const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;

#[derive(Clone)]
pub(crate) struct Dcc {
    pub(crate) o: [u8; 0x420],
    pub(crate) s: [[u8; 256]; 3],
    pub(crate) sc: [u8; 64],
    pub(crate) msg: Vec<String>,
    pub(crate) spill: Vec<(u32, u8)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    Fault(&'static str),
    Undef(&'static str),
}

pub(crate) type R<T> = Result<T, Stop>;

impl Dcc {
    pub(crate) fn g(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.o[off..off + 4].try_into().unwrap())
    }

    pub(crate) fn p(&mut self, off: usize, v: u32) {
        self.o[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub(crate) fn bit(&self, off: usize, i: u32) -> bool {
        let o = off + 4 * (i >> 5) as usize;
        o + 4 <= self.o.len() && self.g(o) >> (i & 31) & 1 != 0
    }

    pub(crate) fn put_bit(&mut self, off: usize, i: u32, on: bool) -> R<()> {
        let o = off + 4 * (i >> 5) as usize;
        if o + 4 > self.o.len() {
            return Err(Stop::Undef("bit store outside the codec object"));
        }
        let b = 1u32 << (i & 31);
        let w = self.g(o);
        self.p(o, if on { w | b } else { w & !b });
        Ok(())
    }

    pub(crate) fn mask_bit(&self, k: u32) -> bool {
        self.bit(0x38, k)
    }

    pub(crate) fn new() -> Dcc {
        let mut d = Dcc { o: [0; 0x420], s: [[0; 256]; 3], sc: [0; 64], msg: Vec::new(), spill: Vec::new() };
        d.p(0x00, 1);
        d.p(0x04, 4);
        d.p(0x08, 4);
        d.p(0x0c, 1);
        d.p(0x14, 0x100);
        d.p(0x18, 0x20);
        d.p(0x1c, 0x100);
        d.o[0x3c] = 1;
        d.o[0x3d] = 1;
        d
    }

    pub(crate) fn setup(&mut self, format: i32, tile_mode: i32, block_shift: u32, mode: u32) -> i32 {
        self.p(0x38, 0xf);
        self.p(0x0c, 1);
        if block_shift == 0 {
            self.o[0x20] = 1;
        }
        self.p(0x1c, 0x40u32.wrapping_shl(block_shift));
        self.p(0x18, 0x20);
        self.p(0x28, mode);
        if tile_mode as u32 > 0x1a {
            return E_INVALIDARG;
        }
        let m = match tile_mode {
            0..=7 => 2,
            8 => 5,
            9..=12 => 0,
            13..=18 => 1,
            _ => 4,
        };
        self.p(0x00, m);
        let [edi, num, ebp, shift] = FORMATS[format as usize].map(u32::from);
        let ebx = if ebp.wrapping_sub(1) & !2 == 0 { 8 } else { 4 };
        let pre = if ebx == 4 { 1 } else { 5 };
        let bpe = 1u32 << shift;
        self.p(0x04, bpe);
        self.p(0x0c, (num != 7) as u32);
        self.p(0x14, match bpe {
            1 => 0x40,
            2 => 0x80,
            _ => 0x100,
        });
        let comps = match edi.wrapping_sub(1) {
            0..=0x3e => match edi {
                3 | 5 | 11 | 20 | 21 | 32 | 33 | 57 | 62 => 2,
                6 | 7 | 13 | 16 | 58 => 3,
                8..=10 | 12 | 14 | 17..=19 | 30 | 31 | 34 | 63 => 4,
                15 | 22 | 23 | 42 | 43 => return E_INVALIDARG,
                _ => 1,
            },
            _ => return E_INVALIDARG,
        };
        self.p(0x08, comps);
        let v24 = match edi {
            6 => pre,
            7 => ebx - 2,
            8 => ebx - 1,
            9 => ebx,
            16 => 0xa,
            17 => 0xc,
            18 => 0xb,
            19 => 9,
            _ => {
                self.p(0x10, ebp);
                0
            }
        };
        self.p(0x24, v24);
        self.p(0x34, 0xff);
        0
    }
}
