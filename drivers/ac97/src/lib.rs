//! Driver AC'97 (controller kiểu Intel ICH, PCI class 0x0401) cho myos.
//! KIND_AUDIO: phát PCM s16le stereo, polling, không dùng ngắt.
//!
//! Giả định: kernel identity-map (địa chỉ ảo == địa chỉ vật lý) và driver nằm dưới 4 GiB,
//! vì BDL chỉ chứa địa chỉ 32-bit. Nếu không đúng thì sửa `phys()`.
//! Test nhanh bằng QEMU: `-device AC97` (8086:2415).
#![no_std]

use core::arch::asm;
use core::ffi::c_void;
use core::ptr::{addr_of_mut, null};

use driver_abi::*; // Makefile.audio build abi thành crate tên driver_abi

// ---------------- hằng số ----------------
const NBUF: usize = 32;
const BUF_SZ: usize = 4096;

// Thanh ghi codec (NAM, truy cập 16-bit)
const NAM_RESET: u16 = 0x00;
const NAM_MASTER: u16 = 0x02;
const NAM_PCM_OUT: u16 = 0x18;
const NAM_POWER: u16 = 0x26;
const NAM_EXT_ID: u16 = 0x28;
const NAM_EXT_CTL: u16 = 0x2A;
const NAM_PCM_RATE: u16 = 0x2C;

// Thanh ghi bus master (NABM)
const PO: u16 = 0x10; // kênh PCM out: +0 BDBAR, +4 CIV, +5 LVI, +6 SR, +8 PICB, +0xB CR
const GLOB_CNT: u16 = 0x2C;
const GLOB_STA: u16 = 0x30;
const CR_RPBM: u8 = 1;
const CR_RR: u8 = 2;
const SR_DCH: u16 = 1;

// ---------------- port I/O ----------------
// Không dùng `nomem`: ghi buffer DMA phải xong trước khi ghi LVI.
#[inline(always)]
unsafe fn outb(p: u16, v: u8) { asm!("out dx, al", in("dx") p, in("al") v, options(nostack, preserves_flags)); }
#[inline(always)]
unsafe fn outw(p: u16, v: u16) { asm!("out dx, ax", in("dx") p, in("ax") v, options(nostack, preserves_flags)); }
#[inline(always)]
unsafe fn outl(p: u16, v: u32) { asm!("out dx, eax", in("dx") p, in("eax") v, options(nostack, preserves_flags)); }
#[inline(always)]
unsafe fn inb(p: u16) -> u8 { let v: u8; asm!("in al, dx", out("al") v, in("dx") p, options(nostack, preserves_flags)); v }
#[inline(always)]
unsafe fn inw(p: u16) -> u16 { let v: u16; asm!("in ax, dx", out("ax") v, in("dx") p, options(nostack, preserves_flags)); v }
#[inline(always)]
unsafe fn inl(p: u16) -> u32 { let v: u32; asm!("in eax, dx", out("eax") v, in("dx") p, options(nostack, preserves_flags)); v }

// ---------------- PCI ----------------
unsafe fn pci_addr(bus: u32, dev: u32, func: u32, off: u32) -> u32 {
    0x8000_0000 | (bus << 16) | (dev << 11) | (func << 8) | (off & 0xFC)
}
unsafe fn pci_read(bus: u32, dev: u32, func: u32, off: u32) -> u32 {
    outl(0xCF8, pci_addr(bus, dev, func, off));
    inl(0xCFC)
}
unsafe fn pci_write(bus: u32, dev: u32, func: u32, off: u32, v: u32) {
    outl(0xCF8, pci_addr(bus, dev, func, off));
    outl(0xCFC, v);
}
/// Tìm thiết bị multimedia audio (class 0x0401) trên bus 0..8.
unsafe fn pci_find_ac97() -> Option<(u32, u32, u32)> {
    for bus in 0..8u32 {
        for dev in 0..32u32 {
            for func in 0..8u32 {
                let id = pci_read(bus, dev, func, 0);
                if id & 0xFFFF == 0xFFFF {
                    if func == 0 { break; }
                    continue;
                }
                if pci_read(bus, dev, func, 8) >> 16 == 0x0401 {
                    return Some((bus, dev, func));
                }
            }
        }
    }
    None
}

