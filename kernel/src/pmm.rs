//! Physical frame allocator (4 KiB). Bump qua cac vung USABLE cua Limine;
//! frame tra lai (free) di vao free list noi trong chinh frame do.
use core::ptr::{addr_of, addr_of_mut, write_bytes};

pub const PAGE: u64 = 4096;
const MAX_REGIONS: usize = 64;

#[derive(Clone, Copy)]
struct Region {
    base: u64,
    end: u64,
}

static mut REGIONS: [Region; MAX_REGIONS] = [Region { base: 0, end: 0 }; MAX_REGIONS];
static mut NREG: usize = 0;
static mut CUR: usize = 0;
static mut NEXT: u64 = 0;
static mut FREE_HEAD: u64 = 0;
static mut HHDM: u64 = 0;
static mut IN_USE: usize = 0;

/// `usable` = (base, length) cua cac vung USABLE trong memory map.
pub unsafe fn init(hhdm: u64, usable: impl Iterator<Item = (u64, u64)>) {
    HHDM = hhdm;
    let regs = &mut *addr_of_mut!(REGIONS);
    let mut n = 0;
    for (base, len) in usable {
        // bo qua vung thap (<1 MiB), canh le theo trang
        let start = (base.max(0x10_0000) + PAGE - 1) & !(PAGE - 1);
        let end = (base + len) & !(PAGE - 1);
        if start < end && n < MAX_REGIONS {
            regs[n] = Region { base: start, end };
            n += 1;
        }
    }
    NREG = n;
    POOL = regs[..n].iter().map(|r| ((r.end - r.base) / PAGE) as usize).sum::<usize>();
    CUR = 0;
    NEXT = if n > 0 { regs[0].base } else { 0 };
}

/// Dia chi ao (HHDM) cua mot dia chi vat ly.
#[inline]
pub fn virt(phys: u64) -> *mut u8 {
    unsafe { (HHDM + phys) as *mut u8 }
}

/// Cap 1 frame da xoa ve 0, tra ve dia chi vat ly.
pub fn alloc() -> Option<u64> {
    unsafe {
        let p = if FREE_HEAD != 0 {
            let p = FREE_HEAD;
            FREE_HEAD = (virt(p) as *const u64).read();
            p
        } else {
            let regs = &*addr_of!(REGIONS);
            loop {
                if CUR >= NREG {
                    return None;
                }
                if NEXT + PAGE <= regs[CUR].end {
                    let p = NEXT;
                    NEXT += PAGE;
                    break p;
                }
                CUR += 1;
                if CUR < NREG {
                    NEXT = regs[CUR].base;
                }
            }
        };
        write_bytes(virt(p), 0, PAGE as usize);
        IN_USE += 1;
        Some(p)
    }
}

pub fn free(phys: u64) {
    unsafe {
        (virt(phys) as *mut u64).write(FREE_HEAD);
        FREE_HEAD = phys;
        IN_USE -= 1;
    }
}

/// So frame dang cap phat (de kiem tra ro ri).
pub fn in_use() -> usize {
    unsafe { IN_USE }
}

static mut POOL: usize = 0;
static mut TOTAL: u64 = 0;

/// Tổng RAM (tính từ memory map của Limine), gọi 1 lần lúc boot.
pub fn set_total(bytes: u64) {
    unsafe { TOTAL = bytes }
}

pub fn total_bytes() -> u64 {
    unsafe { TOTAL }
}

/// Tổng RAM mà allocator có thể cấp (vùng USABLE từ 1 MiB trở lên).
pub fn pool_bytes() -> u64 {
    unsafe { POOL as u64 * PAGE }
}

/// RAM còn trống trong pool.
pub fn free_bytes() -> u64 {
    unsafe { POOL.saturating_sub(IN_USE) as u64 * PAGE }
}

/// Set HHDM sớm (driver nạp trước pmm::init).
pub fn set_hhdm(h: u64) {
    unsafe { HHDM = h }
}
