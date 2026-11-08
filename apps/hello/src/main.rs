#![no_std]
#![no_main]

use core::arch::asm;
use core::fmt::Write;
use core::panic::PanicInfo;

// 0 = chay binh thuong
// 1 = doc dia chi 0x10                (#PF, page-not-present)
// 2 = lenh dac quyen `hlt`            (#GP)
// 3 = doc bo nho kernel               (#PF, protection-violation)
const TEST_FAULT: u32 = 0;

// Syscall Linux x86_64
const SYS_READ: u64 = 0;
const SYS_WRITE: u64 = 1;
const SYS_OPEN: u64 = 2;
const SYS_CLOSE: u64 = 3;
const SYS_EXIT: u64 = 60;

unsafe fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack),
    );
    ret
}

fn write(s: &str) {
    unsafe { syscall3(SYS_WRITE, 1, s.as_ptr() as u64, s.len() as u64) };
}

/// `path` phai ket thuc bang NUL.
fn read_file(path: &[u8], buf: &mut [u8]) -> Option<usize> {
    unsafe {
        let fd = syscall3(SYS_OPEN, path.as_ptr() as u64, 0, 0);
        if (fd as i64) < 0 {
            return None;
        }
        let n = syscall3(SYS_READ, fd, buf.as_mut_ptr() as u64, buf.len() as u64);
        syscall3(SYS_CLOSE, fd, 0, 0);
        if (n as i64) < 0 {
            None
        } else {
            Some(n as usize)
        }
    }
}

fn exit(code: u64) -> ! {
    unsafe { syscall3(SYS_EXIT, code, 0, 0) };
    loop {}
}

struct Out;
impl Write for Out {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write(s);
        Ok(())
    }
}

macro_rules! println {
    ($($a:tt)*) => {{ let _ = writeln!(Out, $($a)*); }};
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let cs: u64;
    unsafe { asm!("mov {}, cs", out(reg) cs, options(nomem, nostack)) };
    println!("Hello from user mode! CPL={}", cs & 3);

    let mut buf = [0u8; 128];
    match read_file(b"/hello.txt\0", &mut buf) {
        Some(n) => println!(
            "hello.txt ({} bytes): {}",
            n,
            core::str::from_utf8(&buf[..n]).unwrap_or("?").trim_end()
        ),
        None => println!("cannot read /hello.txt"),
    }

    unsafe {
        match TEST_FAULT {
            1 => {
                let _ = core::ptr::read_volatile(0x10 as *const u8);
            }
            2 => asm!("hlt"),
            3 => {
                let _ = core::ptr::read_volatile(0xffff_ffff_8000_0000u64 as *const u8);
            }
            _ => {}
        }
    }

    println!("exiting with code 42");
    exit(42)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    exit(101)
}
