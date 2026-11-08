//! libswmapi: API client cho swm (window manager). Giao thức qua 2 pipe: fd 3 (gửi) và fd 4 (nhận).
#![no_std]

use core::arch::asm;

// ---------------- giao thức (swm dùng chung) ----------------
pub mod proto {
    pub const FD_TX: i64 = 3; // client -> swm
    pub const FD_RX: i64 = 4; // swm -> client
    pub const HDR: usize = 24;

    pub const C_CREATE: u32 = 1;
    pub const C_PIXELS: u32 = 2;
    pub const C_TITLE: u32 = 3;
    pub const C_CLOSE: u32 = 4;
    pub const C_RESIZE: u32 = 5;

    pub const S_ACK: u32 = 10;
    pub const S_MOUSE: u32 = 11;
    pub const S_KEY: u32 = 12;
    pub const S_FOCUS: u32 = 13;
    pub const S_CLOSE_REQ: u32 = 14;
    pub const S_RESIZE: u32 = 15;

    pub const TITLE_LEN: usize = 32;

    #[derive(Clone, Copy, Debug)]
    pub struct Msg {
        pub ty: u32,
        pub a: i32,
        pub b: i32,
        pub c: i32,
        pub d: i32,
        pub len: u32,
    }

    impl Msg {
        pub const fn new(ty: u32, a: i32, b: i32, c: i32, d: i32, len: u32) -> Msg {
            Msg { ty, a, b, c, d, len }
        }
        pub fn encode(&self) -> [u8; HDR] {
            let mut o = [0u8; HDR];
            o[0..4].copy_from_slice(&self.ty.to_le_bytes());
            o[4..8].copy_from_slice(&self.a.to_le_bytes());
            o[8..12].copy_from_slice(&self.b.to_le_bytes());
            o[12..16].copy_from_slice(&self.c.to_le_bytes());
            o[16..20].copy_from_slice(&self.d.to_le_bytes());
            o[20..24].copy_from_slice(&self.len.to_le_bytes());
            o
        }
        pub fn decode(b: &[u8; HDR]) -> Msg {
            let u = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
            Msg { ty: u(0), a: u(4) as i32, b: u(8) as i32, c: u(12) as i32, d: u(16) as i32, len: u(20) }
        }
    }
}

use proto::*;

// ---------------- syscall ----------------
#[inline(always)]
unsafe fn sc(n: i64, a: i64, b: i64, c: i64, d: i64, e: i64, f: i64) -> i64 {
    let r: i64;
    asm!("syscall", inlateout("rax") n => r, in("rdi") a, in("rsi") b,
         in("rdx") c, in("r10") d, in("r8") e, in("r9") f,
         lateout("rcx") _, lateout("r11") _, options(nostack));
    r
}

fn sys_read(fd: i64, b: &mut [u8]) -> i64 {
    unsafe { sc(0, fd, b.as_mut_ptr() as i64, b.len() as i64, 0, 0, 0) }
}
fn sys_write(fd: i64, b: &[u8]) -> i64 {
    unsafe { sc(1, fd, b.as_ptr() as i64, b.len() as i64, 0, 0, 0) }
}
fn mmap_anon(len: usize) -> *mut u8 {
    let r = unsafe { sc(9, 0, len as i64, 3, 0x22, -1, 0) };
    if r < 0 && r > -4096 { core::ptr::null_mut() } else { r as *mut u8 }
}
fn munmap(p: *mut u8, len: usize) {
    unsafe { sc(11, p as i64, len as i64, 0, 0, 0, 0) };
}

fn write_all(fd: i64, mut b: &[u8]) -> bool {
    while !b.is_empty() {
        let r = sys_write(fd, b);
        if r <= 0 {
            return false;
        }
        b = &b[r as usize..];
    }
    true
}

fn read_exact(fd: i64, mut b: &mut [u8]) -> bool {
    while !b.is_empty() {
        let r = sys_read(fd, b);
        if r <= 0 {
            return false;
        }
        let tmp = b;
        b = &mut tmp[r as usize..];
    }
    true
}

/// Bỏ `n` byte payload không dùng.
fn skip(fd: i64, mut n: usize) {
    let mut tmp = [0u8; 128];
    while n > 0 {
        let k = n.min(tmp.len());
        if !read_exact(fd, &mut tmp[..k]) {
            return;
        }
        n -= k;
    }
}

