//! Driver manager: nạp .sys, gọi driver_entry, giữ bảng driver đã đăng ký.
use bootvid_abi::BootvidOps;
use core::ffi::c_void;
use core::ptr::{addr_of, addr_of_mut};
use driver_abi::*;

const MAX_DRIVERS: usize = 16;
static mut TABLE: [Option<DriverInfo>; MAX_DRIVERS] = [None; MAX_DRIVERS];

extern "C" fn api_register(info: *const DriverInfo) -> i32 {
    unsafe {
        let t = &mut *addr_of_mut!(TABLE);
        for slot in t.iter_mut() {
            if slot.is_none() {
                *slot = Some(*info);
                return 0;
            }
        }
    }
    -1
}

extern "C" fn api_log(ptr: *const u8, len: usize) {
    let s = unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) };
    crate::kprint!("{}", s);
}

extern "C" fn api_virt_to_phys(p: *const u8) -> u64 {
    crate::paging::virt_to_phys(p as u64)
}

extern "C" fn api_map_mmio(pa: u64, len: usize) -> *mut u8 {
    unsafe { crate::paging::map_mmio(pa, len) as *mut u8 }
}

static API: KernelApi = KernelApi {
    abi_version: DRIVER_ABI,
    log: api_log,
    register: api_register,
    virt_to_phys: api_virt_to_phys,
    map_mmio: api_map_mmio,
};

/// Load 1 file .sys: map vào vùng driver, relocate, gọi driver_entry.
pub unsafe fn load(img: &[u8]) -> Result<(), &'static str> {
    let entry = crate::loader::load(img)?;
    let driver_entry: DriverEntry = core::mem::transmute(entry);
    if driver_entry(&API) != 0 {
        return Err("driver_entry returned error");
    }
    Ok(())
}

/// Driver thứ n (đếm từ 0) thuộc loại `kind`.
fn nth_of_kind(kind: u32, n: usize) -> Option<*const c_void> {
    let mut c = 0;
    unsafe {
        let t = &*addr_of!(TABLE);
        for slot in t.iter() {
            if let Some(i) = slot {
                if i.kind == kind {
                    if c == n {
                        return Some(i.ops);
                    }
                    c += 1;
                }
            }
        }
    }
    None
}

pub fn video_ops() -> Option<&'static BootvidOps> {
    nth_of_kind(KIND_VIDEO, 0).map(|p| unsafe { &*(p as *const BootvidOps) })
}

pub fn serial_ops() -> Option<&'static SerialOps> {
    nth_of_kind(KIND_SERIAL, 0).map(|p| unsafe { &*(p as *const SerialOps) })
}

pub fn fs_ops(n: usize) -> Option<&'static FsOps> {
    nth_of_kind(KIND_FS, n).map(|p| unsafe { &*(p as *const FsOps) })
}

pub fn cstr(b: &[u8]) -> &str {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..end]).unwrap_or("?")
}

pub fn list() {
    unsafe {
        let t = &*addr_of!(TABLE);
        for slot in t.iter() {
            if let Some(i) = slot {
                let kind = match i.kind {
                    KIND_VIDEO => "video",
                    KIND_SERIAL => "serial",
                    KIND_INITRD => "initrd",
                    KIND_FS => "filesystem",
                    KIND_DEBUG => "debug",
                    KIND_KEYBOARD => "keyboard",
                    KIND_GFX => "gfx",
                    KIND_BEEP => "beep",
                    KIND_AUDIO => "audio",
                    KIND_MOUSE => "mouse",
                    KIND_NET => "network",
                    _ => "unknown",
                };
                crate::kprintln!("  {:<10} type={}", cstr(&i.name), kind);
            }
        }
    }
}

pub fn debug_ops() -> Option<&'static DebugOps> {
    nth_of_kind(KIND_DEBUG, 0).map(|p| unsafe { &*(p as *const DebugOps) })
}

pub fn keyboard_ops() -> Option<&'static KeyboardOps> {
    nth_of_kind(KIND_KEYBOARD, 0).map(|p| unsafe { &*(p as *const KeyboardOps) })
}

pub fn beep_ops() -> Option<&'static BeepOps> {
    nth_of_kind(KIND_BEEP, 0).map(|p| unsafe { &*(p as *const BeepOps) })
}

pub fn gfx_ops() -> Option<&'static GfxOps> {
    nth_of_kind(KIND_GFX, 0).map(|p| unsafe { &*(p as *const GfxOps) })
}

pub fn mouse_ops() -> Option<&'static MouseOps> {
    nth_of_kind(KIND_MOUSE, 0).map(|p| unsafe { &*(p as *const MouseOps) })
}

/// Driver audio đầu tiên (KIND_AUDIO), nếu có.
pub fn audio_ops() -> Option<&'static AudioOps> {
    nth_of_kind(KIND_AUDIO, 0).map(|p| unsafe { &*(p as *const AudioOps) })
}

/// Driver mạng đầu tiên (KIND_NET), nếu có.
pub fn net_ops() -> Option<&'static NetOps> {
    nth_of_kind(KIND_NET, 0).map(|p| unsafe { &*(p as *const NetOps) })
}

/// Gọi sau pmm::init + paging::init: driver mạng map MMIO, reset card.
pub fn net_init() {
    if let Some(n) = net_ops() {
        if !(n.init)() {
            crate::kprintln!("net: init failed");
        }
    }
}
