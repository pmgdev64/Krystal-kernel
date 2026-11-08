//! devices://<tên>: các thiết bị ảo của kernel.
use crate::{drivers, pmm, time, video};
use core::fmt::{self, Write};
use driver_abi::{DirEntry, FsStat};

const NAMES: [&str; 13] = ["null", "zero", "tty", "fb0", "mode", "mem", "uptime", "beep", "mouse", "kbd", "audio0", "sysrq", "net0"];

struct Buf {
    b: [u8; 160],
    n: usize,
}

impl Write for Buf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes() {
            if self.n < self.b.len() {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

fn is_text(name: &str) -> bool {
    matches!(name, "mem" | "uptime" | "mode" | "mouse")
}

/// Nội dung văn bản của các device dạng text.
fn text(name: &str) -> Buf {
    let mut w = Buf { b: [0; 160], n: 0 };
    match name {
        "mem" => {
            let p = pmm::pool_bytes();
            let f = pmm::free_bytes();
            let _ = writeln!(
                w,
                "total={} free={} used={} (KiB)",
                pmm::total_bytes() >> 10,
                f >> 10,
                (p - f) >> 10
            );
        }
        "mouse" => {
            let mut s = driver_abi::MouseState { x: 0, y: 0, buttons: 0 };
            if let (Some(m), Some(f)) = (drivers::mouse_ops(), video::saved()) {
                (m.set_bounds)(f.width as i32, f.height as i32);
                (m.poll)();
                (m.get)(&mut s);
            }
            let _ = writeln!(w, "{} {} {}", s.x, s.y, s.buttons);
        }
        "uptime" => {
            let _ = writeln!(w, "{}", time::uptime_ms());
        }
        "mode" => match video::saved() {
            Some(f) => {
                let _ = writeln!(w, "{}x{}x{} pitch={} rs={} gs={} bs={} gen={}", f.width, f.height, f.bpp, f.pitch, f.red_shift, f.green_shift, f.blue_shift, video::mode_gen());
            }
            None => {
                let _ = writeln!(w, "no video");
            }
        },
        _ => {}
    }
    w
}

fn fb() -> Option<(*mut u8, usize)> {
    video::saved().map(|f| (f.addr as *mut u8, f.pitch * f.height))
}

fn clean(name: &str) -> &str {
    name.trim_matches('/')
}

pub fn stat(name: &str) -> Option<FsStat> {
    let name = clean(name);
    if name.is_empty() {
        return Some(FsStat { size: 0, is_dir: true });
    }
    if !NAMES.contains(&name) {
        return None;
    }
    let size = if is_text(name) {
        text(name).n as u64
    } else if name == "fb0" {
        fb().map_or(0, |x| x.1 as u64)
    } else {
        0
    };
    Some(FsStat { size, is_dir: false })
}

pub fn readdir(name: &str, idx: usize) -> Option<DirEntry> {
    if !clean(name).is_empty() {
        return None;
    }
    let n = *NAMES.get(idx)?;
    let mut e = DirEntry::empty();
    e.name[..n.len()].copy_from_slice(n.as_bytes());
    e.size = stat(n).map_or(0, |s| s.size);
    e.is_dir = false;
    Some(e)
}

pub fn read(name: &str, off: u64, buf: &mut [u8]) -> usize {
    let name = clean(name);
    match name {
        "kbd" => {
            let mut n = 0;
            while n < buf.len() {
                if let Some(m) = drivers::mouse_ops() {
                    (m.poll)();
                }
                let c = match drivers::keyboard_ops() {
                    Some(k) if kbd_ready() => (k.poll)(),
                    _ => -1,
                };
                if c < 0 {
                    break;
                }
                buf[n] = c as u8;
                n += 1;
            }
            n

        }
        "net0" => match drivers::net_ops() {
            Some(n) => (n.recv)(buf.as_mut_ptr(), buf.len()),
            None => 0,
        },
        "zero" => {
            for b in buf.iter_mut() {
                *b = 0;
            }
            buf.len()
        }
        "fb0" => match fb() {
            Some((p, size)) => {
                let off = off as usize;
                if off >= size {
                    return 0;
                }
                let n = buf.len().min(size - off);
                unsafe { core::ptr::copy_nonoverlapping(p.add(off), buf.as_mut_ptr(), n) };
                n
            }
            None => 0,
        },
        n if is_text(n) => {
            let t = text(n);
            let off = off as usize;
            if off >= t.n {
                return 0;
            }
            let k = buf.len().min(t.n - off);
            buf[..k].copy_from_slice(&t.b[off..off + k]);
            k
        }
        _ => 0,
    }
}

pub fn write(name: &str, off: u64, data: &[u8]) -> usize {
    let name = clean(name);
    match name {
        "null" | "zero" => data.len(),
        "tty" => {
            if let Ok(s) = core::str::from_utf8(data) {
                crate::kprint!("{}", s);
                data.len()
            } else {
                0
            }
        }
        "fb0" => match fb() {
            Some((p, size)) => {
                let off = off as usize;
                if off >= size {
                    return 0;
                }
                let n = data.len().min(size - off);
                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), p.add(off), n) };
                n
            }
            None => 0,
        },
        "mode" => {
            let s = match core::str::from_utf8(data) {
                Ok(s) => s.trim(),
                Err(_) => return 0,
            };
            let mut it = s.split('x');
            let w = it.next().and_then(|v| v.trim().parse::<u32>().ok());
            let h = it.next().and_then(|v| v.trim().parse::<u32>().ok());
            match (w, h) {
                (Some(w), Some(h)) if video::set_mode(w, h) => data.len(),
                _ => 0,
            }
        }
        "audio0" => match drivers::audio_ops() {
            Some(a) => {
                let mut done = 0usize;
                let mut t0 = time::uptime_ms() as u64;
                while data.len() - done >= 4 {
                    let n = (a.write)(data[done..].as_ptr(), data.len() - done);
                    if n == 0 {
                        if time::uptime_ms() as u64 - t0 > 5000 {
                            break;
                        }
                        crate::task::yield_now();
                    } else {
                        done += n;
                        t0 = time::uptime_ms() as u64;
                    }
                }
                done
            }
            None => 0,
        },
        "net0" => match drivers::net_ops() {
            Some(n) if (n.send)(data.as_ptr(), data.len()) => data.len(),
            _ => 0,
        },
        "sysrq" => match data.first() {
            Some(b'c') => panic!("sysrq: user triggered crash"),
            _ => data.len(),
        },
        "beep" => {
            let f = core::str::from_utf8(data).ok().and_then(|s| s.trim().parse::<u32>().ok());
            match (f, drivers::beep_ops()) {
                (Some(f), Some(b)) => {
                    (b.beep)(f);
                    data.len()
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// true khi byte dang cho trong i8042 la cua ban phim (bit0 = co du lieu, bit5 = tu chuot).
fn kbd_ready() -> bool {
    let s: u8;
    unsafe { core::arch::asm!("in al, dx", in("dx") 0x64u16, out("al") s, options(nomem, nostack)) };
    s & 0x21 == 0x01
}

/// ioctl cho các device. Trả >= 0 nếu ok, số âm (-errno) nếu lỗi.
pub fn ioctl(name: &str, req: u64, arg: u64) -> i64 {
    use driver_abi::{
        AUDIO_IOC_DRAIN, AUDIO_IOC_GET_RATE, AUDIO_IOC_SET_RATE, AUDIO_IOC_SET_VOLUME, AUDIO_IOC_STOP,
    };
    if clean(name) == "net0" {
        return net_ioctl(req);
    }
    if clean(name) != "audio0" {
        return -25; // ENOTTY
    }
    let a = match drivers::audio_ops() {
        Some(a) => a,
        None => return -19, // ENODEV
    };
    match req {
        AUDIO_IOC_SET_RATE => {
            if (a.set_rate)(arg as u32) { 0 } else { -22 }
        }
        AUDIO_IOC_GET_RATE => (a.get_rate)() as i64,
        AUDIO_IOC_SET_VOLUME => {
            (a.set_volume)((arg & 0xFF) as u8, ((arg >> 8) & 0xFF) as u8);
            0
        }
        AUDIO_IOC_DRAIN => {
            (a.drain)();
            0
        }
        AUDIO_IOC_STOP => {
            (a.stop)();
            0
        }
        _ => -25,
    }
}

/// ioctl cho devices://net0.
fn net_ioctl(req: u64) -> i64 {
    use driver_abi::{NET_IOC_GET_MAC, NET_IOC_LINK};
    let n = match drivers::net_ops() {
        Some(n) => n,
        None => return -19, // ENODEV
    };
    match req {
        NET_IOC_GET_MAC => {
            let mut m = [0u8; 8];
            (n.mac)(m.as_mut_ptr());
            u64::from_le_bytes(m) as i64
        }
        NET_IOC_LINK => (n.link_up)() as i64,
        _ => -25,
    }
}