/// poll không chặn: true nếu fd 4 có dữ liệu. `hup` = swm đã đóng.
fn poll_rx() -> (bool, bool) {
    let mut p = [0u8; 8];
    p[0..4].copy_from_slice(&(FD_RX as i32).to_le_bytes());
    p[4..6].copy_from_slice(&1i16.to_le_bytes()); // POLLIN
    let r = unsafe { sc(7, p.as_mut_ptr() as i64, 1, 0, 0, 0, 0) };
    if r <= 0 {
        return (false, false);
    }
    let rev = i16::from_le_bytes([p[6], p[7]]);
    (rev & 1 != 0, rev & 0x10 != 0)
}

// ---------------- API ----------------
#[derive(Clone, Copy, Debug)]
pub enum Error {
    Size,
    Alloc,
    NotConnected,
    Protocol,
    Send,
}

impl Error {
    pub fn as_str(&self) -> &'static str {
        match self {
            Error::Size => "bad window size",
            Error::Alloc => "mmap window buffer failed",
            Error::NotConnected => "not started by swm (fd 3/4 missing)",
            Error::Protocol => "bad reply from swm",
            Error::Send => "write to swm failed",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Event {
    /// Toạ độ trong cửa sổ; buttons: bit0 trái, bit1 phải, bit2 giữa.
    Mouse { x: i32, y: i32, buttons: u32 },
    Key(u8),
    Focus(bool),
    /// Người dùng đóng cửa sổ, hoặc swm đã thoát.
    Close,
    /// Cửa sổ đã đổi kích thước (maximize/restore); buffer mới giữ nội dung cũ ở góc trên trái.
    Resized { w: usize, h: usize },
}

fn title_bytes(t: &str) -> [u8; TITLE_LEN] {
    let mut o = [0u8; TITLE_LEN];
    for (i, &c) in t.as_bytes().iter().take(TITLE_LEN - 1).enumerate() {
        o[i] = if (32..127).contains(&c) { c } else { b'?' };
    }
    o
}

pub struct Window {
    pub w: usize,
    pub h: usize,
    buf: *mut u32,
    len: usize,
}

impl Window {
    /// Tạo cửa sổ w x h. Chỉ chạy được khi app do swm khởi động.
    pub fn create(w: usize, h: usize, title: &str) -> Result<Window, Error> {
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            return Err(Error::Size);
        }
        let len = (w * h * 4 + 4095) & !4095;
        let p = mmap_anon(len);
        if p.is_null() {
            return Err(Error::Alloc);
        }
        let t = title_bytes(title);
        let m = Msg::new(C_CREATE, w as i32, h as i32, 0, 0, TITLE_LEN as u32);
        if !write_all(FD_TX, &m.encode()) || !write_all(FD_TX, &t) {
            munmap(p, len);
            return Err(Error::NotConnected);
        }
        let mut hb = [0u8; HDR];
        if !read_exact(FD_RX, &mut hb) {
            munmap(p, len);
            return Err(Error::NotConnected);
        }
        let r = Msg::decode(&hb);
        skip(FD_RX, r.len as usize);
        if r.ty != S_ACK {
            munmap(p, len);
            return Err(Error::Protocol);
        }
        Ok(Window { w, h, buf: p as *mut u32, len })
    }

    pub fn pixels(&self) -> &[u32] {
        unsafe { core::slice::from_raw_parts(self.buf, self.w * self.h) }
    }

    /// Buffer 0xRRGGBB, w*h pixel, hàng đầu ở trên. Vẽ xong gọi present.
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        unsafe { core::slice::from_raw_parts_mut(self.buf, self.w * self.h) }
    }

    pub fn set_title(&mut self, title: &str) {
        let t = title_bytes(title);
        let m = Msg::new(C_TITLE, 0, 0, 0, 0, TITLE_LEN as u32);
        let _ = write_all(FD_TX, &m.encode()) && write_all(FD_TX, &t);
    }

