//! libttf: ve chu TrueType/OpenType (glyf + CFF) cho app myos.
//! - Parse bang ttf-parser (no_std, khong alloc).
//! - Rasterize anti-alias bang accumulation buffer (kieu font-rs), khong dung sqrt/floor cua libm.
//! - Cache bitmap coverage 8-bit theo (glyph, co chu) trong 1 vung mmap, day thi xoa het.
//! - Ve vao back-buffer cua `libgfx::Gfx` qua `Gfx::blit_alpha8`.
#![no_std]

use core::ptr::write_bytes;
mod base;
pub use base::Canvas;
use base::sys;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

/// Bitmap glyph toi da MAX_DIM x MAX_DIM pixel (co chu toi da ~200 px).
const MAX_DIM: usize = 256;
const ACC_FLOATS: usize = (MAX_DIM + 2) * MAX_DIM + 4;
const ARENA_BYTES: usize = 1 << 20; // 1 MiB bitmap coverage
const CACHE_N: usize = 512;
const MAX_FILE: i64 = 32 << 20;

#[derive(Clone, Copy, Debug)]
pub enum Error {
    Open,
    Read,
    Alloc,
    TooBig,
    Parse,
}
impl Error {
    pub fn as_str(&self) -> &'static str {
        match self {
            Error::Open => "cannot open font file",
            Error::Read => "cannot read font file",
            Error::Alloc => "mmap failed",
            Error::TooBig => "font file too big",
            Error::Parse => "bad ttf/otf data",
        }
    }
}

// ---------------- so thuc khong can libm ----------------
#[inline]
fn ffloor(x: f32) -> i32 {
    let i = x as i32;
    if (i as f32) > x { i - 1 } else { i }
}
#[inline]
fn fceil(x: f32) -> i32 {
    let i = x as i32;
    if (i as f32) < x { i + 1 } else { i }
}
#[inline]
fn fround(x: f32) -> i32 { ffloor(x + 0.5) }
#[inline]
fn absf(x: f32) -> f32 { if x < 0.0 { -x } else { x } }
#[inline]
fn clampf(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo { lo } else if x > hi { hi } else { x }
}

// ---------------- rasterizer ----------------
/// Nhan outline tu ttf-parser, lam phang thanh doan thang va cong don dien tich co dau vao `a`.
struct Raster {
    a: *mut f32,
    stride: usize, // = w + 2 de ghi tran sang phai khong lem sang hang sau
    rows: usize,
    w: f32,
    h: f32,
    s: f32,    // font unit -> pixel
    left: f32, // goc bitmap (pixel)
    top: f32,
    sx: f32, // diem bat dau contour (da map)
    sy: f32,
    cx: f32, // diem hien tai (da map)
    cy: f32,
}

impl Raster {
    #[inline]
    fn map(&self, x: f32, y: f32) -> (f32, f32) {
        (
            clampf(x * self.s - self.left, 0.0, self.w),
            clampf(self.top - y * self.s, 0.0, self.h),
        )
    }

    #[inline(always)]
    fn add(&mut self, i: usize, v: f32) {
        unsafe { *self.a.add(i) += v };
    }

