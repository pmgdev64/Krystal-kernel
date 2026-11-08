use crate::drivers;
use bootvid_abi::{Color, FbInfo};
use core::fmt::{self, Write};
use core::ptr::addr_of_mut;

static mut SAVED: Option<FbInfo> = None;

/// FbInfo da dung de init; panic dung lai de dung Screen sach.
pub fn saved() -> Option<FbInfo> {
    unsafe { *addr_of_mut!(SAVED) }
}

pub fn init(fb: &FbInfo) -> bool {
    unsafe { *addr_of_mut!(SAVED) = Some(*fb) };
    drivers::video_ops()
        .map(|o| (o.init)(fb as *const _))
        .unwrap_or(false)
}

pub fn set_fg(c: Color) {
    if let Some(o) = drivers::video_ops() {
        (o.set_colors)(c, Color::BLACK);
    }
}

pub fn fill_rect(x: usize, y: usize, w: usize, h: usize, c: Color) {
    if let Some(o) = drivers::video_ops() {
        (o.fill_rect)(x, y, w, h, c);
    }
}

pub fn text_size() -> (usize, usize) {
    let (mut c, mut r) = (0usize, 0usize);
    if let Some(o) = drivers::video_ops() {
        (o.text_size)(&mut c, &mut r);
    }
    (c, r)
}

pub fn force_unlock() {
    if let Some(o) = drivers::video_ops() {
        (o.force_unlock)();
    }
}

/// Ghi ra màn hình (bootvid) và mirror ra serial (serial.sys) nếu có.
struct W;
impl Write for W {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let (Some(o), false) = (drivers::video_ops(), gfx_active()) {
            (o.write_str)(s.as_ptr(), s.len());
        }
        if let Some(o) = drivers::serial_ops() {
            (o.write_str)(s.as_ptr(), s.len());
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let _ = W.write_fmt(args);
}

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::video::_print(core::format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($($arg:tt)*) => { $crate::kprint!("{}\n", core::format_args!($($arg)*)) };
}

/// Đổi độ phân giải qua driver gfx rồi init lại bootvid. Lỗi thì quay về mode cũ.
static MODE_GEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

pub fn mode_gen() -> u32 {
    MODE_GEN.load(core::sync::atomic::Ordering::Relaxed)
}

pub fn set_mode(w: u32, h: u32) -> bool {
    let g = match drivers::gfx_ops() {
        Some(g) => g,
        None => return false,
    };
    let old = match saved() {
        Some(f) => f,
        None => return false,
    };
    if !(g.set_mode)(w, h, 32) {
        return false;
    }
    let mut m = driver_abi::GfxMode { width: 0, height: 0, bpp: 0, pitch: 0 };
    if !(g.get_mode)(&mut m) {
        return false;
    }
    let mut fb = old;
    fb.width = m.width as usize;
    fb.height = m.height as usize;
    fb.pitch = m.pitch as usize;
    fb.bpp = m.bpp as usize;
    if init(&fb) {
        MODE_GEN.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return true;
    }
    (g.set_mode)(old.width as u32, old.height as u32, 32);
    init(&old);
    MODE_GEN.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    false
}

/// pid cua process dang giu framebuffer (mmap devices://fb0); -1 = khong ai.
/// Khi co chu, kprint! chi di ra serial de khong ve de len giao dien do hoa.
pub static GFX_OWNER: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(-1);

pub fn gfx_active() -> bool {
    GFX_OWNER.load(core::sync::atomic::Ordering::Relaxed) >= 0
}