    /// Gửi vùng (x, y, w, h) của buffer cho swm.
    pub fn present(&mut self, x: i32, y: i32, w: i32, h: i32) -> Result<(), Error> {
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = ((x + w).max(0) as usize).min(self.w);
        let y1 = ((y + h).max(0) as usize).min(self.h);
        if x0 >= x1 || y0 >= y1 {
            return Ok(());
        }
        let (cw, ch) = (x1 - x0, y1 - y0);
        let m = Msg::new(C_PIXELS, x0 as i32, y0 as i32, cw as i32, ch as i32, (cw * ch * 4) as u32);
        if !write_all(FD_TX, &m.encode()) {
            return Err(Error::Send);
        }
        if x0 == 0 && cw == self.w {
            // vung full-width la lien mach trong bo nho: 1 write thay vi tung hang
            let p = unsafe { self.buf.add(y0 * self.w) } as *const u8;
            let s = unsafe { core::slice::from_raw_parts(p, cw * ch * 4) };
            return if write_all(FD_TX, s) { Ok(()) } else { Err(Error::Send) };
        }
        for row in y0..y1 {
            let p = unsafe { self.buf.add(row * self.w + x0) } as *const u8;
            let s = unsafe { core::slice::from_raw_parts(p, cw * 4) };
            if !write_all(FD_TX, s) {
                return Err(Error::Send);
            }
        }
        Ok(())
    }

    pub fn present_all(&mut self) -> Result<(), Error> {
        let (w, h) = (self.w as i32, self.h as i32);
        self.present(0, 0, w, h)
    }

    /// Lấy 1 sự kiện nếu có, không chặn.
    pub fn poll_event(&mut self) -> Option<Event> {
        loop {
            let (ready, hup) = poll_rx();
            if !ready {
                if hup {
                    return Some(Event::Close);
                }
                sleep_ms(1);
                return None;
            }
            let mut hb = [0u8; HDR];
            if !read_exact(FD_RX, &mut hb) {
                return Some(Event::Close);
            }
            let m = Msg::decode(&hb);
            skip(FD_RX, m.len as usize);
            match m.ty {
                S_MOUSE => return Some(Event::Mouse { x: m.a, y: m.b, buttons: m.c as u32 }),
                S_KEY => return Some(Event::Key(m.a as u8)),
                S_FOCUS => return Some(Event::Focus(m.a != 0)),
                S_CLOSE_REQ => return Some(Event::Close),
                S_RESIZE => {
                    let (nw, nh) = (m.a as usize, m.b as usize);
                    if self.resize(nw, nh) {
                        return Some(Event::Resized { w: nw, h: nh });
                    }
                }
                _ => {} // không biết thì bỏ qua, đọc tiếp
            }
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let m = Msg::new(C_CLOSE, 0, 0, 0, 0, 0);
        let _ = write_all(FD_TX, &m.encode());
        munmap(self.buf as *mut u8, self.len);
    }
}

/// Thoát process.
pub fn exit(code: i32) -> ! {
    unsafe { sc(231, code as i64, 0, 0, 0, 0, 0) };
    loop {}
}

/// `libswmapi::entry!(main);` với `fn main()` không tham số, không trả giá trị.
#[macro_export]
macro_rules! entry {
    ($main:ident) => {
        core::arch::global_asm!(
            ".globl _start",
            "_start:",
            "xor ebp, ebp",
            "mov rdi, rsp",
            "and rsp, -16",
            "call __swm_start",
            "ud2",
        );

        #[no_mangle]
        pub extern "C" fn __swm_start(_sp: *const u64) -> ! {
            $crate::set_sp(_sp);
            $main();
            $crate::exit(0)
        }
    };
}

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

// ================= canvas: vẽ vào buffer cửa sổ =================
pub mod canvas {
    use super::Window;

    pub struct Canvas<'a> {
        px: &'a mut [u32],
        pub w: i32,
        pub h: i32,
    }

