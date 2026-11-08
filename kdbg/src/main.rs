#![no_std]
#![no_main]

use core::ffi::c_void;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicUsize, Ordering};
use driver_abi::*;

static BASE: AtomicUsize = AtomicUsize::new(0);
static LEN: AtomicUsize = AtomicUsize::new(0);

fn hex(b: &[u8]) -> Option<u64> {
    if b.is_empty() {
        return None;
    }
    let mut v = 0u64;
    for &c in b {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        };
        v = (v << 4) | d as u64;
    }
    Some(v)
}

/// Dòng dạng "ffffffff80001000 T kernel::kmain" -> (addr, name)
fn parse(line: &[u8]) -> Option<(u64, &[u8])> {
    let mut sp = 0;
    while sp < line.len() && line[sp] != b' ' {
        sp += 1;
    }
    let addr = hex(&line[..sp])?;
    // sau địa chỉ: " <type> <name>"
    if sp + 3 > line.len() {
        return None;
    }
    let mut name = &line[sp + 3..];
    while let Some(&l) = name.last() {
        if l == b'\r' {
            name = &name[..name.len() - 1];
        } else {
            break;
        }
    }
    Some((addr, name))
}

extern "C" fn op_init(ptr: *const u8, len: usize) -> bool {
    if len == 0 {
        return false;
    }
    BASE.store(ptr as usize, Ordering::Release);
    LEN.store(len, Ordering::Release);
    true
}

extern "C" fn op_resolve(addr: u64, out: *mut u8, cap: usize, off: *mut u64) -> bool {
    let b = BASE.load(Ordering::Acquire);
    if b == 0 || cap == 0 {
        return false;
    }
    let data = unsafe { core::slice::from_raw_parts(b as *const u8, LEN.load(Ordering::Acquire)) };

    let mut best: Option<(u64, &[u8])> = None;
    let mut start = 0;
    while start < data.len() {
        let mut end = start;
        while end < data.len() && data[end] != b'\n' {
            end += 1;
        }
        if let Some((a, n)) = parse(&data[start..end]) {
            if a > addr {
                break; // file đã sort theo địa chỉ
            }
            best = Some((a, n));
        }
        start = end + 1;
    }

    match best {
        Some((a, n)) => unsafe {
            let k = if n.len() < cap - 1 { n.len() } else { cap - 1 };
            for i in 0..k {
                *out.add(i) = n[i];
            }
            *out.add(k) = 0;
            *off = addr - a;
            true
        },
        None => false,
    }
}

static OPS: DebugOps = DebugOps { init: op_init, resolve: op_resolve };

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    let api = unsafe { &*api };
    if api.abi_version != DRIVER_ABI {
        return -1;
    }
    let info = DriverInfo {
        name: name("kdbg"),
        kind: KIND_DEBUG,
        ops: &OPS as *const DebugOps as *const c_void,
    };
    (api.register)(&info)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
