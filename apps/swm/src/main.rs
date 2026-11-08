//! swm: window manager. Chạy lúc boot, giữ framebuffer, ghép cửa sổ của các client (libswmapi).
//! Có thu nhỏ / phóng to / đóng, taskbar có nút cho từng cửa sổ.
#![no_std]
#![no_main]

use core::panic::PanicInfo;
use libgfx::{glyph_col, rgb, sys, Gfx};
use libswmapi::proto::*;

libgfx::entry!(main);
mod wall;

const MAXW: usize = 8;
const B: i32 = 2; // viền
const TB: i32 = 32; // title bar
const TASK: i32 = 40; // taskbar
const TASK_X: i32 = 8; // nut cua so tren taskbar
const ICON_W: i32 = 88;
const ICON_H: i32 = 96;
const ICON_X: i32 = 12;
const ICON_Y: i32 = 12;
const DBL: u32 = 300; // so vong lap toi da giua 2 click de tinh double-click
const C1: u32 = rgb(14, 18, 44);
const C2: u32 = rgb(72, 34, 96);

const APPS: [(&[u8], &[u8]); 5] = [
    (b"Demo", b"/apps/wapp.elf\0"),
    (b"Monitor", b"/apps/monitor.elf\0"),
    (b"Files", b"/apps/files.elf\0"),
    (b"Image", b"/apps/imgview.elf\0"),
    (b"Term", b"/apps/term.elf\0"),
];

#[derive(Clone, Copy)]
struct R {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> R {
    R { x0: x, y0: y, x1: x + w, y1: y + h }
}

#[derive(Clone, Copy)]
struct Win {
    used: bool,
    created: bool,
    min: bool,
    maxed: bool,
    pend: bool, // đang chờ client xác nhận resize
    pmax: bool, // trạng thái sau khi resize xong
    rfd: i64,
    wfd: i64,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    rx: i32, // vị trí/kích thước trước khi maximize
    ry: i32,
    rw: i32,
    rh: i32,
    px: i32, // vị trí sau khi resize xong
    py: i32,
    buf: *mut u32,
    len: usize,
    title: [u8; 32],
}

const EMPTY: Win = Win {
    used: false,
    created: false,
    min: false,
    maxed: false,
    pend: false,
    pmax: false,
    rfd: -1,
    wfd: -1,
    x: 0,
    y: 0,
    w: 0,
    h: 0,
    rx: 0,
    ry: 0,
    rw: 0,
    rh: 0,
    px: 0,
    py: 0,
    buf: core::ptr::null_mut(),
    len: 0,
    title: [0; 32],
};

fn bg(y: i32, h: i32) -> u32 {
    let t = (y.max(0) as u32) * 255 / (h.max(1) as u32);
    let mix = |s: u32| ((C1 >> s & 255) * (255 - t) + (C2 >> s & 255) * t) / 255;
    (mix(16) << 16) | (mix(8) << 8) | mix(0)
}

fn rd(fd: i64, buf: &mut [u8]) -> bool {
    let mut off = 0;
    while off < buf.len() {
        let r = sys::read(fd, &mut buf[off..]);
        if r <= 0 {
            return false;
        }
        off += r as usize;
    }
    true
}

fn skip(fd: i64, mut n: usize) -> bool {
    let mut t = [0u8; 256];
    while n > 0 {
        let k = n.min(256);
        if !rd(fd, &mut t[..k]) {
            return false;
        }
        n -= k;
    }
    true
}

fn sched_yield() {
    unsafe { sys::sc(24, 0, 0, 0, 0, 0, 0) };
}

fn sleep_ms(ms: u64) {
    let ts = [(ms / 1000) as i64, ((ms % 1000) * 1_000_000) as i64];
    unsafe { sys::sc(35, ts.as_ptr() as i64, 0, 0, 0, 0, 0) };
}

/// Nút trên title bar: k = 0 đóng, 1 phóng to, 2 thu nhỏ. Trả (x, y) góc trên trái.
fn tbtn(o: R, k: i32) -> (i32, i32) {
    (o.x1 - B - 28 - k * 28, o.y0 + 4)
}

fn on_btn(o: R, k: i32, mx: i32, my: i32) -> bool {
    let (bx, by) = tbtn(o, k);
    mx >= bx && mx < bx + 24 && my >= by && my < by + 24
}

struct Wm {
    g: Gfx,
    sc: *mut u32, // scene: desktop + cửa sổ, không có con trỏ
    w: i32,
    h: i32,
    c: R,
    wins: [Win; MAXW],
    order: [usize; MAXW], // đáy -> đỉnh (kể cả cửa sổ đang thu nhỏ)
    nord: usize,
    dirty: [R; 6],
    nd: usize,
    drag: Option<(usize, i32, i32)>,
    capture: Option<usize>,
    rsz: Option<usize>,
    spawned: i32,
    quit: bool,
    modebuf: [u8; 160],
    modelen: usize,
    tick: u32,
    wall: *mut u32,
    sel: i32,
    sel_tick: u32,
}

impl Wm {
    // ---------- vẽ vào scene ----------
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        let x0 = x.max(self.c.x0);
        let y0 = y.max(self.c.y0);
        let x1 = (x + w).min(self.c.x1);
        let y1 = (y + h).min(self.c.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for yy in y0..y1 {
            unsafe { libgfx::fast::fill32(self.sc.add((yy * self.w + x0) as usize), c, (x1 - x0) as usize) };
        }
    }

    fn text(&mut self, x: i32, y: i32, s: &[u8], c: u32) {
        {
            let n = s.iter().position(|&b| b == 0).unwrap_or(s.len());
            if let Ok(st) = core::str::from_utf8(&s[..n]) {
                let mut ad = TtfSc { sc: self.sc, w: self.w, x0: self.c.x0, y0: self.c.y0, x1: self.c.x1, y1: self.c.y1 };
                if libgfx::ttf_text(&mut ad, x, y, st, c, 2).is_some() {
                    return;
                }
            }
        }
        let mut cx = x;
        for &ch in s {
            if ch == 0 {
                break;
            }
            if cx + 6 > self.c.x0 && cx < self.c.x1 {
                for col in 0..5usize {
                    let px = cx + col as i32;
                    if px < self.c.x0 || px >= self.c.x1 {
                        continue;
                    }
                    let bits = glyph_col(ch, col);
                    for row in 0..8i32 {
                        let py = y + row;
                        if (bits >> row) & 1 != 0 && py >= self.c.y0 && py < self.c.y1 {
                            unsafe { *self.sc.add((py * self.w + px) as usize) = c };
                        }
                    }
                }
            }
            cx += 6;
        }
    }

