//! Cài các hàm std_* bằng syscall Linux trực tiếp (app này link tĩnh, không dùng libstd.so).
use core::arch::asm;

#[inline(always)]
unsafe fn sc(n: i64, a: i64, b: i64, c: i64, d: i64) -> i64 {
    let r: i64;
    asm!("syscall", inlateout("rax") n => r, in("rdi") a, in("rsi") b,
         in("rdx") c, in("r10") d, lateout("rcx") _, lateout("r11") _,
         options(nostack));
    r
}
#[inline(always)]
unsafe fn sc6(n: i64, a: i64, b: i64, c: i64, d: i64, e: i64, f: i64) -> i64 {
    let r: i64;
    asm!("syscall", inlateout("rax") n => r, in("rdi") a, in("rsi") b,
         in("rdx") c, in("r10") d, in("r8") e, in("r9") f,
         lateout("rcx") _, lateout("r11") _, options(nostack));
    r
}

#[no_mangle] pub unsafe extern "C" fn std_read(fd: i32, buf: *mut u8, n: usize) -> i64 { sc(0, fd as i64, buf as i64, n as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_write(fd: i32, buf: *const u8, n: usize) -> i64 { sc(1, fd as i64, buf as i64, n as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_open(path: *const u8, flags: i32) -> i64 { sc(2, path as i64, flags as i64, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_close(fd: i32) -> i64 { sc(3, fd as i64, 0, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_lseek(fd: i32, off: i64, whence: i32) -> i64 { sc(8, fd as i64, off, whence as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_ioctl(fd: i32, req: u64, arg: u64) -> i64 { sc(16, fd as i64, req as i64, arg as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_munmap(p: *mut u8, len: usize) -> i64 { sc(11, p as i64, len as i64, 0, 0) }
#[no_mangle]
pub unsafe extern "C" fn std_exit(code: i32) -> ! {
    sc(60, code as i64, 0, 0, 0); // giống hello (SYS_EXIT = 60)
    loop {}
}
#[no_mangle]
pub unsafe extern "C" fn std_mmap_anon(len: usize) -> *mut u8 {
    let r = sc6(9, 0, len as i64, 3, 0x22, -1, 0); // PROT_RW, MAP_PRIVATE|MAP_ANONYMOUS
    if r < 0 && r > -4096 { core::ptr::null_mut() } else { r as *mut u8 }
}
#[no_mangle]
pub unsafe extern "C" fn std_print_i64(v: i64) {
    let mut buf = [0u8; 21];
    let mut i = buf.len();
    let neg = v < 0;
    let mut u: u64 = if neg { (!(v as u64)).wrapping_add(1) } else { v as u64 };
    loop {
        i -= 1;
        buf[i] = b'0' + (u % 10) as u8;
        u /= 10;
        if u == 0 { break; }
    }
    if neg { i -= 1; buf[i] = b'-'; }
    std_write(1, buf.as_ptr().add(i), buf.len() - i);
}