    fn line(&mut self, ax: f32, ay: f32, bx: f32, by: f32) {
        if ay == by {
            return;
        }
        let (dir, x0, y0, x1, y1) = if ay < by { (1.0f32, ax, ay, bx, by) } else { (-1.0, bx, by, ax, ay) };
        let dxdy = (x1 - x0) / (y1 - y0);
        let mut x = x0;
        let ys = y0 as usize;
        let ye = (fceil(y1) as usize).min(self.rows);
        for y in ys..ye {
            let ls = y * self.stride;
            let dy = ((y + 1) as f32).min(y1) - (y as f32).max(y0);
            let xn = clampf(x + dxdy * dy, 0.0, self.w);
            let d = dy * dir;
            let (xa, xb) = if x < xn { (x, xn) } else { (xn, x) };
            let xai = ffloor(xa);
            let xbi = fceil(xb);
            let xaf = xai as f32;
            let base = ls + xai as usize;
            if xbi <= xai + 1 {
                let xmf = 0.5 * (x + xn) - xaf;
                self.add(base, d - d * xmf);
                self.add(base + 1, d * xmf);
            } else {
                let s = 1.0 / (xb - xa);
                let x0f = xa - xaf;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = xb - xbi as f32 + 1.0;
                let am = 0.5 * s * x1f * x1f;
                self.add(base, d * a0);
                if xbi == xai + 2 {
                    self.add(base + 1, d * (1.0 - a0 - am));
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.add(base + 1, d * (a1 - a0));
                    for xi in (xai + 2)..(xbi - 1) {
                        self.add(ls + xi as usize, d * s);
                    }
                    let a2 = a1 + (xbi - xai - 3) as f32 * s;
                    self.add(ls + (xbi - 1) as usize, d * (1.0 - a2 - am));
                }
                self.add(ls + xbi as usize, d * am);
            }
            x = xn;
        }
    }

    fn close_path(&mut self) {
        if self.cx != self.sx || self.cy != self.sy {
            let (cx, cy, sx, sy) = (self.cx, self.cy, self.sx, self.sy);
            self.line(cx, cy, sx, sy);
            self.cx = sx;
            self.cy = sy;
        }
    }
}

impl OutlineBuilder for Raster {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close_path();
        let (x, y) = self.map(x, y);
        self.sx = x;
        self.sy = y;
        self.cx = x;
        self.cy = y;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        let (cx, cy) = (self.cx, self.cy);
        self.line(cx, cy, x, y);
        self.cx = x;
        self.cy = y;
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x2, y2) = self.map(x, y);
        let (x0, y0) = (self.cx, self.cy);
        let len = absf(x1 - x0) + absf(y1 - y0) + absf(x2 - x1) + absf(y2 - y1);
        let n = ((len * 0.25) as usize + 2).min(32);
        let (mut px, mut py) = (x0, y0);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let qx = mt * mt * x0 + 2.0 * mt * t * x1 + t * t * x2;
            let qy = mt * mt * y0 + 2.0 * mt * t * y1 + t * t * y2;
            self.line(px, py, qx, qy);
            px = qx;
            py = qy;
        }
        self.cx = x2;
        self.cy = y2;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x2, y2) = self.map(x2, y2);
        let (x3, y3) = self.map(x, y);
        let (x0, y0) = (self.cx, self.cy);
        let len = absf(x1 - x0) + absf(y1 - y0) + absf(x2 - x1) + absf(y2 - y1) + absf(x3 - x2) + absf(y3 - y2);
        let n = ((len * 0.25) as usize + 3).min(32);
        let (mut px, mut py) = (x0, y0);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let mt = 1.0 - t;
            let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
            let qx = a * x0 + b * x1 + c * x2 + d * x3;
            let qy = a * y0 + b * y1 + c * y2 + d * y3;
            self.line(px, py, qx, qy);
            px = qx;
            py = qy;
        }
        self.cx = x3;
        self.cy = y3;
    }
    fn close(&mut self) {
        self.close_path();
    }
}

// ---------------- cache ----------------
#[derive(Clone, Copy)]
struct Entry {
    used: bool,
    gid: u16,
    size: u16,
    left: i16, // lech x so voi diem but (pixel)
    top: i16,  // do cao tu baseline len den hang tren cung cua bitmap
    w: u16,
    h: u16,
    off: u32, // offset trong arena
}
const EMPTY: Entry = Entry { used: false, gid: 0, size: 0, left: 0, top: 0, w: 0, h: 0, off: 0 };

// ---------------- Font ----------------
pub struct Font {
    face: Face<'static>,
    upem: f32,
    file: Option<(*mut u8, usize)>, // vung mmap chua file (neu load tu dia)
    acc: *mut f32,
    arena: *mut u8,
    arena_used: usize,
    table: [Entry; CACHE_N],
}

