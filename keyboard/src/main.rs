#![no_std]
#![no_main]

use core::arch::asm;
use core::ffi::c_void;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};
use driver_abi::*;

const NORMAL: &[u8; 58] = b"\0\x1b1234567890-=\x08\tqwertyuiop[]\n\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
const SHIFTED: &[u8; 58] = b"\0\x1b!@#$%^&*()_+\x08\tQWERTYUIOP{}\n\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";

static SHIFT: AtomicBool = AtomicBool::new(false);
static CTRL: AtomicBool = AtomicBool::new(false);
static CAPS: AtomicBool = AtomicBool::new(false);
static EXT: AtomicBool = AtomicBool::new(false);

unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack));
    v
}

extern "C" fn poll() -> i32 {
    unsafe {
        let st = inb(0x64);
        if st & 1 == 0 {
            return -1;
        }
        let sc = inb(0x60);
        if st & 0x20 != 0 {
            return -1;
        }
        if sc == 0xE0 {
            EXT.store(true, Ordering::Relaxed);
            return -1;
        }
        let ext = EXT.swap(false, Ordering::Relaxed);
        let rel = sc & 0x80 != 0;
        let code = (sc & 0x7f) as usize;

        if ext {
            if code == 0x1d {
                CTRL.store(!rel, Ordering::Relaxed);
            }
            return -1;
        }
        match code {
            0x2a | 0x36 => {
                SHIFT.store(!rel, Ordering::Relaxed);
                return -1;
            }
            0x1d => {
                CTRL.store(!rel, Ordering::Relaxed);
                return -1;
            }
            0x3a => {
                if !rel {
                    CAPS.fetch_xor(true, Ordering::Relaxed);
                }
                return -1;
            }
            _ => {}
        }
        if rel || code >= NORMAL.len() {
            return -1;
        }

        let shift = SHIFT.load(Ordering::Relaxed);
        let base = NORMAL[code];
        let mut c = if base.is_ascii_lowercase() {
            if shift ^ CAPS.load(Ordering::Relaxed) {
                base.to_ascii_uppercase()
            } else {
                base
            }
        } else if shift {
            SHIFTED[code]
        } else {
            base
        };
        if c == 0 {
            return -1;
        }
        if CTRL.load(Ordering::Relaxed) && c.is_ascii_alphabetic() {
            c = c.to_ascii_lowercase() & 0x1f;
        }
        c as i32
    }
}

static OPS: KeyboardOps = KeyboardOps { poll };

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    let api = unsafe { &*api };
    if api.abi_version != DRIVER_ABI {
        return -1;
    }
    unsafe {
        while inb(0x64) & 1 != 0 {
            inb(0x60);
        }
    }
    let info = DriverInfo {
        name: name("keyboard"),
        kind: KIND_KEYBOARD,
        ops: &OPS as *const KeyboardOps as *const c_void,
    };
    (api.register)(&info)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
