#![no_std]
#![no_main]

use core::ffi::c_void;
use core::panic::PanicInfo;
use driver_abi::*;

// 0 = Page Fault (#PF), 1 = Divide by Zero (#DE), 2 = Invalid Opcode (#UD), 3 = General Protection (#GP)
const PANIC_KIND: u32 = 0;

// true = Gây panic ngay khi nạp driver
// false = Gây panic khi kernel gọi readdir (lúc gõ ls)
const PANIC_ON_LOAD: bool = true;

fn boom() -> ! {
    unsafe {
        match PANIC_KIND {
            0 => { core::ptr::read_volatile(0x1000 as *const u8); }
            1 => {
                core::arch::asm!(
                    "xor edx, edx", "div ecx",
                    in("ecx") 0u32, inout("eax") 1u32 => _, out("edx") _,
                );
            }
            2 => { core::arch::asm!("ud2"); }
            3 => { core::ptr::read_volatile(0x8000_0000_0000_0000u64 as *const u8); }
            _ => {}
        }
    }
    loop { core::hint::spin_loop(); }
}

extern "C" fn op_probe(_dev: *const BlockDev) -> bool { true }
extern "C" fn op_mount(_dev: *const BlockDev) -> *mut c_void { 1 as *mut c_void }
extern "C" fn op_unmount(_h: *mut c_void) {}
extern "C" fn op_stat(_h: *mut c_void, _p: *const u8, _l: usize, _out: *mut FsStat) -> bool { false }
extern "C" fn op_read(_h: *mut c_void, _p: *const u8, _l: usize, _off: u64, _buf: *mut u8, _len: usize) -> usize { 0 }
extern "C" fn op_readdir(_h: *mut c_void, _p: *const u8, _l: usize, _idx: usize, _out: *mut DirEntry) -> bool { boom() }

static OPS: FsOps = FsOps {
    fs_name: name("panicfs"),
    probe: op_probe,
    mount: op_mount,
    unmount: op_unmount,
    stat: op_stat,
    read: op_read,
    readdir: op_readdir,
};

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    let api = unsafe { &*api };
    if api.abi_version != DRIVER_ABI { return -1; }

    if PANIC_ON_LOAD {
        let msg = b"panicfs: crashing kernel now\n";
        (api.log)(msg.as_ptr(), msg.len());
        boom();
    }

    let info = DriverInfo {
        name: name("panicfs"),
        kind: KIND_FS,
        ops: &OPS as *const FsOps as *const c_void,
    };
    (api.register)(&info)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}
