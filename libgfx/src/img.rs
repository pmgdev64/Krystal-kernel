//! libgfx::img — giai ma PNG + JPEG (baseline) + BMP. no_std, khong alloc, chi dung mmap.
//!
//! Dung:
//!   let img = libgfx::img::load(b"/resources/a.png\0")?;   // tu nhan dang PNG / JPEG / BMP
//!   gfx.blit_image(&img, x, y);
//!
//! PNG: depth 1/2/4/8/16, gray / rgb / palette / gray+alpha / rgba, KHONG ho tro interlace (Adam7).
//! JPEG: baseline/extended sequential Huffman, 8-bit, gray hoac YCbCr, moi kieu subsampling.
//!       KHONG ho tro progressive, CMYK, arithmetic.
//! Anh co alpha duoc tron len mau nen `bg` (0xRRGGBB), mac dinh den.

use crate::bmp::{self, BmpError, Image};
use crate::{rgb, sys};

#[derive(Clone, Copy, Debug)]
pub enum ImgError {
    Open,
    Read,
    Alloc,
    Format,
    Unsupported,
    TooBig,
}
impl ImgError {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImgError::Open => "cannot open image file",
            ImgError::Read => "cannot read image file",
            ImgError::Alloc => "mmap failed",
            ImgError::Format => "bad image data",
            ImgError::Unsupported => "unsupported image (png: no interlace; jpeg: baseline only)",
            ImgError::TooBig => "image too big",
        }
    }
}
impl From<BmpError> for ImgError {
    fn from(e: BmpError) -> Self {
        match e {
            BmpError::Open => ImgError::Open,
            BmpError::Read => ImgError::Read,
            BmpError::Alloc => ImgError::Alloc,
            BmpError::Format => ImgError::Format,
            BmpError::Unsupported => ImgError::Unsupported,
            BmpError::TooBig => ImgError::TooBig,
        }
    }
}

const MAX_DIM: usize = 8192;

// ---------------- bo nho mmap tam (tu giai phong) ----------------
struct Map {
    p: *mut u8,
    len: usize,
}
impl Map {
    fn new(n: usize) -> Result<Map, ImgError> {
        let len = (n.max(1) + 4095) & !4095;
        let p = sys::mmap_anon(len);
        if p.is_null() {
            Err(ImgError::Alloc)
        } else {
            Ok(Map { p, len })
        }
    }
    fn s(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.p, self.len) }
    }
}
impl Drop for Map {
    fn drop(&mut self) {
        sys::munmap(self.p, self.len);
    }
}

// ---------------- API chinh ----------------
pub fn decode(d: &[u8]) -> Result<Image, ImgError> {
    decode_bg(d, 0x000000)
}

pub fn decode_bg(d: &[u8], bg: u32) -> Result<Image, ImgError> {
    if d.len() >= 8 && d[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        decode_png(d, bg)
    } else if d.len() >= 3 && d[0] == 0xFF && d[1] == 0xD8 {
        decode_jpeg(d)
    } else if d.len() >= 2 && &d[0..2] == b"BM" {
        Ok(bmp::decode(d)?)
    } else {
        Err(ImgError::Format)
    }
}

/// Doc file (path ket thuc NUL) roi giai ma, tu nhan dang dinh dang.
pub fn load(path: &[u8]) -> Result<Image, ImgError> {
    load_bg(path, 0x000000)
}

pub fn load_bg(path: &[u8], bg: u32) -> Result<Image, ImgError> {
    let fd = sys::open(path, 0);
    if fd < 0 {
        return Err(ImgError::Open);
    }
    let size = sys::lseek(fd, 0, 2);
    if size <= 0 || size > (64 << 20) {
        sys::close(fd);
        return Err(ImgError::TooBig);
    }
    sys::lseek(fd, 0, 0);
    let size = size as usize;
    let mut m = match Map::new(size) {
        Ok(m) => m,
        Err(e) => {
            sys::close(fd);
            return Err(e);
        }
    };
    let mut done = 0usize;
    {
        let buf = &mut m.s()[..size];
        while done < size {
            let r = sys::read(fd, &mut buf[done..]);
            if r <= 0 {
                break;
            }
            done += r as usize;
        }
    }
    sys::close(fd);
    if done != size {
        return Err(ImgError::Read);
    }
    decode_bg(&m.s()[..size], bg)
}

// =====================================================================
// INFLATE (zlib/deflate), bang tra nhanh 9 bit + bo dem bit 64
// =====================================================================
const LBASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DEXT: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];

