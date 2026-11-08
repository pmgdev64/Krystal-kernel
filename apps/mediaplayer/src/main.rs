//! mediaplayer: phat WAV dang stream, 2 thread:
//!   decoder: doc file + doi dinh dang (s16 stereo, resample neu can) -> ring buffer
//!   audio (thread chinh): lay tu ring -> devices://audio0, ve progress
#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, AtomicI64, Ordering};

mod stdshim;
mod audiocodec;
mod thread;
use audiocodec::{ac_close, ac_drain, ac_open_path, ac_set_rate, ac_set_volume, ac_stop, ac_wav_header, ac_write, AcWavInfo};
use thread::Spsc;

extern "C" {
    fn std_write(fd: i32, buf: *const u8, n: usize) -> i64;
    fn std_open(path: *const u8, flags: i32) -> i64;
    fn std_close(fd: i32) -> i64;
    fn std_read(fd: i32, buf: *mut u8, n: usize) -> i64;
    fn std_print_i64(v: i64);
}

const WAV_PATH: &[u8] = b"/resources/test.wav\0";
const DEV_PATH: &[u8] = b"devices://audio0\0";
const CH: usize = 16384;
const OUTN: usize = 4096;

static mut INB: [u8; CH] = [0; CH];
static mut OUTB: [u8; OUTN] = [0; OUTN]; // chi thread decoder dung
static mut MBUF: [u8; OUTN] = [0; OUTN]; // chi thread chinh dung
static mut LAST_PCT: i64 = -1;
static mut TOTAL_SECS: u64 = 0;

static RING: Spsc<{ 256 * 1024 }> = Spsc::new();
static DONE: AtomicBool = AtomicBool::new(false);
static ERR: AtomicI64 = AtomicI64::new(0);

struct Job {
    f: i32,
    ch: usize,
    bits: u16,
    fsz: usize,
    src: u64,
    dst: u64,
    start: usize,
    end: usize,
    remaining: usize,
}
static mut JOB: Job = Job { f: 0, ch: 0, bits: 0, fsz: 1, src: 1, dst: 1, start: 0, end: 0, remaining: 0 };

unsafe fn say(s: &str) { std_write(1, s.as_ptr(), s.len()); }
unsafe fn num(v: i64) { std_print_i64(v); }
unsafe fn num2(v: u64) { if v < 10 { say("0"); } num(v as i64); }
unsafe fn time(secs: u64) { num2(secs / 60); say(":"); num2(secs % 60); }

/// exit_group: ket thuc CA process (ke ca thread decoder).
unsafe fn exit_group(code: i32) -> ! {
    asm!("syscall", in("rax") 231i64, in("rdi") code as i64, options(noreturn));
}

unsafe fn fail(msg: &str, code: i64) -> ! {
    say("\n[player] LOI: ");
    say(msg);
    say(" (ma loi ");
    num(code);
    say(")\n");
    exit_group(1)
}

unsafe fn progress(done: u64, total: u64) {
    let pct = if total == 0 { 100 } else { (done * 100 / total) as i64 };
    if pct == LAST_PCT { return; }
    LAST_PCT = pct;
    say("\r[");
    let filled = (pct / 5) as usize;
    let mut i = 0;
    while i < 20 {
        say(if i < filled { "#" } else { "." });
        i += 1;
    }
    say("] ");
    num(pct);
    say("%  ");
    time(TOTAL_SECS * pct as u64 / 100);
    say(" / ");
    time(TOTAL_SECS);
    say("   ");
}

/// Day het n byte vao ring; ring day thi ngu 2ms (nhuong CPU cho swm / audio).
unsafe fn push_all(buf: *const u8, n: usize) {
    let mut d = 0usize;
    while d < n {
        let k = RING.push(core::slice::from_raw_parts(buf.add(d), n - d));
        if k == 0 {
            thread::sleep_ms(2);
        } else {
            d += k;
        }
    }
}

