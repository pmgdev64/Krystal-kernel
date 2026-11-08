#![no_std]

use core::ffi::c_void;

pub const DRIVER_ABI: u32 = 1;

pub const KIND_VIDEO: u32 = 1;
pub const KIND_SERIAL: u32 = 2;

/// Driver tự điền rồi đưa cho kernel qua `register`. Kernel copy lại.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DriverInfo {
    pub name: [u8; 16],
    pub kind: u32,
    pub ops: *const c_void,
}

/// Kernel đưa cho driver lúc gọi driver_entry.
#[repr(C)]
pub struct KernelApi {
    pub abi_version: u32,
    pub log: extern "C" fn(*const u8, usize),
    pub register: extern "C" fn(*const DriverInfo) -> i32,
    pub virt_to_phys: extern "C" fn(*const u8) -> u64,
    pub map_mmio: extern "C" fn(u64, usize) -> *mut u8,
}

/// Chữ ký entry của mọi driver. Trả 0 là thành công.
pub type DriverEntry = extern "C" fn(*const KernelApi) -> i32;

/// Bảng hàm của driver loại KIND_SERIAL.
#[repr(C)]
pub struct SerialOps {
    pub write_str: extern "C" fn(*const u8, usize),
}

pub const fn name(s: &str) -> [u8; 16] {
    let b = s.as_bytes();
    let mut o = [0u8; 16];
    let mut i = 0;
    while i < b.len() && i < 15 {
        o[i] = b[i];
        i += 1;
    }
    o
}

pub const KIND_INITRD: u32 = 3;

/// Một file trong initrd. `name` là chuỗi kết thúc bằng NUL.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FileEntry {
    pub name: [u8; 100],
    pub data: *const u8,
    pub size: usize,
}

impl FileEntry {
    pub const fn empty() -> Self {
        Self { name: [0; 100], data: core::ptr::null(), size: 0 }
    }
}

/// Bảng hàm của driver loại KIND_INITRD.
#[repr(C)]
pub struct InitrdOps {
    pub init: extern "C" fn(*const u8, usize) -> bool,
    pub count: extern "C" fn() -> usize,
    pub get: extern "C" fn(usize, *mut FileEntry) -> bool,
    pub find: extern "C" fn(*const u8, usize, *mut FileEntry) -> bool,
}

pub const KIND_FS: u32 = 4;

