//! Thread toi gian cho app: clone(CLONE_THREAD) + futex. Tu chua, khong alloc.
use core::arch::asm;
use core::cell::UnsafeCell;
use core::ptr::{copy_nonoverlapping, read_volatile};
use core::sync::atomic::{AtomicUsize, Ordering};

#[inline(always)]
unsafe fn sc(n: i64, a: i64, b: i64, c: i64, d: i64, e: i64) -> i64 {
    let r: i64;
    asm!("syscall", inlateout("rax") n => r, in("rdi") a, in("rsi") b,
         in("rdx") c, in("r10") d, in("r8") e,
         lateout("rcx") _, lateout("r11") _, options(nostack));
    r
}

// VM | FS | FILES | SIGHAND | THREAD | CHILD_CLEARTID | CHILD_SETTID
const FLAGS: i64 = 0x100 | 0x200 | 0x400 | 0x800 | 0x1_0000 | 0x20_0000 | 0x0100_0000;
const STACK: usize = 128 * 1024;

pub struct Thread {
    base: *mut u8,
    len: usize,
}

pub fn sleep_ms(ms: u64) {
    let ts = [(ms / 1000) as i64, ((ms % 1000) * 1_000_000) as i64];
    unsafe { sc(35, ts.as_ptr() as i64, 0, 0, 0, 0) };
}

pub fn spawn(f: extern "C" fn(usize), arg: usize) -> Option<Thread> {
    unsafe {
        let r = sc(9, 0, STACK as i64, 3, 0x22, -1);
        if r < 0 && r > -4096 {
            return None;
        }
        let base = r as *mut u8;
        let top = base.add(STACK) as *mut u64; // 16-aligned
        *top.sub(1) = arg as u64;
        *top.sub(2) = f as usize as u64;
        let sp = top.sub(2) as i64;
        let ctid = base as i64; // kernel ghi tid vao day, khi thread thoat ghi 0 + futex_wake
        let ret: i64;
        asm!(
            "syscall",
            "test rax, rax",
            "jnz 2f",
            "xor ebp, ebp",
            "pop rax",
            "pop rdi",
            "call rax",
            "mov eax, 60",
            "xor edi, edi",
            "syscall",
            "ud2",
            "2:",
            inlateout("rax") 56i64 => ret,
            in("rdi") FLAGS,
            in("rsi") sp,
            in("rdx") 0i64,
            in("r10") ctid,
            in("r8") 0i64,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
        if ret <= 0 {
            sc(11, base as i64, STACK as i64, 0, 0, 0);
            return None;
        }
        Some(Thread { base, len: STACK })
    }
}

impl Thread {
    /// Cho thread ket thuc roi giai phong stack.
    pub fn join(self) {
        let p = self.base as *mut u32;
        loop {
            let v = unsafe { read_volatile(p) };
            if v == 0 {
                break;
            }
            unsafe { sc(202, p as i64, 0, v as i64, 0, 0) }; // futex WAIT
        }
        unsafe { sc(11, self.base as i64, self.len as i64, 0, 0, 0) };
    }
}

/// Ring buffer 1 producer / 1 consumer.
pub struct Spsc<const N: usize> {
    buf: UnsafeCell<[u8; N]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}
unsafe impl<const N: usize> Sync for Spsc<N> {}

impl<const N: usize> Spsc<N> {
    pub const fn new() -> Self {
        Self { buf: UnsafeCell::new([0; N]), head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }
    pub fn len(&self) -> usize {
        self.tail.load(Ordering::Acquire).wrapping_sub(self.head.load(Ordering::Acquire))
    }
    /// Producer: tra ve so byte da ghi (0 neu day).
    pub fn push(&self, d: &[u8]) -> usize {
        let t = self.tail.load(Ordering::Relaxed);
        let h = self.head.load(Ordering::Acquire);
        let n = d.len().min(N - t.wrapping_sub(h));
        let (p, s) = (self.buf.get() as *mut u8, t % N);
        let first = n.min(N - s);
        unsafe {
            copy_nonoverlapping(d.as_ptr(), p.add(s), first);
            copy_nonoverlapping(d.as_ptr().add(first), p, n - first);
        }
        self.tail.store(t.wrapping_add(n), Ordering::Release);
        n
    }
    /// Consumer: tra ve so byte da doc (0 neu rong).
    pub fn pop(&self, out: &mut [u8]) -> usize {
        let h = self.head.load(Ordering::Relaxed);
        let t = self.tail.load(Ordering::Acquire);
        let n = out.len().min(t.wrapping_sub(h));
        let (p, s) = (self.buf.get() as *const u8, h % N);
        let first = n.min(N - s);
        unsafe {
            copy_nonoverlapping(p.add(s), out.as_mut_ptr(), first);
            copy_nonoverlapping(p, out.as_mut_ptr().add(first), n - first);
        }
        self.head.store(h.wrapping_add(n), Ordering::Release);
        n
    }
}
