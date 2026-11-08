#![no_std]

/// v2: FbInfo mang day du thong tin mask cua Limine framebuffer.
pub const ABI_VERSION: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const GRAY: Color = Color::rgb(170, 170, 170);
    pub const RED: Color = Color::rgb(255, 80, 80);
    pub const GREEN: Color = Color::rgb(80, 255, 80);
    pub const BLUE: Color = Color::rgb(80, 120, 255);
    pub const YELLOW: Color = Color::rgb(255, 220, 80);
}

/// Mo ta framebuffer, lay nguyen tu Limine framebuffer response
/// (UEFI GOP hoac BIOS VBE deu ra cung dinh dang nay).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FbInfo {
    pub addr: *mut u8,
    pub width: usize,
    pub height: usize,
    /// So byte moi dong (co the lon hon width * bytes_per_pixel)
    pub pitch: usize,
    pub bpp: usize,
    pub red_size: u8,
    pub red_shift: u8,
    pub green_size: u8,
    pub green_shift: u8,
    pub blue_size: u8,
    pub blue_shift: u8,
}

impl FbInfo {
    /// Framebuffer RGB truc tiep 15/16/24/32 bpp thi dung duoc; tra ve ly do neu khong.
    pub fn supported(&self) -> Result<(), &'static str> {
        if self.addr.is_null() {
            return Err("framebuffer address is null");
        }
        if !matches!(self.bpp, 15 | 16 | 24 | 32) {
            return Err("unsupported bpp (need 15/16/24/32)");
        }
        if self.width < 64 || self.height < 64 {
            return Err("framebuffer too small");
        }
        let bytes = (self.bpp + 7) / 8;
        if self.pitch < self.width * bytes {
            return Err("pitch smaller than one row");
        }
        let align = if bytes == 3 { 1 } else { bytes };
        if (self.addr as usize) % align != 0 || self.pitch % align != 0 {
            return Err("misaligned framebuffer");
        }
        let masks = [
            (self.red_size, self.red_shift),
            (self.green_size, self.green_shift),
            (self.blue_size, self.blue_shift),
        ];
        for (size, shift) in masks {
            if size == 0 || size > 8 || size as usize + shift as usize > bytes * 8 {
                return Err("bad colour mask");
            }
        }
        Ok(())
    }
}

#[repr(C)]
pub struct BootvidOps {
    pub abi_version: u32,
    pub init: extern "C" fn(*const FbInfo) -> bool,
    pub clear: extern "C" fn(),
    pub set_colors: extern "C" fn(Color, Color),
    pub put_pixel: extern "C" fn(usize, usize, Color),
    pub fill_rect: extern "C" fn(usize, usize, usize, usize, Color),
    pub write_str: extern "C" fn(*const u8, usize),
    pub text_size: extern "C" fn(*mut usize, *mut usize),
    pub force_unlock: extern "C" fn(),
    pub set_transparent: extern "C" fn(bool),
    pub set_cursor: extern "C" fn(usize, usize),
    pub set_scroll: extern "C" fn(bool),
}
