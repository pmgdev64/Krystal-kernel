//! libstd: thư viện chuẩn cho app của myos. Mọi hàm export có tiền tố `std_`.
#![no_std]

use core::arch::asm;

// ---------------- syscall ----------------
#[inline(always)]
unsafe fn sc(n: i64, a: i64, b: i64, c: i64, d: i64) -> i64 {
    let r: i64;
    asm!("syscall", inlateout("rax") n => r, in("rdi") a, in("rsi") b,
         in("rdx") c, in("r10") d, lateout("rcx") _, lateout("r11") _,
         options(nostack));
    r
}

#[no_mangle] pub unsafe extern "C" fn std_read(fd: i32, buf: *mut u8, n: usize) -> i64 { sc(0, fd as i64, buf as i64, n as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_write(fd: i32, buf: *const u8, n: usize) -> i64 { sc(1, fd as i64, buf as i64, n as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_open(path: *const u8, flags: i32) -> i64 { sc(2, path as i64, flags as i64, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_close(fd: i32) -> i64 { sc(3, fd as i64, 0, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_lseek(fd: i32, off: i64, whence: i32) -> i64 { sc(8, fd as i64, off, whence as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_getcwd(buf: *mut u8, n: usize) -> i64 { sc(79, buf as i64, n as i64, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_chdir(path: *const u8) -> i64 { sc(80, path as i64, 0, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_getdents64(fd: i32, buf: *mut u8, n: usize) -> i64 { sc(217, fd as i64, buf as i64, n as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_dup2(old: i32, new: i32) -> i64 { sc(33, old as i64, new as i64, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_getpid() -> i64 { sc(39, 0, 0, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_exit(code: i32) -> ! {
    sc(231, code as i64, 0, 0, 0);
    loop {}
}

// mmap anonymous: PROT_READ|WRITE, MAP_PRIVATE|MAP_ANONYMOUS
#[no_mangle]
pub unsafe extern "C" fn std_mmap_anon(len: usize) -> *mut u8 {
    let r = sc(9, 0, len as i64, 3, 0x22);
    if r < 0 && r > -4096 { core::ptr::null_mut() } else { r as *mut u8 }
}
#[no_mangle] pub unsafe extern "C" fn std_munmap(p: *mut u8, len: usize) -> i64 { sc(11, p as i64, len as i64, 0, 0) }

// ---------------- chuỗi / bộ nhớ ----------------
#[no_mangle]
pub unsafe extern "C" fn std_strlen(s: *const u8) -> usize {
    let mut n = 0;
    while *s.add(n) != 0 { n += 1; }
    n
}
#[no_mangle]
pub unsafe extern "C" fn std_strcmp(a: *const u8, b: *const u8) -> i32 {
    let mut i = 0;
    loop {
        let (x, y) = (*a.add(i), *b.add(i));
        if x != y || x == 0 { return x as i32 - y as i32; }
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn std_strcpy(d: *mut u8, s: *const u8) -> *mut u8 {
    let mut i = 0;
    loop {
        let c = *s.add(i);
        *d.add(i) = c;
        if c == 0 { break; }
        i += 1;
    }
    d
}
#[no_mangle]
pub unsafe extern "C" fn std_memcpy(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    asm!("rep movsb", inout("rcx") n => _, inout("rdi") d => _, inout("rsi") s => _, options(nostack, preserves_flags));
    d
}
#[no_mangle]
pub unsafe extern "C" fn std_memmove(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    if (d as usize) <= (s as usize) {
        for i in 0..n { *d.add(i) = *s.add(i); }
    } else {
        for i in (0..n).rev() { *d.add(i) = *s.add(i); }
    }
    d
}
#[no_mangle]
pub unsafe extern "C" fn std_memset(d: *mut u8, v: i32, n: usize) -> *mut u8 {
    asm!("rep stosb", inout("rcx") n => _, inout("rdi") d => _, in("al") v as u8, options(nostack, preserves_flags));
    d
}
#[no_mangle]
pub unsafe extern "C" fn std_memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        let (x, y) = (*a.add(i), *b.add(i));
        if x != y { return x as i32 - y as i32; }
    }
    0
}

// ---------------- in ra ----------------
#[no_mangle]
pub unsafe extern "C" fn std_puts(s: *const u8) {
    std_write(1, s, std_strlen(s));
    std_write(1, b"\n".as_ptr(), 1);
}
#[no_mangle]
pub unsafe extern "C" fn std_putchar(c: u8) { std_write(1, &c, 1); }