    fn blit(&mut self, src: *const u32, sw: i32, sh: i32, dx: i32, dy: i32) {
        let x0 = dx.max(self.c.x0);
        let x1 = (dx + sw).min(self.c.x1);
        let y0 = dy.max(self.c.y0);
        let y1 = (dy + sh).min(self.c.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for yy in y0..y1 {
            unsafe {
                libgfx::fast::copy32(
                    self.sc.add((yy * self.w + x0) as usize),
                    src.add(((yy - dy) * sw + (x0 - dx)) as usize),
                    (x1 - x0) as usize,
                );
            }
        }
    }

    fn outer(&self, i: usize) -> R {
        let k = &self.wins[i];
        rect(k.x, k.y, k.w + 2 * B, k.h + TB + B)
    }

    /// Cửa sổ trên cùng đang hiển thị (không tính cửa sổ thu nhỏ).
    fn top(&self) -> Option<usize> {
        for k in (0..self.nord).rev() {
            let i = self.order[k];
            if !self.wins[i].min {
                return Some(i);
            }
        }
        None
    }

    fn tw(&self, s: &[u8]) -> i32 {
        let n = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        match core::str::from_utf8(&s[..n]) {
            Ok(st) => libgfx::ttf_text_width(st, 2).unwrap_or(n as i32 * 12),
            Err(_) => n as i32 * 12,
        }
    }

    /// Chu canh giua trong hinh chu nhat (x, y, w, h).
    fn text_c(&mut self, x: i32, y: i32, w: i32, h: i32, s: &[u8], c: u32) {
        let tx = x + (w - self.tw(s)) / 2;
        self.text(tx, y + (h - 16) / 2, s, c);
    }

    /// Hinh chu nhat bo goc ban kinh r.
    fn rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, c: u32) {
        fn isqrt(n: i32) -> i32 {
            let mut r = 0;
            while (r + 1) * (r + 1) <= n {
                r += 1;
            }
            r
        }
        for k in 0..r {
            let d = r - k;
            let inset = r - isqrt(r * r - d * d);
            self.fill(x + inset, y + k, w - 2 * inset, 1, c);
            self.fill(x + inset, y + h - 1 - k, w - 2 * inset, 1, c);
        }
        self.fill(x, y + r, w, h - 2 * r, c);
    }

    /// Doan thang khu rang cua, toa do theo 1/16 pixel. Day = 2*half/16 px, dau tron.
    fn aa_seg(&mut self, ax: i32, ay: i32, bx: i32, by: i32, half: i32, c: u32) {
        fn isqrt(n: i64) -> i64 {
            if n <= 0 {
                return 0;
            }
            let mut x = n;
            let mut y = (x + 1) / 2;
            while y < x {
                x = y;
                y = (x + n / x) / 2;
            }
            x
        }
        let (sr, sg, sb) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
        let pad = half + 16;
        let x0 = ((ax.min(bx) - pad) >> 4).max(self.c.x0);
        let x1 = (((ax.max(bx) + pad) >> 4) + 1).min(self.c.x1);
        let y0 = ((ay.min(by) - pad) >> 4).max(self.c.y0);
        let y1 = (((ay.max(by) + pad) >> 4) + 1).min(self.c.y1);
        let (dx, dy) = ((bx - ax) as i64, (by - ay) as i64);
        let len2 = (dx * dx + dy * dy).max(1);
        for py in y0..y1 {
            for px in x0..x1 {
                let (qx, qy) = ((px * 16 + 8 - ax) as i64, (py * 16 + 8 - ay) as i64);
                let t = ((qx * dx + qy * dy) * 256 / len2).clamp(0, 256);
                let (ex, ey) = (qx - dx * t / 256, qy - dy * t / 256);
                let d = isqrt(ex * ex + ey * ey) as i32;
                let cov = ((half + 8 - d) * 255 / 16).clamp(0, 255) as u32;
                if cov == 0 {
                    continue;
                }
                unsafe {
                    let p = self.sc.add((py * self.w + px) as usize);
                    if cov == 255 {
                        *p = c & 0x00FF_FFFF;
                    } else {
                        let ia = 255 - cov;
                        let dd = *p;
                        let r = (sr * cov + ((dd >> 16) & 255) * ia + 127) / 255;
                        let g = (sg * cov + ((dd >> 8) & 255) * ia + 127) / 255;
                        let b = (sb * cov + (dd & 255) * ia + 127) / 255;
                        *p = (r << 16) | (g << 8) | b;
                    }
                }
            }
        }
    }