    impl Window {
        /// Mượn buffer cửa sổ để vẽ. Vẽ xong thả canvas rồi gọi present.
        pub fn canvas(&mut self) -> Canvas<'_> {
            let (w, h) = (self.w as i32, self.h as i32);
            Canvas { px: self.pixels_mut(), w, h }
        }
    }

    impl<'a> Canvas<'a> {
        pub fn pixel(&mut self, x: i32, y: i32, c: u32) {
            if x >= 0 && y >= 0 && x < self.w && y < self.h {
                self.px[(y * self.w + x) as usize] = c;
            }
        }

        pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
            let (x0, y0) = (x.max(0), y.max(0));
            let (x1, y1) = ((x + w).min(self.w), (y + h).min(self.h));
            if x0 >= x1 || y0 >= y1 {
                return;
            }
            for yy in y0..y1 {
                let o = (yy * self.w) as usize;
                unsafe { libgfx::fast::fill32(self.px.as_mut_ptr().add(o + x0 as usize), c, (x1 - x0) as usize) };
            }
        }

        pub fn clear(&mut self, c: u32) {
            unsafe { libgfx::fast::fill32(self.px.as_mut_ptr(), c, self.px.len()) };
        }

        pub fn hline(&mut self, x: i32, y: i32, len: i32, c: u32) {
            self.fill_rect(x, y, len, 1, c);
        }

        pub fn vline(&mut self, x: i32, y: i32, len: i32, c: u32) {
            self.fill_rect(x, y, 1, len, c);
        }

        /// Viền 1px.
        pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
            self.hline(x, y, w, c);
            self.hline(x, y + h - 1, w, c);
            self.vline(x, y, h, c);
            self.vline(x + w - 1, y, h, c);
        }

        pub fn text_width(&self, s: &str, scale: i32) -> i32 {
            libgfx::ttf_text_width(s, scale.max(1)).unwrap_or(s.len() as i32 * 6 * scale.max(1))
        }

        /// Vẽ chuỗi ASCII, trả về x sau ký tự cuối.
        pub fn text(&mut self, mut x: i32, y: i32, s: &str, c: u32, scale: i32) -> i32 {
            {
                struct AaAd<'b> {
                    px: &'b mut [u32],
                    w: i32,
                    h: i32,
                }
                impl<'b> libgfx::Canvas for AaAd<'b> {
                    fn blit_alpha8(&mut self, cov: &[u8], sw: usize, sh: usize, x: i32, y: i32, c: u32) {
                        let (sr, sg, sb) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
                        for j in 0..sh as i32 {
                            let py = y + j;
                            if py < 0 || py >= self.h {
                                continue;
                            }
                            for i in 0..sw as i32 {
                                let px = x + i;
                                if px < 0 || px >= self.w {
                                    continue;
                                }
                                let a = cov[j as usize * sw + i as usize] as u32;
                                if a == 0 {
                                    continue;
                                }
                                let p = &mut self.px[(py * self.w + px) as usize];
                                if a == 255 {
                                    *p = c & 0x00FF_FFFF;
                                } else {
                                    let ia = 255 - a;
                                    let d = *p;
                                    let r = (sr * a + ((d >> 16) & 255) * ia + 127) / 255;
                                    let g = (sg * a + ((d >> 8) & 255) * ia + 127) / 255;
                                    let b = (sb * a + (d & 255) * ia + 127) / 255;
                                    *p = (r << 16) | (g << 8) | b;
                                }
                            }
                        }
                    }
                }
                let (cw, ch) = (self.w, self.h);
                let mut ad = AaAd { px: &mut *self.px, w: cw, h: ch };
                if let Some(nx) = libgfx::ttf_text(&mut ad, x, y, s, c, scale) {
                    return nx;
                }
            }
            let sc = scale.max(1);
            for &ch in s.as_bytes() {
                for col in 0..5usize {
                    let bits = libgfx::glyph_col(ch, col);
                    for row in 0..8 {
                        if (bits >> row) & 1 != 0 {
                            if sc == 1 {
                                self.pixel(x + col as i32, y + row, c);
                            } else {
                                self.fill_rect(x + col as i32 * sc, y + row * sc, sc, sc, c);
                            }
                        }
                    }
                }
                x += 6 * sc;
            }
            x
        }

        /// Chép ảnh 0xRRGGBB (sw x sh) vào (x, y).
        pub fn blit(&mut self, src: &[u32], sw: i32, sh: i32, x: i32, y: i32) {
            if sw <= 0 || sh <= 0 || (src.len() as i64) < sw as i64 * sh as i64 {
                return;
            }
            let (x0, y0) = (x.max(0), y.max(0));
            let (x1, y1) = ((x + sw).min(self.w), (y + sh).min(self.h));
            if x0 >= x1 || y0 >= y1 {
                return;
            }
            for yy in y0..y1 {
                let s = ((yy - y) * sw + (x0 - x)) as usize;
                let d = (yy * self.w + x0) as usize;
                let n = (x1 - x0) as usize;
                unsafe { libgfx::fast::copy32(self.px.as_mut_ptr().add(d), src.as_ptr().add(s), n) };
            }
        }
    }
}

