//! Bảng thiết bị khối. Tên thiết bị = <loại><số>: rd0, ata0, nvme0...
use core::ffi::c_void;
use core::ptr::{addr_of_mut, null};
use driver_abi::BlockDev;

#[derive(Clone, Copy)]
pub struct Dev {
    pub kind: &'static str,
    pub index: usize,
    pub dev: BlockDev,
}

const MAX: usize = 16;
static mut DEVS: [Option<Dev>; MAX] = [None; MAX];

/// Đăng ký thiết bị khối, trả về số thứ tự trong loại đó (rd0, rd1...).
pub fn register(kind: &'static str, dev: BlockDev) -> Option<usize> {
    unsafe {
        let t = &mut *addr_of_mut!(DEVS);
        let mut index = 0;
        for s in t.iter() {
            if let Some(d) = s {
                if d.kind == kind {
                    index += 1;
                }
            }
        }
        for s in t.iter_mut() {
            if s.is_none() {
                *s = Some(Dev { kind, index, dev });
                return Some(index);
            }
        }
    }
    None
}

pub fn get(i: usize) -> Option<Dev> {
    unsafe { (*addr_of_mut!(DEVS)).get(i).copied().flatten() }
}

// ---------- ramdisk (module do Limine nạp) ----------

#[derive(Clone, Copy)]
struct RamDisk {
    base: *const u8,
    len: usize,
}

static mut RAMDISKS: [RamDisk; 4] = [RamDisk { base: null(), len: 0 }; 4];
static mut NRD: usize = 0;

extern "C" fn rd_read(ctx: *mut c_void, off: u64, buf: *mut u8, len: usize) -> bool {
    let r = unsafe { &*(ctx as *const RamDisk) };
    let end = match off.checked_add(len as u64) {
        Some(e) => e,
        None => return false,
    };
    if end > r.len as u64 {
        return false;
    }
    unsafe { core::ptr::copy_nonoverlapping(r.base.add(off as usize), buf, len) };
    true
}

pub fn add_ramdisk(img: &'static [u8]) -> Option<usize> {
    unsafe {
        let slot = NRD;
        if slot >= 4 {
            return None;
        }
        NRD += 1;
        let rds = &mut *addr_of_mut!(RAMDISKS);
        rds[slot] = RamDisk { base: img.as_ptr(), len: img.len() };
        let ctx = &mut rds[slot] as *mut RamDisk as *mut c_void;
        register("rd", BlockDev { ctx, size: img.len() as u64, read: rd_read })
    }
}
