//! wapp: client mẫu của swm. 30 fps, ô vuông chạy, pad theo chuột, phím đổi màu.
#![no_std]
#![no_main]

use libswmapi::{rgb, uptime_ms, Event, Window};

libswmapi::entry!(main);

const BG: u32 = rgb(236, 236, 242);
const PAD: u32 = rgb(255, 255, 255);
const COLS: [u32; 5] = [0xDC3C3C, 0x3CB450, 0x3C78DC, 0xE6B432, 0xA03CC8];
const FRAME_MS: u64 = 33;

fn main() {
    let mut win = match Window::create(320, 220, "Demo") {
        Ok(w) => w,
        Err(_) => libswmapi::exit(1),
    };
    let mut col = 0usize;
    {
        let mut c = win.canvas();
        c.clear(BG);
        c.fill_rect(10, 80, 300, 100, PAD);
        c.fill_rect(10, 190, 300, 20, COLS[col]);
    }
    let _ = win.present_all();

    let mut f = 0i32;
    let mut ox = 10i32;
    let mut dot: Option<(i32, i32)> = None;
    let mut last = uptime_ms();
    loop {
        let mut mouse: Option<(i32, i32, u32)> = None;
        let mut keyed = false;
        while let Some(e) = win.poll_event() {
            match e {
                Event::Mouse { x, y, buttons } => mouse = Some((x, y, buttons)), // chi giu su kien moi nhat
                Event::Key(_) => keyed = true,
                Event::Close => return,
                Event::Focus(_) | Event::Resized { .. } => {}
            }
        }
        if keyed {
            col = (col + 1) % 5;
            win.canvas().fill_rect(10, 190, 300, 20, COLS[col]);
            let _ = win.present(10, 190, 300, 20);
        }
        if let Some((x, y, b)) = mouse {
            if let Some((dx, dy)) = dot.take() {
                win.canvas().fill_rect(dx - 4, dy - 4, 9, 9, PAD);
                let _ = win.present(dx - 4, dy - 4, 9, 9);
            }
            if x >= 14 && x < 306 && y >= 84 && y < 176 {
                let c = if b & 1 != 0 { 0xFF0000 } else { COLS[col] };
                win.canvas().fill_rect(x - 3, y - 3, 7, 7, c);
                let _ = win.present(x - 3, y - 3, 7, 7);
                dot = Some((x, y));
            }
        }

        let now = uptime_ms();
        if now.wrapping_sub(last) < FRAME_MS {
            continue; // poll_event o dau vong da ngu 1ms
        }
        last = now;
        f += 1;
        let nx = 10 + (f * 4) % 260;
        let (px, pw) = if nx >= ox { (ox, nx + 28 - ox) } else { (0, 320) };
        {
            let mut c = win.canvas();
            c.fill_rect(px, 40, pw, 32, BG);
            c.fill_rect(nx, 42, 28, 28, COLS[col]);
        }
        let _ = win.present(px, 40, pw, 32);
        ox = nx;
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    libswmapi::exit(101)
}