    fn draw_win(&mut self, i: usize, focused: bool) {
        let (x, y, w, h, buf, title, maxed) = {
            let k = &self.wins[i];
            (k.x, k.y, k.w, k.h, k.buf, k.title, k.maxed)
        };
        let ow = w + 2 * B;
        let frame = if focused { rgb(88, 112, 230) } else { rgb(58, 62, 88) };
        let bar = if focused { rgb(34, 38, 62) } else { rgb(28, 30, 46) };
        let tcol = if focused { rgb(240, 242, 255) } else { rgb(140, 144, 168) };
        // title bar: gradient doc + vach chia + vien
        let (t0, t1) = if focused { (rgb(48, 54, 90), rgb(30, 34, 58)) } else { (rgb(36, 38, 58), rgb(26, 28, 42)) };
        for r in 0..TB {
            let t = (r * 255 / (TB - 1)) as u32;
            let mix = |s: u32| (((t0 >> s) & 255) * (255 - t) + ((t1 >> s) & 255) * t) / 255;
            self.fill(x, y + r, ow, 1, (mix(16) << 16) | (mix(8) << 8) | mix(0));
        }
        if focused {
            self.fill(x, y, ow, 2, frame);
        }
        self.fill(x, y + TB - 1, ow, 1, rgb(18, 20, 32));
        self.fill(x, y + TB, B, h + B, frame);
        self.fill(x + ow - B, y + TB, B, h + B, frame);
        self.fill(x + B, y + TB + h, w, B, frame);
        self.text(x + 10, y + (TB - 16) / 2 + 1, &title, tcol);
        // nut: 0 dong, 1 phong to, 2 thu nho (icon ve bang net khu rang cua)
        let o = rect(x, y, ow, h + TB + B);
        let ic = if focused { rgb(240, 242, 255) } else { rgb(170, 174, 198) };
        let hf = 14; // nua be day net (1/16 px) ~ 1.75 px
        for k in 0..3 {
            let (bx, by) = tbtn(o, k);
            let col = if k == 0 { rgb(214, 70, 84) } else { rgb(62, 68, 104) };
            self.rrect(bx, by, 24, 24, 6, col);
            let (cx, cy) = ((bx + 12) * 16, (by + 12) * 16);
            match k {
                0 => {
                    self.aa_seg(cx - 64, cy - 64, cx + 64, cy + 64, hf, ic);
                    self.aa_seg(cx + 64, cy - 64, cx - 64, cy + 64, hf, ic);
                }
                1 => {
                    if maxed {
                        // restore: o vuong truoc + chu L phia sau
                        self.aa_seg(cx - 40, cy - 72, cx + 72, cy - 72, hf, ic);
                        self.aa_seg(cx + 72, cy - 72, cx + 72, cy + 40, hf, ic);
                        self.aa_seg(cx - 72, cy - 40, cx + 40, cy - 40, hf, ic);
                        self.aa_seg(cx + 40, cy - 40, cx + 40, cy + 72, hf, ic);
                        self.aa_seg(cx + 40, cy + 72, cx - 72, cy + 72, hf, ic);
                        self.aa_seg(cx - 72, cy + 72, cx - 72, cy - 40, hf, ic);
                        self.aa_seg(cx - 40, cy - 72, cx - 40, cy - 40, hf, ic);
                    } else {
                        self.aa_seg(cx - 64, cy - 64, cx + 64, cy - 64, hf, ic);
                        self.aa_seg(cx + 64, cy - 64, cx + 64, cy + 64, hf, ic);
                        self.aa_seg(cx + 64, cy + 64, cx - 64, cy + 64, hf, ic);
                        self.aa_seg(cx - 64, cy + 64, cx - 64, cy - 64, hf, ic);
                    }
                }
                _ => self.aa_seg(cx - 64, cy + 32, cx + 64, cy + 32, hf, ic),
            }
        }
        self.blit(buf, w, h, x + B, y + TB);
        if !maxed {
            for r in 0..3i32 {
                for j in 0..=r {
                    self.fill(x + B + w - 5 - j * 4, y + TB + h - 5 - (2 - r) * 4, 2, 2, rgb(200, 204, 230));
                }
            }
        }
    }

    fn taskbar(&mut self) {
        let (w, h) = (self.w, self.h);
        let top = self.top();
        let ty = h - TASK;
        self.fill(0, ty, w, TASK, rgb(16, 18, 30));
        self.fill(0, ty, w, 1, rgb(48, 52, 82));
        // nut cua so
        let mut n = 0;
        for i in 0..MAXW {
            if !(self.wins[i].used && self.wins[i].created) {
                continue;
            }
            let bx = TASK_X + n * 132;
            let (col, tc) = if self.wins[i].min {
                (rgb(28, 30, 46), rgb(120, 124, 150))
            } else if top == Some(i) {
                (rgb(52, 80, 190), rgb(255, 255, 255))
            } else {
                (rgb(40, 44, 70), rgb(200, 204, 228))
            };
            self.rrect(bx, ty + 6, 128, 28, 4, col);
            if top == Some(i) && !self.wins[i].min {
                self.fill(bx + 10, ty + 32, 108, 2, rgb(130, 160, 255));
            }
            let t = self.wins[i].title;
            self.text(bx + 10, ty + 12, &t[..12], tc);
            n += 1;
        }
        // thoat
        self.rrect(w - 104, ty + 6, 96, 28, 4, rgb(176, 52, 64));
        self.text_c(w - 104, ty + 6, 96, 28, b"Quit", rgb(255, 255, 255));
    }

    /// Tron mau c len vung hien co (alpha 0..255), co clip.
    fn blend(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32, a: u32) {
        let x0 = x.max(self.c.x0);
        let y0 = y.max(self.c.y0);
        let x1 = (x + w).min(self.c.x1);
        let y1 = (y + h).min(self.c.y1);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (sr, sg, sb) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
        let ia = 255 - a;
        for yy in y0..y1 {
            for xx in x0..x1 {
                unsafe {
                    let p = self.sc.add((yy * self.w + xx) as usize);
                    let d = *p;
                    let r = (sr * a + ((d >> 16) & 255) * ia) / 255;
                    let g = (sg * a + ((d >> 8) & 255) * ia) / 255;
                    let b = (sb * a + (d & 255) * ia) / 255;
                    *p = (r << 16) | (g << 8) | b;
                }
            }
        }
    }

    /// Nhu blend nhung bo goc ban kinh r.
    fn rblend(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, c: u32, a: u32) {
        let r = r.min(w / 2).min(h / 2);
        for k in 0..h {
            let d = if k < r { r - k } else if k >= h - r { k - (h - 1 - r) } else { 0 };
            let mut inset = 0;
            if d > 0 {
                let mut q = 0;
                while (q + 1) * (q + 1) <= r * r - d * d {
                    q += 1;
                }
                inset = r - q;
            }
            self.blend(x + inset, y + k, w - 2 * inset, 1, c, a);
        }
    }

    fn icon_rect(&self, i: usize) -> R {
        let rows = ((self.h - TASK - ICON_Y) / ICON_H).max(1);
        let (col, row) = (i as i32 / rows, i as i32 % rows);
        rect(ICON_X + col * ICON_W, ICON_Y + row * ICON_H, ICON_W, ICON_H)
    }

