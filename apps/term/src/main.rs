//! term: terminal chạy `busybox sh -i` qua pipe, hiển thị lưới ký tự.
//! Cỡ ban đầu 80x24; khi cửa sổ đổi kích thước (maximize/restore) lưới co giãn theo.
#![no_std]
#![no_main]

use libswmapi::{rgb, syscall, Event, Window};

libswmapi::entry!(main);

macro_rules! bm {
    ($c:expr, $x:expr, $y:expr, $ch:expr, $col:expr) => {{
        for gx in 0..5i32 {
            let bits = libgfx::glyph_col($ch, gx as usize);
            for gy in 0..8i32 {
                if (bits >> gy) & 1 != 0 {
                    $c.fill_rect($x + 1 + gx * 2, $y + gy * 2, 2, 2, $col);
                }
            }
        }
    }};
}

const COLS0: usize = 80;
const ROWS0: usize = 24;
// Lưới tối đa (đủ cho cửa sổ lớn nhất ở mode 2560x1440)
const MAXC: usize = 432;
const MAXR: usize = 144;
const CW: i32 = 12;
const CH: i32 = 18;
const PAD: i32 = 4;
const W: usize = COLS0 * 12 + 8;
const H: usize = ROWS0 * 18 + 8;   // = ROWS0 * CH + 2 * PAD
const BG: u32 = rgb(16, 16, 24);
const FG: u32 = rgb(200, 225, 200);
const CUR: u32 = rgb(220, 220, 220);

struct Term {
    g: [[u8; MAXC]; MAXR], // bất biến: ô nằm ngoài (cols, rows) luôn là ' '
    cols: usize,
    rows: usize,
    cx: usize,
    cy: usize,
    esc: u8,
    arg: i32,
    d0: usize,
    d1: usize,
}

impl Term {
    fn mark(&mut self, r: usize) {
        if r < self.d0 {
            self.d0 = r;
        }
        if self.d1 == usize::MAX || r > self.d1 {
            self.d1 = r;
        }
    }

    fn mark_all(&mut self) {
        let last = self.rows - 1;
        self.mark(0);
        self.mark(last);
    }

    /// Đổi cỡ lưới. Nội dung giữ ở góc trên trái; nếu con trỏ nằm ngoài vùng mới thì cuộn lên.
    fn resize(&mut self, nc: usize, nr: usize) {
        let nc = nc.clamp(20, MAXC);
        let nr = nr.clamp(5, MAXR);
        if self.cy >= nr {
            let sh = self.cy + 1 - nr;
            for r in sh..MAXR {
                self.g[r - sh] = self.g[r];
            }
            for r in MAXR - sh..MAXR {
                self.g[r] = [b' '; MAXC];
            }
            self.cy -= sh;
        }
        // xoá mọi ô nằm ngoài vùng mới để giữ bất biến
        for r in 0..MAXR {
            let from = if r < nr { nc } else { 0 };
            for x in from..MAXC {
                self.g[r][x] = b' ';
            }
        }
        self.cols = nc;
        self.rows = nr;
        if self.cx > nc {
            self.cx = nc;
        }
        self.d0 = usize::MAX;
        self.d1 = usize::MAX;
        self.mark_all();
    }

    fn newline(&mut self) {
        self.cx = 0;
        if self.cy + 1 < self.rows {
            self.cy += 1;
        } else {
            for r in 1..self.rows {
                self.g[r - 1] = self.g[r];
            }
            let last = self.rows - 1;
            self.g[last] = [b' '; MAXC];
            self.mark(0);
            self.mark(last);
        }
    }

    fn put(&mut self, c: u8) {
        if self.cx >= self.cols {
            self.newline();
        }
        self.g[self.cy][self.cx] = c;
        let r = self.cy;
        self.mark(r);
        self.cx += 1;
    }

    fn clear_all(&mut self) {
        for r in 0..MAXR {
            self.g[r] = [b' '; MAXC];
        }
        self.mark_all();
    }

