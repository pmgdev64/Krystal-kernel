//! Paging cho user-space. Moi app co 1 PML4 rieng: nua tren (256..512) copy tu
//! page table kernel (Limine) de kernel van map khi dang o CR3 cua app;
//! nua duoi (0..256) la cua app.
use crate::pmm;
use core::arch::asm;

pub const PRESENT: u64 = 1 << 0;
pub const WRITE: u64 = 1 << 1;
pub const USER: u64 = 1 << 2;
pub const NX: u64 = 1 << 63;
const HUGE: u64 = 1 << 7;
/// Bit phan mem: trang tro toi MMIO (framebuffer), khong thuoc pmm.
pub const AVL: u64 = 1 << 9;
const ADDR: u64 = 0x000f_ffff_ffff_f000;

/// Vung dia chi user = [0, USER_END)
pub const USER_END: u64 = 0x0000_8000_0000_0000;

static mut KERNEL_CR3: u64 = 0;

#[derive(Clone, Copy)]
pub struct Space {
    /// Dia chi vat ly cua PML4
    pub pml4: u64,
}

pub fn read_cr3() -> u64 {
    let v: u64;
    unsafe { asm!("mov {}, cr3", out(reg) v, options(nomem, nostack)) };
    v
}

pub unsafe fn write_cr3(v: u64) {
    asm!("mov cr3, {}", in(reg) v, options(nostack));
}

pub fn kernel_cr3() -> u64 {
    unsafe { KERNEL_CR3 }
}

pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack));
    ((hi as u64) << 32) | lo as u64
}

pub unsafe fn wrmsr(msr: u32, v: u64) {
    asm!("wrmsr", in("ecx") msr, in("eax") v as u32, in("edx") (v >> 32) as u32,
         options(nomem, nostack));
}

/// Goi sau pmm::init.
pub unsafe fn init() {
    KERNEL_CR3 = read_cr3();
    // EFER.NXE de dung bit NX
    wrmsr(0xC000_0080, rdmsr(0xC000_0080) | (1 << 11));
    mmio_window_init();
}

pub unsafe fn new_space() -> Option<Space> {
    let p = pmm::alloc()?;
    let new = pmm::virt(p) as *mut u64;
    let kern = pmm::virt(KERNEL_CR3 & ADDR) as *const u64;
    for i in 256..512 {
        new.add(i).write(kern.add(i).read());
    }
    Some(Space { pml4: p })
}

/// Tim entry PT (level 1) cua `va`; `create` = tao bang trung gian neu thieu.
unsafe fn walk(sp: &Space, va: u64, create: bool) -> Option<*mut u64> {
    let idx = [(va >> 39) & 511, (va >> 30) & 511, (va >> 21) & 511, (va >> 12) & 511];
    let mut t = pmm::virt(sp.pml4) as *mut u64;
    for lvl in 0..3 {
        let e = t.add(idx[lvl] as usize);
        let v = e.read();
        let next = if v & PRESENT != 0 {
            if v & HUGE != 0 {
                return None;
            }
            v & ADDR
        } else {
            if !create {
                return None;
            }
            let f = pmm::alloc()?;
            e.write(f | PRESENT | WRITE | USER);
            f
        };
        t = pmm::virt(next) as *mut u64;
    }
    Some(t.add(idx[3] as usize))
}

/// Dam bao trang `va` (user) da map; chua co thi cap frame moi (da xoa 0).
/// Trang da co thi chi noi them quyen. Tra ve dia chi vat ly cua frame.
pub unsafe fn ensure_user_page(sp: &Space, va: u64, write: bool, exec: bool) -> Option<u64> {
    if va >= USER_END || va & 0xfff != 0 {
        return None;
    }
    let e = walk(sp, va, true)?;
    let v = e.read();
    if v & PRESENT != 0 {
        let mut nv = v;
        if write {
            nv |= WRITE;
        }
        if exec {
            nv &= !NX;
        }
        e.write(nv);
        return Some(v & ADDR);
    }
    let f = pmm::alloc()?;
    let mut fl = PRESENT | USER;
    if write {
        fl |= WRITE;
    }
    if !exec {
        fl |= NX;
    }
    e.write(f | fl);
    Some(f)
}

/// (dia chi vat ly kem offset, flags cua PTE) hoac None neu chua map.
pub unsafe fn translate(sp: &Space, va: u64) -> Option<(u64, u64)> {
    if va >= USER_END {
        return None;
    }
    let e = walk(sp, va & !0xfff, false)?.read();
    if e & PRESENT == 0 {
        return None;
    }
    Some(((e & ADDR) | (va & 0xfff), e))
}