struct Huff {
    fast: [u16; 512], // (len << 9) | sym, 0 = ma dai hon 9 bit -> duong cham
    count: [u16; 16],
    sym: [u16; 288],
}

fn rev_bits(c: u32, l: u32) -> u32 {
    let mut r = 0;
    for i in 0..l {
        r |= ((c >> i) & 1) << (l - 1 - i);
    }
    r
}

impl Huff {
    fn new(len: &[u8]) -> Huff {
        let mut h = Huff { fast: [0; 512], count: [0; 16], sym: [0; 288] };
        for &l in len {
            h.count[(l & 15) as usize] += 1;
        }
        h.count[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..15 {
            offs[i + 1] = offs[i] + h.count[i];
        }
        for (s, &l) in len.iter().enumerate() {
            let l = (l & 15) as usize;
            if l != 0 {
                if let Some(slot) = h.sym.get_mut(offs[l] as usize) {
                    *slot = s as u16;
                }
                offs[l] += 1;
            }
        }
        // bang tra nhanh cho ma <= 9 bit
        let mut next = [0u32; 16];
        let mut code = 0u32;
        for l in 1..16 {
            code = (code + h.count[l - 1] as u32) << 1;
            next[l] = code;
        }
        for (s, &l) in len.iter().enumerate() {
            let l = (l & 15) as u32;
            if l == 0 || l > 9 {
                continue;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            let e = (s as u16) | ((l as u16) << 9);
            let mut k = rev_bits(c, l) as usize;
            while k < 512 {
                h.fast[k] = e;
                k += 1usize << l;
            }
        }
        h
    }
}

struct Inflater<'a> {
    d: &'a [u8],
    pos: usize,
    buf: u64,
    cnt: u32,
}
impl<'a> Inflater<'a> {
    #[inline(always)]
    fn refill(&mut self) {
        if self.pos + 8 <= self.d.len() {
            let mut a = [0u8; 8];
            a.copy_from_slice(&self.d[self.pos..self.pos + 8]);
            self.buf |= u64::from_le_bytes(a) << self.cnt;
            let n = (63 - self.cnt) >> 3;
            self.pos += n as usize;
            self.cnt += n << 3;
        } else {
            while self.cnt <= 55 && self.pos < self.d.len() {
                self.buf |= (self.d[self.pos] as u64) << self.cnt;
                self.pos += 1;
                self.cnt += 8;
            }
        }
    }

    #[inline(always)]
    fn bits(&mut self, n: u32) -> Result<u32, ()> {
        if self.cnt < n {
            self.refill();
            if self.cnt < n {
                return Err(());
            }
        }
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.cnt -= n;
        Ok(v)
    }