impl Font {
    /// Font nhung san: `Font::from_static(include_bytes!("../font.ttf"))`.
    pub fn from_static(data: &'static [u8]) -> Result<Font, Error> {
        Self::build(data, None)
    }

    /// Doc font tu file. `path` ket thuc bang NUL, vd `b"/fonts/DejaVuSans.ttf\0"`.
    pub fn load(path: &[u8]) -> Result<Font, Error> {
        let fd = sys::open(path, 0);
        if fd < 0 {
            return Err(Error::Open);
        }
        let size = sys::lseek(fd, 0, 2);
        if size <= 0 || size > MAX_FILE {
            sys::close(fd);
            return Err(Error::TooBig);
        }
        sys::lseek(fd, 0, 0);
        let size = size as usize;
        let len = (size + 4095) & !4095;
        let p = sys::mmap_anon(len);
        if p.is_null() {
            sys::close(fd);
            return Err(Error::Alloc);
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
        if done != size {
            sys::munmap(p, len);
            return Err(Error::Read);
        }
        // Vung nay song den khi Font bi drop (Drop se munmap sau khi face khong con dung).
        let data: &'static [u8] = unsafe { core::slice::from_raw_parts(p, size) };
        match Self::build(data, Some((p, len))) {
            Ok(f) => Ok(f),
            Err(e) => {
                sys::munmap(p, len);
                Err(e)
            }
        }
    }

    fn build(data: &'static [u8], file: Option<(*mut u8, usize)>) -> Result<Font, Error> {
        let face = Face::parse(data, 0).map_err(|_| Error::Parse)?;
        let upem = face.units_per_em() as f32;
        if upem <= 0.0 {
            return Err(Error::Parse);
        }
        let acc_len = (ACC_FLOATS * 4 + 4095) & !4095;
        let acc = sys::mmap_anon(acc_len);
        if acc.is_null() {
            return Err(Error::Alloc);
        }
        let arena = sys::mmap_anon(ARENA_BYTES);
        if arena.is_null() {
            sys::munmap(acc, acc_len);
            return Err(Error::Alloc);
        }
        Ok(Font {
            face,
            upem,
            file,
            acc: acc as *mut f32,
            arena,
            arena_used: 0,
            table: [EMPTY; CACHE_N],
        })
    }

    #[inline]
    fn scale(&self, size: u32) -> f32 { size as f32 / self.upem }

    /// Ascent (pixel, > 0).
    pub fn ascent(&self, size: u32) -> i32 { fround(self.face.ascender() as f32 * self.scale(size)) }
    /// Descent (pixel, thuong < 0).
    pub fn descent(&self, size: u32) -> i32 { fround(self.face.descender() as f32 * self.scale(size)) }
    /// Chieu cao 1 dong = ascent - descent + line gap.
    pub fn line_height(&self, size: u32) -> i32 {
        let f = &self.face;
        fround((f.ascender() as f32 - f.descender() as f32 + f.line_gap() as f32) * self.scale(size))
    }

    fn glyph_id(&self, ch: char) -> GlyphId { self.face.glyph_index(ch).unwrap_or(GlyphId(0)) }

    /// Kerning ngang tu bang `kern` (font chi co GPOS thi tra 0).
    fn kern(&self, a: GlyphId, b: GlyphId) -> i16 {
        if let Some(k) = self.face.tables().kern {
            for st in k.subtables {
                if st.horizontal && !st.variable {
                    if let Some(v) = st.glyphs_kerning(a, b) {
                        return v;
                    }
                }
            }
        }
        0
    }

    /// Do rong chuoi (pixel), da tinh kerning.
    pub fn text_width(&self, s: &str, size: u32) -> i32 {
        let sc = self.scale(size);
        let mut pen = 0.0f32;
        let mut prev: Option<GlyphId> = None;
        for ch in s.chars() {
            let g = self.glyph_id(ch);
            if let Some(p) = prev {
                pen += self.kern(p, g) as f32 * sc;
            }
            pen += self.face.glyph_hor_advance(g).unwrap_or(0) as f32 * sc;
            prev = Some(g);
        }
        fround(pen)
    }

    /// Lay bitmap coverage tu cache, chua co thi rasterize.
    fn glyph(&mut self, gid: GlyphId, size: u32) -> Entry {
        let idx = (gid.0 as usize * 31 + size as usize * 131) % CACHE_N;
        let e = self.table[idx];
        if e.used && e.gid == gid.0 && e.size == size as u16 {
            return e;
        }
        let mut ent = Entry { used: true, gid: gid.0, size: size as u16, ..EMPTY };
        if let Some(r) = self.face.glyph_bounding_box(gid) {
            let s = self.scale(size);
            let left = ffloor(r.x_min as f32 * s);
            let right = fceil(r.x_max as f32 * s);
            let bot = ffloor(r.y_min as f32 * s);
            let top = fceil(r.y_max as f32 * s);
            let (w, h) = ((right - left).max(0) as usize, (top - bot).max(0) as usize);
            if w > 0 && h > 0 && w <= MAX_DIM && h <= MAX_DIM {
                if self.arena_used + w * h > ARENA_BYTES {
                    self.arena_used = 0;
                    for t in self.table.iter_mut() {
                        t.used = false;
                    }
                }
                let stride = w + 2;
                unsafe { write_bytes(self.acc, 0, stride * h + 2) };
                let mut rz = Raster {
                    a: self.acc,
                    stride,
                    rows: h,
                    w: w as f32,
                    h: h as f32,
                    s,
                    left: left as f32,
                    top: top as f32,
                    sx: 0.0,
                    sy: 0.0,
                    cx: 0.0,
                    cy: 0.0,
                };
                self.face.outline_glyph(gid, &mut rz);
                rz.close_path();
                let off = self.arena_used;
                unsafe {
                    let dst = self.arena.add(off);
                    for y in 0..h {
                        let mut acc = 0.0f32;
                        for x in 0..w {
                            acc += *self.acc.add(y * stride + x);
                            let v = absf(acc).min(1.0);
                            *dst.add(y * w + x) = (v * 255.0 + 0.5) as u8;
                        }
                    }
                }
                self.arena_used += w * h;
                ent.left = left as i16;
                ent.top = top as i16;
                ent.w = w as u16;
                ent.h = h as u16;
                ent.off = off as u32;
            }
        }
        self.table[idx] = ent;
        ent
    }

    /// Ve 1 dong chu. `(x, y)` la goc tren-trai cua dong (giong `Gfx::text`).
    /// `size` la co chu theo pixel (em). Tra ve x sau ky tu cuoi.
    pub fn draw_text<C: Canvas>(&mut self, g: &mut C, x: i32, y: i32, s: &str, color: u32, size: u32) -> i32 {
        let size = size.clamp(4, 200);
        let sc = self.scale(size);
        let base = y + self.ascent(size);
        let mut pen = x as f32;
        let mut prev: Option<GlyphId> = None;
        for ch in s.chars() {
            let gid = self.glyph_id(ch);
            if let Some(p) = prev {
                pen += self.kern(p, gid) as f32 * sc;
            }
            let e = self.glyph(gid, size);
            if e.w > 0 {
                let cov = unsafe { core::slice::from_raw_parts(self.arena.add(e.off as usize), e.w as usize * e.h as usize) };
                g.blit_alpha8(cov, e.w as usize, e.h as usize, fround(pen) + e.left as i32, base - e.top as i32, color);
            }
            pen += self.face.glyph_hor_advance(gid).unwrap_or(0) as f32 * sc;
            prev = Some(gid);
        }
        fround(pen)
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        sys::munmap(self.acc as *mut u8, (ACC_FLOATS * 4 + 4095) & !4095);
        sys::munmap(self.arena, ARENA_BYTES);
        if let Some((p, len)) = self.file {
            sys::munmap(p, len);
        }
    }
}