/// Đọc tối đa buf.len() byte từ đầu file (path kết thúc NUL). 0 nếu lỗi.
pub fn read_file(path: &[u8], buf: &mut [u8]) -> usize {
    let fd = unsafe { sc(2, path.as_ptr() as i64, 0, 0, 0, 0, 0) };
    if fd < 0 {
        return 0;
    }
    let n = sys_read(fd, buf);
    unsafe { sc(3, fd, 0, 0, 0, 0, 0) };
    if n < 0 { 0 } else { n as usize }
}

pub use libgfx::bmp;

static mut SP: u64 = 0;

#[doc(hidden)]
pub fn set_sp(sp: *const u64) {
    unsafe { SP = sp as u64 };
}

/// Đối số dòng lệnh thứ n (0 = tên chương trình).
pub fn arg(n: usize) -> Option<&'static str> {
    unsafe {
        if SP == 0 {
            return None;
        }
        let sp = SP as *const u64;
        if n as u64 >= *sp {
            return None;
        }
        let p = *sp.add(1 + n) as *const u8;
        if p.is_null() {
            return None;
        }
        let mut len = 0;
        while *p.add(len) != 0 && len < 256 {
            len += 1;
        }
        core::str::from_utf8(core::slice::from_raw_parts(p, len)).ok()
    }
}

/// Ngủ ms mili giây (nhường CPU thật sự cho tiến trình khác).
pub fn sleep_ms(ms: u64) {
    let ts = [(ms / 1000) as i64, ((ms % 1000) * 1_000_000) as i64];
    unsafe { sc(35, ts.as_ptr() as i64, 0, 0, 0, 0, 0) };
}

static mut UFD: i64 = -2;

/// Thời gian từ lúc boot (ms), đọc devices://uptime. 0 nếu lỗi.
pub fn uptime_ms() -> u64 {
    unsafe {
        if UFD == -2 {
            UFD = sc(2, b"devices://uptime\0".as_ptr() as i64, 0, 0, 0, 0, 0);
        }
        if UFD < 0 {
            return 0;
        }
        sc(8, UFD, 0, 0, 0, 0, 0);
        let mut b = [0u8; 24];
        let n = sys_read(UFD, &mut b);
        if n <= 0 {
            return 0;
        }
        let mut v = 0u64;
        for &c in &b[..n as usize] {
            if c.is_ascii_digit() {
                v = v * 10 + (c - b'0') as u64;
            } else {
                break;
            }
        }
        v
    }
}

/// Syscall thô (tối đa 4 đối số) cho app cần pipe/fork.
pub unsafe fn syscall(n: i64, a: i64, b: i64, c: i64, d: i64) -> i64 {
    sc(n, a, b, c, d, 0, 0)
}

impl Window {
    /// Đổi kích thước buffer (giữ nội dung cũ ở góc trên trái), báo swm rồi gửi lại toàn cửa sổ.
    pub fn resize(&mut self, w: usize, h: usize) -> bool {
        if w == 0 || h == 0 || w > 4096 || h > 4096 {
            return false;
        }
        let len = (w * h * 4 + 4095) & !4095;
        let p = mmap_anon(len) as *mut u32;
        if p.is_null() {
            return false;
        }
        unsafe {
            libgfx::fast::fill32(p, 0x00EC_ECF2, w * h);
            let (cw, ch) = (w.min(self.w), h.min(self.h));
            for y in 0..ch {
                libgfx::fast::copy32(p.add(y * w), self.buf.add(y * self.w), cw);
            }
        }
        munmap(self.buf as *mut u8, self.len);
        self.buf = p;
        self.len = len;
        self.w = w;
        self.h = h;
        let m = Msg::new(C_RESIZE, w as i32, h as i32, 0, 0, 0);
        if !write_all(FD_TX, &m.encode()) {
            return false;
        }
        self.present_all().is_ok()
    }
}