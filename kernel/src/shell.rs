//! Shell tối giản trong kernel: đọc dòng, chạy app trong rd0://apps hoặc busybox applet.
use crate::{app, drivers, tty, vfs};
use crate::{kprint, kprintln};

fn run(path: &str, argv: &[&str]) {
    match app::run(path, argv) {
        Ok(code) if code < 0 => kprintln!("[crashed: exception vector {}]", -code - 1),
        Ok(0) => {}
        Ok(code) => kprintln!("[exit {}]", code),
        Err(e) => kprintln!("[load failed: {}]", e),
    }
}

pub fn run_shell() -> ! {
    kprintln!("\nmyos shell. Go 'help' de xem lenh. Vi du: ls /, cat /hello.txt, echo hi");
    loop {
        kprint!("myos> ");
        let mut buf = [0u8; 200];
        let n = tty::read_line(&mut buf);
        let line = core::str::from_utf8(&buf[..n]).unwrap_or("");

        let mut args: [&str; 16] = [""; 16];
        let mut argc = 0;
        for w in line.split_whitespace() {
            if argc < 16 {
                args[argc] = w;
                argc += 1;
            }
        }
        if argc == 0 {
            continue;
        }
        let cmd = args[0];

        match cmd {
            "help" => {
                kprintln!("help | clear | <app> (rd0://apps/<app>.elf) | <busybox applet> [args]");
                kprintln!("vi du: ls /, cat /hello.txt, wc -c /hello.txt, uname -a, hello");
            }
            "free" | "mem" => crate::mem_report(),
            "mode" => match drivers::gfx_ops() {
                None => kprintln!("mode: no gfx driver (chi chay duoc tren QEMU -vga std)"),
                Some(g) => {
                    if argc < 2 {
                        let mut cur = driver_abi::GfxMode { width: 0, height: 0, bpp: 0, pitch: 0 };
                        (g.get_mode)(&mut cur);
                        kprintln!("current {}x{}x{} pitch={}, vram {} MiB", cur.width, cur.height, cur.bpp, cur.pitch, (g.vram_bytes)() >> 20);
                        for i in 0..(g.mode_count)() {
                            let mut m = driver_abi::GfxMode { width: 0, height: 0, bpp: 0, pitch: 0 };
                            if (g.mode_at)(i, &mut m) {
                                kprintln!("  {}x{}x{}", m.width, m.height, m.bpp);
                            }
                        }
                    } else {
                        let mut it = args[1].split('x');
                        let w = it.next().and_then(|s| s.parse::<u32>().ok());
                        let h = it.next().and_then(|s| s.parse::<u32>().ok());
                        match (w, h) {
                            (Some(w), Some(h)) => {
                                if crate::video::set_mode(w, h) {
                                    kprintln!("mode {}x{} OK", w, h);
                                } else {
                                    kprintln!("mode {}x{} failed", w, h);
                                }
                            }
                            _ => kprintln!("usage: mode WxH   (vd: mode 1280x720)"),
                        }
                    }
                }
            },
            "clear" => {
                if let Some(v) = drivers::video_ops() {
                    (v.clear)();
                }
            }
            _ => {
                // app riêng: rd0://apps/<cmd>.elf
                let mut p = [0u8; 64];
                let pre = b"rd0://apps/";
                let suf = b".elf";
                if pre.len() + cmd.len() + suf.len() <= p.len() {
                    p[..pre.len()].copy_from_slice(pre);
                    p[pre.len()..pre.len() + cmd.len()].copy_from_slice(cmd.as_bytes());
                    let e = pre.len() + cmd.len();
                    p[e..e + suf.len()].copy_from_slice(suf);
                    let path = core::str::from_utf8(&p[..e + suf.len()]).unwrap_or("");
                    if vfs::stat(path).is_some() {
                        run(path, &args[..argc]);
                        continue;
                    }
                }
                // còn lại: busybox applet ("busybox ls" hoặc "ls" đều được)
                let mut bargv: [&str; 17] = [""; 17];
                bargv[0] = "busybox";
                let start = if cmd == "busybox" { 1 } else { 0 };
                let mut k = 1;
                for i in start..argc {
                    bargv[k] = args[i];
                    k += 1;
                }
                run("rd0://bin/busybox", &bargv[..k]);
            }
        }
    }
}

/// Boot thẳng vào busybox ash; thoát thì mở lại.
pub fn run_sh() -> ! {
    loop {
        match app::run("rd0://apps/swm.elf", &["swm"]) {
            Ok(code) if code < 0 => kprintln!("[swm crashed: exception vector {}]", -code - 1),
            _ => {}
        }
        match app::run("rd0://apps/dm.elf", &["dm"]) {
            Ok(code) if code < 0 => kprintln!("[dm crashed: exception vector {}]", -code - 1),
            Ok(_) => {}
            Err(_) => {
                // không có dm.elf: lùi về busybox sh
                run("rd0://bin/busybox", &["busybox", "sh"]);
                kprintln!("[sh exited, restarting]");
            }
        }
    }
}

/// Vẽ viền + 4 góc để so khớp framebuffer với cửa sổ QEMU.
fn calibrate() {
    use bootvid_abi::Color;
    let fb = match crate::video::saved() {
        Some(f) => f,
        None => return,
    };
    let (w, h) = (fb.width, fb.height);
    let wh = Color::rgb(255, 255, 255);
    crate::video::fill_rect(0, 0, w, 1, wh);
    crate::video::fill_rect(0, h - 1, w, 1, wh);
    crate::video::fill_rect(0, 0, 1, h, wh);
    crate::video::fill_rect(w - 1, 0, 1, h, wh);
    crate::video::fill_rect(0, 0, 16, 16, Color::RED);
    crate::video::fill_rect(w - 16, 0, 16, 16, Color::GREEN);
    crate::video::fill_rect(0, h - 16, 16, 16, Color::BLUE);
    crate::video::fill_rect(w - 16, h - 16, 16, 16, Color::YELLOW);
    kprintln!("fb {}x{} pitch={} bpp={} addr={:p}", w, h, fb.pitch, fb.bpp, fb.addr);
    let (c, r) = crate::video::text_size();
    kprintln!("console {}x{}", c, r);
    kprintln!("ANSI test: \x1b[31mDO\x1b[32mXANH\x1b[34mXANH-DUONG\x1b[0m binh thuong", );
    kprintln!("Do: 4 goc mau + vien trang co day du trong cua so QEMU khong? Enter de tiep.");
    let mut b = [0u8; 4];
    tty::read_line(&mut b);
    if let Some(v) = drivers::video_ops() {
        (v.clear)();
    }
}
