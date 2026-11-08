#![no_std]
#![no_main]

use core::fmt::Write;
use core::panic::PanicInfo;
use libgfx::{rgb, sys, Error, Gfx, StrBuf};

libgfx::entry!(main);

const C1: u32 = rgb(20, 30, 80);
const C2: u32 = rgb(110, 40, 120);
const BAR: u32 = rgb(15, 15, 25);
const WW: i32 = 460;
const WH: i32 = 260;

fn fail(e: Error) -> ! {
    sys::log("gfxdemo: ");
    sys::log(e.as_str());
    sys::log("\n");
    sys::exit(1)
}

/// Màu nền gradient tại dòng y (khớp với gradient_v).
fn bg(y: i32, h: i32) -> u32 {
    let t = (y.max(0) as u32) * 255 / (h.max(1) as u32);
    let mix = |s: u32| ((C1 >> s & 255) * (255 - t) + (C2 >> s & 255) * t) / 255;
    (mix(16) << 16) | (mix(8) << 8) | mix(0)
}

fn draw_bar(g: &mut Gfx) {
    let w = g.w as i32;
    g.fill_rect(0, 0, w, 24, BAR);
    g.text(8, 8, "MYOS  libgfx demo", rgb(255, 220, 80), 1);
}

/// Xoá 16 dòng bắt đầu từ y về lại nền (dùng để xoá con trỏ cũ).
fn erase_rows(g: &mut Gfx, y: i32) {
    let (w, h) = (g.w as i32, g.h as i32);
    for yy in y.max(0)..(y + 16).min(h) {
        let c = if yy < 24 { BAR } else { bg(yy, h) };
        g.fill_rect(0, yy, w, 1, c);
    }
}

fn draw_window(g: &mut Gfx, wx: i32, wy: i32, f: i32, mx: i32, my: i32, clicks: u32, hover: bool) {
    // khoi phuc nen duoi cua so + bong do, tranh alpha chong len nhau moi frame
    let h = g.h as i32;
    for yy in wy..wy + WH + 8 {
        g.fill_rect(wx, yy, WW + 8, 1, bg(yy, h));
    }
    g.fill_rect_alpha(wx + 8, wy + 8, WW, WH, 0x000000, 110);
    g.fill_rect(wx, wy, WW, WH, rgb(236, 236, 242));
    g.fill_rect(wx, wy, WW, 22, rgb(40, 90, 200));
    g.text(wx + 8, wy + 7, "Window", rgb(255, 255, 255), 1);
    g.rect(wx, wy, WW, WH, rgb(20, 20, 60));
    g.text(wx + 16, wy + 40, "Hello from user space!", rgb(0, 0, 0), 2);

    let mut s = StrBuf::<64>::new();
    let _ = write!(s, "fb {}x{}  pitch {}", g.w, g.h, g.pitch);
    g.text(wx + 16, wy + 80, s.as_str(), rgb(40, 40, 40), 1);

    s.clear();
    let _ = write!(s, "mouse {},{}  clicks {}", mx, my, clicks);
    g.text(wx + 16, wy + 100, s.as_str(), rgb(40, 40, 40), 1);

    g.text(wx + 16, wy + 120, "Colors:", rgb(40, 40, 40), 1);
    g.fill_rect(wx + 70, wy + 118, 30, 12, rgb(255, 0, 0));
    g.fill_rect(wx + 106, wy + 118, 30, 12, rgb(0, 255, 0));
    g.fill_rect(wx + 142, wy + 118, 30, 12, rgb(0, 0, 255));

    // ô vuông chạy ngang
    let x = wx + 16 + (f * 2) % (WW - 70);
    g.fill_rect(x, wy + 150, 32, 32, rgb(220, 60, 60));
    g.rect(x, wy + 150, 32, 32, rgb(90, 10, 10));

    g.text(wx + 16, wy + 232, "Click Thoat hoac chuot phai de thoat", rgb(90, 90, 90), 1);

    // nút Thoat
    let (bx, by) = (wx + WW - 100, wy + WH - 36);
    let c = if hover { rgb(220, 70, 70) } else { rgb(170, 40, 40) };
    g.fill_rect(bx, by, 84, 22, c);
    g.rect(bx, by, 84, 22, rgb(60, 10, 10));
    g.text(bx + 22, by + 7, "Thoat", rgb(255, 255, 255), 1);
}

fn main() {
    let mut g = match Gfx::open() {
        Ok(g) => g,
        Err(e) => fail(e),
    };
    let (w, h) = (g.w as i32, g.h as i32);
    let (wx, wy) = (w / 2 - WW / 2, h / 2 - WH / 2);
    let (bx, by) = (wx + WW - 100, wy + WH - 36);

    g.gradient_v(0, 0, w, h, C1, C2);
    draw_bar(&mut g);

    // Không có driver chuột: hiện cửa sổ rồi chờ Enter như bản cũ.
    if !g.poll_mouse() {
        draw_window(&mut g, wx, wy, 0, 0, 0, 0, false);
        g.text(wx + 16, wy + 200, "(khong co mouse driver) Nhan Enter", rgb(160, 30, 30), 1);
        if let Err(e) = g.present_all() {
            fail(e);
        }
        let mut b = [0u8; 8];
        sys::read(0, &mut b);
        return;
    }

    if let Err(e) = g.present_all() {
        fail(e);
    }

    let (mut px, mut py) = (g.mouse_x(), g.mouse_y());
    let mut prev_btn = 0u32;
    let mut clicks = 0u32;
    let mut f = 0i32;

    loop {
        g.poll_mouse();
        let (mx, my, btn) = (g.mouse_x(), g.mouse_y(), g.mouse_buttons());
        let hover = mx >= bx && mx < bx + 84 && my >= by && my < by + 22;

        if btn & 2 != 0 {
            break;
        }
        if btn & 1 != 0 && prev_btn & 1 == 0 {
            if hover {
                break;
            }
            clicks += 1;
        }
        prev_btn = btn;

        erase_rows(&mut g, py);
        draw_bar(&mut g);
        draw_window(&mut g, wx, wy, f, mx, my, clicks, hover);
        g.draw_cursor();
        if let Err(e) = g.present() {
            fail(e);
        }
        px = mx;
        py = my;
        f += 1;
    }
    let _ = px;
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    sys::exit(101)
}