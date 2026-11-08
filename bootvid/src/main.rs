#![no_std]
#![no_main]

use bootvid_abi::{BootvidOps, Color, FbInfo, ABI_VERSION};
use core::cell::UnsafeCell;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};
use noto_sans_mono_bitmap::{get_raster, get_raster_width, FontWeight, RasterHeight};

const CH: usize = 24;
const TRIM_W: usize = 2; // bot be ngang moi o
const TRIM_H: usize = 4; // bot chieu cao moi dong
const H: RasterHeight = RasterHeight::Size24;
const W: FontWeight = FontWeight::Bold;

struct Screen {
    info: FbInfo,
    bytes: usize,
    scale: usize,
    cw: usize, // rong o chu (chua nhan scale)
    cols: usize,
    rows: usize,
    cx: usize,
    cy: usize,
    fg: Color,
    bg: Color,
    def_fg: Color,
    esc: u8,
    par: [u16; 4],
    npar: usize,
    transparent: bool,
    can_scroll: bool,
}

fn comp(v: u8, size: u8, shift: u8) -> u32 {
    ((v as u32) >> (8 - size.min(8) as u32)) << shift as u32
}

fn mix(a: u8, b: u8, i: u8) -> u8 {
    (((a as u32) * (255 - i as u32) + (b as u32) * i as u32) / 255) as u8
}

impl Screen {
    fn new(info: FbInfo) -> Self {
        let w = info.width;
        let scale = if w >= 3200 { 3 } else if w >= 1600 { 2 } else { 1 };
        let cw = get_raster_width(W, H);
        Self {
            info,
            bytes: (info.bpp + 7) / 8,
            scale,
            cw,
            cols: w / ((cw - TRIM_W) * scale),
            rows: info.height / ((CH - TRIM_H) * scale),
            cx: 0,
            cy: 0,
            fg: Color::GRAY,
            bg: Color::BLACK,
            def_fg: Color::GRAY,
            esc: 0,
            par: [0; 4],
            npar: 0,
            transparent: false,
            can_scroll: true,
        }
    }

    fn cell_h(&self) -> usize {
        (CH - TRIM_H) * self.scale
    }

    fn cell_w(&self) -> usize {
        (self.cw - TRIM_W) * self.scale
    }

    fn pack(&self, c: Color) -> u32 {
        let i = &self.info;
        comp(c.r, i.red_size, i.red_shift)
            | comp(c.g, i.green_size, i.green_shift)
            | comp(c.b, i.blue_size, i.blue_shift)
    }

    #[inline]
    unsafe fn put_raw(&self, x: usize, y: usize, v: u32) {
        let p = self.info.addr.add(y * self.info.pitch + x * self.bytes);
        match self.bytes {
            4 => (p as *mut u32).write_volatile(v),
            2 => (p as *mut u16).write_volatile(v as u16),
            _ => {
                p.write_volatile(v as u8);
                p.add(1).write_volatile((v >> 8) as u8);
                p.add(2).write_volatile((v >> 16) as u8);
            }
        }
    }

