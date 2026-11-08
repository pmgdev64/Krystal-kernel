//! Hinh nen desktop: doc PNG/JPEG, cat "cover", phong to bang bilinear / thu nho bang trung binh theo dien tich.
use libgfx::sys;

const PATHS: [&[u8]; 2] = [
    b"/resources/desktop/background.png\0", // rd0://resources/desktop/background.png
    b"/resources/background.png\0",
];

fn bytes(w: i32, h: i32) -> usize {
    ((w * h * 4) as usize + 4095) & !4095
}

fn load_any() -> Result<libgfx::bmp::Image, libgfx::img::ImgError> {
    let mut last = Err(libgfx::img::ImgError::Open);
    for p in PATHS.iter() {
        match libgfx::img::load(p) {
            Ok(i) => return Ok(i),
            Err(e) => {
                sys::log("swm: thu ");
                sys::log(core::str::from_utf8(&p[..p.len() - 1]).unwrap_or("?"));
                sys::log(" -> ");
                sys::log(e.as_str());
                sys::log("\n");
                last = Err(e);
            }
        }
    }
    last
}

#[inline]
fn lerp_c(a: u32, b: u32, t: u32) -> u32 {
    (a * (256 - t) + b * t) >> 8
}

/// Tra ve buffer w*h 0xRRGGBB, hoac null neu khong doc duoc anh (swm quay lai gradient).
pub fn make(w: i32, h: i32) -> *mut u32 {
    let img = match load_any() {
        Ok(i) => i,
        Err(_) => return core::ptr::null_mut(),
    };
    let (iw, ih) = (img.w as i64, img.h as i64);
    let (wl, hl) = (w as i64, h as i64);
    if iw == 0 || ih == 0 || wl == 0 || hl == 0 {
        return core::ptr::null_mut();
    }
    let p = sys::mmap_anon(bytes(w, h)) as *mut u32;
    if p.is_null() {
        return p;
    }
    // cover: giu ti le, cat phan thua o giua
    let (cx0, cy0, cw, ch);
    if iw * hl > ih * wl {
        cw = (ih * wl / hl).max(1);
        ch = ih;
        cx0 = (iw - cw) / 2;
        cy0 = 0;
    } else {
        cw = iw;
        ch = (iw * hl / wl).max(1);
        cx0 = 0;
        cy0 = (ih - ch) / 2;
    }
    let src = img.pixels();
    let up = cw <= wl && ch <= hl; // phong to (hoac bang) -> bilinear

    for y in 0..hl {
        if up {
            let sy = (((2 * y + 1) * ch * 128 / hl) - 128).max(0);
            let y0 = (sy >> 8).min(ch - 1);
            let y1 = (y0 + 1).min(ch - 1);
            let fy = (sy & 255) as u32;
            let r0 = ((cy0 + y0) * iw + cx0) as usize;
            let r1 = ((cy0 + y1) * iw + cx0) as usize;
            for x in 0..wl {
                let sx = (((2 * x + 1) * cw * 128 / wl) - 128).max(0);
                let x0 = (sx >> 8).min(cw - 1) as usize;
                let x1 = ((sx >> 8) + 1).min(cw - 1) as usize;
                let fx = (sx & 255) as u32;
                let (a, b, c, d) = (src[r0 + x0], src[r0 + x1], src[r1 + x0], src[r1 + x1]);
                let mut out = 0u32;
                for sh in [16u32, 8, 0] {
                    let (pa, pb, pc, pd) = ((a >> sh) & 255, (b >> sh) & 255, (c >> sh) & 255, (d >> sh) & 255);
                    out |= lerp_c(lerp_c(pa, pb, fx), lerp_c(pc, pd, fx), fy) << sh;
                }
                unsafe { *p.add((y * wl + x) as usize) = out };
            }
        } else {
            // thu nho: trung binh theo dien tich chinh xac (fixed-point 1/256 pixel nguon)
            let ya = y * ch * 256 / hl;
            let yb = (y + 1) * ch * 256 / hl;
            let yy0 = ya >> 8;
            let yy1 = ((yb + 255) >> 8).min(ch);
            for x in 0..wl {
                let xa = x * cw * 256 / wl;
                let xb = (x + 1) * cw * 256 / wl;
                let xx0 = xa >> 8;
                let xx1 = ((xb + 255) >> 8).min(cw);
                let (mut r, mut g, mut b, mut wsum) = (0u64, 0u64, 0u64, 0u64);
                for yy in yy0..yy1 {
                    let wy = (yb.min((yy + 1) << 8) - ya.max(yy << 8)).max(0) as u64;
                    if wy == 0 {
                        continue;
                    }
                    let row = ((cy0 + yy) * iw + cx0) as usize;
                    for xx in xx0..xx1 {
                        let wx = (xb.min((xx + 1) << 8) - xa.max(xx << 8)).max(0) as u64;
                        let wgt = wx * wy;
                        let c = src[row + xx as usize];
                        r += (((c >> 16) & 255) as u64) * wgt;
                        g += (((c >> 8) & 255) as u64) * wgt;
                        b += ((c & 255) as u64) * wgt;
                        wsum += wgt;
                    }
                }
                let wsum = wsum.max(1);
                let v = (((r + wsum / 2) / wsum) as u32) << 16
                    | (((g + wsum / 2) / wsum) as u32) << 8
                    | (((b + wsum / 2) / wsum) as u32);
                unsafe { *p.add((y * wl + x) as usize) = v };
            }
        }
    }
    p // img drop o day -> giai phong anh goc
}

pub fn free(p: *mut u32, w: i32, h: i32) {
    if !p.is_null() {
        sys::munmap(p as *mut u8, bytes(w, h));
    }
}