/// Copy user -> kernel. false neu co trang nao khong map / khong phai USER.
pub unsafe fn copy_from_user(sp: &Space, src: u64, dst: &mut [u8]) -> bool {
    if src.checked_add(dst.len() as u64).map_or(true, |e| e > USER_END) {
        return false;
    }
    let mut done = 0usize;
    while done < dst.len() {
        let va = src + done as u64;
        let (pa, fl) = match translate(sp, va) {
            Some(x) => x,
            None => return false,
        };
        if fl & USER == 0 {
            return false;
        }
        let n = (dst.len() - done).min(4096 - (va & 0xfff) as usize);
        core::ptr::copy_nonoverlapping(pmm::virt(pa), dst.as_mut_ptr().add(done), n);
        done += n;
    }
    true
}

/// Copy kernel -> user. `need_write` = bat buoc trang phai co quyen ghi
/// (syscall: true; loader tu ghi vao trang read-only de relocate: false).
pub unsafe fn copy_to_user(sp: &Space, dst: u64, src: &[u8], need_write: bool) -> bool {
    if dst.checked_add(src.len() as u64).map_or(true, |e| e > USER_END) {
        return false;
    }
    let mut done = 0usize;
    while done < src.len() {
        let va = dst + done as u64;
        let (pa, fl) = match translate(sp, va) {
            Some(x) => x,
            None => return false,
        };
        if fl & USER == 0 || (need_write && fl & WRITE == 0) {
            return false;
        }
        let n = (src.len() - done).min(4096 - (va & 0xfff) as usize);
        core::ptr::copy_nonoverlapping(src.as_ptr().add(done), pmm::virt(pa), n);
        done += n;
    }
    true
}

/// Giai phong toan bo nua user (frame + bang trang) va PML4.
pub unsafe fn destroy(sp: Space) {
    let pml4 = pmm::virt(sp.pml4) as *const u64;
    for i in 0..256 {
        let e = pml4.add(i).read();
        if e & PRESENT != 0 {
            free_tree(e & ADDR, 3);
        }
    }
    pmm::free(sp.pml4);
}

/// level 3 = PDPT, 2 = PD, 1 = PT (entry cua PT tro toi frame du lieu).
unsafe fn free_tree(phys: u64, level: u32) {
    let t = pmm::virt(phys) as *const u64;
    for i in 0..512 {
        let e = t.add(i).read();
        if e & PRESENT == 0 {
            continue;
        }
        if level == 1 {
            if e & AVL == 0 {
                pmm::free(e & ADDR);
            }
        } else {
            free_tree(e & ADDR, level - 1);
        }
    }
    pmm::free(phys);
}

/// Bo map 1 trang user va tra frame ve pmm (khong lam gi neu chua map).
pub unsafe fn unmap_user_page(sp: &Space, va: u64) {
    if va >= USER_END {
        return;
    }
    if let Some(e) = walk(sp, va & !0xfff, false) {
        let v = e.read();
        if v & PRESENT != 0 {
            if v & AVL == 0 {
                pmm::free(v & ADDR);
            }
            e.write(0);
        }
    }
}

/// Doi quyen 1 trang user da map.
pub unsafe fn protect_user_page(sp: &Space, va: u64, write: bool, exec: bool) {
    if va >= USER_END {
        return;
    }
    if let Some(e) = walk(sp, va & !0xfff, false) {
        let mut v = e.read();
        if v & PRESENT != 0 {
            v &= !(WRITE | NX);
            if write {
                v |= WRITE;
            }
            if !exec {
                v |= NX;
            }
            e.write(v);
        }
    }
}

/// Xoa TLB (nap lai CR3).
pub unsafe fn flush() {
    write_cr3(read_cr3());
}

/// Nhan ban toan bo nua user cua `sp` (frame + bang trang) cho fork.
pub unsafe fn clone_space(sp: &Space) -> Option<Space> {
    let new = new_space()?;
    let src = pmm::virt(sp.pml4) as *const u64;
    let dst = pmm::virt(new.pml4) as *mut u64;
    for i in 0..256 {
        let e = src.add(i).read();
        if e & PRESENT != 0 {
            match clone_tree(e & ADDR, 3) {
                Some(f) => dst.add(i).write((e & !ADDR) | f),
                None => {
                    destroy(new);
                    return None;
                }
            }
        }
    }
    Some(new)
}

