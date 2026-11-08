//! Đồng hồ dựa trên TSC, hiệu chuẩn bằng PIT kênh 2 (không cần IRQ).
use core::arch::asm;

static mut TSC_PER_MS: u64 = 0;
static mut TSC0: u64 = 0;

unsafe fn inb(p: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", in("dx") p, out("al") v, options(nomem, nostack));
    v
}

unsafe fn outb(p: u16, v: u8) {
    asm!("out dx, al", in("dx") p, in("al") v, options(nomem, nostack));
}

#[inline]
pub fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack)) };
    ((hi as u64) << 32) | lo as u64
}

/// Gọi 1 lần, TRƯỚC khi dùng beep (beep cũng dùng PIT kênh 2).
pub unsafe fn init() {
    let old = inb(0x61);
    outb(0x61, old & 0xFC); // gate thấp, loa tắt
    outb(0x43, 0xB0); // kênh 2, lobyte/hibyte, mode 0
    let count: u16 = 59659; // 1193182 Hz * 50 ms
    outb(0x42, count as u8);
    outb(0x42, (count >> 8) as u8);
    outb(0x61, (old & 0xFC) | 1); // gate cao: bắt đầu đếm
    let t0 = rdtsc();
    let mut guard = 0u64;
    while inb(0x61) & 0x20 == 0 {
        guard += 1;
        if guard > 2_000_000_000 {
            break;
        }
    }
    let t1 = rdtsc();
    outb(0x61, old & 0xFC);
    let per_ms = (t1 - t0) / 50;
    TSC_PER_MS = if per_ms == 0 { 1_000_000 } else { per_ms };
    TSC0 = rdtsc();
}

pub fn uptime_ms() -> u64 {
    unsafe {
        if TSC_PER_MS == 0 {
            return 0;
        }
        (rdtsc() - TSC0) / TSC_PER_MS
    }
}

pub fn sleep_ms(ms: u64) {
    let end = uptime_ms() + ms;
    while uptime_ms() < end {
        crate::task::yield_now();
        core::hint::spin_loop();
    }
}

static mut TICKS: u64 = 0;

pub fn ticks() -> u64 {
    unsafe { TICKS }
}

/// Bat timer IRQ0 `hz` lan/giay (PIC remap -> vector 32, chi mo IRQ0).
/// Ngat chi duoc giao khi CPU o ring 3 (kernel luon chay IF=0).
pub unsafe fn start_tick(hz: u32) {
    outb(0x20, 0x11);
    outb(0xA0, 0x11);
    outb(0x21, 0x20);
    outb(0xA1, 0x28);
    outb(0x21, 0x04);
    outb(0xA1, 0x02);
    outb(0x21, 0x01);
    outb(0xA1, 0x01);
    outb(0x21, 0xFE); // chi IRQ0
    outb(0xA1, 0xFF);
    let div = (1_193_182u32 / hz.max(19)).min(65535) as u16;
    outb(0x43, 0x34); // kenh 0, lo/hi, mode 2
    outb(0x40, div as u8);
    outb(0x40, (div >> 8) as u8);
}

/// Goi tu handler IRQ0.
pub fn tick_eoi() {
    unsafe {
        TICKS += 1;
        outb(0x20, 0x20);
    }
}
