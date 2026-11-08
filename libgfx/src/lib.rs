//! libgfx: đồ họa 32bpp cho app myos, nói chuyện với devices://fb0 và devices://mode.
#![no_std]

use core::fmt::{self, Write};
pub mod img;
pub mod thread;

// ---------------- syscall thô ----------------
pub mod sys {
    use core::arch::asm;

    #[inline(always)]
    pub unsafe fn sc(n: i64, a: i64, b: i64, c: i64, d: i64, e: i64, f: i64) -> i64 {
        let r: i64;
        asm!("syscall", inlateout("rax") n => r, in("rdi") a, in("rsi") b,
             in("rdx") c, in("r10") d, in("r8") e, in("r9") f,
             lateout("rcx") _, lateout("r11") _, options(nostack));
        r
    }

    pub fn write(fd: i64, b: &[u8]) -> i64 {
        unsafe { sc(1, fd, b.as_ptr() as i64, b.len() as i64, 0, 0, 0) }
    }
    pub fn read(fd: i64, b: &mut [u8]) -> i64 {
        unsafe { sc(0, fd, b.as_mut_ptr() as i64, b.len() as i64, 0, 0, 0) }
    }
    /// `path` phải kết thúc bằng NUL. flags: 0 = O_RDONLY, 1 = O_WRONLY, 2 = O_RDWR.
    pub fn open(path: &[u8], flags: i64) -> i64 {
        unsafe { sc(2, path.as_ptr() as i64, flags, 0, 0, 0, 0) }
    }
    pub fn close(fd: i64) -> i64 {
        unsafe { sc(3, fd, 0, 0, 0, 0, 0) }
    }
    pub fn lseek(fd: i64, off: i64, whence: i64) -> i64 {
        unsafe { sc(8, fd, off, whence, 0, 0, 0) }
    }
    pub fn mmap_anon(len: usize) -> *mut u8 {
        let r = unsafe { sc(9, 0, len as i64, 3, 0x22, -1, 0) };
        if r < 0 && r > -4096 { core::ptr::null_mut() } else { r as *mut u8 }
    }
    pub fn munmap(p: *mut u8, len: usize) -> i64 {
        unsafe { sc(11, p as i64, len as i64, 0, 0, 0, 0) }
    }
    pub fn exit(code: i32) -> ! {
        unsafe { sc(231, code as i64, 0, 0, 0, 0, 0) };
        loop {}
    }
    pub fn log(s: &str) {
        write(1, s.as_bytes());
    }
}

// ---------------- tiện ích ----------------
/// Màu logic 0xRRGGBB.
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

/// Buffer chuỗi cố định để dùng với write!/format.
pub struct StrBuf<const N: usize> {
    b: [u8; N],
    n: usize,
}
impl<const N: usize> StrBuf<N> {
    pub const fn new() -> Self {
        Self { b: [0; N], n: 0 }
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
    pub fn clear(&mut self) {
        self.n = 0;
    }
}
impl<const N: usize> Write for StrBuf<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Error {
    OpenFb,
    Mode,
    Format,
    Alloc,
    Write,
}
impl Error {
    pub fn as_str(&self) -> &'static str {
        match self {
            Error::OpenFb => "cannot open devices://fb0 (kernel chua noi open/write cho devices://?)",
            Error::Mode => "cannot read/write devices://mode",
            Error::Format => "unsupported framebuffer format (need 32bpp)",
            Error::Alloc => "mmap back-buffer failed",
            Error::Write => "write to devices://fb0 failed",
        }
    }
}

// ---------------- font 5x7 (cột, bit0 = hàng trên), ASCII 32..126 ----------------
const FONT: [u8; 475] = [
    0x00,0x00,0x00,0x00,0x00, 0x00,0x00,0x5F,0x00,0x00, 0x00,0x07,0x00,0x07,0x00, 0x14,0x7F,0x14,0x7F,0x14,
    0x24,0x2A,0x7F,0x2A,0x12, 0x23,0x13,0x08,0x64,0x62, 0x36,0x49,0x56,0x20,0x50, 0x00,0x08,0x07,0x03,0x00,
    0x00,0x1C,0x22,0x41,0x00, 0x00,0x41,0x22,0x1C,0x00, 0x2A,0x1C,0x7F,0x1C,0x2A, 0x08,0x08,0x3E,0x08,0x08,
    0x00,0x80,0x70,0x30,0x00, 0x08,0x08,0x08,0x08,0x08, 0x00,0x00,0x60,0x60,0x00, 0x20,0x10,0x08,0x04,0x02,
    0x3E,0x51,0x49,0x45,0x3E, 0x00,0x42,0x7F,0x40,0x00, 0x72,0x49,0x49,0x49,0x46, 0x21,0x41,0x49,0x4D,0x33,
    0x18,0x14,0x12,0x7F,0x10, 0x27,0x45,0x45,0x45,0x39, 0x3C,0x4A,0x49,0x49,0x31, 0x41,0x21,0x11,0x09,0x07,
    0x36,0x49,0x49,0x49,0x36, 0x46,0x49,0x49,0x29,0x1E, 0x00,0x00,0x14,0x00,0x00, 0x00,0x40,0x34,0x00,0x00,
    0x00,0x08,0x14,0x22,0x41, 0x14,0x14,0x14,0x14,0x14, 0x00,0x41,0x22,0x14,0x08, 0x02,0x01,0x59,0x09,0x06,
    0x3E,0x41,0x5D,0x59,0x4E, 0x7C,0x12,0x11,0x12,0x7C, 0x7F,0x49,0x49,0x49,0x36, 0x3E,0x41,0x41,0x41,0x22,
    0x7F,0x41,0x41,0x41,0x3E, 0x7F,0x49,0x49,0x49,0x41, 0x7F,0x09,0x09,0x09,0x01, 0x3E,0x41,0x41,0x51,0x73,
    0x7F,0x08,0x08,0x08,0x7F, 0x00,0x41,0x7F,0x41,0x00, 0x20,0x40,0x41,0x3F,0x01, 0x7F,0x08,0x14,0x22,0x41,
    0x7F,0x40,0x40,0x40,0x40, 0x7F,0x02,0x1C,0x02,0x7F, 0x7F,0x04,0x08,0x10,0x7F, 0x3E,0x41,0x41,0x41,0x3E,
    0x7F,0x09,0x09,0x09,0x06, 0x3E,0x41,0x51,0x21,0x5E, 0x7F,0x09,0x19,0x29,0x46, 0x26,0x49,0x49,0x49,0x32,
    0x03,0x01,0x7F,0x01,0x03, 0x3F,0x40,0x40,0x40,0x3F, 0x1F,0x20,0x40,0x20,0x1F, 0x3F,0x40,0x38,0x40,0x3F,
    0x63,0x14,0x08,0x14,0x63, 0x03,0x04,0x78,0x04,0x03, 0x61,0x59,0x49,0x4D,0x43, 0x00,0x7F,0x41,0x41,0x41,
    0x02,0x04,0x08,0x10,0x20, 0x00,0x41,0x41,0x41,0x7F, 0x04,0x02,0x01,0x02,0x04, 0x40,0x40,0x40,0x40,0x40,
    0x00,0x03,0x07,0x08,0x00, 0x20,0x54,0x54,0x78,0x40, 0x7F,0x28,0x44,0x44,0x38, 0x38,0x44,0x44,0x44,0x28,
    0x38,0x44,0x44,0x28,0x7F, 0x38,0x54,0x54,0x54,0x18, 0x00,0x08,0x7E,0x09,0x02, 0x18,0xA4,0xA4,0x9C,0x78,
    0x7F,0x08,0x04,0x04,0x78, 0x00,0x44,0x7D,0x40,0x00, 0x20,0x40,0x40,0x3D,0x00, 0x7F,0x10,0x28,0x44,0x00,
    0x00,0x41,0x7F,0x40,0x00, 0x7C,0x04,0x78,0x04,0x78, 0x7C,0x08,0x04,0x04,0x78, 0x38,0x44,0x44,0x44,0x38,
    0xFC,0x18,0x24,0x24,0x18, 0x18,0x24,0x24,0x18,0xFC, 0x7C,0x08,0x04,0x04,0x08, 0x48,0x54,0x54,0x54,0x24,
    0x04,0x04,0x3F,0x44,0x24, 0x3C,0x40,0x40,0x20,0x7C, 0x1C,0x20,0x40,0x20,0x1C, 0x3C,0x40,0x30,0x40,0x3C,
    0x44,0x28,0x10,0x28,0x44, 0x4C,0x90,0x90,0x90,0x7C, 0x44,0x64,0x54,0x4C,0x44, 0x00,0x08,0x36,0x41,0x00,
    0x00,0x00,0x77,0x00,0x00, 0x00,0x41,0x36,0x08,0x00, 0x02,0x01,0x02,0x04,0x02,
];

