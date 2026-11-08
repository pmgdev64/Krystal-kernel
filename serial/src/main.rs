#![no_std]
#![no_main]

use core::arch::asm;
use core::ffi::c_void;
use core::panic::PanicInfo;
use driver_abi::{name, DriverInfo, KernelApi, SerialOps, DRIVER_ABI, KIND_SERIAL};

const COM1: u16 = 0x3F8;

unsafe fn outb(port: u16, v: u8) {
    asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack));
}

unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack));
    v
}

fn hw_init() {
    unsafe {
        outb(COM1 + 1, 0x00); // tắt interrupt
        outb(COM1 + 3, 0x80); // bật DLAB
        outb(COM1, 0x01); // 115200 baud
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8N1
        outb(COM1 + 2, 0xC7); // FIFO
        outb(COM1 + 4, 0x0B);
    }
}

extern "C" fn write_str(ptr: *const u8, len: usize) {
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    for &b in bytes {
        unsafe {
            while inb(COM1 + 5) & 0x20 == 0 {
                core::hint::spin_loop();
            }
            outb(COM1, b);
        }
    }
}

static OPS: SerialOps = SerialOps { write_str };

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    let api = unsafe { &*api };
    if api.abi_version != DRIVER_ABI {
        return -1;
    }
    hw_init();
    let info = DriverInfo {
        name: name("serial"),
        kind: KIND_SERIAL,
        ops: &OPS as *const SerialOps as *const c_void,
    };
    (api.register)(&info)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