    fn feed(&mut self, c: u8) {
        match self.esc {
            1 => {
                self.esc = if c == b'[' { 2 } else { 0 };
                self.arg = 0;
                return;
            }
            2 => {
                if c.is_ascii_digit() {
                    self.arg = self.arg * 10 + (c - b'0') as i32;
                    return;
                }
                if c == b';' {
                    self.arg = 0;
                    return;
                }
                self.esc = 0;
                let (cols, rows) = (self.cols, self.rows);
                let (cx, cy) = (self.cx.min(cols - 1), self.cy);
                match c {
                    b'J' => {
                        if self.arg == 2 {
                            self.clear_all();
                        } else if self.arg == 0 {
                            for x in cx..cols {
                                self.g[cy][x] = b' ';
                            }
                            for r in cy + 1..rows {
                                self.g[r] = [b' '; MAXC];
                            }
                            self.mark(cy);
                            self.mark(rows - 1);
                        }
                    }
                    b'K' => {
                        let (a, b) = if self.arg == 2 { (0, cols) } else { (cx, cols) };
                        for x in a..b {
                            self.g[cy][x] = b' ';
                        }
                        self.mark(cy);
                    }
                    b'H' => {
                        self.cx = 0;
                        self.cy = 0;
                    }
                    _ => {}
                }
                return;
            }
            _ => {}
        }
        match c {
            0x1b => self.esc = 1,
            b'\n' => self.newline(),
            b'\r' => self.cx = 0,
            0x08 => {
                if self.cx > 0 {
                    self.cx -= 1;
                }
            }
            b'\t' => {
                let n = (self.cx / 8 + 1) * 8;
                while self.cx < n && self.cx < self.cols {
                    self.put(b' ');
                }
            }
            0x20..=0x7e => self.put(c),
            _ => {}
        }
    }
}

fn wr(fd: i64, b: &[u8]) {
    unsafe { syscall(1, fd, b.as_ptr() as i64, b.len() as i64, 0) };
}

fn close(fd: i64) {
    unsafe { syscall(3, fd, 0, 0, 0) };
}

/// `full` = vẽ lại + gửi toàn bộ cửa sổ (sau khi đổi kích thước).
fn render(win: &mut Window, t: &mut Term, full: bool) {
    if !full && (t.d0 > t.d1 || t.d1 == usize::MAX) {
        return;
    }
    let last = t.rows - 1;
    let (d0, d1) = if full { (0, last) } else { (t.d0, t.d1.min(last)) };
    if d0 > d1 {
        t.d0 = usize::MAX;
        t.d1 = usize::MAX;
        return;
    }
    let ww = win.w as i32;
    {
        let mut c = win.canvas();
        if full {
            c.clear(BG);
        }
        for r in d0..=d1 {
            let y = PAD + r as i32 * CH;
            c.fill_rect(0, y, ww, CH, BG);
            for cx in 0..t.cols {
                let ch = t.g[r][cx];
                if ch > b' ' && ch < 0x7f {
                    bm!(c, PAD + cx as i32 * CW, y + 1, ch, FG);
                }
            }
            if r == t.cy {
                let cx = t.cx.min(t.cols - 1);
                let x = PAD + cx as i32 * CW;
                c.fill_rect(x, y, CW, CH, CUR);
                bm!(c, x, y + 1, t.g[r][cx], BG);
            }
        }
    }
    if full {
        let _ = win.present_all();
    } else {
        let _ = win.present(0, PAD + d0 as i32 * CH, ww, (d1 - d0 + 1) as i32 * CH);
    }
    t.d0 = usize::MAX;
    t.d1 = usize::MAX;
}