/// Số byte ghi tối đa cho mỗi syscall write khi present.
const CHUNK: usize = 16 * 1024;

struct Mode {
    w: usize,
    h: usize,
    bpp: usize,
    pitch: usize,
    rs: u32,
    gs: u32,
    bs: u32,
}

fn read_mode() -> Result<Mode, Error> {
    let fd = sys::open(b"devices://mode\0", 0);
    if fd < 0 {
        return Err(Error::Mode);
    }
    let mut b = [0u8; 160];
    let n = sys::read(fd, &mut b);
    sys::close(fd);
    if n <= 0 {
        return Err(Error::Mode);
    }
    let s = core::str::from_utf8(&b[..n as usize]).map_err(|_| Error::Mode)?;
    let mut it = s.split_whitespace();
    let first = it.next().ok_or(Error::Mode)?;
    let mut p = first.split('x');
    let w = p.next().and_then(|v| v.parse::<usize>().ok()).ok_or(Error::Mode)?;
    let h = p.next().and_then(|v| v.parse::<usize>().ok()).ok_or(Error::Mode)?;
    let bpp = p.next().and_then(|v| v.parse::<usize>().ok()).ok_or(Error::Mode)?;
    let mut m = Mode { w, h, bpp, pitch: w * 4, rs: 16, gs: 8, bs: 0 };
    for t in it {
        if let Some(v) = t.strip_prefix("pitch=") {
            m.pitch = v.parse::<usize>().map_err(|_| Error::Mode)?;
        } else if let Some(v) = t.strip_prefix("rs=") {
            m.rs = v.parse::<u32>().unwrap_or(16);
        } else if let Some(v) = t.strip_prefix("gs=") {
            m.gs = v.parse::<u32>().unwrap_or(8);
        } else if let Some(v) = t.strip_prefix("bs=") {
            m.bs = v.parse::<u32>().unwrap_or(0);
        }
    }
    Ok(m)
}

pub struct Gfx {
    fd: i64,
    pub w: usize,
    pub h: usize,
    pub pitch: usize,
    stride: usize,
    bb: *mut u32,
    bb_len: usize,
    rs: u32,
    gs: u32,
    bs: u32,
    dy0: usize,
    dy1: usize,
    dx0: usize,
    dx1: usize,
    fb: *mut u8,
    fb_len: usize,
}

impl Gfx {
    pub fn open() -> Result<Gfx, Error> {
        let mut fd = sys::open(b"devices://fb0\0", 2);
        if fd < 0 {
            fd = sys::open(b"/dev/fb0\0", 2);
        }
        if fd < 0 {
            return Err(Error::OpenFb);
        }
        let mut g = Gfx {
            fd,
            w: 0,
            h: 0,
            pitch: 0,
            stride: 0,
            bb: core::ptr::null_mut(),
            bb_len: 0,
            rs: 16,
            gs: 8,
            bs: 0,
            dy0: usize::MAX,
            dy1: 0,
            dx0: usize::MAX,
            dx1: 0,
            fb: core::ptr::null_mut(),
            fb_len: 0,
        };
        g.reload()?; // lỗi thì g bị drop -> tự đóng fd
        Ok(g)
    }

    fn reload(&mut self) -> Result<(), Error> {
        let m = read_mode()?;
        if m.bpp != 32 || m.pitch < m.w * 4 || m.pitch % 4 != 0 {
            return Err(Error::Format);
        }
        if !self.bb.is_null() {
            sys::munmap(self.bb as *mut u8, self.bb_len);
            self.bb = core::ptr::null_mut();
        }
        let len = (m.pitch * m.h + 4095) & !4095;
        let p = sys::mmap_anon(len);
        if p.is_null() {
            return Err(Error::Alloc);
        }
        self.bb = p as *mut u32;
        self.bb_len = len;
        self.w = m.w;
        self.h = m.h;
        self.pitch = m.pitch;
        self.stride = m.pitch / 4;
        self.rs = m.rs;
        self.gs = m.gs;
        self.bs = m.bs;
        self.dy0 = usize::MAX;
        self.dy1 = 0;
        self.dx0 = usize::MAX;
        self.dx1 = 0;
        self.map_fb();
        self.clear(0);
        Ok(())
    }

    /// Đổi độ phân giải qua devices://mode (cần gfx.sys). Back-buffer được cấp lại và xóa đen.
    pub fn set_mode(&mut self, w: u32, h: u32) -> Result<(), Error> {
        let mut s = StrBuf::<32>::new();
        let _ = write!(s, "{}x{}", w, h);
        let fd = sys::open(b"devices://mode\0", 1);
        if fd < 0 {
            return Err(Error::Mode);
        }
        let n = sys::write(fd, s.as_str().as_bytes());
        sys::close(fd);
        if n <= 0 {
            return Err(Error::Mode);
        }
        self.reload()
    }

    #[inline]
    fn px(&self, c: u32) -> u32 {
        if self.rs == 16 && self.gs == 8 && self.bs == 0 {
            c & 0x00FF_FFFF
        } else {
            (((c >> 16) & 255) << self.rs) | (((c >> 8) & 255) << self.gs) | ((c & 255) << self.bs)
        }
    }

    #[inline]
    fn dirty(&mut self, y0: usize, y1: usize) {
        if y0 < self.dy0 {
            self.dy0 = y0;
        }
        if y1 > self.dy1 {
            self.dy1 = y1;
        }
    }

    #[inline]
    fn dirty_x(&mut self, x0: usize, x1: usize) {
        if x0 < self.dx0 {
            self.dx0 = x0;
        }
        if x1 > self.dx1 {
            self.dx1 = x1;
        }
    }

    /// Map devices://fb0 vao user (kernel moi co); loi thi present dung write().
    fn map_fb(&mut self) {
        if !self.fb.is_null() {
            sys::munmap(self.fb, self.fb_len);
            self.fb = core::ptr::null_mut();
        }
        let len = (self.pitch * self.h + 4095) & !4095;
        let r = unsafe { sys::sc(9, 0, len as i64, 3, 1, self.fd, 0) };
        if !(r < 0 && r > -4096) {
            self.fb = r as *mut u8;
            self.fb_len = len;
        }
    }