    fn desktop_icons(&mut self) {
        for i in 0..APPS.len() {
            let r = self.icon_rect(i);
            if r.x1 <= self.c.x0 || r.x0 >= self.c.x1 || r.y1 <= self.c.y0 || r.y0 >= self.c.y1 {
                continue;
            }
            if self.sel == i as i32 {
                self.rblend(r.x0 + 2, r.y0 + 2, ICON_W - 4, ICON_H - 4, 8, rgb(120, 150, 255), 70);
            }
            let (tx, ty) = (r.x0 + (ICON_W - 48) / 2, r.y0 + 8);
            self.rblend(tx, ty + 3, 48, 48, 10, rgb(0, 0, 0), 80); // bong bo goc
            self.rrect(tx, ty, 48, 48, 10, rgb(66, 78, 128));
            self.rrect(tx, ty, 48, 24, 10, rgb(78, 92, 146)); // vang sang phia tren
            self.fill(tx, ty + 14, 48, 10, rgb(78, 92, 146));
            // icon "unknown app": cua so co title bar + dau ?
            let (cx, cy) = ((tx + 24) * 16, (ty + 24) * 16);
            let ic = rgb(236, 240, 255);
            let (hx, hy, hf) = (256, 208, 14);
            self.aa_seg(cx - hx, cy - hy, cx + hx, cy - hy, hf, ic);
            self.aa_seg(cx + hx, cy - hy, cx + hx, cy + hy, hf, ic);
            self.aa_seg(cx + hx, cy + hy, cx - hx, cy + hy, hf, ic);
            self.aa_seg(cx - hx, cy + hy, cx - hx, cy - hy, hf, ic);
            self.aa_seg(cx - hx, cy - 112, cx + hx, cy - 112, hf, ic);
            self.text_c(tx, ty + 19, 48, 16, b"?", ic);
            let tw = self.tw(APPS[i].0);
            self.rblend(r.x0 + (ICON_W - tw) / 2 - 6, r.y0 + 62, tw + 12, 22, 8, rgb(0, 0, 0), 140);
            self.text(r.x0 + (ICON_W - tw) / 2, r.y0 + 65, APPS[i].0, rgb(255, 255, 255));
        }
    }

    fn desktop_click(&mut self, mx: i32, my: i32) {
        let old = self.sel;
        let mut hit = -1;
        for i in 0..APPS.len() {
            let r = self.icon_rect(i);
            if mx >= r.x0 && mx < r.x1 && my >= r.y0 && my < r.y1 {
                hit = i as i32;
                break;
            }
        }
        if hit >= 0 && hit == old && self.tick.wrapping_sub(self.sel_tick) < DBL {
            self.sel = -1;
            self.spawn(APPS[hit as usize].1);
        } else {
            self.sel = hit;
            self.sel_tick = self.tick;
        }
        for s in [old, self.sel] {
            if s >= 0 {
                let r = self.icon_rect(s as usize);
                self.mark(r);
            }
        }
    }

    fn compose(&mut self, r: R) {
        let c = R { x0: r.x0.max(0), y0: r.y0.max(0), x1: r.x1.min(self.w), y1: r.y1.min(self.h) };
        if c.x0 >= c.x1 || c.y0 >= c.y1 {
            return;
        }
        self.c = c;
        let h = self.h;
        let mut rects = [rect(0, 0, 0, 0); MAXW];
        let mut nr = 0;
        for k in 0..self.nord {
            let i = self.order[k];
            if self.wins[i].min {
                continue;
            }
            let o = self.outer(i);
            if o.x1 > c.x0 && o.x0 < c.x1 && o.y1 > c.y0 && o.y0 < c.y1 {
                rects[nr] = o;
                nr += 1;
            }
        }
        // nền: chỉ vẽ phần không bị cửa sổ che
        let ybg = c.y1.min(h - TASK);
        for y in c.y0..ybg {
            let mut segs = [(0i32, 0i32); MAXW + 2];
            segs[0] = (c.x0, c.x1);
            let mut cnt = 1usize;
            for j in 0..nr {
                let o = rects[j];
                if y < o.y0 || y >= o.y1 {
                    continue;
                }
                let mut k = 0;
                while k < cnt {
                    let (a, b) = segs[k];
                    if o.x1 <= a || o.x0 >= b {
                        k += 1;
                        continue;
                    }
                    segs[k] = segs[cnt - 1];
                    cnt -= 1;
                    if o.x0 > a {
                        segs[cnt] = (a, o.x0);
                        cnt += 1;
                    }
                    if o.x1 < b {
                        segs[cnt] = (o.x1, b);
                        cnt += 1;
                    }
                }
            }
            let col = bg(y, h);
            for k in 0..cnt {
                if self.wall.is_null() { self.fill(segs[k].0, y, segs[k].1 - segs[k].0, 1, col); } else { unsafe { libgfx::fast::copy32(self.sc.add((y * self.w + segs[k].0) as usize), self.wall.add((y * self.w + segs[k].0) as usize), (segs[k].1 - segs[k].0) as usize) }; }
            }
        }
        let top = self.top();
        self.desktop_icons();
        for k in 0..self.nord {
            let i = self.order[k];
            if self.wins[i].min {
                continue;
            }
            let o = self.outer(i);
            if o.x1 <= c.x0 || o.x0 >= c.x1 || o.y1 <= c.y0 || o.y0 >= c.y1 {
                continue;
            }
            self.draw_win(i, top == Some(i));
        }
        if c.y1 > h - TASK {
            self.taskbar();
        }
    }