    #[inline(always)]
    fn decode(&mut self, h: &Huff) -> Result<usize, ()> {
        if self.cnt < 15 {
            self.refill();
        }
        let e = h.fast[(self.buf & 511) as usize];
        if e != 0 {
            let l = (e >> 9) as u32;
            if l > self.cnt {
                return Err(());
            }
            self.buf >>= l;
            self.cnt -= l;
            return Ok((e & 511) as usize);
        }
        // duong cham: ma dai 10..15 bit (kieu puff)
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=15u32 {
            code |= ((self.buf >> (len - 1)) & 1) as i32;
            let count = h.count[len as usize] as i32;
            if code - count < first {
                if len > self.cnt {
                    return Err(());
                }
                self.buf >>= len;
                self.cnt -= len;
                return h.sym.get((index + (code - first)) as usize).map(|&v| v as usize).ok_or(());
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(())
    }

    fn codes(&mut self, out: &mut [u8], o: &mut usize, lc: &Huff, dc: &Huff) -> Result<(), ()> {
        loop {
            let s = self.decode(lc)?;
            if s < 256 {
                if *o >= out.len() {
                    return Err(());
                }
                out[*o] = s as u8;
                *o += 1;
            } else if s == 256 {
                return Ok(());
            } else {
                let s = s - 257;
                if s >= 29 {
                    return Err(());
                }
                let len = LBASE[s] as usize + self.bits(LEXT[s] as u32)? as usize;
                let ds = self.decode(dc)?;
                if ds >= 30 {
                    return Err(());
                }
                let dist = DBASE[ds] as usize + self.bits(DEXT[ds] as u32)? as usize;
                if dist > *o || *o + len > out.len() {
                    return Err(());
                }
                let p = *o;
                if dist >= len {
                    out.copy_within(p - dist..p - dist + len, p);
                } else {
                    for k in 0..len {
                        out[p + k] = out[p + k - dist];
                    }
                }
                *o += len;
            }
        }
    }
}

/// Giai nen raw deflate (khong co header zlib) vao `out`. Tra ve so byte da ghi.
fn inflate(src: &[u8], out: &mut [u8]) -> Result<usize, ()> {
    let mut s = Inflater { d: src, pos: 0, buf: 0, cnt: 0 };
    let mut o = 0usize;
    loop {
        let last = s.bits(1)?;
        match s.bits(2)? {
            0 => {
                // stored: bo bit le, tra lai byte nguyen chua dung trong bo dem
                let drop = s.cnt & 7;
                s.buf >>= drop;
                s.cnt -= drop;
                s.pos -= (s.cnt / 8) as usize;
                s.buf = 0;
                s.cnt = 0;
                if s.pos + 4 > s.d.len() {
                    return Err(());
                }
                let len = u16::from_le_bytes([s.d[s.pos], s.d[s.pos + 1]]);
                let nlen = u16::from_le_bytes([s.d[s.pos + 2], s.d[s.pos + 3]]);
                if len != !nlen {
                    return Err(());
                }
                let len = len as usize;
                s.pos += 4;
                if s.pos + len > s.d.len() || o + len > out.len() {
                    return Err(());
                }
                out[o..o + len].copy_from_slice(&s.d[s.pos..s.pos + len]);
                s.pos += len;
                o += len;
            }
            1 => {
                let mut l = [0u8; 288];
                for (i, v) in l.iter_mut().enumerate() {
                    *v = if i < 144 { 8 } else if i < 256 { 9 } else if i < 280 { 7 } else { 8 };
                }
                let lc = Huff::new(&l);
                let dc = Huff::new(&[5u8; 30]);
                s.codes(out, &mut o, &lc, &dc)?;
            }
            2 => {
                const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                let nlen = s.bits(5)? as usize + 257;
                let ndist = s.bits(5)? as usize + 1;
                let ncode = s.bits(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err(());
                }
                let mut cl = [0u8; 19];
                for i in 0..ncode {
                    cl[ORDER[i]] = s.bits(3)? as u8;
                }
                let clc = Huff::new(&cl);
                let mut lens = [0u8; 320];
                let total = nlen + ndist;
                let mut idx = 0usize;
                while idx < total {
                    let sym = s.decode(&clc)?;
                    if sym < 16 {
                        lens[idx] = sym as u8;
                        idx += 1;
                    } else {
                        let (prev, rep) = match sym {
                            16 => {
                                if idx == 0 {
                                    return Err(());
                                }
                                (lens[idx - 1], 3 + s.bits(2)? as usize)
                            }
                            17 => (0, 3 + s.bits(3)? as usize),
                            _ => (0, 11 + s.bits(7)? as usize),
                        };
                        if idx + rep > total {
                            return Err(());
                        }
                        for _ in 0..rep {
                            lens[idx] = prev;
                            idx += 1;
                        }
                    }
                }
                let lc = Huff::new(&lens[..nlen]);
                let dc = Huff::new(&lens[nlen..total]);
                s.codes(out, &mut o, &lc, &dc)?;
            }
            _ => return Err(()),
        }
        if last == 1 {
            break;
        }
    }
    Ok(o)
}

// =====================================================================
// PNG
// =====================================================================
fn be32(d: &[u8], o: usize) -> usize {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i32, b as i32, c as i32);
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

/// Lay mau thu `idx` trong hang. depth 16 -> byte cao.
#[inline]
fn sample(row: &[u8], idx: usize, depth: usize) -> u32 {
    match depth {
        8 => row[idx] as u32,
        16 => row[idx * 2] as u32,
        _ => {
            let bit = idx * depth;
            let shift = 8 - depth - (bit % 8);
            ((row[bit / 8] >> shift) as u32) & ((1u32 << depth) - 1)
        }
    }
}

#[inline]
fn blend(s: u32, a: u32, d: u32) -> u32 {
    (s * a + d * (255 - a) + 127) / 255
}

fn decode_png(d: &[u8], bg: u32) -> Result<Image, ImgError> {
    let f = ImgError::Format;
    let (mut w, mut h, mut depth, mut ctype, mut interlace) = (0usize, 0usize, 0usize, 0u8, 0u8);
    let mut have_ihdr = false;
    let mut pal = [0u8; 768];
    let mut alpha = [255u8; 256];
    let mut idat_total = 0usize;

    // lan 1: doc chunk
    let mut pos = 8usize;
    while pos + 12 <= d.len() {
        let len = be32(d, pos);
        let end = pos.checked_add(12).and_then(|v| v.checked_add(len)).ok_or(f)?;
        if end > d.len() {
            return Err(f);
        }
        let typ = &d[pos + 4..pos + 8];
        let data = &d[pos + 8..pos + 8 + len];
        match typ {
            b"IHDR" => {
                if len < 13 || data[10] != 0 || data[11] != 0 {
                    return Err(f);
                }
                w = be32(data, 0);
                h = be32(data, 4);
                depth = data[8] as usize;
                ctype = data[9];
                interlace = data[12];
                have_ihdr = true;
            }
            b"PLTE" => {
                let n = (len / 3).min(256);
                pal[..n * 3].copy_from_slice(&data[..n * 3]);
            }
            b"tRNS" => {
                if ctype == 3 {
                    let n = len.min(256);
                    alpha[..n].copy_from_slice(&data[..n]);
                }
            }
            b"IDAT" => idat_total += len,
            b"IEND" => break,
            _ => {}
        }
        pos = end;
    }
    if !have_ihdr || w == 0 || h == 0 {
        return Err(f);
    }
    if w > MAX_DIM || h > MAX_DIM {
        return Err(ImgError::TooBig);
    }
    if interlace != 0 {
        return Err(ImgError::Unsupported);
    }
    let ch = match (ctype, depth) {
        (0, 1 | 2 | 4 | 8 | 16) => 1,
        (2, 8 | 16) => 3,
        (3, 1 | 2 | 4 | 8) => 1,
        (4, 8 | 16) => 2,
        (6, 8 | 16) => 4,
        _ => return Err(f),
    };
    if idat_total < 7 {
        return Err(f);
    }
    let stride = (w * ch * depth + 7) / 8;
    let raw_len = (stride + 1) * h;
    if raw_len > (128 << 20) {
        return Err(ImgError::TooBig);
    }

    // gom IDAT lai thanh 1 luong zlib lien tuc
    let mut zb = Map::new(idat_total)?;
    {
        let z = zb.s();
        let (mut pos, mut zo) = (8usize, 0usize);
        while pos + 12 <= d.len() {
            let len = be32(d, pos);
            if &d[pos + 4..pos + 8] == b"IDAT" {
                z[zo..zo + len].copy_from_slice(&d[pos + 8..pos + 8 + len]);
                zo += len;
            }
            pos += 12 + len;
        }
    }
    let mut rawm = Map::new(raw_len)?;
    let raw = &mut rawm.s()[..raw_len];
    // bo 2 byte header zlib, bo qua adler32 o cuoi (inflate tu dung o block cuoi)
    let got = inflate(&zb.s()[2..idat_total], raw).map_err(|_| f)?;
    if got < raw_len {
        return Err(f);
    }

    // bo loc (filter) tai cho, tach theo loai filter
    let bpp = ((ch * depth) + 7) / 8;
    let mut zm = Map::new(stride)?;
    let zero: &[u8] = &zm.s()[..stride];
    for y in 0..h {
        let r0 = y * (stride + 1);
        let (a, b) = raw.split_at_mut(r0);
        let ft = b[0];
        if ft == 0 {
            continue;
        }
        let cur = &mut b[1..1 + stride];
        let prev: &[u8] = if y > 0 { &a[r0 - stride..r0] } else { zero };
        match ft {
            1 => {
                for i in bpp..stride {
                    cur[i] = cur[i].wrapping_add(cur[i - bpp]);
                }
            }
            2 => {
                for i in 0..stride {
                    cur[i] = cur[i].wrapping_add(prev[i]);
                }
            }
            3 => {
                for i in 0..bpp.min(stride) {
                    cur[i] = cur[i].wrapping_add(prev[i] >> 1);
                }
                for i in bpp..stride {
                    cur[i] = cur[i].wrapping_add(((cur[i - bpp] as u16 + prev[i] as u16) >> 1) as u8);
                }
            }
            4 => {
                for i in 0..bpp.min(stride) {
                    cur[i] = cur[i].wrapping_add(prev[i]);
                }
                for i in bpp..stride {
                    cur[i] = cur[i].wrapping_add(paeth(cur[i - bpp], prev[i], prev[i - bpp]));
                }
            }
            _ => return Err(f),
        }
    }

    // doi sang 0xRRGGBB
    let mut img = Image::new(w, h).ok_or(ImgError::Alloc)?;
    let (br, bgc, bb) = ((bg >> 16) & 255, (bg >> 8) & 255, bg & 255);
    let maxv = ((1u32 << depth.min(8)) - 1).max(1);
    // bang mau palette da tron san voi nen
    let mut pc = [0u32; 256];
    for i in 0..256 {
        let a = alpha[i] as u32;
        let (r, g, b) = (pal[i * 3] as u32, pal[i * 3 + 1] as u32, pal[i * 3 + 2] as u32);
        pc[i] = if a == 255 {
            (r << 16) | (g << 8) | b
        } else {
            (blend(r, a, br) << 16) | (blend(g, a, bgc) << 8) | blend(b, a, bb)
        };
    }
    let out = img.pixels_mut();
    for y in 0..h {
        let row = &raw[y * (stride + 1) + 1..y * (stride + 1) + 1 + stride];
        let orow = &mut out[y * w..(y + 1) * w];
        if ctype == 3 {
            if depth == 8 {
                for (px, &i) in orow.iter_mut().zip(row.iter()) {
                    *px = pc[i as usize];
                }
            } else {
                for x in 0..w {
                    orow[x] = pc[(sample(row, x, depth) as usize) & 255];
                }
            }
            continue;
        }
        if depth == 8 && ctype == 2 {
            for (px, c) in orow.iter_mut().zip(row.chunks_exact(3)) {
                *px = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32;
            }
            continue;
        }
        if depth == 8 && ctype == 6 {
            for (px, c) in orow.iter_mut().zip(row.chunks_exact(4)) {
                let a = c[3] as u32;
                let (r, g, b) = (c[0] as u32, c[1] as u32, c[2] as u32);
                *px = if a == 255 {
                    (r << 16) | (g << 8) | b
                } else {
                    (blend(r, a, br) << 16) | (blend(g, a, bgc) << 8) | blend(b, a, bb)
                };
            }
            continue;
        }
        if depth == 8 && ctype == 0 {
            for (px, &v) in orow.iter_mut().zip(row.iter()) {
                let v = v as u32;
                *px = (v << 16) | (v << 8) | v;
            }
            continue;
        }
        for x in 0..w {
            let (r, g, b, a) = match ctype {
                0 => {
                    let mut v = sample(row, x, depth);
                    if depth < 8 {
                        v = v * 255 / maxv;
                    }
                    (v, v, v, 255)
                }
                2 => (sample(row, x * 3, depth), sample(row, x * 3 + 1, depth), sample(row, x * 3 + 2, depth), 255),
                4 => {
                    let v = sample(row, x * 2, depth);
                    (v, v, v, sample(row, x * 2 + 1, depth))
                }
                _ => (
                    sample(row, x * 4, depth),
                    sample(row, x * 4 + 1, depth),
                    sample(row, x * 4 + 2, depth),
                    sample(row, x * 4 + 3, depth),
                ),
            };
            orow[x] = if a == 255 {
                (r << 16) | (g << 8) | b
            } else {
                (blend(r, a, br) << 16) | (blend(g, a, bgc) << 8) | blend(b, a, bb)
            };
        }
    }
    Ok(img)
}

// =====================================================================
// JPEG baseline
// =====================================================================
const ZZ: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61,
    54, 47, 55, 62, 63,
];

#[derive(Clone, Copy)]
struct JHuff {
    count: [u16; 17],
    sym: [u8; 256],
}
const JH0: JHuff = JHuff { count: [0; 17], sym: [0; 256] };

#[derive(Clone, Copy)]
struct Comp {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    td: usize,
    ta: usize,
    pred: i32,
    pw: usize,
    off: usize,
}
const C0: Comp = Comp { id: 0, h: 1, v: 1, tq: 0, td: 0, ta: 0, pred: 0, pw: 0, off: 0 };

fn be16(d: &[u8], o: usize) -> Option<usize> {
    Some(((*d.get(o)? as usize) << 8) | *d.get(o + 1)? as usize)
}

struct JBits<'a> {
    d: &'a [u8],
    pos: usize,
    cur: u32,
    cnt: u32,
    marker: bool,
}
impl<'a> JBits<'a> {
    fn fill(&mut self) {
        self.cnt = 8;
        if self.marker || self.pos >= self.d.len() {
            self.marker = true;
            self.cur = 0;
            return;
        }
        let b = self.d[self.pos];
        if b == 0xFF {
            let n = self.d.get(self.pos + 1).copied().unwrap_or(0xD9);
            if n == 0 {
                self.pos += 2;
                self.cur = 0xFF;
            } else {
                self.marker = true; // gap marker: dung lai, cap bit 0
                self.cur = 0;
            }
        } else {
            self.pos += 1;
            self.cur = b as u32;
        }
    }
    #[inline]
    fn bit(&mut self) -> u32 {
        if self.cnt == 0 {
            self.fill();
        }
        self.cnt -= 1;
        (self.cur >> self.cnt) & 1
    }
    fn recv(&mut self, n: u32) -> i32 {
        let mut v = 0i32;
        for _ in 0..n {
            v = (v << 1) | self.bit() as i32;
        }
        v
    }
    fn huff(&mut self, h: &JHuff) -> Option<u8> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=16usize {
            code |= self.bit() as i32;
            let count = h.count[len] as i32;
            if code - count < first {
                return h.sym.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        None
    }
    /// Sang RSTn tiep theo, reset bo dem bit.
    fn restart(&mut self) {
        self.cnt = 0;
        self.marker = false;
        while self.pos + 1 < self.d.len() && !(self.d[self.pos] == 0xFF && (0xD0..=0xD7).contains(&self.d[self.pos + 1])) {
            self.pos += 1;
        }
        self.pos += 2;
    }
}

#[inline]
fn extend(v: i32, n: u32) -> i32 {
    if n == 0 { 0 } else if v < (1 << (n - 1)) { v - (1 << n) + 1 } else { v }
}

fn decode_block(br: &mut JBits, dc: &JHuff, ac: &JHuff, q: &[u16; 64], pred: &mut i32, out: &mut [i32; 64]) -> Option<()> {
    *out = [0; 64];
    let t = br.huff(dc)? as u32;
    if t > 16 {
        return None;
    }
    let diff = if t == 0 { 0 } else { extend(br.recv(t), t) };
    *pred += diff;
    out[0] = *pred * q[0] as i32;
    let mut k = 1usize;
    while k < 64 {
        let rs = br.huff(ac)?;
        let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
        if s == 0 {
            if r == 15 {
                k += 16;
                continue;
            }
            break;
        }
        k += r;
        if k > 63 {
            break;
        }
        out[ZZ[k] as usize] = extend(br.recv(s), s) * q[k] as i32;
        k += 1;
    }
    Some(())
}

// IDCT so nguyen (kieu jidctint)
#[inline(always)]
fn idct_core(s: [i32; 8]) -> [i32; 8] {
    let p1 = (s[2] + s[6]) * 2217;
    let t2 = p1 + s[6] * -7567;
    let t3 = p1 + s[2] * 3135;
    let t0 = (s[0] + s[4]) * 4096;
    let t1 = (s[0] - s[4]) * 4096;
    let (x0, x3, x1, x2) = (t0 + t3, t0 - t3, t1 + t2, t1 - t2);
    let (mut t0, mut t1, mut t2, mut t3) = (s[7], s[5], s[3], s[1]);
    let mut p3 = t0 + t2;
    let mut p4 = t1 + t3;
    let mut p1 = t0 + t3;
    let mut p2 = t1 + t2;
    let p5 = (p3 + p4) * 4816;
    t0 *= 1223;
    t1 *= 8410;
    t2 *= 12586;
    t3 *= 6149;
    p1 = p5 + p1 * -3685;
    p2 = p5 + p2 * -10497;
    p3 *= -8034;
    p4 *= -1597;
    t3 += p1 + p4;
    t2 += p2 + p3;
    t1 += p2 + p4;
    t0 += p1 + p3;
    [x0, x1, x2, x3, t0, t1, t2, t3]
}

#[inline]
fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

fn idct(c: &[i32; 64], out: &mut [u8], stride: usize) {
    let mut tmp = [0i32; 64];
    for col in 0..8 {
        let r = idct_core([c[col], c[col + 8], c[col + 16], c[col + 24], c[col + 32], c[col + 40], c[col + 48], c[col + 56]]);
        let (x0, x1, x2, x3) = (r[0] + 512, r[1] + 512, r[2] + 512, r[3] + 512);
        let (t0, t1, t2, t3) = (r[4], r[5], r[6], r[7]);
        tmp[col] = (x0 + t3) >> 10;
        tmp[col + 56] = (x0 - t3) >> 10;
        tmp[col + 8] = (x1 + t2) >> 10;
        tmp[col + 48] = (x1 - t2) >> 10;
        tmp[col + 16] = (x2 + t1) >> 10;
        tmp[col + 40] = (x2 - t1) >> 10;
        tmp[col + 24] = (x3 + t0) >> 10;
        tmp[col + 32] = (x3 - t0) >> 10;
    }
    for row in 0..8 {
        let t = &tmp[row * 8..row * 8 + 8];
        let r = idct_core([t[0], t[1], t[2], t[3], t[4], t[5], t[6], t[7]]);
        let k = 65536 + (128 << 17);
        let (x0, x1, x2, x3) = (r[0] + k, r[1] + k, r[2] + k, r[3] + k);
        let (t0, t1, t2, t3) = (r[4], r[5], r[6], r[7]);
        let o = &mut out[row * stride..row * stride + 8];
        o[0] = clamp8((x0 + t3) >> 17);
        o[7] = clamp8((x0 - t3) >> 17);
        o[1] = clamp8((x1 + t2) >> 17);
        o[6] = clamp8((x1 - t2) >> 17);
        o[2] = clamp8((x2 + t1) >> 17);
        o[5] = clamp8((x2 - t1) >> 17);
        o[3] = clamp8((x3 + t0) >> 17);
        o[4] = clamp8((x3 - t0) >> 17);
    }
}

fn decode_jpeg(d: &[u8]) -> Result<Image, ImgError> {
    let f = ImgError::Format;
    if d.len() < 4 || d[0] != 0xFF || d[1] != 0xD8 {
        return Err(f);
    }
    let mut pos = 2usize;
    let mut q = [[0u16; 64]; 4];
    let mut dct = [JH0; 4];
    let mut act = [JH0; 4];
    let mut comps = [C0; 3];
    let mut nc = 0usize;
    let (mut w, mut h) = (0usize, 0usize);
    let mut ri = 0usize;
    let mut have_sof = false;

    loop {
        while pos < d.len() && d[pos] != 0xFF {
            pos += 1;
        }
        while pos < d.len() && d[pos] == 0xFF {
            pos += 1;
        }
        if pos >= d.len() {
            return Err(f);
        }
        let m = d[pos];
        pos += 1;
        if m == 0x00 || m == 0x01 || (0xD0..=0xD8).contains(&m) {
            continue;
        }
        if m == 0xD9 {
            return Err(f);
        }
        let len = be16(d, pos).ok_or(f)?;
        if len < 2 {
            return Err(f);
        }
        let seg = d.get(pos + 2..pos + len).ok_or(f)?;
        pos += len;
        match m {
            0xDB => {
                let mut i = 0usize;
                while i < seg.len() {
                    let pq = (seg[i] >> 4) as usize;
                    let tq = (seg[i] & 15) as usize;
                    i += 1;
                    if tq > 3 || pq > 1 || i + 64 * (pq + 1) > seg.len() {
                        return Err(f);
                    }
                    for k in 0..64 {
                        if pq == 0 {
                            q[tq][k] = seg[i] as u16;
                            i += 1;
                        } else {
                            q[tq][k] = ((seg[i] as u16) << 8) | seg[i + 1] as u16;
                            i += 2;
                        }
                    }
                }
            }
            0xC4 => {
                let mut i = 0usize;
                while i + 17 <= seg.len() {
                    let tc = seg[i] >> 4;
                    let th = (seg[i] & 15) as usize;
                    if tc > 1 || th > 3 {
                        return Err(f);
                    }
                    let mut t = JH0;
                    let mut total = 0usize;
                    for l in 1..=16 {
                        t.count[l] = seg[i + l] as u16;
                        total += seg[i + l] as usize;
                    }
                    if total > 256 || i + 17 + total > seg.len() {
                        return Err(f);
                    }
                    t.sym[..total].copy_from_slice(&seg[i + 17..i + 17 + total]);
                    if tc == 0 { dct[th] = t } else { act[th] = t }
                    i += 17 + total;
                }
            }
            0xC0 | 0xC1 => {
                if have_sof || seg.len() < 6 || seg[0] != 8 {
                    return Err(ImgError::Unsupported);
                }
                h = be16(seg, 1).ok_or(f)?;
                w = be16(seg, 3).ok_or(f)?;
                nc = seg[5] as usize;
                if nc != 1 && nc != 3 {
                    return Err(ImgError::Unsupported);
                }
                if seg.len() < 6 + 3 * nc || w == 0 || h == 0 {
                    return Err(f);
                }
                if w > MAX_DIM || h > MAX_DIM {
                    return Err(ImgError::TooBig);
                }
                for i in 0..nc {
                    let hv = seg[7 + 3 * i];
                    let (hs, vs) = ((hv >> 4) as usize, (hv & 15) as usize);
                    let tq = seg[8 + 3 * i] as usize;
                    if !(1..=4).contains(&hs) || !(1..=4).contains(&vs) || tq > 3 {
                        return Err(f);
                    }
                    comps[i] = Comp { id: seg[6 + 3 * i], h: hs, v: vs, tq, ..C0 };
                }
                have_sof = true;
            }
            0xC2 | 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => return Err(ImgError::Unsupported),
            0xDD => ri = be16(seg, 0).ok_or(f)?,
            0xDA => {
                if !have_sof || seg.is_empty() || seg[0] as usize != nc || seg.len() < 1 + 2 * nc + 3 {
                    return Err(ImgError::Unsupported);
                }
                for i in 0..nc {
                    let cid = seg[1 + 2 * i];
                    let t = seg[2 + 2 * i];
                    let c = comps[..nc].iter_mut().find(|c| c.id == cid).ok_or(f)?;
                    c.td = (t >> 4) as usize;
                    c.ta = (t & 15) as usize;
                    if c.td > 3 || c.ta > 3 {
                        return Err(f);
                    }
                }
                break; // pos tro thang vao du lieu entropy
            }
            _ => {}
        }
    }

    // ---- chuan bi plane cho tung component ----
    let hmax = comps[..nc].iter().map(|c| c.h).max().unwrap_or(1);
    let vmax = comps[..nc].iter().map(|c| c.v).max().unwrap_or(1);
    let (mcux, mcuy) = ((w + 8 * hmax - 1) / (8 * hmax), (h + 8 * vmax - 1) / (8 * vmax));
    let mut total = 0usize;
    for c in comps[..nc].iter_mut() {
        c.pw = mcux * c.h * 8;
        c.off = total;
        total += c.pw * mcuy * c.v * 8;
    }
    let mut planes = Map::new(total)?;
    let mut img = Image::new(w, h).ok_or(ImgError::Alloc)?;

    // ---- giai ma entropy ----
    {
        let pl = planes.s();
        let mut br = JBits { d, pos, cur: 0, cnt: 0, marker: false };
        let mut coef = [0i32; 64];
        let mut mcu_n = 0usize;
        for my in 0..mcuy {
            for mx in 0..mcux {
                if ri > 0 && mcu_n > 0 && mcu_n % ri == 0 {
                    br.restart();
                    for c in comps[..nc].iter_mut() {
                        c.pred = 0;
                    }
                }
                for ci in 0..nc {
                    let c = comps[ci];
                    let mut pred = c.pred;
                    for by in 0..c.v {
                        for bx in 0..c.h {
                            decode_block(&mut br, &dct[c.td], &act[c.ta], &q[c.tq], &mut pred, &mut coef).ok_or(f)?;
                            let o = (my * c.v + by) * 8 * c.pw + (mx * c.h + bx) * 8;
                            idct(&coef, &mut pl[c.off + o..], c.pw);
                        }
                    }
                    comps[ci].pred = pred;
                }
                mcu_n += 1;
            }
        }
    }

    // ---- ghep plane -> 0xRRGGBB (nearest upsample) ----
    let pl = planes.s();
    let out = img.pixels_mut();
    for y in 0..h {
        for x in 0..w {
            let mut s = [0i32; 3];
            for ci in 0..nc {
                let c = &comps[ci];
                s[ci] = pl[c.off + (y * c.v / vmax) * c.pw + x * c.h / hmax] as i32;
            }
            out[y * w + x] = if nc == 1 {
                rgb(s[0] as u8, s[0] as u8, s[0] as u8)
            } else {
                let (cb, cr) = (s[1] - 128, s[2] - 128);
                let r = s[0] + ((91881 * cr) >> 16);
                let g = s[0] - ((22554 * cb + 46802 * cr) >> 16);
                let b = s[0] + ((116130 * cb) >> 16);
                rgb(clamp8(r), clamp8(g), clamp8(b))
            };
        }
    }
    Ok(img)
}