    pub fn clear(&mut self, c: u32) {
        let (w, h) = (self.w as i32, self.h as i32);
        self.fill_rect(0, 0, w, h, c);
    }

    pub fn pixel(&mut self, x: i32, y: i32, c: u32) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let v = self.px(c);
        unsafe { *self.bb.add(y as usize * self.stride + x as usize) = v };
        self.dirty(y as usize, y as usize);
        self.dirty_x(x as usize, x as usize + 1);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.w as i32);
        let y1 = (y + h).min(self.h as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let v = self.px(c);
        let n = (x1 - x0) as usize;
        for yy in y0..y1 {
            unsafe {
                let row = self.bb.add(yy as usize * self.stride + x0 as usize);
                fast::fill32(row, v, n);
            }
        }
        self.dirty(y0 as usize, (y1 - 1) as usize);
        self.dirty_x(x0 as usize, x1 as usize);
    }

    /// Tô hình chữ nhật trong suốt: alpha 0..=255.
    #[allow(dead_code)]
    fn fill_rect_alpha_slow(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32, a: u8) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.w as i32);
        let y1 = (y + h).min(self.h as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (sr, sg, sb) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
        let a = a as u32;
        let ia = 255 - a;
        let (rs, gs, bs) = (self.rs, self.gs, self.bs);
        for yy in y0..y1 {
            for xx in x0..x1 {
                unsafe {
                    let p = self.bb.add(yy as usize * self.stride + xx as usize);
                    let d = *p;
                    let r = (sr * a + ((d >> rs) & 255) * ia) / 255;
                    let g = (sg * a + ((d >> gs) & 255) * ia) / 255;
                    let b = (sb * a + ((d >> bs) & 255) * ia) / 255;
                    *p = (r << rs) | (g << gs) | (b << bs);
                }
            }
        }
        self.dirty(y0 as usize, (y1 - 1) as usize);
        self.dirty_x(x0 as usize, x1 as usize);
    }

    pub fn hline(&mut self, x: i32, y: i32, len: i32, c: u32) {
        self.fill_rect(x, y, len, 1, c);
    }

    pub fn vline(&mut self, x: i32, y: i32, len: i32, c: u32) {
        self.fill_rect(x, y, 1, len, c);
    }

    /// Viền chữ nhật 1px.
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        self.hline(x, y, w, c);
        self.hline(x, y + h - 1, w, c);
        self.vline(x, y, h, c);
        self.vline(x + w - 1, y, h, c);
    }

    /// Gradient dọc từ c1 (trên) xuống c2 (dưới).
    pub fn gradient_v(&mut self, x: i32, y: i32, w: i32, h: i32, c1: u32, c2: u32) {
        if h <= 0 {
            return;
        }
        for i in 0..h {
            let t = i as u32 * 255 / (h as u32).max(1);
            let mix = |s: u32| -> u32 {
                let a = (c1 >> s) & 255;
                let b = (c2 >> s) & 255;
                (a * (255 - t) + b * t) / 255
            };
            let c = (mix(16) << 16) | (mix(8) << 8) | mix(0);
            self.fill_rect(x, y + i, w, 1, c);
        }
    }

    /// Chép ảnh 0xRRGGBB (sw x sh) vào (x, y).
    #[allow(dead_code)]
    fn blit_slow(&mut self, src: &[u32], sw: usize, sh: usize, x: i32, y: i32) {
        if src.len() < sw * sh {
            return;
        }
        for j in 0..sh {
            let yy = y + j as i32;
            if yy < 0 || yy >= self.h as i32 {
                continue;
            }
            for i in 0..sw {
                let xx = x + i as i32;
                if xx < 0 || xx >= self.w as i32 {
                    continue;
                }
                let v = self.px(src[j * sw + i]);
                unsafe { *self.bb.add(yy as usize * self.stride + xx as usize) = v };
            }
            self.dirty(yy as usize, yy as usize);
            self.dirty_x(x.max(0) as usize, (x + sw as i32).clamp(0, self.w as i32) as usize);
        }
    }

    pub fn text_width_bitmap(&self, s: &str, scale: i32) -> i32 {
        s.len() as i32 * 6 * scale
    }

    /// Vẽ chuỗi ASCII, trả về x sau ký tự cuối.
    pub fn text_bitmap(&mut self, mut x: i32, y: i32, s: &str, c: u32, scale: i32) -> i32 {
        let sc = scale.max(1);
        let v = self.px(c);
        let (w, h) = (self.w as i32, self.h as i32);
        let tx0 = x;
        for &ch in s.as_bytes() {
            if x + 6 * sc > 0 && x < w {
                for col in 0..5usize {
                    let bits = glyph_col(ch, col);
                    for row in 0..8i32 {
                        if (bits >> row) & 1 == 0 {
                            continue;
                        }
                        let px0 = x + col as i32 * sc;
                        let py0 = y + row * sc;
                        let (ax, ay) = (px0.max(0), py0.max(0));
                        let (bx, by) = ((px0 + sc).min(w), (py0 + sc).min(h));
                        for yy in ay..by {
                            for xx in ax..bx {
                                unsafe { *self.bb.add(yy as usize * self.stride + xx as usize) = v };
                            }
                        }
                    }
                }
            }
            x += 6 * sc;
        }
        let (y0, y1) = (y.max(0), (y + 8 * sc).min(h));
        let (x0, x1) = (tx0.max(0), x.min(w));
        if y0 < y1 && x0 < x1 {
            self.dirty(y0 as usize, (y1 - 1) as usize);
            self.dirty_x(x0 as usize, x1 as usize);
        }
        x
    }

    /// Ghi các hàng đã thay đổi ra devices://fb0.
    pub fn present(&mut self) -> Result<(), Error> {
        if self.dy0 > self.dy1 || self.dx0 >= self.dx1 {
            self.dy0 = usize::MAX;
            self.dy1 = 0;
            self.dx0 = usize::MAX;
            self.dx1 = 0;
            return Ok(());
        }
        let y1 = self.dy1.min(self.h.saturating_sub(1));
        let (x0, x1) = (self.dx0.min(self.w), self.dx1.min(self.w));
        if !self.fb.is_null() {
            // ghi thang vao framebuffer da map: khong syscall, chi copy vung doi
            unsafe {
                if x0 == 0 && x1 == self.w {
                    let off = self.dy0 * self.pitch;
                    let n = (y1 - self.dy0 + 1) * self.pitch;
                    fast::copy32(self.fb.add(off) as *mut u32, (self.bb as *const u8).add(off) as *const u32, n / 4);
                } else {
                    for y in self.dy0..=y1 {
                        let off = y * self.pitch + x0 * 4;
                        fast::copy32(self.fb.add(off) as *mut u32, (self.bb as *const u8).add(off) as *const u32, x1 - x0);
                    }
                }
            }
        } else {
            let off = self.dy0 * self.pitch;
            let total = (y1 - self.dy0 + 1) * self.pitch;
            if sys::lseek(self.fd, off as i64, 0) < 0 {
                return Err(Error::Write);
            }
            let base = unsafe { (self.bb as *const u8).add(off) };
            let mut done = 0usize;
            while done < total {
                let n = (total - done).min(CHUNK);
                let r = unsafe { sys::sc(1, self.fd, base.add(done) as i64, n as i64, 0, 0, 0) };
                if r <= 0 {
                    return Err(Error::Write);
                }
                done += r as usize;
            }
        }
        self.dy0 = usize::MAX;
        self.dy1 = 0;
        self.dx0 = usize::MAX;
        self.dx1 = 0;
        Ok(())
    }

    pub fn present_all(&mut self) -> Result<(), Error> {
        self.dy0 = 0;
        self.dy1 = self.h.saturating_sub(1);
        self.dx0 = 0;
        self.dx1 = self.w;
        self.present()
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        if !self.fb.is_null() {
            sys::munmap(self.fb, self.fb_len);
        }
        if !self.bb.is_null() {
            sys::munmap(self.bb as *mut u8, self.bb_len);
        }
        sys::close(self.fd);
    }
}