    fn flush(&mut self, r: R) {
        let x0 = r.x0.max(0);
        let y0 = r.y0.max(0);
        let x1 = r.x1.min(self.w);
        let y1 = r.y1.min(self.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (mx, my) = (self.g.mouse_x(), self.g.mouse_y());
        let (cw, chh) = self.g.cursor_size();
        let cur = mx < x1 && mx + cw > x0 && my < y1 && my + chh > y0;
        if self.g.direct_fb() {
            self.g.blit_scene_fb(self.sc as *const u32, self.w as usize, x0 as usize, y0 as usize, x1 as usize, y1 as usize);
            if cur {
                self.g.cursor_fb();
            }
            return;
        }
        self.g.copy_from(self.sc as *const u32, self.w as usize, x0 as usize, y0 as usize, x1 as usize, y1 as usize);
        if cur {
            self.g.draw_cursor();
        }
        let _ = self.g.present();
    }

    fn mark(&mut self, r: R) {
        let mut r = r;
        let mut i = 0;
        while i < self.nd {
            let a = self.dirty[i];
            if r.x0 <= a.x1 && a.x0 <= r.x1 && r.y0 <= a.y1 && a.y0 <= r.y1 {
                r = R { x0: r.x0.min(a.x0), y0: r.y0.min(a.y0), x1: r.x1.max(a.x1), y1: r.y1.max(a.y1) };
                self.dirty[i] = self.dirty[self.nd - 1];
                self.nd -= 1;
                i = 0;
            } else {
                i += 1;
            }
        }
        if self.nd < 6 {
            self.dirty[self.nd] = r;
            self.nd += 1;
        } else {
            let mut u = r;
            for k in 0..self.nd {
                let a = self.dirty[k];
                u = R { x0: u.x0.min(a.x0), y0: u.y0.min(a.y0), x1: u.x1.max(a.x1), y1: u.y1.max(a.y1) };
            }
            self.dirty[0] = u;
            self.nd = 1;
        }
    }

    fn mark_taskbar(&mut self) {
        let r = rect(0, self.h - TASK, self.w, TASK);
        self.mark(r);
    }

    // ---------- cửa sổ / z-order ----------
    fn send(&mut self, i: usize, ty: u32, a: i32, b: i32, c: i32, d: i32) {
        let m = Msg::new(ty, a, b, c, d, 0);
        sys::write(self.wins[i].wfd, &m.encode());
    }

    fn hit(&self, x: i32, y: i32) -> Option<usize> {
        for k in (0..self.nord).rev() {
            let i = self.order[k];
            if self.wins[i].min {
                continue;
            }
            let o = self.outer(i);
            if x >= o.x0 && x < o.x1 && y >= o.y0 && y < o.y1 {
                return Some(i);
            }
        }
        None
    }

    fn in_client(&self, i: usize, x: i32, y: i32) -> bool {
        let k = &self.wins[i];
        x >= k.x + B && x < k.x + B + k.w && y >= k.y + TB && y < k.y + TB + k.h
    }

    fn unorder(&mut self, i: usize) {
        let mut k = 0;
        while k < self.nord && self.order[k] != i {
            k += 1;
        }
        if k == self.nord {
            return;
        }
        while k + 1 < self.nord {
            self.order[k] = self.order[k + 1];
            k += 1;
        }
        self.nord -= 1;
    }

    /// Đưa cửa sổ lên trên cùng (và bỏ trạng thái thu nhỏ).
    fn activate(&mut self, i: usize) {
        let was = self.top();
        if was == Some(i) {
            return;
        }
        self.wins[i].min = false;
        self.unorder(i);
        self.order[self.nord] = i;
        self.nord += 1;
        if let Some(o) = was {
            self.send(o, S_FOCUS, 0, 0, 0, 0);
            let r = self.outer(o);
            self.mark(r);
        }
        self.send(i, S_FOCUS, 1, 0, 0, 0);
        let r = self.outer(i);
        self.mark(r);
        self.mark_taskbar();
    }

    fn minimize(&mut self, i: usize) {
        if self.wins[i].min {
            return;
        }
        let was_top = self.top() == Some(i);
        self.wins[i].min = true;
        let r = self.outer(i);
        self.mark(r);
        if self.drag.map_or(false, |d| d.0 == i) {
            self.drag = None;
        }
        if self.capture == Some(i) {
            self.capture = None;
        }
        self.send(i, S_FOCUS, 0, 0, 0, 0);
        if was_top {
            if let Some(t) = self.top() {
                self.send(t, S_FOCUS, 1, 0, 0, 0);
                let r = self.outer(t);
                self.mark(r);
            }
        }
        self.mark_taskbar();
    }

    /// Xin client đổi kích thước; cửa sổ chỉ đổi khi client gửi C_RESIZE.
    fn toggle_max(&mut self, i: usize) {
        if self.wins[i].pend {
            return;
        }
        let (tw, th, px, py, pmax);
        if !self.wins[i].maxed {
            let k = &mut self.wins[i];
            k.rx = k.x;
            k.ry = k.y;
            k.rw = k.w;
            k.rh = k.h;
            tw = self.w - 2 * B;
            th = self.h - TASK - TB - B;
            px = 0;
            py = 0;
            pmax = true;
        } else {
            let k = &self.wins[i];
            tw = k.rw;
            th = k.rh;
            px = k.rx;
            py = k.ry;
            pmax = false;
        }
        let k = &mut self.wins[i];
        k.pend = true;
        k.pmax = pmax;
        k.px = px;
        k.py = py;
        self.send(i, S_RESIZE, tw, th, 0, 0);
    }

    fn drop_client(&mut self, i: usize) {
        if !self.wins[i].used {
            return;
        }
        let (rfd, wfd, created, buf, len) = {
            let k = &self.wins[i];
            (k.rfd, k.wfd, k.created, k.buf, k.len)
        };
        if created {
            let o = self.outer(i);
            self.mark(o);
            self.unorder(i);
            sys::munmap(buf as *mut u8, len);
        }
        sys::close(rfd);
        sys::close(wfd);
        self.wins[i] = EMPTY;
        if self.drag.map_or(false, |d| d.0 == i) {
            self.drag = None;
        }
        if self.capture == Some(i) {
            self.capture = None;
        }
        if let Some(t) = self.top() {
            self.send(t, S_FOCUS, 1, 0, 0, 0);
            let o = self.outer(t);
            self.mark(o);
        }
        self.mark_taskbar();
        loop {
            let r = unsafe { sys::sc(61, -1, 0, 1, 0, 0, 0) };
            if r <= 0 {
                break;
            }
        }
    }

    // ---------- khởi chạy client ----------
    fn spawn(&mut self, path: &'static [u8]) {
        let slot = match (0..MAXW).find(|&i| !self.wins[i].used) {
            Some(i) => i,
            None => return,
        };
        let mut p1 = [0i32; 2];
        let mut p2 = [0i32; 2];
        unsafe {
            if sys::sc(22, p1.as_mut_ptr() as i64, 0, 0, 0, 0, 0) < 0 {
                return;
            }
            if sys::sc(22, p2.as_mut_ptr() as i64, 0, 0, 0, 0, 0) < 0 {
                sys::close(p1[0] as i64);
                sys::close(p1[1] as i64);
                return;
            }
            let (r1, w1, r2, w2) = (p1[0] as i64, p1[1] as i64, p2[0] as i64, p2[1] as i64);
            let pid = sys::sc(57, 0, 0, 0, 0, 0, 0);
            if pid == 0 {
                sys::sc(33, w1, 30, 0, 0, 0, 0);
                sys::sc(33, r2, 31, 0, 0, 0, 0);
                for fd in 3..30 {
                    sys::close(fd);
                }
                sys::sc(33, 30, 3, 0, 0, 0, 0);
                sys::sc(33, 31, 4, 0, 0, 0, 0);
                sys::close(30);
                sys::close(31);
                let argv = [path.as_ptr() as u64, 0u64];
                sys::sc(59, path.as_ptr() as i64, argv.as_ptr() as i64, 0, 0, 0, 0);
                sys::exit(127);
            }
            sys::close(w1);
            sys::close(r2);
            if pid < 0 {
                sys::close(r1);
                sys::close(w2);
                return;
            }
            self.wins[slot] = Win { used: true, rfd: r1, wfd: w2, ..EMPTY };
        }
    }

    // ---------- nhận message từ client ----------
    fn pump(&mut self) {
        for _ in 0..8 {
            let mut pf = [0u8; 8 * MAXW];
            let mut idx = [0usize; MAXW];
            let mut n = 0;
            for i in 0..MAXW {
                if self.wins[i].used {
                    let o = n * 8;
                    pf[o..o + 4].copy_from_slice(&(self.wins[i].rfd as i32).to_le_bytes());
                    pf[o + 4..o + 6].copy_from_slice(&1i16.to_le_bytes());
                    idx[n] = i;
                    n += 1;
                }
            }
            if n == 0 {
                return;
            }
            let r = unsafe { sys::sc(7, pf.as_mut_ptr() as i64, n as i64, 0, 0, 0, 0) };
            if r <= 0 {
                return;
            }
            for k in 0..n {
                let i = idx[k];
                if !self.wins[i].used {
                    continue;
                }
                let rev = i16::from_le_bytes([pf[k * 8 + 6], pf[k * 8 + 7]]);
                if rev & 1 != 0 {
                    self.handle(i);
                } else if rev & 0x38 != 0 {
                    self.drop_client(i);
                }
            }
        }
    }

    fn handle(&mut self, i: usize) {
        let rfd = self.wins[i].rfd;
        let mut hb = [0u8; HDR];
        if !rd(rfd, &mut hb) {
            self.drop_client(i);
            return;
        }
        let m = Msg::decode(&hb);
        if m.len > (64 << 20) {
            self.drop_client(i);
            return;
        }
        match m.ty {
            C_CREATE => self.on_create(i, &m),
            C_PIXELS => self.on_pixels(i, &m),
            C_RESIZE => {
                if m.len != 0 && !skip(rfd, m.len as usize) {
                    self.drop_client(i);
                    return;
                }
                self.on_resize(i, &m);
            }
            C_TITLE => {
                let mut t = [0u8; 32];
                if m.len == 32 {
                    if !rd(rfd, &mut t) {
                        self.drop_client(i);
                        return;
                    }
                    t[31] = 0;
                    self.wins[i].title = t;
                    if self.wins[i].created {
                        let o = self.outer(i);
                        self.mark(o);
                        self.mark_taskbar();
                    }
                } else if !skip(rfd, m.len as usize) {
                    self.drop_client(i);
                }
            }
            C_CLOSE => self.drop_client(i),
            _ => {
                if !skip(rfd, m.len as usize) {
                    self.drop_client(i);
                }
            }
        }
    }

    fn on_create(&mut self, i: usize, m: &Msg) {
        let rfd = self.wins[i].rfd;
        let mut t = [0u8; 32];
        if m.len == 32 {
            if !rd(rfd, &mut t) {
                self.drop_client(i);
                return;
            }
            t[31] = 0;
        } else {
            if !skip(rfd, m.len as usize) {
                self.drop_client(i);
                return;
            }
            t[..3].copy_from_slice(b"app");
        }
        if self.wins[i].created {
            return;
        }
        let maxw = self.w - 2 * B;
        let maxh = self.h - TASK - TB - B;
        if m.a < 16 || m.b < 16 || m.a > maxw || m.b > maxh {
            self.drop_client(i);
            return;
        }
        let len = ((m.a as usize * m.b as usize * 4) + 4095) & !4095;
        let p = sys::mmap_anon(len) as *mut u32;
        if p.is_null() {
            self.drop_client(i);
            return;
        }
        unsafe { libgfx::fast::fill32(p, 0x00EC_ECF2, (m.a * m.b) as usize) };
        let n = self.spawned;
        self.spawned += 1;
        let ow = m.a + 2 * B;
        let oh = m.b + TB + B;
        let x = (40 + (n % 8) * 30).min(self.w - ow).max(0);
        let y = (30 + (n % 8) * 26).min(self.h - TASK - oh).max(0);
        {
            let k = &mut self.wins[i];
            k.w = m.a;
            k.h = m.b;
            k.x = x;
            k.y = y;
            k.buf = p;
            k.len = len;
            k.title = t;
            k.created = true;
        }
        let old = self.top();
        self.order[self.nord] = i;
        self.nord += 1;
        self.send(i, S_ACK, 0, 0, 0, 0);
        if let Some(o) = old {
            self.send(o, S_FOCUS, 0, 0, 0, 0);
            let r = self.outer(o);
            self.mark(r); // ve lai title bar sang mau mat focus
        }
        self.send(i, S_FOCUS, 1, 0, 0, 0);
        let o = self.outer(i);
        self.mark(o);
        self.mark_taskbar();
    }

    fn on_resize(&mut self, i: usize, m: &Msg) {
        if !self.wins[i].created {
            return;
        }
        let maxw = self.w - 2 * B;
        let maxh = self.h - TASK - TB - B;
        if m.a < 16 || m.b < 16 || m.a > maxw || m.b > maxh {
            self.drop_client(i);
            return;
        }
        let len = ((m.a as usize * m.b as usize * 4) + 4095) & !4095;
        let p = sys::mmap_anon(len) as *mut u32;
        if p.is_null() {
            self.drop_client(i);
            return;
        }
        unsafe { libgfx::fast::fill32(p, 0x00EC_ECF2, (m.a * m.b) as usize) };
        let old = self.outer(i);
        self.mark(old);
        let (ob, ol) = (self.wins[i].buf, self.wins[i].len);
        sys::munmap(ob as *mut u8, ol);
        {
            let k = &mut self.wins[i];
            k.buf = p;
            k.len = len;
            k.w = m.a;
            k.h = m.b;
            if k.pend {
                k.x = k.px;
                k.y = k.py;
                k.maxed = k.pmax;
                k.pend = false;
            }
        }
        // giữ cửa sổ trong màn hình
        let ow = m.a + 2 * B;
        let oh = m.b + TB + B;
        let nx = self.wins[i].x.min(self.w - ow).max(0);
        let ny = self.wins[i].y.min(self.h - TASK - oh).max(0);
        self.wins[i].x = nx;
        self.wins[i].y = ny;
        let o = self.outer(i);
        self.mark(o);
    }

    fn on_pixels(&mut self, i: usize, m: &Msg) {
        let rfd = self.wins[i].rfd;
        let (x, y, cw, ch) = (m.a, m.b, m.c, m.d);
        let (w, h, buf, created) = {
            let k = &self.wins[i];
            (k.w, k.h, k.buf, k.created)
        };
        let ok = created
            && cw > 0
            && ch > 0
            && x >= 0
            && y >= 0
            && x + cw <= w
            && y + ch <= h
            && m.len as i64 == cw as i64 * ch as i64 * 4;
        if !ok {
            if !skip(rfd, m.len as usize) {
                self.drop_client(i);
            }
            return;
        }
        if x == 0 && cw == w {
            // vung full-width: doc 1 lan thang vao buffer
            let p = unsafe { buf.add((y * w) as usize) } as *mut u8;
            let s = unsafe { core::slice::from_raw_parts_mut(p, cw as usize * ch as usize * 4) };
            if !rd(rfd, s) {
                self.drop_client(i);
                return;
            }
        } else {
            for row in 0..ch {
                let p = unsafe { buf.add(((y + row) * w + x) as usize) } as *mut u8;
                let s = unsafe { core::slice::from_raw_parts_mut(p, cw as usize * 4) };
                if !rd(rfd, s) {
                    self.drop_client(i);
                    return;
                }
            }
        }
        if self.wins[i].min {
            return; // chỉ cập nhật buffer, không vẽ
        }
        let (wx, wy) = (self.wins[i].x, self.wins[i].y);
        let reg = rect(wx + B + x, wy + TB + y, cw, ch);
        let grip = !self.wins[i].maxed && reg.x1 > wx + B + w - 20 && reg.y1 > wy + TB + h - 20;
        if self.top() == Some(i) && self.nd == 0 && !grip {
            // cửa sổ trên cùng: blit thẳng vào scene, không compose lại
            self.c = reg;
            self.blit(buf, w, h, wx + B, wy + TB);
            self.flush(reg);
            return;
        }
        self.mark(reg);
    }

    // ---------- doi do phan giai ----------
    fn mode_changed(&mut self) -> bool {
        let mut b = [0u8; 160];
        let fd = sys::open(b"devices://mode\0", 0);
        if fd < 0 {
            return false;
        }
        let n = sys::read(fd, &mut b);
        sys::close(fd);
        if n <= 0 {
            return false;
        }
        let n = n as usize;
        let first = self.modelen == 0;
        let changed = !first && (n != self.modelen || b[..n] != self.modebuf[..n]);
        if first || changed {
            self.modebuf[..n].copy_from_slice(&b[..n]);
            self.modelen = n;
        }
        changed
    }

    fn remode(&mut self) {
        let g = match Gfx::open() {
            Ok(g) => g,
            Err(_) => return,
        };
        let (w, h) = (g.w as i32, g.h as i32);
        let len = ((w * h * 4) as usize + 4095) & !4095;
        let sc = sys::mmap_anon(len) as *mut u32;
        if sc.is_null() {
            return;
        }
        let old_len = ((self.w * self.h * 4) as usize + 4095) & !4095;
        sys::munmap(self.sc as *mut u8, old_len);
        wall::free(self.wall, self.w, self.h);
        self.g = g; // Gfx cu bi drop
        self.sc = sc;
        self.w = w;
        self.h = h;
        self.c = rect(0, 0, w, h);
        self.wall = wall::make(w, h);
        self.nd = 0;
        self.drag = None;
        self.capture = None;
        self.rsz = None;
        let maxw = w - 2 * B;
        let maxh = h - TASK - TB - B;
        for i in 0..MAXW {
            if !(self.wins[i].used && self.wins[i].created) || self.wins[i].pend {
                continue;
            }
            let k = self.wins[i];
            let ow = k.w + 2 * B;
            let oh = k.h + TB + B;
            if k.maxed {
                let kk = &mut self.wins[i];
                kk.pend = true;
                kk.pmax = true;
                kk.px = 0;
                kk.py = 0;
                self.send(i, S_RESIZE, maxw, maxh, 0, 0);
            } else if k.w > maxw || k.h > maxh {
                let (tw, th) = (k.w.min(maxw), k.h.min(maxh));
                let kk = &mut self.wins[i];
                kk.pend = true;
                kk.pmax = false;
                kk.px = 0;
                kk.py = 0;
                self.send(i, S_RESIZE, tw, th, 0, 0);
            } else {
                self.wins[i].x = k.x.min(w - ow).max(0);
                self.wins[i].y = k.y.min(h - TASK - oh).max(0);
            }
        }
        self.mark(rect(0, 0, w, h));
    }

    // ---------- input ----------
    fn key(&mut self, c: u8) {
        if c == 17 {
            self.quit = true; // Ctrl+Q
            return;
        }
        if let Some(t) = self.top() {
            self.send(t, S_KEY, c as i32, 0, 0, 0);
        }
    }

    fn taskbar_click(&mut self, mx: i32, my: i32) {
        let by = self.h - TASK + 6;
        if my < by || my >= by + 28 {
            return;
        }
        if mx >= self.w - 104 && mx < self.w - 8 {
            self.quit = true;
            return;
        }
        let mut n = 0;
        for i in 0..MAXW {
            if !(self.wins[i].used && self.wins[i].created) {
                continue;
            }
            let bx = TASK_X + n * 132;
            if mx >= bx && mx < bx + 128 {
                if self.wins[i].min {
                    self.activate(i);
                    self.mark_taskbar();
                } else if self.top() == Some(i) {
                    self.minimize(i);
                } else {
                    self.activate(i);
                }
                return;
            }
            n += 1;
        }
    }

    fn mouse(&mut self, mx: i32, my: i32, btn: u32, pmx: i32, pmy: i32, pbtn: u32) {
        if btn & 1 != 0 && pbtn & 1 == 0 {
            if my >= self.h - TASK {
                self.taskbar_click(mx, my);
            } else if let Some(i) = self.hit(mx, my) {
                self.activate(i);
                let o = self.outer(i);
                if my < o.y0 + TB {
                    if on_btn(o, 0, mx, my) {
                        self.send(i, S_CLOSE_REQ, 0, 0, 0, 0);
                    } else if on_btn(o, 1, mx, my) {
                        self.toggle_max(i);
                    } else if on_btn(o, 2, mx, my) {
                        self.minimize(i);
                    } else if !self.wins[i].maxed {
                        self.drag = Some((i, mx - o.x0, my - o.y0));
                    }
                } else if !self.wins[i].maxed && mx >= o.x1 - 18 && my >= o.y1 - 18 {
                    self.rsz = Some(i);
                } else {
                    self.capture = Some(i);
                }
            } else {
                self.desktop_click(mx, my);
            }
        }
        if let Some(i) = self.rsz {
            let released = btn & 1 == 0;
            if !(self.wins[i].used && self.wins[i].created) {
                self.rsz = None;
            } else if !self.wins[i].pend {
                // resize live: gui yeu cau moi ngay khi client da xac nhan yeu cau truoc
                let (kx, ky) = (self.wins[i].x, self.wins[i].y);
                let tw = (mx - kx - B).max(200).min(self.w - kx - 2 * B);
                let th = (my - ky - TB).max(120).min(self.h - TASK - ky - TB - B);
                if tw != self.wins[i].w || th != self.wins[i].h {
                    let k = &mut self.wins[i];
                    k.pend = true;
                    k.pmax = false;
                    k.px = kx;
                    k.py = ky;
                    self.send(i, S_RESIZE, tw, th, 0, 0);
                } else if released {
                    self.rsz = None;
                }
            }
            // released nhung con pend: giu rsz, vong sau gui not kich thuoc cuoi
        }
        if let Some((i, dx, dy)) = self.drag {
            if btn & 1 == 0 {
                self.drag = None;
            } else if mx != pmx || my != pmy {
                let o = self.outer(i);
                self.mark(o);
                let ow = o.x1 - o.x0;
                let nx = (mx - dx).max(40 - ow).min(self.w - 40);
                let ny = (my - dy).max(0).min(self.h - TASK - TB);
                self.wins[i].x = nx;
                self.wins[i].y = ny;
                let o = self.outer(i);
                self.mark(o);
            }
        }
        let target = if let Some(i) = self.capture {
            Some(i)
        } else if self.drag.is_none() && self.rsz.is_none() {
            match self.hit(mx, my) {
                Some(i) if self.in_client(i, mx, my) => Some(i),
                _ => None,
            }
        } else {
            None
        };
        if let Some(i) = target {
            if mx != pmx || my != pmy || btn != pbtn {
                let (cx, cy) = (self.wins[i].x + B, self.wins[i].y + TB);
                self.send(i, S_MOUSE, mx - cx, my - cy, btn as i32, 0);
            }
        }
        if btn == 0 {
            self.capture = None;
        }
    }
}

fn main() {
    let g = match Gfx::open() {
        Ok(g) => g,
        Err(_) => sys::exit(1),
    };
    let (w, h) = (g.w as i32, g.h as i32);
    let len = ((w * h * 4) as usize + 4095) & !4095;
    let sc = sys::mmap_anon(len) as *mut u32;
    if sc.is_null() {
        sys::exit(1);
    }
    let kbd = sys::open(b"devices://kbd\0", 0);
    let mut wm = Wm {
        g,
        sc,
        w,
        h,
        c: rect(0, 0, w, h),
        wins: [EMPTY; MAXW],
        order: [0; MAXW],
        nord: 0,
        dirty: [rect(0, 0, 0, 0); 6],
        nd: 0,
        drag: None,
        capture: None,
        rsz: None,
        spawned: 0,
        quit: false,
        modebuf: [0; 160],
        modelen: 0,
        tick: 0,
        wall: wall::make(w, h),
        sel: -1,
        sel_tick: 0,
    };
    wm.g.poll_mouse();
    let (mut pmx, mut pmy, mut pbtn) = (wm.g.mouse_x(), wm.g.mouse_y(), wm.g.mouse_buttons());
    wm.mark(rect(0, 0, w, h));

    loop {
        if kbd >= 0 {
            let mut b = [0u8; 16];
            loop {
                let n = sys::read(kbd, &mut b);
                if n <= 0 {
                    break;
                }
                for k in 0..n as usize {
                    wm.key(b[k]);
                }
                if n < 16 {
                    break;
                }
            }
        }
        if wm.quit {
            break;
        }
        wm.tick = wm.tick.wrapping_add(1);
        if wm.tick & 31 == 0 && wm.mode_changed() {
            wm.remode();
            pmx = wm.g.mouse_x();
            pmy = wm.g.mouse_y();
        }
        wm.g.poll_mouse();
        let (mx, my, btn) = (wm.g.mouse_x(), wm.g.mouse_y(), wm.g.mouse_buttons());
        wm.mouse(mx, my, btn, pmx, pmy, pbtn);
        if mx != pmx || my != pmy {
            let (cw, chh) = wm.g.cursor_size();
            wm.mark(rect(pmx, pmy, cw, chh));
            wm.mark(rect(mx, my, cw, chh));
        }
        pmx = mx;
        pmy = my;
        pbtn = btn;
        wm.pump();
        let had = wm.nd > 0;
        while wm.nd > 0 {
            wm.nd -= 1;
            let r = wm.dirty[wm.nd];
            wm.compose(r);
            wm.flush(r);
        }
        if wm.quit {
            break;
        }
        if had {
            sched_yield();
        } else {
            sleep_ms(2);
        }
    }
    wm.g.clear(0);
    let _ = wm.g.present_all();
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    sys::exit(101)
}
/// Canvas cho libttf: ve coverage len scene buffer cua swm (co clip).
struct TtfSc {
    sc: *mut u32,
    w: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}
impl libgfx::Canvas for TtfSc {
    fn blit_alpha8(&mut self, cov: &[u8], sw: usize, sh: usize, x: i32, y: i32, c: u32) {
        let (sr, sg, sb) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
        for j in 0..sh as i32 {
            let py = y + j;
            if py < self.y0 || py >= self.y1 {
                continue;
            }
            for i in 0..sw as i32 {
                let px = x + i;
                if px < self.x0 || px >= self.x1 {
                    continue;
                }
                let a = cov[j as usize * sw + i as usize] as u32;
                if a == 0 {
                    continue;
                }
                unsafe {
                    let p = self.sc.add((py * self.w + px) as usize);
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
}