fn main() {
    let mut win = match Window::create(W, H, "Terminal") {
        Ok(w) => w,
        Err(_) => libswmapi::exit(1),
    };
    win.canvas().clear(BG);
    let _ = win.present_all();

    let mut t = Term {
        g: [[b' '; MAXC]; MAXR],
        cols: COLS0,
        rows: ROWS0,
        cx: 0,
        cy: 0,
        esc: 0,
        arg: 0,
        d0: usize::MAX,
        d1: usize::MAX,
    };

    let mut pin = [0i32; 2];
    let mut pout = [0i32; 2];
    unsafe {
        if syscall(22, pin.as_mut_ptr() as i64, 0, 0, 0) < 0 || syscall(22, pout.as_mut_ptr() as i64, 0, 0, 0) < 0 {
            libswmapi::exit(1);
        }
        let pid = syscall(57, 0, 0, 0, 0);
        if pid == 0 {
            syscall(33, pin[0] as i64, 0, 0, 0);
            syscall(33, pout[1] as i64, 1, 0, 0);
            syscall(33, pout[1] as i64, 2, 0, 0);
            for fd in 3..32 {
                syscall(3, fd, 0, 0, 0);
            }
            let path = b"/bin/busybox\0";
            let a0 = b"busybox\0";
            let a1 = b"sh\0";
            let a2 = b"-i\0";
            let argv = [a0.as_ptr() as u64, a1.as_ptr() as u64, a2.as_ptr() as u64, 0u64];
            syscall(59, path.as_ptr() as i64, argv.as_ptr() as i64, 0, 0);
            libswmapi::exit(127);
        }
        close(pin[0] as i64);
        close(pout[1] as i64);
        if pid < 0 {
            for &b in b"fork failed" {
                t.feed(b);
            }
        }
    }
    let (sin, sout) = (pin[1] as i64, pout[0] as i64);
    let mut sin_open = true;

    let mut line = [0u8; 200];
    let mut ll = 0usize;
    let (mut pcx, mut pcy) = (usize::MAX, usize::MAX);
    t.mark_all();
    let mut full = false;

    'main: loop {
        while let Some(e) = win.poll_event() {
            match e {
                Event::Close => break 'main,
                Event::Resized { w, h } => {
                    let nc = ((w as i32 - 2 * PAD) / CW).max(1) as usize;
                    let nr = ((h as i32 - 2 * PAD) / CH).max(1) as usize;
                    t.resize(nc, nr);
                    pcx = usize::MAX;
                    pcy = usize::MAX;
                    full = true;
                }
                Event::Key(k) => match k {
                    0x20..=0x7e => {
                        if ll < 199 {
                            line[ll] = k;
                            ll += 1;
                            t.feed(k);
                        }
                    }
                    0x08 | 0x7f => {
                        if ll > 0 {
                            ll -= 1;
                            t.feed(0x08);
                            t.feed(b' ');
                            t.feed(0x08);
                        }
                    }
                    b'\n' | b'\r' => {
                        t.feed(b'\n');
                        line[ll] = b'\n';
                        if sin_open {
                            wr(sin, &line[..=ll]);
                        }
                        ll = 0;
                    }
                    3 => {
                        ll = 0;
                        for &b in b"^C\n" {
                            t.feed(b);
                        }
                        if sin_open {
                            wr(sin, b"\n");
                        }
                    }
                    4 => {
                        if ll == 0 && sin_open {
                            close(sin);
                            sin_open = false;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        // doc output cua shell
        let mut total = 0;
        let mut dead = false;
        loop {
            let mut p = [0u8; 8];
            p[0..4].copy_from_slice(&(sout as i32).to_le_bytes());
            p[4..6].copy_from_slice(&1i16.to_le_bytes());
            let r = unsafe { syscall(7, p.as_mut_ptr() as i64, 1, 0, 0) };
            if r <= 0 {
                break;
            }
            if i16::from_le_bytes([p[6], p[7]]) & 1 != 0 {
                let mut b = [0u8; 512];
                let n = unsafe { syscall(0, sout, b.as_mut_ptr() as i64, 512, 0) };
                if n <= 0 {
                    dead = true;
                    break;
                }
                for k in 0..n as usize {
                    t.feed(b[k]);
                }
                total += n as usize;
                if total >= 4096 {
                    break;
                }
            } else {
                dead = true;
                break;
            }
        }
        if dead {
            break;
        }
        unsafe { syscall(61, -1, 0, 1, 0) }; // thu zombie

        if (t.cx, t.cy) != (pcx, pcy) {
            if pcy < t.rows {
                t.mark(pcy);
            }
            let cy = t.cy;
            t.mark(cy);
            pcx = t.cx;
            pcy = t.cy;
        }
        render(&mut win, &mut t, full);
        full = false;
    }
    if sin_open {
        close(sin);
    }
    close(sout);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    libswmapi::exit(101)
}