// ---------------- DMA ----------------
#[repr(C)]
#[derive(Clone, Copy)]
struct Bd {
    addr: u32,
    /// bit 0..15 = số sample 16-bit (không phải frame), bit 31 = IOC, bit 30 = BUP
    ctl: u32,
}
#[repr(C, align(4096))]
struct Bufs([u8; NBUF * BUF_SZ]);
#[repr(C, align(16))]
struct Bdl([Bd; NBUF]);

static mut BUFS: Bufs = Bufs([0; NBUF * BUF_SZ]);
static mut BDL: Bdl = Bdl([Bd { addr: 0, ctl: 0 }; NBUF]);

fn phys<T>(p: *const T) -> u32 { p as usize as u32 }

// ---------------- trạng thái ----------------
struct State {
    ready: bool,
    nam: u16,
    nabm: u16,
    vra: bool,
    rate: u32,
    running: bool,
    head: usize, // buffer kế tiếp sẽ được điền
}
static mut ST: State = State { ready: false, nam: 0, nabm: 0, vra: false, rate: 48000, running: false, head: 0 };
static mut API: *const KernelApi = null();

fn st() -> &'static mut State { unsafe { &mut *addr_of_mut!(ST) } }

unsafe fn log(s: &str) {
    if !API.is_null() { ((*API).log)(s.as_ptr(), s.len()); }
}

unsafe fn wait(mut cond: impl FnMut() -> bool) -> bool {
    let mut n = 0u32;
    while n < 2_000_000 {
        if cond() { return true; }
        core::hint::spin_loop();
        n += 1;
    }
    false
}

// ---------------- điều khiển kênh PCM out ----------------
unsafe fn chan_reset(s: &mut State) {
    let po = s.nabm + PO;
    outb(po + 0xB, 0);
    outb(po + 0xB, CR_RR);
    wait(|| inb(po + 0xB) & CR_RR == 0);
    outl(po, phys(addr_of_mut!(BDL) as *const Bdl));
    s.running = false;
    s.head = 0;
}

unsafe fn chan_start(s: &mut State) {
    let po = s.nabm + PO;
    outb(po + 0xB, CR_RPBM);
    wait(|| inw(po + 6) & SR_DCH == 0);
    s.running = true;
}

// ---------------- ops ----------------
extern "C" fn set_rate(r: u32) -> bool {
    unsafe {
        let s = st();
        if !s.ready || r < 8000 || r > 48000 { return false; }
        if !s.vra { return r == 48000; }
        stop();
        outw(s.nam + NAM_PCM_RATE, r as u16);
        let got = inw(s.nam + NAM_PCM_RATE) as u32;
        s.rate = got;
        got == r
    }
}

extern "C" fn get_rate() -> u32 { st().rate }

fn att(v: u8) -> u16 { (100 - v.min(100) as u16) * 31 / 100 }

extern "C" fn set_volume(l: u8, r: u8) {
    unsafe {
        let s = st();
        if !s.ready { return; }
        let mut val = (att(l) << 8) | att(r);
        if l == 0 && r == 0 { val |= 0x8000; } // mute
        outw(s.nam + NAM_MASTER, val);
    }
}