#[no_mangle]
pub unsafe extern "C" fn std_print_i64(mut v: i64) {
    let mut buf = [0u8; 21];
    let mut i = buf.len();
    let neg = v < 0;
    let mut u = if neg { (v as i128).unsigned_abs() as u64 } else { v as u64 };
    v = 0; let _ = v;
    loop {
        i -= 1;
        buf[i] = b'0' + (u % 10) as u8;
        u /= 10;
        if u == 0 { break; }
    }
    if neg { i -= 1; buf[i] = b'-'; }
    std_write(1, buf.as_ptr().add(i), buf.len() - i);
}
#[no_mangle]
pub unsafe extern "C" fn std_print_hex(mut v: u64) {
    let mut buf = [0u8; 16];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b"0123456789abcdef"[(v & 15) as usize];
        v >>= 4;
        if v == 0 { break; }
    }
    std_write(1, b"0x".as_ptr(), 2);
    std_write(1, buf.as_ptr().add(i), buf.len() - i);
}

// ---------------- malloc (bump trên mmap, free chỉ giải phóng khối cuối) ----------------
static mut HEAP_CUR: usize = 0;
static mut HEAP_END: usize = 0;

#[no_mangle]
pub unsafe extern "C" fn std_malloc(n: usize) -> *mut u8 {
    let n = (n + 15) & !15;
    if n == 0 { return core::ptr::null_mut(); }
    if HEAP_CUR + n > HEAP_END {
        let chunk = ((n + 4095) & !4095).max(256 * 1024);
        let p = std_mmap_anon(chunk);
        if p.is_null() { return p; }
        HEAP_CUR = p as usize;
        HEAP_END = HEAP_CUR + chunk;
    }
    let r = HEAP_CUR;
    HEAP_CUR += n;
    r as *mut u8
}
#[no_mangle]
pub unsafe extern "C" fn std_free(_p: *mut u8) {} // bump: chưa tái sử dụng

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }

// ---------------- process / thời gian / thiết bị ----------------
#[no_mangle] pub unsafe extern "C" fn std_fork() -> i64 { sc(57, 0, 0, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_execve(path: *const u8, argv: *const *const u8, envp: *const *const u8) -> i64 { sc(59, path as i64, argv as i64, envp as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_wait4(pid: i64, status: *mut i32, opts: i32) -> i64 { sc(61, pid, status as i64, opts as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_ioctl(fd: i32, req: u64, arg: u64) -> i64 { sc(16, fd as i64, req as i64, arg as i64, 0) }
#[no_mangle] pub unsafe extern "C" fn std_nanosleep(req: *const i64, rem: *mut i64) -> i64 { sc(35, req as i64, rem as i64, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_clock_gettime(clk: i32, ts: *mut i64) -> i64 { sc(228, clk as i64, ts as i64, 0, 0) }
#[no_mangle] pub unsafe extern "C" fn std_sysinfo(info: *mut u8) -> i64 { sc(99, info as i64, 0, 0, 0) }

/// mmap một fd (ví dụ /dev/fb0): PROT_READ|WRITE, MAP_SHARED.
#[no_mangle]
pub unsafe extern "C" fn std_mmap_fd(fd: i32, len: usize, off: i64) -> *mut u8 {
    let r = sc6(9, 0, len as i64, 3, 1, fd as i64, off);
    if r < 0 && r > -4096 { core::ptr::null_mut() } else { r as *mut u8 }
}

#[inline(always)]
unsafe fn sc6(n: i64, a: i64, b: i64, c: i64, d: i64, e: i64, f: i64) -> i64 {
    let r: i64;
    asm!("syscall", inlateout("rax") n => r, in("rdi") a, in("rsi") b,
         in("rdx") c, in("r10") d, in("r8") e, in("r9") f,
         lateout("rcx") _, lateout("r11") _, options(nostack));
    r
}

/// Ngủ ms mili giây.
#[no_mangle]
pub unsafe extern "C" fn std_sleep_ms(ms: u64) {
    let ts = [(ms / 1000) as i64, ((ms % 1000) * 1_000_000) as i64];
    std_nanosleep(ts.as_ptr(), core::ptr::null_mut());
}