    fn put_pixel(&mut self, x: usize, y: usize, c: Color) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }
        let v = self.pack(c);
        unsafe { self.put_raw(x, y, v) };
    }

    fn get_color(&self, x: usize, y: usize) -> Color {
        if x >= self.info.width || y >= self.info.height {
            return Color::BLACK;
        }
        let i = &self.info;
        let p = unsafe { i.addr.add(y * i.pitch + x * self.bytes) };
        let v: u32 = unsafe {
            match self.bytes {
                4 => (p as *const u32).read_volatile(),
                2 => (p as *const u16).read_volatile() as u32,
                _ => (p.read_volatile() as u32)
                    | ((p.add(1).read_volatile() as u32) << 8)
                    | ((p.add(2).read_volatile() as u32) << 16),
            }
        };
        let ch = |size: u8, shift: u8| -> u8 {
            let sz = size.min(8) as u32;
            if sz == 0 {
                return 0;
            }
            (((v >> shift as u32) & ((1u32 << sz) - 1)) << (8 - sz)) as u8
        };
        Color::rgb(ch(i.red_size, i.red_shift), ch(i.green_size, i.green_shift), ch(i.blue_size, i.blue_shift))
    }

    fn fill_px(&self, x: usize, y: usize, w: usize, h: usize, v: u32) {
        let x_end = x.saturating_add(w).min(self.info.width);
        let y_end = y.saturating_add(h).min(self.info.height);
        for yy in y..y_end {
            for xx in x..x_end {
                unsafe { self.put_raw(xx, yy, v) };
            }
        }
    }

    fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: Color) {
        let v = self.pack(c);
        self.fill_px(x, y, w, h, v);
    }

    fn clear(&mut self) {
        let (w, h) = (self.info.width, self.info.height);
        let v = self.pack(self.bg);
        self.fill_px(0, 0, w, h, v);
        self.cx = 0;
        self.cy = 0;
    }

    fn draw_glyph(&mut self, ch: char, col: usize, row: usize) {
        let ras = match get_raster(ch, W, H).or_else(|| get_raster('?', W, H)) {
            Some(r) => r,
            None => return,
        };
        let s = self.scale;
        let (ox, oy) = (col * self.cell_w(), row * self.cell_h());
        let (fg, bg) = (self.fg, self.bg);
        for (gy, line) in ras.raster().iter().enumerate() {
            for (gx, &i) in line.iter().enumerate() {
                let (px, py) = (ox + gx * s, oy + gy * s);
                let c = if self.transparent {
                    if i == 0 {
                        continue;
                    }
                    let o = self.get_color(px, py);
                    Color::rgb(mix(o.r, fg.r, i), mix(o.g, fg.g, i), mix(o.b, fg.b, i))
                } else {
                    Color::rgb(mix(bg.r, fg.r, i), mix(bg.g, fg.g, i), mix(bg.b, fg.b, i))
                };
                let v = self.pack(c);
                self.fill_px(px, py, s, s, v);
            }
        }
    }

    fn scroll(&mut self) {
        let ch = self.cell_h();
        let row_bytes = self.info.pitch * ch;
        let total = row_bytes * (self.rows - 1);
        unsafe {
            let dst = self.info.addr;
            let src = self.info.addr.add(row_bytes);
            if ((dst as usize) | row_bytes) & 7 == 0 {
                let (d, s) = (dst as *mut u64, src as *const u64);
                for i in 0..total / 8 {
                    d.add(i).write_volatile(s.add(i).read_volatile());
                }
            } else {
                for i in 0..total {
                    dst.add(i).write_volatile(src.add(i).read_volatile());
                }
            }
        }
        let (w, bg) = (self.info.width, self.pack(self.bg));
        self.fill_px(0, (self.rows - 1) * ch, w, self.info.height - (self.rows - 1) * ch, bg);
    }

    fn sgr(&mut self) {
        const PAL: [Color; 8] = [
            Color::rgb(0, 0, 0),
            Color::rgb(205, 49, 49),
            Color::rgb(13, 188, 121),
            Color::rgb(229, 229, 16),
            Color::rgb(80, 120, 255),
            Color::rgb(188, 63, 188),
            Color::rgb(17, 168, 205),
            Color::rgb(229, 229, 229),
        ];
        for i in 0..self.npar.max(1) {
            let p = self.par[i];
            match p {
                0 | 39 => self.fg = self.def_fg,
                30..=37 => self.fg = PAL[(p - 30) as usize],
                90..=97 => self.fg = PAL[(p - 90) as usize],
                _ => {}
            }
        }
    }

    fn put_char(&mut self, ch: char) {
        match self.esc {
            1 => {
                if ch == '[' {
                    self.esc = 2;
                    self.par = [0; 4];
                    self.npar = 0;
                } else {
                    self.esc = 0;
                }
            }
            2 => match ch {
                '0'..='9' => {
                    if self.npar == 0 {
                        self.npar = 1;
                    }
                    let i = self.npar - 1;
                    self.par[i] = self.par[i].saturating_mul(10) + ch as u16 - '0' as u16;
                }
                ';' => {
                    if self.npar < 4 {
                        self.npar += 1;
                    }
                }
                'm' => {
                    self.sgr();
                    self.esc = 0;
                }
                'J' | 'K' => {
                    let (cw, chh) = (self.cell_w(), self.cell_h());
                    let bg = self.pack(self.bg);
                    let (w, h) = (self.info.width, self.info.height);
                    if ch == 'J' && self.npar > 0 && self.par[0] == 2 {
                        self.clear();
                    } else {
                        self.fill_px(self.cx * cw, self.cy * chh, w, chh, bg);
                        if ch == 'J' {
                            self.fill_px(0, (self.cy + 1) * chh, w, h, bg);
                        }
                    }
                    self.esc = 0;
                }
                'C' | 'D' => {
                    let n = (self.par[0] as usize).max(1);
                    if ch == 'C' {
                        self.cx = (self.cx + n).min(self.cols.saturating_sub(1));
                    } else {
                        self.cx = self.cx.saturating_sub(n);
                    }
                    self.esc = 0;
                }
                'H' => {
                    let r = (self.par[0] as usize).max(1) - 1;
                    let c = (self.par[1] as usize).max(1) - 1;
                    self.cy = r.min(self.rows.saturating_sub(1));
                    self.cx = c.min(self.cols.saturating_sub(1));
                    self.esc = 0;
                }
                _ => self.esc = 0,
            },
            _ => {
                if ch == '\x1b' {
                    self.esc = 1;
                } else {
                    self.put_char_raw(ch);
                }
            }
        }
    }

    fn newline(&mut self) {
        self.cx = 0;
        self.cy += 1;
        if self.cy >= self.rows {
            if self.can_scroll {
                self.scroll();
                self.cy = self.rows - 1;
            } else {
                self.cy = 0; // khong scroll: quay ve dong dau
            }
        }
    }

    fn put_char_raw(&mut self, ch: char) {
        match ch {
            '\n' => self.newline(),
            '\r' => self.cx = 0,
            '\t' => {
                for _ in 0..4 {
                    self.put_char_raw(' ');
                }
            }
            '\x08' => {
                if self.cx > 0 {
                    self.cx -= 1;
                    self.draw_glyph(' ', self.cx, self.cy);
                }
            }
            c => {
                if self.cx >= self.cols {
                    self.newline();
                }
                self.draw_glyph(c, self.cx, self.cy);
                self.cx += 1;
            }
        }
    }
}