extern "C" fn write(buf: *const u8, len: usize) -> usize {
    unsafe {
        let s = st();
        if !s.ready { return 0; }
        let po = s.nabm + PO;
        let len = len & !3; // chỉ nhận số nguyên frame (4 byte)
        let mut done = 0usize;
        while done < len {
            // Phát hết / underrun: reset kênh, bắt đầu lại từ buffer 0.
            if s.running && inw(po + 6) & SR_DCH != 0 { chan_reset(s); }
            let civ = if s.running { inb(po + 4) as usize } else { 0 };
            let used = (s.head + NBUF - civ) % NBUF;
            if used >= NBUF - 1 {
                if !s.running { chan_start(s); }
                core::hint::spin_loop();
                continue;
            }
            let n = core::cmp::min(len - done, BUF_SZ);
            let h = s.head;
            let dst = (addr_of_mut!(BUFS) as *mut u8).add(h * BUF_SZ);
            core::ptr::copy_nonoverlapping(buf.add(done), dst, n);
            (*addr_of_mut!(BDL)).0[h] = Bd { addr: phys(dst as *const u8), ctl: (n / 2) as u32 };
            outb(po + 5, h as u8); // LVI = buffer vừa điền
            s.head = (h + 1) % NBUF;
            done += n;
            if !s.running && used >= 3 { chan_start(s); }
        }
        if !s.running && s.head != 0 { chan_start(s); }
        done
    }
}

extern "C" fn drain() {
    unsafe {
        let s = st();
        if !s.ready || !s.running { return; }
        let po = s.nabm + PO;
        let mut n = 0u64;
        while inw(po + 6) & SR_DCH == 0 && n < 2_000_000_000 {
            core::hint::spin_loop();
            n += 1;
        }
        chan_reset(s);
    }
}

extern "C" fn stop() {
    unsafe {
        let s = st();
        if s.ready { chan_reset(s); }
    }
}

static OPS: AudioOps = AudioOps { set_rate, get_rate, set_volume, write, drain, stop };

// ---------------- khởi tạo phần cứng ----------------
unsafe fn hw_init() -> bool {
    let (bus, dev, func) = match pci_find_ac97() { Some(x) => x, None => return false };
    let bar0 = pci_read(bus, dev, func, 0x10);
    let bar1 = pci_read(bus, dev, func, 0x14);
    if bar0 & 1 == 0 || bar1 & 1 == 0 { return false; } // chỉ hỗ trợ I/O BAR
    let s = st();
    s.nam = (bar0 & 0xFFFC) as u16;
    s.nabm = (bar1 & 0xFFFC) as u16;

    // I/O space + bus master
    let cmd = pci_read(bus, dev, func, 0x04);
    pci_write(bus, dev, func, 0x04, cmd | 0x5);

    // Thoát cold reset, chờ codec chính sẵn sàng
    outl(s.nabm + GLOB_CNT, 0x02);
    if !wait(|| inl(s.nabm + GLOB_STA) & 0x100 != 0) { return false; }

    // Reset mixer, chờ ADC/DAC/analog/ref sẵn sàng
    outw(s.nam + NAM_RESET, 0);
    if !wait(|| inw(s.nam + NAM_POWER) & 0xF == 0xF) { return false; }

    // Variable Rate Audio (nếu codec hỗ trợ)
    if inw(s.nam + NAM_EXT_ID) & 1 != 0 {
        outw(s.nam + NAM_EXT_CTL, inw(s.nam + NAM_EXT_CTL) | 1);
        s.vra = true;
    }
    s.rate = if s.vra { inw(s.nam + NAM_PCM_RATE) as u32 } else { 48000 };

    outw(s.nam + NAM_PCM_OUT, 0x0808);
    s.ready = true;
    set_volume(80, 80);
    chan_reset(s);
    true
}

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    unsafe {
        if (*api).abi_version != DRIVER_ABI { return -1; }
        API = api;
        if !hw_init() {
            log("ac97: khong tim thay / khoi tao that bai");
            return -2;
        }
        log("ac97: san sang");
        let info = DriverInfo {
            name: name("ac97"),
            kind: KIND_AUDIO,
            ops: &OPS as *const AudioOps as *const c_void,
        };
        ((*api).register)(&info)
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