/// Đăng ký `fn main() -> i32` làm entry point: `libgfx::entry!(main);`
#[macro_export]
macro_rules! entry {
    ($main:ident) => {
        core::arch::global_asm!(
            ".globl _start",
            "_start:",
            "xor ebp, ebp",
            "mov rdi, rsp",
            "and rsp, -16",
            "call __myos_start",
            "ud2",
        );

        #[no_mangle]
        pub extern "C" fn __myos_start(_sp: *const u64) -> ! {
            $main();
            $crate::sys::exit(0)
        }
    };
}

// ================= mouse =================
pub mod mouse {
    use super::sys;
    const UNOPEN: i64 = i64::MIN;
    static mut FD: i64 = UNOPEN;
    static mut X: i32 = 0;
    static mut Y: i32 = 0;
    static mut B: u32 = 0;

    /// Đọc devices://mouse ("x y buttons"). true nếu cập nhật được.
    pub fn poll() -> bool {
        unsafe {
            if FD == UNOPEN {
                FD = sys::open(b"devices://mouse\0", 0);
            }
            if FD < 0 || sys::lseek(FD, 0, 0) < 0 {
                return false;
            }
            let mut b = [0u8; 48];
            let n = sys::read(FD, &mut b);
            if n <= 0 {
                return false;
            }
            let s = match core::str::from_utf8(&b[..n as usize]) {
                Ok(s) => s,
                Err(_) => return false,
            };
            let mut it = s.split_whitespace();
            let x = it.next().and_then(|v| v.parse::<i32>().ok());
            let y = it.next().and_then(|v| v.parse::<i32>().ok());
            let bt = it.next().and_then(|v| v.parse::<u32>().ok());
            match (x, y, bt) {
                (Some(x), Some(y), Some(bt)) => {
                    X = x;
                    Y = y;
                    B = bt;
                    true
                }
                _ => false,
            }
        }
    }
    pub fn x() -> i32 { unsafe { X } }
    pub fn y() -> i32 { unsafe { Y } }
    pub fn buttons() -> u32 { unsafe { B } }
}

const ARROW: [&str; 15] = [
    "B           ",
    "BB          ",
    "BWB         ",
    "BWWB        ",
    "BWWWB       ",
    "BWWWWB      ",
    "BWWWWWB     ",
    "BWWWWWWB    ",
    "BWWWWWWWB   ",
    "BWWWWWBBBB  ",
    "BWWBWWB     ",
    "BWB BWWB    ",
    "BB  BWWB    ",
    "B    BWWB   ",
    "     BBB    ",
];

impl Gfx {
    /// Cập nhật vị trí chuột từ kernel. Gọi mỗi frame.
    pub fn poll_mouse(&mut self) -> bool { mouse::poll() }
    pub fn mouse_x(&self) -> i32 { mouse::x() }
    pub fn mouse_y(&self) -> i32 { mouse::y() }
    /// bit0 = trái, bit1 = phải, bit2 = giữa.
    pub fn mouse_buttons(&self) -> u32 { mouse::buttons() }

    /// Vẽ con trỏ mũi tên tại vị trí chuột. Vẽ SAU CÙNG của frame (không tự khôi phục nền).
    pub fn draw_cursor(&mut self) {
        if self.cursor_sprite_draw(false) { return; }
        let (mx, my) = (mouse::x(), mouse::y());
        for (j, row) in ARROW.iter().enumerate() {
            for (i, ch) in row.bytes().enumerate() {
                match ch {
                    b'B' => self.pixel(mx + i as i32, my + j as i32, 0x000000),
                    b'W' => self.pixel(mx + i as i32, my + j as i32, 0xFFFFFF),
                    _ => {}
                }
            }
        }
    }
}

// ================= BMP codec =================
pub mod bmp {
    use super::{rgb, sys, Gfx};

