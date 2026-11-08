//! testplayer: app thử audio. Phát tone 440Hz để kiểm tra phần cứng,
//! rồi đọc rd0://resources/test.wav và phát.
//! Link với libstd (std_*) và libaudiocodec (ac_*).
#![no_std]

use core::arch::asm;

#[repr(C)]
#[derive(Clone, Copy)]
struct AcWav {
    rate: u32,
    channels: u16,
    bits: u16,
    data: *const u8,
    len: usize,
}

extern "C" {
    // libstd
    fn std_write(fd: i32, buf: *const u8, n: usize) -> i64;
    fn std_exit(code: i32) -> !;
    fn std_open(path: *const u8, flags: i32) -> i64;
    fn std_close(fd: i32) -> i64;
    fn std_read(fd: i32, buf: *mut u8, n: usize) -> i64;
    fn std_lseek(fd: i32, off: i64, whence: i32) -> i64;
    fn std_mmap_anon(len: usize) -> *mut u8;
    fn std_munmap(p: *mut u8, len: usize) -> i64;
    fn std_print_i64(v: i64);
    // libaudiocodec
    fn ac_open() -> i32;
    fn ac_close(fd: i32);
    fn ac_set_volume(fd: i32, l: u8, r: u8) -> i64;
    fn ac_tone(fd: i32, freq: u32, ms: u32, vol: u32) -> i64;
    fn ac_wav_parse(buf: *const u8, len: usize, out: *mut AcWav) -> bool;
    fn ac_play_wav(fd: i32, w: *const AcWav) -> i64;
}

const WAV_PATH: &[u8] = b"rd0://resources/test.wav\0";

unsafe fn say(s: &str) { std_write(1, s.as_ptr(), s.len()); }
unsafe fn num(v: i64) { std_print_i64(v); }

unsafe fn fail(msg: &str, code: i64) -> ! {
    say("[player] LOI: ");
    say(msg);
    say(" (ma loi ");
    num(code);
    say(")\n");
    std_exit(1)
}

#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    say("[player] mo /dev/audio0\n");
    let fd = ac_open();
    if fd < 0 { fail("khong mo duoc /dev/audio0", fd as i64); }
    ac_set_volume(fd, 80, 80);

    say("[player] tone 440Hz 300ms (kiem tra phan cung)\n");
    let r = ac_tone(fd, 440, 300, 40);
    if r < 0 { fail("phat tone that bai", r); }

    say("[player] doc rd0://resources/test.wav\n");
    let f = std_open(WAV_PATH.as_ptr(), 0);
    if f < 0 { fail("khong mo duoc file wav", f); }
    let f = f as i32;

    let size = std_lseek(f, 0, 2);
    if size <= 0 { fail("kich thuoc file khong hop le", size); }
    std_lseek(f, 0, 0);
    say("[player] kich thuoc file: ");
    num(size);
    say(" byte\n");

    let size = size as usize;
    let cap = (size + 4095) & !4095;
    let buf = std_mmap_anon(cap);
    if buf.is_null() { fail("khong du bo nho", -12); }

    let mut got = 0usize;
    while got < size {
        let r = std_read(f, buf.add(got), size - got);
        if r <= 0 { break; }
        got += r as usize;
    }
    std_close(f);
    if got < size {
        say("[player] CANH BAO: chi doc duoc ");
        num(got as i64);
        say(" byte\n");
    }

    let mut w = AcWav { rate: 0, channels: 0, bits: 0, data: core::ptr::null(), len: 0 };
    if !ac_wav_parse(buf, got, &mut w) {
        fail("file khong phai WAV PCM hop le", -22);
    }
    say("[player] WAV: ");
    num(w.rate as i64);
    say(" Hz, ");
    num(w.channels as i64);
    say(" kenh, ");
    num(w.bits as i64);
    say(" bit, ");
    num(w.len as i64);
    say(" byte du lieu\n");

    say("[player] dang phat...\n");
    let r = ac_play_wav(fd, &w);
    if r < 0 { fail("phat wav that bai", r); }

    std_munmap(buf, cap);
    ac_close(fd);
    say("[player] xong\n");
    std_exit(0)
}

// rustc/LLVM có thể sinh lời gọi memcpy/memset. Nếu link báo "duplicate symbol",
// xoá hai hàm này (libc/kernel của bạn đã cung cấp).
#[no_mangle]
pub unsafe extern "C" fn memcpy(d: *mut u8, s: *const u8, n: usize) -> *mut u8 {
    asm!("rep movsb", inout("rcx") n => _, inout("rdi") d => _, inout("rsi") s => _, options(nostack, preserves_flags));
    d
}
#[no_mangle]
pub unsafe extern "C" fn memset(d: *mut u8, v: i32, n: usize) -> *mut u8 {
    asm!("rep stosb", inout("rcx") n => _, inout("rdi") d => _, in("al") v as u8, options(nostack, preserves_flags));
    d
}