/// Thiết bị khối: kernel/driver ổ đĩa cung cấp, driver FS đọc qua đây.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct BlockDev {
    pub ctx: *mut c_void,
    pub size: u64,
    pub read: extern "C" fn(*mut c_void, u64, *mut u8, usize) -> bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FsStat {
    pub size: u64,
    pub is_dir: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DirEntry {
    pub name: [u8; 100],
    pub size: u64,
    pub is_dir: bool,
}

impl DirEntry {
    pub const fn empty() -> Self {
        Self { name: [0; 100], size: 0, is_dir: false }
    }
}

/// Bảng hàm của driver loại KIND_FS. Path luôn tương đối so với root của volume.
#[repr(C)]
pub struct FsOps {
    pub fs_name: [u8; 16],
    pub probe: extern "C" fn(*const BlockDev) -> bool,
    pub mount: extern "C" fn(*const BlockDev) -> *mut c_void,
    pub unmount: extern "C" fn(*mut c_void),
    pub stat: extern "C" fn(*mut c_void, *const u8, usize, *mut FsStat) -> bool,
    /// (inst, path, path_len, offset, buf, buf_len) -> số byte đọc được
    pub read: extern "C" fn(*mut c_void, *const u8, usize, u64, *mut u8, usize) -> usize,
    /// (inst, path, path_len, index, out) -> false nếu hết
    pub readdir: extern "C" fn(*mut c_void, *const u8, usize, usize, *mut DirEntry) -> bool,
}

pub const KIND_DEBUG: u32 = 5;

/// Bảng hàm của driver loại KIND_DEBUG (symbol resolver).
#[repr(C)]
pub struct DebugOps {
    /// (ptr, len) của file kernel.sym (output của `nm -n -C`)
    pub init: extern "C" fn(*const u8, usize) -> bool,
    /// (addr, buf, cap, out_offset) -> true nếu tìm được; buf là chuỗi NUL-terminated
    pub resolve: extern "C" fn(u64, *mut u8, usize, *mut u64) -> bool,
}

pub const KIND_KEYBOARD: u32 = 6;

/// Bảng hàm của driver loại KIND_KEYBOARD.
#[repr(C)]
pub struct KeyboardOps {
    /// Trả về 1 byte ASCII (Enter = '\n', Backspace = 0x08, Ctrl+X = 1..26), -1 nếu chưa có phím.
    pub poll: extern "C" fn() -> i32,
}

pub const KIND_BEEP: u32 = 7;

#[repr(C)]
pub struct BeepOps {
    pub beep: extern "C" fn(freq: u32),
}

pub const KIND_GFX: u32 = 8;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GfxMode {
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub pitch: u32,
}

/// Bảng hàm của driver loại KIND_GFX (đổi độ phân giải lúc chạy).
#[repr(C)]
pub struct GfxOps {
    pub vram_bytes: extern "C" fn() -> u64,
    pub get_mode: extern "C" fn(*mut GfxMode) -> bool,
    /// (width, height, bpp) -> true nếu đổi được; lỗi thì tự khôi phục mode cũ
    pub set_mode: extern "C" fn(u32, u32, u32) -> bool,
    pub mode_count: extern "C" fn() -> usize,
    pub mode_at: extern "C" fn(usize, *mut GfxMode) -> bool,
}

pub const KIND_MOUSE: u32 = 9;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MouseState {
    pub x: i32,
    pub y: i32,
    /// bit0 = trái, bit1 = phải, bit2 = giữa
    pub buttons: i32,
}

/// Bảng hàm của driver loại KIND_MOUSE (PS/2, polling).
#[repr(C)]
pub struct MouseOps {
    /// Kích thước vùng kẹp con trỏ (thường = độ phân giải màn hình).
    pub set_bounds: extern "C" fn(i32, i32),
    /// Hút hết packet đang chờ trong i8042.
    pub poll: extern "C" fn(),
    pub get: extern "C" fn(*mut MouseState) -> bool,
}

pub const KIND_AUDIO: u32 = 10;

/// ioctl cho /dev/audio0. Kernel map các lệnh này sang AudioOps.
pub const AUDIO_IOC_SET_RATE: u64 = 0x4101; // arg = Hz; trả 0 nếu ok, <0 nếu lỗi
pub const AUDIO_IOC_GET_RATE: u64 = 0x4102; // trả Hz hiện tại
pub const AUDIO_IOC_SET_VOLUME: u64 = 0x4103; // arg = left | (right << 8), mỗi kênh 0..=100
pub const AUDIO_IOC_DRAIN: u64 = 0x4104; // chờ phát hết
pub const AUDIO_IOC_STOP: u64 = 0x4105; // dừng và bỏ dữ liệu còn lại

/// Bảng hàm của driver loại KIND_AUDIO. PCM luôn là signed 16-bit little-endian, stereo.
#[repr(C)]
pub struct AudioOps {
    pub set_rate: extern "C" fn(u32) -> bool,
    pub get_rate: extern "C" fn() -> u32,
    /// (left, right) 0..=100
    pub set_volume: extern "C" fn(u8, u8),
    /// (buf, bytes) -> số byte đã nhận. Chặn (polling) tới khi có chỗ trong ring.
    pub write: extern "C" fn(*const u8, usize) -> usize,
    pub drain: extern "C" fn(),
    pub stop: extern "C" fn(),
    /// Panic: giu DMA quay vong ring buffer cu, KHONG cleanup (giong Windows BSOD).
    pub hold: extern "C" fn(),
}
pub const KIND_NET: u32 = 11;

pub const NET_MAX_FRAME: usize = 1514; // không gồm FCS

/// Bảng hàm của driver loại KIND_NET (Ethernet, polling).
#[repr(C)]
pub struct NetOps {
    /// Kernel goi 1 lan sau khi init pmm/paging: map MMIO, reset card, dung ring.
    pub init: extern "C" fn() -> bool,
    /// Ghi 6 byte MAC vào buffer.
    pub mac: extern "C" fn(*mut u8),
    pub link_up: extern "C" fn() -> bool,
    /// Gửi 1 frame Ethernet (14..=1514 byte). false nếu ring TX đầy hoặc frame sai.
    pub send: extern "C" fn(*const u8, usize) -> bool,
    /// Lấy 1 frame đã nhận, copy tối đa cap byte. Trả số byte copy, 0 = chưa có frame.
    pub recv: extern "C" fn(*mut u8, usize) -> usize,
}

/// ioctl cho /dev/net0. Trả >= 0 nếu ok.
pub const NET_IOC_GET_MAC: u64 = 0x4201; // trả MAC dạng u64 little-endian (byte0 = bit 0..7)
pub const NET_IOC_LINK: u64 = 0x4202; // trả 1 nếu link up, 0 nếu down