    #[derive(Clone, Copy, Debug)]
    pub enum BmpError {
        Open,
        Read,
        Alloc,
        Format,
        Unsupported,
        TooBig,
    }
    impl BmpError {
        pub fn as_str(&self) -> &'static str {
            match self {
                BmpError::Open => "cannot open bmp file",
                BmpError::Read => "cannot read bmp file",
                BmpError::Alloc => "mmap failed",
                BmpError::Format => "bad bmp data",
                BmpError::Unsupported => "unsupported bmp (need 8/24/32bpp uncompressed)",
                BmpError::TooBig => "bmp too big / buffer too small",
            }
        }
    }

    /// Ảnh 0xRRGGBB, bộ nhớ cấp bằng mmap, tự giải phóng khi drop.
    pub struct Image {
        pub w: usize,
        pub h: usize,
        px: *mut u32,
        len: usize,
    }
    impl Image {
        pub fn new(w: usize, h: usize) -> Option<Image> {
            if w == 0 || h == 0 { return None; }
            let len = (w * h * 4 + 4095) & !4095;
            let p = sys::mmap_anon(len) as *mut u32;
            if p.is_null() { None } else { Some(Image { w, h, px: p, len }) }
        }
        pub fn pixels(&self) -> &[u32] {
            unsafe { core::slice::from_raw_parts(self.px, self.w * self.h) }
        }
        pub fn pixels_mut(&mut self) -> &mut [u32] {
            unsafe { core::slice::from_raw_parts_mut(self.px, self.w * self.h) }
        }
    }
    impl Drop for Image {
        fn drop(&mut self) {
            if !self.px.is_null() {
                sys::munmap(self.px as *mut u8, self.len);
            }
        }
    }

    fn u16le(d: &[u8], o: usize) -> u16 { u16::from_le_bytes([d[o], d[o + 1]]) }
    fn u32le(d: &[u8], o: usize) -> u32 { u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) }

    /// Giải mã BMP trong bộ nhớ.
    pub fn decode(d: &[u8]) -> Result<Image, BmpError> {
        if d.len() < 54 || &d[0..2] != b"BM" {
            return Err(BmpError::Format);
        }
        let off = u32le(d, 10) as usize;
        let dib = u32le(d, 14) as usize;
        if dib < 40 {
            return Err(BmpError::Unsupported);
        }
        let wi = u32le(d, 18) as i32;
        let hi = u32le(d, 22) as i32;
        let bpp = u16le(d, 28) as usize;
        let comp = u32le(d, 30);
        if wi <= 0 || hi == 0 {
            return Err(BmpError::Format);
        }
        if !matches!(bpp, 8 | 24 | 32) || !(comp == 0 || (comp == 3 && bpp == 32)) {
            return Err(BmpError::Unsupported);
        }
        let w = wi as usize;
        let h = hi.unsigned_abs() as usize;
        let top_down = hi < 0;
        if w > 8192 || h > 8192 {
            return Err(BmpError::TooBig);
        }
        let stride = (w * bpp + 31) / 32 * 4;
        if off.checked_add(stride * h).map_or(true, |e| e > d.len()) {
            return Err(BmpError::Format);
        }
        let mut pal = [0u32; 256];
        if bpp == 8 {
            let mut n = u32le(d, 46) as usize;
            if n == 0 || n > 256 {
                n = 256;
            }
            let po = 14 + dib;
            if po + n * 4 > d.len() {
                return Err(BmpError::Format);
            }
            for i in 0..n {
                let e = po + i * 4;
                pal[i] = rgb(d[e + 2], d[e + 1], d[e]);
            }
        }
        let len = (w * h * 4 + 4095) & !4095;
        let p = sys::mmap_anon(len) as *mut u32;
        if p.is_null() {
            return Err(BmpError::Alloc);
        }
        for y in 0..h {
            let sy = if top_down { y } else { h - 1 - y };
            let row = &d[off + sy * stride..off + sy * stride + stride];
            for x in 0..w {
                let c = match bpp {
                    8 => pal[row[x] as usize],
                    24 => rgb(row[x * 3 + 2], row[x * 3 + 1], row[x * 3]),
                    _ => rgb(row[x * 4 + 2], row[x * 4 + 1], row[x * 4]),
                };
                unsafe { *p.add(y * w + x) = c };
            }
        }
        Ok(Image { w, h, px: p, len })
    }

    /// Đọc file BMP (path kết thúc NUL, vd b"/logo.bmp\0").
    pub fn load(path: &[u8]) -> Result<Image, BmpError> {
        let fd = sys::open(path, 0);
        if fd < 0 {
            return Err(BmpError::Open);
        }
        let size = sys::lseek(fd, 0, 2);
        if size <= 0 || size > (64 << 20) {
            sys::close(fd);
            return Err(BmpError::TooBig);
        }
        sys::lseek(fd, 0, 0);
        let size = size as usize;
        let len = (size + 4095) & !4095;
        let p = sys::mmap_anon(len);
        if p.is_null() {
            sys::close(fd);
            return Err(BmpError::Alloc);
        }
        let buf = unsafe { core::slice::from_raw_parts_mut(p, size) };
        let mut done = 0usize;
        while done < size {
            let r = sys::read(fd, &mut buf[done..]);
            if r <= 0 {
                break;
            }
            done += r as usize;
        }
        sys::close(fd);
        let res = if done == size { decode(buf) } else { Err(BmpError::Read) };
        sys::munmap(p, len);
        res
    }

    /// Số byte cần cho buffer encode.
    pub fn size_for(w: usize, h: usize) -> usize {
        54 + ((w * 3 + 3) & !3) * h
    }

    fn write_bmp<F: Fn(usize, usize) -> u32>(w: usize, h: usize, out: &mut [u8], get: F) -> Result<usize, BmpError> {
        if w == 0 || h == 0 {
            return Err(BmpError::Format);
        }
        let stride = (w * 3 + 3) & !3;
        let total = 54 + stride * h;
        if out.len() < total {
            return Err(BmpError::TooBig);
        }
        out[..54].fill(0);
        out[0] = b'B';
        out[1] = b'M';
        out[2..6].copy_from_slice(&(total as u32).to_le_bytes());
        out[10..14].copy_from_slice(&54u32.to_le_bytes());
        out[14..18].copy_from_slice(&40u32.to_le_bytes());
        out[18..22].copy_from_slice(&(w as u32).to_le_bytes());
        out[22..26].copy_from_slice(&(h as u32).to_le_bytes());
        out[26..28].copy_from_slice(&1u16.to_le_bytes());
        out[28..30].copy_from_slice(&24u16.to_le_bytes());
        out[34..38].copy_from_slice(&((stride * h) as u32).to_le_bytes());
        for y in 0..h {
            let sy = h - 1 - y; // BMP lưu từ dưới lên
            let row = &mut out[54 + y * stride..54 + (y + 1) * stride];
            for x in 0..w {
                let c = get(x, sy);
                row[x * 3] = c as u8;
                row[x * 3 + 1] = (c >> 8) as u8;
                row[x * 3 + 2] = (c >> 16) as u8;
            }
            for i in w * 3..stride {
                row[i] = 0;
            }
        }
        Ok(total)
    }

    /// Mã hoá ảnh 0xRRGGBB (w x h) thành BMP 24bpp vào `out`, trả số byte đã ghi.
    pub fn encode(src: &[u32], w: usize, h: usize, out: &mut [u8]) -> Result<usize, BmpError> {
        if src.len() < w * h {
            return Err(BmpError::Format);
        }
        write_bmp(w, h, out, |x, y| src[y * w + x])
    }

    impl Gfx {
        pub fn blit_image(&mut self, img: &Image, x: i32, y: i32) {
            self.blit(img.pixels(), img.w, img.h, x, y);
        }

        /// Chụp back-buffer thành BMP. `out` cần >= bmp::size_for(w, h).
        pub fn screenshot_bmp(&self, out: &mut [u8]) -> Result<usize, BmpError> {
            let (rs, gs, bs) = (self.rs, self.gs, self.bs);
            let (stride, bb) = (self.stride, self.bb);
            write_bmp(self.w, self.h, out, |x, y| unsafe {
                let d = *bb.add(y * stride + x);
                (((d >> rs) & 255) << 16) | (((d >> gs) & 255) << 8) | ((d >> bs) & 255)
            })
        }
    }
}

/// Cột glyph font 5x7 (bit0 = hàng trên) cho ASCII.
pub fn glyph_col(ch: u8, col: usize) -> u8 {
    let ch = if (32..=126).contains(&ch) { ch } else { b'?' };
    FONT[(ch - 32) as usize * 5 + col.min(4)]
}

impl Gfx {
    /// Chep vung [x0,x1) x [y0,y1) tu buffer 0xRRGGBB (stride tinh theo pixel) vao back-buffer.
    pub fn copy_from(&mut self, src: *const u32, sstride: usize, x0: usize, y0: usize, x1: usize, y1: usize) {
        let x1 = x1.min(self.w);
        let y1 = y1.min(self.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let n = x1 - x0;
        let ident = self.rs == 16 && self.gs == 8 && self.bs == 0;
        for y in y0..y1 {
            unsafe {
                let s = src.add(y * sstride + x0);
                let d = self.bb.add(y * self.stride + x0);
                if ident {
                    fast::copy32(d, s, n);
                } else {
                    for i in 0..n {
                        *d.add(i) = self.px(*s.add(i));
                    }
                }
            }
        }
        self.dirty(y0, y1 - 1);
        self.dirty_x(x0, x1);
    }
}

/// Fill/copy 32-bit bằng rep stosd/movsd (target không có SSE nên vòng lặp thường rất chậm).
pub mod fast {
    use core::arch::asm;

    #[inline(always)]
    pub unsafe fn fill32(d: *mut u32, v: u32, n: usize) {
        if n < 8 {
            for i in 0..n {
                *d.add(i) = v;
            }
            return;
        }
        asm!("rep stosd", inout("rcx") n => _, inout("rdi") d => _, in("eax") v,
             options(nostack, preserves_flags));
    }

