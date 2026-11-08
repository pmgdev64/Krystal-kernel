//! Syscall toi thieu + trait Canvas (de libttf khong phu thuoc libgfx).
pub mod sys {
    use core::arch::asm;

    #[inline(always)]
    pub unsafe fn sc(n: i64, a: i64, b: i64, c: i64, d: i64, e: i64, f: i64) -> i64 {
        let r: i64;
        asm!("syscall", inlateout("rax") n => r, in("rdi") a, in("rsi") b,
             in("rdx") c, in("r10") d, in("r8") e, in("r9") f,
             lateout("rcx") _, lateout("r11") _, options(nostack));
        r
    }
    pub fn read(fd: i64, b: &mut [u8]) -> i64 {
        unsafe { sc(0, fd, b.as_mut_ptr() as i64, b.len() as i64, 0, 0, 0) }
    }
    /// `path` ket thuc bang NUL.
    pub fn open(path: &[u8], flags: i64) -> i64 {
        unsafe { sc(2, path.as_ptr() as i64, flags, 0, 0, 0, 0) }
    }
    pub fn close(fd: i64) -> i64 {
        unsafe { sc(3, fd, 0, 0, 0, 0, 0) }
    }
    pub fn lseek(fd: i64, off: i64, whence: i64) -> i64 {
        unsafe { sc(8, fd, off, whence, 0, 0, 0) }
    }
    pub fn mmap_anon(len: usize) -> *mut u8 {
        let r = unsafe { sc(9, 0, len as i64, 3, 0x22, -1, 0) };
        if r < 0 && r > -4096 { core::ptr::null_mut() } else { r as *mut u8 }
    }
    pub fn munmap(p: *mut u8, len: usize) -> i64 {
        unsafe { sc(11, p as i64, len as i64, 0, 0, 0, 0) }
    }
}

/// Noi nhan bitmap coverage 8-bit (Gfx implement trait nay).
pub trait Canvas {
    fn blit_alpha8(&mut self, cov: &[u8], sw: usize, sh: usize, x: i32, y: i32, c: u32);
}
