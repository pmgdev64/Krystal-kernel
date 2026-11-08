//! mouse.sys: chuột PS/2 (i8042 aux), polling, không IRQ.
#![no_std]
#![no_main]

use core::arch::asm;
use core::ffi::c_void;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicUsize, Ordering::Relaxed};
use driver_abi::*;

static X: AtomicI32 = AtomicI32::new(0);
static Y: AtomicI32 = AtomicI32::new(0);
static B: AtomicI32 = AtomicI32::new(0);
static W: AtomicI32 = AtomicI32::new(1280);
static H: AtomicI32 = AtomicI32::new(800);
static CENTERED: AtomicBool = AtomicBool::new(false);
static STUCK: AtomicU8 = AtomicU8::new(0);
static DISCARD: AtomicBool = AtomicBool::new(false);
static N: AtomicUsize = AtomicUsize::new(0);
static P0: AtomicU8 = AtomicU8::new(0);
static P1: AtomicU8 = AtomicU8::new(0);

unsafe fn inb(p: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", in("dx") p, out("al") v, options(nomem, nostack));
    v
}

unsafe fn outb(p: u16, v: u8) {
    asm!("out dx, al", in("dx") p, in("al") v, options(nomem, nostack));
}

unsafe fn wait_in() {
    for _ in 0..100_000 {
        if inb(0x64) & 2 == 0 {
            return;
        }
    }
}

unsafe fn wait_out() -> bool {
    for _ in 0..100_000 {
        if inb(0x64) & 1 != 0 {
            return true;
        }
    }
    false
}

unsafe fn cmd(c: u8) {
    wait_in();
    outb(0x64, c);
}

/// Gửi 1 byte tới chuột, chờ ACK 0xFA.
unsafe fn mouse_write(b: u8) -> bool {
    cmd(0xD4);
    wait_in();
    outb(0x60, b);
    wait_out() && inb(0x60) == 0xFA
}

unsafe fn init_ps2() -> bool {
    for _ in 0..32 {
        if inb(0x64) & 1 != 0 {
            inb(0x60);
        } else {
            break;
        }
    }
    cmd(0xA8); // bật cổng aux
    cmd(0x20); // đọc command byte
    if !wait_out() {
        return false;
    }
    let mut cfg = inb(0x60);
    cfg &= !0x22; // tắt IRQ12 (bit1), bật clock aux (bit5)
    cmd(0x60);
    wait_in();
    outb(0x60, cfg);
    if !mouse_write(0xF6) {
        return false;
    }
    mouse_write(0xF4) // bật báo cáo dữ liệu
}

fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo { lo } else if v > hi { hi } else { v }
}

extern "C" fn set_bounds(w: i32, h: i32) {
    if !CENTERED.load(Relaxed) { DISCARD.store(true, Relaxed); } // devfs goi set_bounds roi poll: lan poll nay bo byte ban phim
    if w <= 0 || h <= 0 {
        return;
    }
    W.store(w, Relaxed);
    H.store(h, Relaxed);
    if !CENTERED.swap(true, Relaxed) {
        X.store(w / 2, Relaxed);
        Y.store(h / 2, Relaxed);
    }
    X.store(clamp(X.load(Relaxed), 0, w - 1), Relaxed);
    Y.store(clamp(Y.load(Relaxed), 0, h - 1), Relaxed);
}

extern "C" fn poll() {
    unsafe {
        let discard = DISCARD.swap(false, Relaxed);
        for _ in 0..256 {
            let st = inb(0x64);
            if st & 1 == 0 {
                STUCK.store(0, Relaxed);
                break; // hết dữ liệu
            }
            if st & 0x20 == 0 {
                let s = STUCK.fetch_add(1, Relaxed) + 1;
                if discard || s >= 3 {
                    inb(0x60); // byte ban phim bi ket o dau buffer, khong ai doc
                    STUCK.store(0, Relaxed);
                    continue;
                }
                break; // byte của bàn phím: để keyboard driver đọc
            }
            let b = inb(0x60);
            let n = N.load(Relaxed);
            if n == 0 {
                if b & 8 == 0 {
                    continue; // lệch packet, bỏ qua tới byte đầu hợp lệ
                }
                P0.store(b, Relaxed);
                N.store(1, Relaxed);
            } else if n == 1 {
                P1.store(b, Relaxed);
                N.store(2, Relaxed);
            } else {
                N.store(0, Relaxed);
                let f = P0.load(Relaxed);
                if f & 0xC0 != 0 {
                    continue; // overflow
                }
                let mut dx = P1.load(Relaxed) as i32;
                let mut dy = b as i32;
                if f & 0x10 != 0 {
                    dx -= 256;
                }
                if f & 0x20 != 0 {
                    dy -= 256;
                }
                let (w, h) = (W.load(Relaxed), H.load(Relaxed));
                X.store(clamp(X.load(Relaxed) + dx, 0, w - 1), Relaxed);
                Y.store(clamp(Y.load(Relaxed) - dy, 0, h - 1), Relaxed); // PS/2: Y hướng lên
                B.store((f & 7) as i32, Relaxed);
            }
        }
    }
}

extern "C" fn get(out: *mut MouseState) -> bool {
    if out.is_null() {
        return false;
    }
    unsafe {
        *out = MouseState { x: X.load(Relaxed), y: Y.load(Relaxed), buttons: B.load(Relaxed) };
    }
    true
}

static OPS: MouseOps = MouseOps { set_bounds, poll, get };

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    let api = unsafe { &*api };
    if api.abi_version != DRIVER_ABI {
        return -1;
    }
    if !unsafe { init_ps2() } {
        (api.log)(b"mouse: no PS/2 mouse\n".as_ptr(), 21);
        return -2;
    }
    let info = DriverInfo {
        name: name("mouse"),
        kind: KIND_MOUSE,
        ops: &OPS as *const MouseOps as *const c_void,
    };
    (api.register)(&info)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}