unsafe fn clone_tree(phys: u64, level: u32) -> Option<u64> {
    let nf = pmm::alloc()?;
    let s = pmm::virt(phys) as *const u64;
    let d = pmm::virt(nf) as *mut u64;
    for i in 0..512 {
        let e = s.add(i).read();
        if e & PRESENT == 0 {
            continue;
        }
        if level == 1 && e & AVL != 0 {
            d.add(i).write(e);
            continue;
        }
        let f = if level == 1 {
            match pmm::alloc() {
                Some(df) => {
                    core::ptr::copy_nonoverlapping(pmm::virt(e & ADDR), pmm::virt(df), 4096);
                    Some(df)
                }
                None => None,
            }
        } else {
            clone_tree(e & ADDR, level - 1)
        };
        match f {
            Some(df) => d.add(i).write((e & !ADDR) | df),
            None => {
                free_tree(nf, level);
                return None;
            }
        }
    }
    Some(nf)
}

/// Map 1 trang vat ly `pa` (MMIO) vao user `va`, danh dau AVL de khong bi free/copy.
pub unsafe fn map_user_phys(sp: &Space, va: u64, pa: u64, write: bool) -> bool {
    if va >= USER_END || va & 0xfff != 0 {
        return false;
    }
    let e = match walk(sp, va, true) {
        Some(e) => e,
        None => return false,
    };
    let old = e.read();
    if old & PRESENT != 0 && old & AVL == 0 {
        pmm::free(old & ADDR);
    }
    let mut fl = PRESENT | USER | AVL | NX;
    if write {
        fl |= WRITE;
    }
    e.write((pa & ADDR) | fl);
    true
}

/// Dịch địa chỉ ảo kernel -> vật lý bằng CR3 hiện tại. 0 nếu chưa map.
pub fn virt_to_phys(va: u64) -> u64 {
    unsafe {
        let mut t = pmm::virt(read_cr3() & ADDR) as *const u64;
        for lvl in 0..4u32 {
            let shift = 39 - 9 * lvl;
            let e = t.add(((va >> shift) & 511) as usize).read();
            if e & PRESENT == 0 {
                return 0;
            }
            if lvl == 3 || (lvl > 0 && e & HUGE != 0) {
                let mask = (1u64 << shift) - 1;
                return (e & ADDR & !mask) | (va & mask);
            }
            t = pmm::virt(e & ADDR) as *const u64;
        }
        0
    }
}

// ---- Cua so MMIO cho driver (HDA...): 1 PML4 slot rieng, tao luc init, moi address space dung chung ----
static mut MMIO_BASE: u64 = 0;
static mut MMIO_PDPT: u64 = 0;

unsafe fn mmio_window_init() {
    let kp = pmm::virt(KERNEL_CR3 & ADDR) as *mut u64;
    for idx in (257..500usize).rev() {
        if kp.add(idx).read() & PRESENT == 0 {
            if let Some(f) = pmm::alloc() {
                kp.add(idx).write(f | PRESENT | WRITE);
                MMIO_PDPT = f;
                MMIO_BASE = 0xffff_0000_0000_0000 | ((idx as u64) << 39);
            }
            return;
        }
    }
}

unsafe fn sub_table(t: *mut u64, i: usize) -> Option<*mut u64> {
    let e = t.add(i).read();
    if e & PRESENT != 0 {
        return Some(pmm::virt(e & ADDR) as *mut u64);
    }
    let f = pmm::alloc()?;
    t.add(i).write(f | PRESENT | WRITE);
    Some(pmm::virt(f) as *mut u64)
}

/// Map MMIO [pa, pa+len) uncached (PCD|PWT). Tra ve dia chi ao, 0 neu loi.
pub unsafe fn map_mmio(pa: u64, len: usize) -> u64 {
    if MMIO_PDPT == 0 || pa >= (1u64 << 39) {
        return 0;
    }
    let start = pa & !0xfff;
    let end = (pa + len as u64 + 0xfff) & !0xfff;
    let mut p = start;
    while p < end {
        let va = MMIO_BASE + p;
        let pdpt = pmm::virt(MMIO_PDPT) as *mut u64;
        let pd = match sub_table(pdpt, ((va >> 30) & 511) as usize) { Some(t) => t, None => return 0 };
        let pt = match sub_table(pd, ((va >> 21) & 511) as usize) { Some(t) => t, None => return 0 };
        pt.add(((va >> 12) & 511) as usize).write(p | PRESENT | WRITE | (1 << 3) | (1 << 4));
        p += 4096;
    }
    MMIO_BASE + pa
}