    /// Chép tiến n pixel, vùng không chồng nhau.
    #[inline(always)]
    pub unsafe fn copy32(d: *mut u32, s: *const u32, n: usize) {
        if n < 8 {
            for i in 0..n {
                *d.add(i) = *s.add(i);
            }
            return;
        }
        asm!("rep movsd", inout("rcx") n => _, inout("rdi") d => _, inout("rsi") s => _,
             options(nostack, preserves_flags));
    }
}

impl Gfx {
    /// Tô hình chữ nhật trong suốt, alpha 0..=255. Trộn R+B cùng lúc, không chia.
    pub fn fill_rect_alpha(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32, a: u8) {
        if !(self.rs == 16 && self.gs == 8 && self.bs == 0) {
            return self.fill_rect_alpha_slow(x, y, w, h, c, a);
        }
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.w as i32);
        let y1 = (y + h).min(self.h as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let a = a as u32;
        let ia = 255 - a;
        let s_rb = (c & 0x00FF_00FF) * a;
        let s_g = (c & 0x0000_FF00) * a;
        let n = (x1 - x0) as usize;
        for yy in y0..y1 {
            unsafe {
                let row = self.bb.add(yy as usize * self.stride + x0 as usize);
                for i in 0..n {
                    let d = *row.add(i);
                    let rb = (d & 0x00FF_00FF) * ia + s_rb;
                    let rb = ((rb + 0x0080_0080 + ((rb >> 8) & 0x00FF_00FF)) >> 8) & 0x00FF_00FF;
                    let g = (d & 0x0000_FF00) * ia + s_g;
                    let g = ((g + 0x0000_8000 + ((g >> 8) & 0x0000_FF00)) >> 8) & 0x0000_FF00;
                    *row.add(i) = rb | g;
                }
            }
        }
        self.dirty(y0 as usize, (y1 - 1) as usize);
        self.dirty_x(x0 as usize, x1 as usize);
    }

    /// Chép ảnh 0xRRGGBB (sw x sh) vào (x, y), nguyên hàng một lần.
    pub fn blit(&mut self, src: &[u32], sw: usize, sh: usize, x: i32, y: i32) {
        if sw == 0 || sh == 0 || src.len() < sw * sh {
            return;
        }
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + sw as i32).min(self.w as i32);
        let y1 = (y + sh as i32).min(self.h as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let n = (x1 - x0) as usize;
        let ident = self.rs == 16 && self.gs == 8 && self.bs == 0;
        for yy in y0..y1 {
            unsafe {
                let s = src.as_ptr().add((yy - y) as usize * sw + (x0 - x) as usize);
                let d = self.bb.add(yy as usize * self.stride + x0 as usize);
                if ident {
                    fast::copy32(d, s, n);
                } else {
                    for i in 0..n {
                        *d.add(i) = self.px(*s.add(i));
                    }
                }
            }
        }
        self.dirty(y0 as usize, (y1 - 1) as usize);
        self.dirty_x(x0 as usize, x1 as usize);
    }
}

impl Gfx {
    /// true nếu có thể ghi thẳng vào framebuffer đã map (định dạng RGB chuẩn).
    pub fn direct_fb(&self) -> bool {
        !self.fb.is_null() && self.rs == 16 && self.gs == 8 && self.bs == 0
    }

    /// Chép vùng [x0,x1) x [y0,y1) từ scene (stride tính theo pixel) thẳng ra fb0.
    pub fn blit_scene_fb(&mut self, src: *const u32, sstride: usize, x0: usize, y0: usize, x1: usize, y1: usize) {
        let x1 = x1.min(self.w);
        let y1 = y1.min(self.h);
        if self.fb.is_null() || x0 >= x1 || y0 >= y1 {
            return;
        }
        for y in y0..y1 {
            unsafe {
                let d = self.fb.add(y * self.pitch + x0 * 4) as *mut u32;
                fast::copy32(d, src.add(y * sstride + x0), x1 - x0);
            }
        }
    }

    /// Vẽ con trỏ thẳng lên fb0 (không đụng back-buffer).
    pub fn cursor_fb(&mut self) {
        if self.cursor_sprite_draw(true) { return; }
        if self.fb.is_null() {
            return;
        }
        let (mx, my) = (mouse::x(), mouse::y());
        for (j, row) in ARROW.iter().enumerate() {
            let y = my + j as i32;
            if y < 0 || y >= self.h as i32 {
                continue;
            }
            for (i, ch) in row.bytes().enumerate() {
                let x = mx + i as i32;
                if x < 0 || x >= self.w as i32 {
                    continue;
                }
                let v = match ch {
                    b'B' => 0x000000u32,
                    b'W' => 0xFFFFFF,
                    _ => continue,
                };
                unsafe { *(self.fb.add(y as usize * self.pitch + x as usize * 4) as *mut u32) = v };
            }
        }
    }
}

impl Gfx {
    /// Ve bitmap coverage 8-bit (sw x sh) tai (x, y) voi mau c = 0xRRGGBB (anti-alias, dung cho chu).
    pub fn blit_alpha8(&mut self, cov: &[u8], sw: usize, sh: usize, x: i32, y: i32, c: u32) {
        if sw == 0 || sh == 0 || cov.len() < sw * sh {
            return;
        }
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + sw as i32).min(self.w as i32);
        let y1 = (y + sh as i32).min(self.h as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (sr, sg, sb) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
        let (rs, gs, bs) = (self.rs, self.gs, self.bs);
        let full = self.px(c);
        for yy in y0..y1 {
            let crow = &cov[(yy - y) as usize * sw..];
            for xx in x0..x1 {
                let a = crow[(xx - x) as usize] as u32;
                if a == 0 {
                    continue;
                }
                unsafe {
                    let p = self.bb.add(yy as usize * self.stride + xx as usize);
                    if a == 255 {
                        *p = full;
                    } else {
                        let ia = 255 - a;
                        let d = *p;
                        let r = (sr * a + ((d >> rs) & 255) * ia + 127) / 255;
                        let g = (sg * a + ((d >> gs) & 255) * ia + 127) / 255;
                        let b = (sb * a + ((d >> bs) & 255) * ia + 127) / 255;
                        *p = (r << rs) | (g << gs) | (b << bs);
                    }
                }
            }
        }
        self.dirty(y0 as usize, (y1 - 1) as usize);
        self.dirty_x(x0 as usize, x1 as usize);
    }
}

// ================= chu TTF (libttf) =================
/// Font mac dinh nam trong initrd.
pub const FONT_PATH: &[u8] = b"/resources/NotoSansJP.ttf\0";

static mut TTF: Option<libttf::Font> = None;
static mut TTF_STATE: u8 = 0; // 0 = chua thu nap, 1 = san sang, 2 = loi/tat -> dung font bitmap

fn ttf() -> Option<&'static mut libttf::Font> {
    unsafe {
        if TTF_STATE == 0 {
            match libttf::Font::load(FONT_PATH) {
                Ok(f) => {
                    *core::ptr::addr_of_mut!(TTF) = Some(f);
                    TTF_STATE = 1;
                }
                Err(e) => {
                    sys::log("libgfx: khong nap duoc font ttf, dung font bitmap: ");
                    sys::log(e.as_str());
                    sys::log("\n");
                    TTF_STATE = 2;
                }
            }
        }
        if TTF_STATE == 1 { (*core::ptr::addr_of_mut!(TTF)).as_mut() } else { None }
    }
}