extern "C" fn decoder(_: usize) {
    unsafe {
        let j = &*addr_of!(JOB);
        let inb = addr_of_mut!(INB) as *mut u8;
        let outb = addr_of_mut!(OUTB) as *mut u8;
        let (f, ch, bits, fsz, src, dst) = (j.f, j.ch, j.bits, j.fsz, j.src, j.dst);
        let (mut start, mut end, mut remaining) = (j.start, j.end, j.remaining);
        let mut acc: u64 = 0;
        let mut o = 0usize;

        loop {
            let mut avail = end - start;
            if avail > remaining { avail = remaining; }
            let nframes = avail / fsz;
            let mut i = 0usize;
            while i < nframes {
                let p = inb.add(start + i * fsz);
                let (l, r): (i16, i16) = if bits == 16 {
                    let l = (*p as u16 | (*p.add(1) as u16) << 8) as i16;
                    let r = if ch >= 2 { (*p.add(2) as u16 | (*p.add(3) as u16) << 8) as i16 } else { l };
                    (l, r)
                } else {
                    let l = ((*p as i16) - 128) << 8;
                    let r = if ch >= 2 { ((*p.add(1) as i16) - 128) << 8 } else { l };
                    (l, r)
                };
                acc += dst;
                while acc >= src {
                    acc -= src;
                    *outb.add(o) = l as u8;
                    *outb.add(o + 1) = (l >> 8) as u8;
                    *outb.add(o + 2) = r as u8;
                    *outb.add(o + 3) = (r >> 8) as u8;
                    o += 4;
                    if o == OUTN {
                        push_all(outb, o);
                        o = 0;
                    }
                }
                i += 1;
            }
            let used = nframes * fsz;
            start += used;
            remaining -= used;
            if remaining < fsz { break; }
            // don phan du (< 1 frame) len dau roi doc tiep
            let left = end - start;
            let mut k = 0usize;
            while k < left { *inb.add(k) = *inb.add(start + k); k += 1; }
            start = 0;
            end = left;
            let r = std_read(f, inb.add(end), CH - end);
            if r < 0 { ERR.store(r, Ordering::Release); break; }
            if r == 0 { break; }
            end += r as usize;
        }
        if o > 0 { push_all(outb, o); }
        DONE.store(true, Ordering::Release);
    }
}

#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    say("=== Media Player ===\n");
    let inb = addr_of_mut!(INB) as *mut u8;
    let mbuf = addr_of_mut!(MBUF) as *mut u8;

    let fd = ac_open_path(DEV_PATH.as_ptr());
    if fd < 0 { fail("khong mo duoc devices://audio0", fd as i64); }
    ac_set_volume(fd, 80, 80);

    say("File : rd0://resources/test.wav\n");
    let f = std_open(WAV_PATH.as_ptr(), 0);
    if f < 0 { fail("khong mo duoc file wav", f); }
    let f = f as i32;

    // khoi dau: chua header
    let mut end = 0usize;
    while end < CH {
        let r = std_read(f, inb.add(end), CH - end);
        if r < 0 { fail("loi doc file", r); }
        if r == 0 { break; }
        end += r as usize;
        if end >= 1024 { break; }
    }
    let mut info = AcWavInfo { rate: 0, channels: 0, bits: 0, data_off: 0, data_len: 0 };
    if !ac_wav_header(inb, end, &mut info) { fail("khong phai WAV PCM hop le", -22); }
    if info.channels == 0 || info.channels > 8 || (info.bits != 8 && info.bits != 16) || info.rate == 0 {
        fail("dinh dang WAV khong ho tro", -22);
    }

    let ch = info.channels as usize;
    let fsz = ch * (info.bits as usize / 8);
    let src = info.rate as u64;
    let bps = src * fsz as u64;
    TOTAL_SECS = info.data_len as u64 / bps;
    let total_frames = (info.data_len / fsz) as u64;

    say("Dinh dang: ");
    num(info.rate as i64);
    say(" Hz, ");
    if ch == 1 { say("mono"); } else if ch == 2 { say("stereo"); } else { num(ch as i64); say(" kenh"); }
    say(", ");
    num(info.bits as i64);
    say(" bit\nThoi luong: ");
    time(TOTAL_SECS);
    say("\n");

    let mut dst = src;
    if ac_set_rate(fd, info.rate) < 0 {
        dst = 48000;
        if ac_set_rate(fd, 48000) < 0 { fail("khong dat duoc tan so", -5); }
    }

    // giao viec cho thread decoder
    *addr_of_mut!(JOB) = Job {
        f,
        ch,
        bits: info.bits,
        fsz,
        src,
        dst,
        start: info.data_off,
        end,
        remaining: info.data_len,
    };
    let t = match thread::spawn(decoder, 0) {
        Some(t) => t,
        None => fail("khong tao duoc thread decoder", -11),
    };

    // thread chinh: ring -> audio0. Luon lay boi so cua 4 byte (1 frame s16 stereo).
    let mut played: u64 = 0; // byte da ghi ra audio
    loop {
        let avail = RING.len() & !3;
        let want = if avail > OUTN { OUTN } else { avail };
        if want == 0 {
            let e = ERR.load(Ordering::Acquire);
            if e != 0 { fail("loi doc file", e); }
            if DONE.load(Ordering::Acquire) && RING.len() < 4 { break; }
            thread::sleep_ms(1);
            continue;
        }
        let n = RING.pop(core::slice::from_raw_parts_mut(mbuf, want));
        let w = ac_write(fd, mbuf, n);
        if w < 0 { fail("ghi am thanh that bai", w); }
        played += n as u64;
        let fr = (played / 4) * src / dst;
        progress(if fr > total_frames { total_frames } else { fr }, total_frames);
    }

    ac_drain(fd);
    progress(total_frames, total_frames);
    say("\n");
    ac_stop(fd);
    t.join();
    std_close(f);
    ac_close(fd);
    say("Xong.\n");
    exit_group(0)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { asm!("syscall", in("rax") 231i64, in("rdi") 101i64, options(noreturn)) }
}

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