struct Global {
    lock: AtomicBool,
    screen: UnsafeCell<Option<Screen>>,
}
unsafe impl Sync for Global {}

static VID: Global = Global {
    lock: AtomicBool::new(false),
    screen: UnsafeCell::new(None),
};

fn with<R>(f: impl FnOnce(&mut Screen) -> R) -> Option<R> {
    while VID
        .lock
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let r = unsafe { (*VID.screen.get()).as_mut().map(f) };
    VID.lock.store(false, Ordering::Release);
    r
}

extern "C" fn op_init(info: *const FbInfo) -> bool {
    let info = unsafe { *info };
    if info.supported().is_err() {
        return false;
    }
    let mut s = Screen::new(info);
    s.clear();
    unsafe { *VID.screen.get() = Some(s) };
    true
}

extern "C" fn op_clear() {
    with(|s| s.clear());
}

extern "C" fn op_set_colors(fg: Color, bg: Color) {
    with(|s| {
        s.fg = fg;
        s.def_fg = fg;
        s.bg = bg;
    });
}

extern "C" fn op_put_pixel(x: usize, y: usize, c: Color) {
    with(|s| s.put_pixel(x, y, c));
}

extern "C" fn op_fill_rect(x: usize, y: usize, w: usize, h: usize, c: Color) {
    with(|s| s.fill_rect(x, y, w, h, c));
}

extern "C" fn op_write_str(ptr: *const u8, len: usize) {
    let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) };
    with(|scr| {
        for c in s.chars() {
            scr.put_char(c);
        }
    });
}

extern "C" fn op_text_size(cols: *mut usize, rows: *mut usize) {
    let (c, r) = with(|s| (s.cols, s.rows)).unwrap_or((0, 0));
    unsafe {
        *cols = c;
        *rows = r;
    }
}

extern "C" fn op_force_unlock() {
    VID.lock.store(false, Ordering::Release);
}

extern "C" fn op_set_transparent(on: bool) {
    with(|s| s.transparent = on);
}

extern "C" fn op_set_cursor(col: usize, row: usize) {
    with(|s| {
        s.cx = col.min(s.cols.saturating_sub(1));
        s.cy = row.min(s.rows.saturating_sub(1));
        s.esc = 0;
    });
}

extern "C" fn op_set_scroll(on: bool) {
    with(|s| s.can_scroll = on);
}

static OPS: BootvidOps = BootvidOps {
    abi_version: ABI_VERSION,
    init: op_init,
    clear: op_clear,
    set_colors: op_set_colors,
    put_pixel: op_put_pixel,
    fill_rect: op_fill_rect,
    write_str: op_write_str,
    text_size: op_text_size,
    force_unlock: op_force_unlock,
    set_transparent: op_set_transparent,
    set_cursor: op_set_cursor,
    set_scroll: op_set_scroll,
};

#[no_mangle]
pub extern "C" fn driver_entry(api: *const driver_abi::KernelApi) -> i32 {
    let api = unsafe { &*api };
    if api.abi_version != driver_abi::DRIVER_ABI {
        return -1;
    }
    let info = driver_abi::DriverInfo {
        name: driver_abi::name("bootvid"),
        kind: driver_abi::KIND_VIDEO,
        ops: &OPS as *const BootvidOps as *const core::ffi::c_void,
    };
    (api.register)(&info)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
