//! libaudiocodec: thư viện âm thanh cho app myos. Mọi hàm export có tiền tố `ac_`.
//! Dựa trên libstd (`std_*`) và thiết bị /dev/audio0 (kernel map ioctl sang AudioOps).
//! Hỗ trợ: parse WAV PCM 8/16-bit (mono/stereo), đổi về s16 stereo, resample
//! nearest-neighbor nếu driver không hỗ trợ đúng tần số, phát tone.
#![no_std]

extern "C" {
    fn std_open(path: *const u8, flags: i32) -> i64;
    fn std_close(fd: i32) -> i64;
    fn std_read(fd: i32, buf: *mut u8, n: usize) -> i64;
    fn std_write(fd: i32, buf: *const u8, n: usize) -> i64;
    fn std_lseek(fd: i32, off: i64, whence: i32) -> i64;
    fn std_ioctl(fd: i32, req: u64, arg: u64) -> i64;
    fn std_mmap_anon(len: usize) -> *mut u8;
    fn std_munmap(p: *mut u8, len: usize) -> i64;
}

// Phải trùng với abi lib.rs
const AUDIO_IOC_SET_RATE: u64 = 0x4101;
const AUDIO_IOC_GET_RATE: u64 = 0x4102;
const AUDIO_IOC_SET_VOLUME: u64 = 0x4103;
const AUDIO_IOC_DRAIN: u64 = 0x4104;
const AUDIO_IOC_STOP: u64 = 0x4105;

const EINVAL: i64 = -22;
const EIO: i64 = -5;
const ENOMEM: i64 = -12;

/// Dữ liệu PCM thô đã parse từ WAV. `data` trỏ vào buffer gốc, không copy.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AcWav {
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub data: *const u8,
    pub len: usize,
}

// ---------------- thiết bị ----------------
/// Mở /dev/audio0. Trả fd (>= 0) hoặc số âm nếu lỗi.
#[no_mangle]
pub unsafe extern "C" fn ac_open() -> i32 {
    std_open(b"/dev/audio0\0".as_ptr(), 1) as i32 // O_WRONLY
}
#[no_mangle]
pub unsafe extern "C" fn ac_close(fd: i32) { std_close(fd); }

#[no_mangle]
pub unsafe extern "C" fn ac_set_rate(fd: i32, hz: u32) -> i64 { std_ioctl(fd, AUDIO_IOC_SET_RATE, hz as u64) }
#[no_mangle]
pub unsafe extern "C" fn ac_get_rate(fd: i32) -> i64 { std_ioctl(fd, AUDIO_IOC_GET_RATE, 0) }
/// l, r: 0..=100
#[no_mangle]
pub unsafe extern "C" fn ac_set_volume(fd: i32, l: u8, r: u8) -> i64 {
    std_ioctl(fd, AUDIO_IOC_SET_VOLUME, l as u64 | ((r as u64) << 8))
}
#[no_mangle]
pub unsafe extern "C" fn ac_drain(fd: i32) -> i64 { std_ioctl(fd, AUDIO_IOC_DRAIN, 0) }
#[no_mangle]
pub unsafe extern "C" fn ac_stop(fd: i32) -> i64 { std_ioctl(fd, AUDIO_IOC_STOP, 0) }

/// Ghi PCM s16le stereo thô, ghi đủ n byte. Trả n hoặc số âm nếu lỗi.
#[no_mangle]
pub unsafe extern "C" fn ac_write(fd: i32, buf: *const u8, n: usize) -> i64 {
    let mut done = 0usize;
    while done < n {
        let r = std_write(fd, buf.add(done), n - done);
        if r < 0 { return r; }
        if r == 0 { break; }
        done += r as usize;
    }
    done as i64
}

// ---------------- WAV ----------------
unsafe fn rd16(p: *const u8) -> u16 { *p as u16 | (*p.add(1) as u16) << 8 }
unsafe fn rd32(p: *const u8) -> u32 { rd16(p) as u32 | (rd16(p.add(2)) as u32) << 16 }
unsafe fn tag(p: *const u8, t: &[u8; 4]) -> bool {
    *p == t[0] && *p.add(1) == t[1] && *p.add(2) == t[2] && *p.add(3) == t[3]
}

/// Parse file WAV PCM trong bộ nhớ. Trả false nếu không hợp lệ / không phải PCM.
#[no_mangle]
pub unsafe extern "C" fn ac_wav_parse(buf: *const u8, len: usize, out: *mut AcWav) -> bool {
    if len < 12 || !tag(buf, b"RIFF") || !tag(buf.add(8), b"WAVE") { return false; }
    let (mut rate, mut ch, mut bits, mut have_fmt) = (0u32, 0u16, 0u16, false);
    let mut off = 12usize;
    while off + 8 <= len {
        let id = buf.add(off);
        let sz = rd32(id.add(4)) as usize;
        let body = off + 8;
        if tag(id, b"fmt ") {
            if sz < 16 || body + 16 > len { return false; }
            let fmt = rd16(buf.add(body));
            if fmt != 1 && fmt != 0xFFFE { return false; } // chỉ PCM
            ch = rd16(buf.add(body + 2));
            rate = rd32(buf.add(body + 4));
            bits = rd16(buf.add(body + 14));
            have_fmt = true;
        } else if tag(id, b"data") {
            if !have_fmt { return false; }
            let n = core::cmp::min(sz, len - body);
            *out = AcWav { rate, channels: ch, bits, data: buf.add(body), len: n };
            return true;
        }
        off = body + sz + (sz & 1);
    }
    false
}