/// Bat/tat font TTF (tat -> text() ve bang font bitmap 5x7 cu).
pub fn set_ttf(on: bool) {
    unsafe {
        TTF_STATE = if !on { 2 } else if (*core::ptr::addr_of_mut!(TTF)).is_some() { 1 } else { 0 };
    }
}

/// Doi `scale` (nhan font bitmap) sang co chu TTF theo pixel: 1 -> 10, 2 -> 16, 3 -> 22 ...
pub const fn ttf_px(scale: i32) -> u32 {
    (6 * (if scale < 1 { 1 } else { scale }) + 4) as u32
}

impl libttf::Canvas for Gfx {
    fn blit_alpha8(&mut self, cov: &[u8], sw: usize, sh: usize, x: i32, y: i32, c: u32) {
        Gfx::blit_alpha8(self, cov, sw, sh, x, y, c)
    }
}

impl Gfx {
    /// Do rong chuoi (pixel). `scale` giong font bitmap, quy doi bang `ttf_px`.
    pub fn text_width(&self, s: &str, scale: i32) -> i32 {
        match ttf() {
            Some(f) => f.text_width(s, ttf_px(scale)),
            None => self.text_width_bitmap(s, scale),
        }
    }

    /// Ve chuoi UTF-8 bang font TTF, tra ve x sau ky tu cuoi.
    /// Vi tri `y` giu nghia cu: duong co so nam o y + 7*scale (nhu font bitmap).
    pub fn text(&mut self, x: i32, y: i32, s: &str, c: u32, scale: i32) -> i32 {
        let sc = if scale < 1 { 1 } else { scale };
        match ttf() {
            Some(f) => {
                let px = ttf_px(sc);
                let top = y + 7 * sc - f.ascent(px);
                f.draw_text(self, x, top, s, c, px)
            }
            None => self.text_bitmap(x, y, s, c, scale),
        }
    }

    /// Nhu `text` nhung chon co chu theo pixel; `y` la goc tren cua dong chu.
    pub fn text_px(&mut self, x: i32, y: i32, s: &str, c: u32, px: u32) -> i32 {
        match ttf() {
            Some(f) => f.draw_text(self, x, y, s, c, px),
            None => self.text_bitmap(x, y, s, c, ((px / 8).max(1)) as i32),
        }
    }

    pub fn text_width_px(&self, s: &str, px: u32) -> i32 {
        match ttf() {
            Some(f) => f.text_width(s, px),
            None => self.text_width_bitmap(s, ((px / 8).max(1)) as i32),
        }
    }

    /// Chieu cao 1 dong (pixel) ung voi co chu `px`.
    pub fn line_height_px(&self, px: u32) -> i32 {
        match ttf() {
            Some(f) => f.line_height(px),
            None => ((px / 8).max(1) * 9) as i32,
        }
    }
}

// ---- API ve chu TTF len canvas bat ky (swm, libswmapi dung) ----
pub use libttf::Canvas;

/// Ve chuoi bang TTF len `c`. None neu khong nap duoc font (nguoi goi tu fallback bitmap).
pub fn ttf_text<C: libttf::Canvas>(c: &mut C, x: i32, y: i32, s: &str, color: u32, scale: i32) -> Option<i32> {
    let sc = if scale < 1 { 1 } else { scale };
    let f = ttf()?;
    let px = ttf_px(sc);
    let top = y + 7 * sc - f.ascent(px);
    Some(f.draw_text(c, x, top, s, color, px))
}

pub fn ttf_text_width(s: &str, scale: i32) -> Option<i32> {
    let f = ttf()?;
    Some(f.text_width(s, ttf_px(if scale < 1 { 1 } else { scale })))
}

// ================= cursor sprite (BMP 32bpp co alpha) =================
pub const CURSOR_PATH: &[u8] = b"/resource/cursor.bmp\0"; // rd0://resource/cursor.bmp
pub const CURSOR_PATH2: &[u8] = b"/resources/cursor.bmp\0"; // du phong
static mut CSPR: *mut u32 = core::ptr::null_mut(); // ARGB
static mut CSW: usize = 0;
static mut CSH: usize = 0;
static mut CSTATE: u8 = 0; // 0 chua thu, 1 ok, 2 loi -> dung ARROW

fn clog(msg: &[u8]) {
    sys::write(2, b"[cursor] ");
    sys::write(2, msg);
    sys::write(2, b"\n");
}

fn cursor_load() -> bool {
    unsafe {
        if CSTATE != 0 {
            return CSTATE == 1;
        }
        CSTATE = 2;
        let mut fd = sys::open(CURSOR_PATH, 0);
        if fd < 0 {
            fd = sys::open(CURSOR_PATH2, 0);
        }
        if fd < 0 {
            clog(b"khong mo duoc rd0://resource/cursor.bmp (thieu trong tar?) -> dung mui ten mac dinh");
            return false;
        }
        let size = sys::lseek(fd, 0, 2);
        if size < 54 || size > (4 << 20) {
            sys::close(fd);
            clog(b"cursor.bmp kich thuoc file khong hop le");
            return false;
        }
        sys::lseek(fd, 0, 0);
        let size = size as usize;
        let flen = (size + 4095) & !4095;
        let p = sys::mmap_anon(flen);
        if p.is_null() {
            sys::close(fd);
            clog(b"het bo nho khi doc cursor.bmp");
            return false;
        }
        let d = core::slice::from_raw_parts_mut(p, size);
        let mut done = 0usize;
        while done < size {
            let r = sys::read(fd, &mut d[done..]);
            if r <= 0 {
                break;
            }
            done += r as usize;
        }
        sys::close(fd);
        let u32le = |o: usize| u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        if done != size || &d[0..2] != b"BM" {
            sys::munmap(p, flen);
            clog(b"cursor.bmp doc loi hoac khong phai BMP");
            return false;
        }
        let bpp = u16::from_le_bytes([d[28], d[29]]);
        let comp = u32le(30);
        if (bpp != 32 && bpp != 24) || (comp != 0 && comp != 3) {
            sys::munmap(p, flen);
            clog(b"cursor.bmp phai la 24/32 bpp, khong nen");
            return false;
        }
        let off = u32le(10) as usize;
        let wi = u32le(18) as i32;
        let hi = u32le(22) as i32;
        let (w, h) = (wi.max(0) as usize, hi.unsigned_abs() as usize);
        let bytes = (bpp / 8) as usize;
        let stride = (w * bytes + 3) & !3;
        if w == 0 || h == 0 || w > 512 || h > 512 || off + stride * h > size {
            sys::munmap(p, flen);
            clog(b"cursor.bmp kich thuoc sai (toi da 512x512)");
            return false;
        }
        let slen = (w * h * 4 + 4095) & !4095;
        let sp = sys::mmap_anon(slen) as *mut u32;
        if sp.is_null() {
            sys::munmap(p, flen);
            return false;
        }
        // BMP 32bpp co alpha = 0 toan bo: coi la khong trong suot, chi mau hong (255,0,255) trong suot.
        let mut any_a = false;
        if bpp == 32 {
            'scan: for y in 0..h {
                for x in 0..w {
                    if d[off + y * stride + x * 4 + 3] != 0 {
                        any_a = true;
                        break 'scan;
                    }
                }
            }
        }
        for y in 0..h {
            let sy = if hi < 0 { y } else { h - 1 - y };
            for x in 0..w {
                let o = off + sy * stride + x * bytes;
                let (b, g, r) = (d[o] as u32, d[o + 1] as u32, d[o + 2] as u32);
                let a = if bpp == 32 && any_a {
                    d[o + 3] as u32
                } else if r == 255 && g == 0 && b == 255 {
                    0
                } else {
                    255
                };
                *sp.add(y * w + x) = (a << 24) | (r << 16) | (g << 8) | b;
            }
        }
        sys::munmap(p, flen);
        // cursor_flat(sp, w, h); // BO: lam phang mau thanh xam
        cursor_finish(sp, w, h, slen)
    }
}

