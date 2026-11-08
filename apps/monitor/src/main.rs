//! monitor: uptime, RAM, display mode. Làm mới mỗi giây.
#![no_std]
#![no_main]

use core::fmt::Write;
use libswmapi::{read_file, rgb, Event, Window};

libswmapi::entry!(main);

const W: usize = 340;
const H: usize = 150;
const BG: u32 = rgb(236, 236, 242);
const FG: u32 = rgb(30, 30, 40);
const DIM: u32 = rgb(100, 100, 120);

struct Buf {
    b: [u8; 96],
    n: usize,
}
impl Buf {
    fn new() -> Buf {
        Buf { b: [0; 96], n: 0 }
    }
    fn s(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}
impl Write for Buf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < self.b.len() {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// Số nguyên ngay sau `key` (key rỗng = số ở đầu chuỗi).
fn num_after(s: &[u8], key: &[u8]) -> u64 {
    let mut i = 0;
    while i + key.len() <= s.len() {
        if &s[i..i + key.len()] == key {
            let mut j = i + key.len();
            let mut v = 0u64;
            while j < s.len() && s[j].is_ascii_digit() {
                v = v * 10 + (s[j] - b'0') as u64;
                j += 1;
            }
            return v;
        }
        i += 1;
    }
    0
}

fn main() {
    let mut win = match Window::create(W, H, "Monitor") {
        Ok(w) => w,
        Err(_) => libswmapi::exit(1),
    };

    // phần tĩnh
    let mut mb = [0u8; 96];
    let n = read_file(b"devices://mode\0", &mut mb);
    let mode = core::str::from_utf8(&mb[..n]).unwrap_or("?").trim_end();
    let first = mode.split_whitespace().next().unwrap_or("?");
    {
        let mut c = win.canvas();
        c.clear(BG);
        c.text(10, 10, "Uptime", DIM, 1);
        c.text(10, 34, "Memory", DIM, 1);
        c.text(10, 82, "Display", DIM, 1);
        c.text(90, 82, first, FG, 1);
        c.text(10, 98, mode, DIM, 1);
        c.text(10, 130, "Cap nhat moi giay", DIM, 1);
    }
    let _ = win.present_all();

    let mut last = u64::MAX;
    loop {
        while let Some(e) = win.poll_event() {
            if let Event::Close = e {
                return;
            }
        }
        let mut tb = [0u8; 32];
        let n = read_file(b"devices://uptime\0", &mut tb);
        let ms = num_after(&tb[..n], b"");
        let sec = ms / 1000;
        if sec == last {
            continue;
        }
        last = sec;

        let mut mm = [0u8; 96];
        let n = read_file(b"devices://mem\0", &mut mm);
        let total = num_after(&mm[..n], b"total=");
        let used = num_after(&mm[..n], b"used=");

        let mut up = Buf::new();
        let _ = write!(up, "{:02}:{:02}:{:02}  ({} ms)", sec / 3600, sec / 60 % 60, sec % 60, ms);
        let mut ms_txt = Buf::new();
        let pct = if total > 0 { used * 100 / total } else { 0 };
        let _ = write!(ms_txt, "{} / {} MiB  ({}%)", used / 1024, total / 1024, pct);
        let fill = if total > 0 { (used * 316 / total) as i32 } else { 0 };

        {
            let mut c = win.canvas();
            c.fill_rect(90, 8, 240, 14, BG);
            c.text(90, 10, up.s(), FG, 1);
            c.fill_rect(90, 32, 240, 14, BG);
            c.text(90, 34, ms_txt.s(), FG, 1);
            c.fill_rect(12, 52, 316, 14, rgb(200, 200, 215));
            c.fill_rect(12, 52, fill.min(316), 14, rgb(60, 130, 220));
            c.rect(12, 52, 316, 14, rgb(20, 20, 60));
        }
        let _ = win.present(90, 8, 240, 14);
        let _ = win.present(10, 32, 320, 44);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    libswmapi::exit(101)
}
