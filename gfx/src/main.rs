#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;
use driver_abi::*;

const IDX: u16 = 0x1CE;
const DAT: u16 = 0x1CF;
const R_ID: u16 = 0;
const R_XRES: u16 = 1;
const R_YRES: u16 = 2;
const R_BPP: u16 = 3;
const R_ENABLE: u16 = 4;
const R_VIRT_W: u16 = 6;
const R_XOFF: u16 = 8;
const R_YOFF: u16 = 9;
const R_VMEM: u16 = 10;

unsafe fn outw(port: u16, v: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") v, options(nomem, nostack));
}

unsafe fn inw(port: u16) -> u16 {
    let v: u16;
    asm!("in ax, dx", in("dx") port, out("ax") v, options(nomem, nostack));
    v
}

fn rd(i: u16) -> u16 {
    unsafe {
        outw(IDX, i);
        inw(DAT)
    }
}

fn wr(i: u16, v: u16) {
    unsafe {
        outw(IDX, i);
        outw(DAT, v);
    }
}

fn detect() -> bool {
    let id = rd(R_ID);
    (0xB0C0..=0xB0C5).contains(&id)
}

fn vram() -> u64 {
    if rd(R_ID) >= 0xB0C4 {
        rd(R_VMEM) as u64 * 65536
    } else {
        4 * 1024 * 1024
    }
}

fn apply(w: u32, h: u32) -> bool {
    if w < 320 || h < 200 || w > 8192 || h > 8192 {
        return false;
    }
    wr(R_ENABLE, 0);
    wr(R_BPP, 32);
    wr(R_XRES, w as u16);
    wr(R_YRES, h as u16);
    wr(R_VIRT_W, w as u16);
    wr(R_XOFF, 0);
    wr(R_YOFF, 0);
    wr(R_ENABLE, 0x41); // enabled + LFB
    rd(R_XRES) as u32 == w && rd(R_YRES) as u32 == h && rd(R_BPP) == 32
}

const MODES: [(u32, u32); 10] = [
    (640, 480),
    (800, 600),
    (1024, 768),
    (1280, 720),
    (1280, 800),
    (1280, 1024),
    (1366, 768),
    (1600, 900),
    (1920, 1080),
    (2560, 1440),
];

fn fits(w: u32, h: u32) -> bool {
    (w as u64) * (h as u64) * 4 <= vram()
}

extern "C" fn vram_bytes() -> u64 {
    vram()
}

extern "C" fn get_mode(out: *mut GfxMode) -> bool {
    let bpp = rd(R_BPP) as u32;
    let w = rd(R_XRES) as u32;
    let vw = rd(R_VIRT_W) as u32;
    let stride = if vw >= w { vw } else { w };
    unsafe {
        *out = GfxMode { width: w, height: rd(R_YRES) as u32, bpp, pitch: stride * bpp / 8 };
    }
    true
}

extern "C" fn set_mode(w: u32, h: u32, bpp: u32) -> bool {
    if bpp != 32 || !fits(w, h) {
        return false;
    }
    let (ow, oh) = (rd(R_XRES) as u32, rd(R_YRES) as u32);
    if apply(w, h) {
        return true;
    }
    apply(ow, oh); // khôi phục
    false
}

extern "C" fn mode_count() -> usize {
    MODES.iter().filter(|m| fits(m.0, m.1)).count()
}

extern "C" fn mode_at(i: usize, out: *mut GfxMode) -> bool {
    let mut n = 0;
    for m in MODES.iter() {
        if fits(m.0, m.1) {
            if n == i {
                unsafe {
                    *out = GfxMode { width: m.0, height: m.1, bpp: 32, pitch: m.0 * 4 };
                }
                return true;
            }
            n += 1;
        }
    }
    false
}

static OPS: GfxOps = GfxOps { vram_bytes, get_mode, set_mode, mode_count, mode_at };

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    let api = unsafe { &*api };
    if api.abi_version != DRIVER_ABI {
        return -1;
    }
    if !detect() {
        let msg = b"gfx: no Bochs/QEMU VBE adapter, skipped\n";
        (api.log)(msg.as_ptr(), msg.len());
        return -2;
    }
    let msg = b"gfx: Bochs VBE found, runtime mode switching ready\n";
    (api.log)(msg.as_ptr(), msg.len());

    let info = DriverInfo {
        name: name("gfx"),
        kind: KIND_GFX,
        ops: &OPS as *const GfxOps as *const core::ffi::c_void,
    };
    (api.register)(&info)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