impl Gfx {
    /// Kich thuoc cursor hien tai (sprite hoac mui ten mac dinh).
    pub fn cursor_size(&self) -> (i32, i32) {
        if cursor_load() { unsafe { (CSW as i32, CSH as i32) } } else { (12, 15) }
    }

    /// Ve sprite co alpha. true neu da ve (false -> nguoi goi ve ARROW).
    fn cursor_sprite_draw(&mut self, to_fb: bool) -> bool {
        if !cursor_load() || (to_fb && self.fb.is_null()) {
            return false;
        }
        unsafe {
            let (cw, ch) = (CSW as i32, CSH as i32);
            let (mx, my) = (mouse::x(), mouse::y());
            let (rs, gs, bs) = if to_fb { (16u32, 8u32, 0u32) } else { (self.rs, self.gs, self.bs) };
            for j in 0..ch {
                let y = my + j;
                if y < 0 || y >= self.h as i32 {
                    continue;
                }
                for i in 0..cw {
                    let x = mx + i;
                    if x < 0 || x >= self.w as i32 {
                        continue;
                    }
                    let s = *CSPR.add((j * cw + i) as usize);
                    let a = s >> 24;
                    if a == 0 {
                        continue;
                    }
                    let dp: *mut u32 = if to_fb {
                        self.fb.add(y as usize * self.pitch + x as usize * 4) as *mut u32
                    } else {
                        self.bb.add(y as usize * self.stride + x as usize)
                    };
                    if a == 255 {
                        *dp = (((s >> 16) & 255) << rs) | (((s >> 8) & 255) << gs) | ((s & 255) << bs);
                    } else {
                        let ia = 255 - a;
                        let d = *dp;
                        let r = (((s >> 16) & 255) * a + ((d >> rs) & 255) * ia + 127) / 255;
                        let g = (((s >> 8) & 255) * a + ((d >> gs) & 255) * ia + 127) / 255;
                        let b = ((s & 255) * a + ((d >> bs) & 255) * ia + 127) / 255;
                        *dp = (r << rs) | (g << gs) | (b << bs);
                    }
                }
            }
            if !to_fb {
                let (y0, y1) = (my.max(0) as usize, ((my + ch).min(self.h as i32) - 1).max(0) as usize);
                self.dirty(y0, y1);
                self.dirty_x(mx.max(0) as usize, ((mx + cw).min(self.w as i32)).max(0) as usize);
            }
        }
        true
    }
}

/// Chieu cao cursor sau khi thu nho (pixel).
pub const CURSOR_PX: usize = 24;

/// Cat vien trong suot, thu nho (box filter, tron theo alpha), luu vao CSPR/CSW/CSH.
unsafe fn cursor_finish(sp: *mut u32, w: usize, h: usize, slen: usize) -> bool {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if (*sp.add(y * w + x)) >> 24 >= 16 {
                if x < x0 { x0 = x; }
                if y < y0 { y0 = y; }
                if x > x1 { x1 = x; }
                if y > y1 { y1 = y; }
            }
        }
    }
    if x0 > x1 || y0 > y1 {
        x0 = 0; y0 = 0; x1 = w - 1; y1 = h - 1;
    }
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let (nw, nh) = if ch > CURSOR_PX { (((cw * CURSOR_PX) / ch).max(1), CURSOR_PX) } else { (cw, ch) };
    let dlen = (nw * nh * 4 + 4095) & !4095;
    let dp = sys::mmap_anon(dlen) as *mut u32;
    if dp.is_null() {
        sys::munmap(sp as *mut u8, slen);
        return false;
    }
    for dy in 0..nh {
        let sy0 = y0 + dy * ch / nh;
        let sy1 = (y0 + (dy + 1) * ch / nh).max(sy0 + 1);
        for dx in 0..nw {
            let sx0 = x0 + dx * cw / nw;
            let sx1 = (x0 + (dx + 1) * cw / nw).max(sx0 + 1);
            let (mut sa, mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for yy in sy0..sy1 {
                for xx in sx0..sx1 {
                    let s = *sp.add(yy * w + xx);
                    let a = s >> 24;
                    sa += a;
                    sr += ((s >> 16) & 255) * a;
                    sg += ((s >> 8) & 255) * a;
                    sb += (s & 255) * a;
                    n += 1;
                }
            }
            let v = if sa == 0 || n == 0 {
                0
            } else {
                ((sa / n) << 24) | ((sr / sa) << 16) | ((sg / sa) << 8) | (sb / sa)
            };
            *dp.add(dy * nw + dx) = v;
        }
    }
    sys::munmap(sp as *mut u8, slen);
    cursor_harden(dp, nw * nh);
    CSPR = dp;
    CSW = nw;
    CSH = nh;
    CSTATE = 1;
    true
}

/// Pixel sang hon nguong nay (luma 0..255) bi coi la vien/quang sang va xoa.
pub const CURSOR_LUMA: u32 = 150;

/// Xoa vien sang + pixel alpha qua thap truoc khi cat/thu nho.
unsafe fn cursor_clean(sp: *mut u32, w: usize, h: usize) {
    for i in 0..w * h {
        let s = *sp.add(i);
        let a = s >> 24;
        let (r, g, b) = ((s >> 16) & 255, (s >> 8) & 255, s & 255);
        let luma = (r * 77 + g * 150 + b * 29) >> 8;
        if a < 32 || luma > CURSOR_LUMA {
            *sp.add(i) = 0;
        }
    }
}

/// Lam cung alpha sau khi thu nho: bo quang mo, giu rim muot.
unsafe fn cursor_harden(p: *mut u32, n: usize) {
    for i in 0..n {
        let s = *p.add(i);
        let a = s >> 24;
        if a < 48 {
            *p.add(i) = 0;
        } else if a > 208 {
            *p.add(i) = s | 0xFF00_0000;
        }
    }
}

/// Cursor mot mau: gan mau trung binh cua pixel dac (a >= 250) cho moi pixel co alpha.
/// Loai bo hoan toan mau rim (trang) lot vao khi blend. Chi giu alpha.
#[allow(dead_code)]
unsafe fn cursor_flat(sp: *mut u32, w: usize, h: usize) {
    let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
    for i in 0..w * h {
        let s = *sp.add(i);
        if s >> 24 >= 250 {
            sr += (s >> 16) & 255;
            sg += (s >> 8) & 255;
            sb += s & 255;
            n += 1;
        }
    }
    let rgb = if n > 0 { ((sr / n) << 16) | ((sg / n) << 8) | (sb / n) } else { 0x202020 };
    for i in 0..w * h {
        let a = (*sp.add(i)) >> 24;
        *sp.add(i) = if a < 32 { 0 } else { (a << 24) | rgb };
    }
}
