use core::ffi::c_void;

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
    fn DeleteDC(dc: *mut c_void) -> i32;
    fn CreateDIBSection(dc: *mut c_void, bmi: *const u8, usage: u32, bits: *mut *mut u8, section: *mut c_void, offset: u32) -> *mut c_void;
    fn SelectObject(dc: *mut c_void, obj: *mut c_void) -> *mut c_void;
    fn DeleteObject(obj: *mut c_void) -> i32;
    fn CreateFontW(h: i32, w: i32, esc: i32, orient: i32, weight: i32, italic: u32, underline: u32, strike: u32, charset: u32, out_prec: u32, clip_prec: u32, quality: u32, pitch: u32, face: *const u16) -> *mut c_void;
    fn SetTextColor(dc: *mut c_void, c: u32) -> u32;
    fn SetBkMode(dc: *mut c_void, mode: i32) -> i32;
    fn GetTextMetricsW(dc: *mut c_void, tm: *mut [i32; 15]) -> i32;
    fn GetTextExtentPoint32W(dc: *mut c_void, s: *const u16, n: i32, size: *mut [i32; 2]) -> i32;
    fn TextOutW(dc: *mut c_void, x: i32, y: i32, s: *const u16, n: i32) -> i32;
    fn GdiFlush() -> i32;
}

pub const COLS: u32 = 16;
pub const GLYPHS: u32 = 96;
pub const DEGREE: u32 = 95;

pub struct Atlas {
    pub texels: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub pad: u32,
    pub advance: u32,
    pub line: u32,
}

impl Atlas {
    pub fn glyph(&self, c: char) -> Option<u32> {
        match c {
            ' ' => None,
            '°' => Some(DEGREE),
            c if (c as u32) > 32 && (c as u32) < 127 => Some(c as u32 - 32),
            _ => Some('?' as u32 - 32),
        }
    }

    pub fn origin(&self, g: u32) -> (u32, u32) {
        ((g % COLS) * self.cell_w, (g / COLS) * self.cell_h)
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn build(px: i32) -> Result<Atlas, String> {
    unsafe {
        let dc = CreateCompatibleDC(core::ptr::null_mut());
        if dc.is_null() {
            return Err("CreateCompatibleDC failed".into());
        }
        let face = wide("Consolas");
        let font = CreateFontW(-px, 0, 0, 0, 700, 0, 0, 0, 1, 4, 0, 4, 1 | 0x30, face.as_ptr());
        if font.is_null() {
            DeleteDC(dc);
            return Err("CreateFontW(Consolas) failed".into());
        }
        let old_font = SelectObject(dc, font);
        let mut tm = [0i32; 15];
        GetTextMetricsW(dc, &mut tm);
        let m = wide("M");
        let mut sz = [0i32; 2];
        GetTextExtentPoint32W(dc, m.as_ptr(), 1, &mut sz);
        let advance = sz[0].max(tm[5]).max(1) as u32;
        let glyph_h = tm[0].max(1) as u32;
        let radius = ((px as u32) / 14).max(1);
        let pad = radius + 1;
        let cell_w = advance + pad * 2;
        let cell_h = glyph_h + pad * 2;
        let rows = GLYPHS.div_ceil(COLS);
        let width = COLS * cell_w;
        let height = rows * cell_h;
        let mut bmi = [0u8; 44];
        bmi[0..4].copy_from_slice(&40u32.to_le_bytes());
        bmi[4..8].copy_from_slice(&(width as i32).to_le_bytes());
        bmi[8..12].copy_from_slice(&(-(height as i32)).to_le_bytes());
        bmi[12..14].copy_from_slice(&1u16.to_le_bytes());
        bmi[14..16].copy_from_slice(&32u16.to_le_bytes());
        let mut bits: *mut u8 = core::ptr::null_mut();
        let bmp = CreateDIBSection(dc, bmi.as_ptr(), 0, &mut bits, core::ptr::null_mut(), 0);
        if bmp.is_null() || bits.is_null() {
            SelectObject(dc, old_font);
            DeleteObject(font);
            DeleteDC(dc);
            return Err("CreateDIBSection failed".into());
        }
        let old_bmp = SelectObject(dc, bmp);
        SetTextColor(dc, 0x00ff_ffff);
        SetBkMode(dc, 1);
        for g in 0..GLYPHS {
            let ch = if g == DEGREE { '°' } else { char::from_u32(g + 32).unwrap_or('?') };
            let s: Vec<u16> = ch.encode_utf16(&mut [0u16; 2]).to_vec();
            let (x, y) = ((g % COLS) * cell_w + pad, (g / COLS) * cell_h + pad);
            TextOutW(dc, x as i32, y as i32, s.as_ptr(), s.len() as i32);
        }
        GdiFlush();
        let n = (width * height) as usize;
        let src = core::slice::from_raw_parts(bits, n * 4);
        let fill: Vec<u8> = (0..n).map(|i| src[i * 4].max(src[i * 4 + 1]).max(src[i * 4 + 2])).collect();
        SelectObject(dc, old_bmp);
        SelectObject(dc, old_font);
        DeleteObject(bmp);
        DeleteObject(font);
        DeleteDC(dc);
        let r = radius as i32;
        let r2 = (r * r + r) as i32;
        let mut texels = vec![0u32; n];
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let mut o = 0u8;
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy > r2 {
                            continue;
                        }
                        let (sx, sy) = (x + dx, y + dy);
                        if sx < 0 || sy < 0 || sx >= width as i32 || sy >= height as i32 {
                            continue;
                        }
                        o = o.max(fill[(sy as u32 * width + sx as u32) as usize]);
                    }
                }
                let i = (y as u32 * width + x as u32) as usize;
                texels[i] = fill[i] as u32 | (o as u32) << 8;
            }
        }
        Ok(Atlas { texels, width, height, cell_w, cell_h, pad, advance, line: glyph_h })
    }
}
