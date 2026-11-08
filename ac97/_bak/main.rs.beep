#![no_std]
#![no_main]

use core::panic::PanicInfo;
use driver_abi::*;

unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val);
}

unsafe fn inb(port: u16) -> u8 {
    let mut ret: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") ret);
    ret
}

#[inline(always)]
extern "C" fn play_beep(freq: u32) {
    if freq == 0 {
        unsafe {
            let tmp = inb(0x61) & 0xFC;
            outb(0x61, tmp);
        }
        return;
    }

    let divisor = 1193182 / freq;
    unsafe {
        outb(0x43, 0xB6);
        outb(0x42, (divisor & 0xFF) as u8);
        outb(0x42, ((divisor >> 8) & 0xFF) as u8);
        let tmp = inb(0x61);
        if (tmp & 3) != 3 {
            outb(0x61, tmp | 3);
        }
    }
}

static OPS: BeepOps = BeepOps { beep: play_beep };

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    let api = unsafe { &*api };
    if api.abi_version != DRIVER_ABI { return -1; }

    let msg = b"beep: driver loaded!\n";
    (api.log)(msg.as_ptr(), msg.len());


    let info = DriverInfo {
        name: name("beep"),
        kind: KIND_BEEP,
        ops: &OPS as *const BeepOps as *const core::ffi::c_void,
    };
    (api.register)(&info)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! { loop { core::hint::spin_loop(); } }