/// Phát một AcWav: đổi về s16 stereo, tự đặt tần số (không được thì resample về 48000).
#[no_mangle]
pub unsafe extern "C" fn ac_play_wav(fd: i32, w: *const AcWav) -> i64 {
    let w = &*w;
    if w.rate == 0 || w.channels == 0 || w.channels > 8 || (w.bits != 8 && w.bits != 16) {
        return EINVAL;
    }
    let mut dst = w.rate;
    if ac_set_rate(fd, dst) < 0 {
        dst = 48000;
        if ac_set_rate(fd, dst) < 0 { return EIO; }
    }
    let ch = w.channels as usize;
    let fsz = ch * (w.bits as usize / 8);
    let total = w.len / fsz;
    let step: u64 = ((w.rate as u64) << 16) / dst as u64;
    let mut pos: u64 = 0;
    let mut out = [0u8; 4096];
    loop {
        let mut o = 0usize;
        while o < out.len() {
            let idx = (pos >> 16) as usize;
            if idx >= total { break; }
            let p = w.data.add(idx * fsz);
            let (l, r): (i16, i16) = if w.bits == 16 {
                let l = rd16(p) as i16;
                (l, if ch >= 2 { rd16(p.add(2)) as i16 } else { l })
            } else {
                let l = ((*p as i16) - 128) << 8;
                (l, if ch >= 2 { ((*p.add(1) as i16) - 128) << 8 } else { l })
            };
            out[o] = l as u8; out[o + 1] = (l >> 8) as u8;
            out[o + 2] = r as u8; out[o + 3] = (r >> 8) as u8;
            o += 4;
            pos += step;
        }
        if o == 0 { break; }
        let r = ac_write(fd, out.as_ptr(), o);
        if r < 0 { return r; }
        if o < out.len() { break; }
    }
    ac_drain(fd);
    0
}

/// Đọc file WAV từ đường dẫn (NUL-terminated) rồi phát. Trả 0 nếu ok.
#[no_mangle]
pub unsafe extern "C" fn ac_play_file(fd: i32, path: *const u8) -> i64 {
    let f = std_open(path, 0); // O_RDONLY
    if f < 0 { return f; }
    let f = f as i32;
    let size = std_lseek(f, 0, 2); // SEEK_END
    if size <= 0 || std_lseek(f, 0, 0) < 0 { std_close(f); return EIO; }
    let size = size as usize;
    let cap = (size + 4095) & !4095;
    let buf = std_mmap_anon(cap);
    if buf.is_null() { std_close(f); return ENOMEM; }
    let mut got = 0usize;
    while got < size {
        let r = std_read(f, buf.add(got), size - got);
        if r <= 0 { break; }
        got += r as usize;
    }
    std_close(f);
    let mut w = AcWav { rate: 0, channels: 0, bits: 0, data: core::ptr::null(), len: 0 };
    let res = if ac_wav_parse(buf, got, &mut w) { ac_play_wav(fd, &w) } else { EINVAL };
    std_munmap(buf, cap);
    res
}

// ---------------- tone ----------------
/// Phát sóng sin xấp xỉ (parabol) `freq` Hz trong `ms` mili giây, âm lượng 0..=100.
#[no_mangle]
pub unsafe extern "C" fn ac_tone(fd: i32, freq: u32, ms: u32, vol: u32) -> i64 {
    let rate = ac_get_rate(fd);
    if rate <= 0 { return EIO; }
    let rate = rate as u64;
    let mut left = rate * ms as u64 / 1000; // số frame còn lại
    let inc = (freq as u64 * 65536 / rate) as u32;
    let amp = vol.min(100) as i32 * 327;
    let mut phase = 0u32;
    let mut out = [0u8; 4096];
    while left > 0 {
        let mut o = 0usize;
        while o < out.len() && left > 0 {
            let ph = phase & 0xFFFF;
            let u = (ph & 0x7FFF) as i32;
            let y = ((u * (32768 - u)) >> 13).min(32767);
            let s = if ph < 0x8000 { y } else { -y };
            let v = ((s * amp) >> 15) as i16;
            out[o] = v as u8; out[o + 1] = (v >> 8) as u8;
            out[o + 2] = v as u8; out[o + 3] = (v >> 8) as u8;
            o += 4;
            left -= 1;
            phase = phase.wrapping_add(inc);
        }
        let r = ac_write(fd, out.as_ptr(), o);
        if r < 0 { return r; }
    }
    ac_drain(fd);
    0
}

// libstd đã có panic_handler. Chỉ bật feature này nếu build lib này độc lập
// (make -f Makefile.audio PANIC=1).
#[cfg(feature = "panic_handler")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
