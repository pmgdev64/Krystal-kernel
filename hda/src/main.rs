//! Driver Intel HD Audio (ICH6+, PCI class 0x0403) cho myos.
//! KIND_AUDIO: PCM s16le stereo, polling, khong ngat. Noi chuyen codec bang CORB/RIRB.
//! Ring 32 x 4 KiB; DMA HDA tu quay lai BDL[0] sau LVI nen driver tu theo doi LPIB.
//! Khoi tao phan cung LUON o lan dung dau (driver_entry chay truoc pmm::init).
#![no_std]
#![no_main]

use core::arch::asm;
use core::ffi::c_void;
use core::ptr::{addr_of_mut, copy_nonoverlapping, null, null_mut, read_volatile, write_bytes, write_volatile};

use driver_abi::*;

const NBUF: usize = 52; // 52 x 4096 B ~ 1.11 s o 48kHz (tiếng rè BSOD lặp quanh độ dài này)
const BUF_SZ: usize = 4096;
const RING: usize = NBUF * BUF_SZ;
const TAG: u32 = 1; // stream tag
const START_BYTES: usize = 8 * BUF_SZ;
const MARGIN: usize = BUF_SZ;

// ===== PANIC HOLD (chinh o day) =====
/// Do dai doan am thanh lap lai khi kernel panic (ms). Duoc lam tron ve uoc cua RING gan nhat.
const HOLD_MS: u32 = 208;
/// Khoang dem phia truoc vi tri phat (ms) de kip ghi de ring. Tang len neu nghe vap luc vao panic.
const HOLD_AHEAD_MS: u32 = 85;
/// Kich thuoc buffer toi da cho doan lap (RING/2 ~ 0.55 s o 48 kHz).
const HOLD_MAX: usize = RING / 2;

// ---------------- port I/O (chi cho PCI config) ----------------
#[inline(always)]
unsafe fn outb(p: u16, v: u8) { asm!("out dx, al", in("dx") p, in("al") v, options(nostack, preserves_flags)); }
#[inline(always)]
unsafe fn outl(p: u16, v: u32) { asm!("out dx, eax", in("dx") p, in("eax") v, options(nostack, preserves_flags)); }
#[inline(always)]
unsafe fn inl(p: u16) -> u32 { let v: u32; asm!("in eax, dx", out("eax") v, in("dx") p, options(nostack, preserves_flags)); v }

unsafe fn pci_addr(bus: u32, dev: u32, func: u32, off: u32) -> u32 {
    0x8000_0000 | (bus << 16) | (dev << 11) | (func << 8) | (off & 0xFC)
}
unsafe fn pci_read(bus: u32, dev: u32, func: u32, off: u32) -> u32 {
    outl(0xCF8, pci_addr(bus, dev, func, off)); inl(0xCFC)
}
unsafe fn pci_write(bus: u32, dev: u32, func: u32, off: u32, v: u32) {
    outl(0xCF8, pci_addr(bus, dev, func, off)); outl(0xCFC, v);
}
unsafe fn pci_find_hda() -> Option<(u32, u32, u32)> {
    for bus in 0..8u32 { for dev in 0..32u32 { for func in 0..8u32 {
        let id = pci_read(bus, dev, func, 0);
        if id & 0xFFFF == 0xFFFF { if func == 0 { break; } continue; }
        if pci_read(bus, dev, func, 8) >> 16 == 0x0403 { return Some((bus, dev, func)); }
    }}}
    None
}

// ---------------- MMIO ----------------
static mut MMIO: *mut u8 = null_mut();
#[inline(always)] unsafe fn r8(o: usize) -> u8 { read_volatile(MMIO.add(o)) }
#[inline(always)] unsafe fn r16(o: usize) -> u16 { read_volatile(MMIO.add(o) as *const u16) }
#[inline(always)] unsafe fn r32(o: usize) -> u32 { read_volatile(MMIO.add(o) as *const u32) }
#[inline(always)] unsafe fn w8(o: usize, v: u8) { write_volatile(MMIO.add(o), v) }
#[inline(always)] unsafe fn w16(o: usize, v: u16) { write_volatile(MMIO.add(o) as *mut u16, v) }
#[inline(always)] unsafe fn w32(o: usize, v: u32) { write_volatile(MMIO.add(o) as *mut u32, v) }

// ---------------- bo nho DMA ----------------
#[repr(C)] #[derive(Clone, Copy)]
struct Bd { addr: u64, len: u32, flags: u32 }
#[repr(C, align(4096))] struct Ring([u8; RING]);
#[repr(C, align(128))] struct Bdl([Bd; NBUF]);
#[repr(C, align(128))] struct Corb([u32; 256]);
#[repr(C, align(128))] struct Rirb([u64; 256]);

static mut RING_BUF: Ring = Ring([0; RING]);
static mut BDL: Bdl = Bdl([Bd { addr: 0, len: 0, flags: 0 }; NBUF]);
static mut CORB: Corb = Corb([0; 256]);
static mut RIRB: Rirb = Rirb([0; 256]);

fn ring() -> *mut u8 { addr_of_mut!(RING_BUF) as *mut u8 }

static mut API: *const KernelApi = null();

unsafe fn phys<T>(p: *const T) -> u64 { ((*API).virt_to_phys)(p as *const u8) }

// ---------------- log ----------------
unsafe fn log(s: &str) {
    if !API.is_null() {
        ((*API).log)(s.as_ptr(), s.len());
        ((*API).log)(b"\n".as_ptr(), 1);
    }
}
unsafe fn log_hex(tag: &str, v: u64) {
    let mut b = [0u8; 80];
    let mut n = 0usize;
    for &c in tag.as_bytes() { if n < 48 { b[n] = c; n += 1; } }
    b[n] = b'0'; b[n + 1] = b'x'; n += 2;
    let mut i = 16;
    while i > 0 {
        i -= 1;
        let d = ((v >> (i * 4)) & 15) as u8;
        b[n] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        n += 1;
    }
    b[n] = b'\n'; n += 1;
    if !API.is_null() { ((*API).log)(b.as_ptr(), n); }
}

unsafe fn wait(n: u32, mut cond: impl FnMut() -> bool) -> bool {
    let mut i = 0u32;
    while i < n { if cond() { return true; } core::hint::spin_loop(); i += 1; }
    false
}
unsafe fn delay_us(n: u32) { for _ in 0..n { outb(0x80, 0); } }

// ---------------- state ----------------
struct State {
    detected: bool, ready: bool, failed: bool,
    bus: u32, dev: u32, func: u32,
    cad: u32, afg: u32, dac: u32, pin: u32,
    has_amp: bool, amp_steps: u32, rates: u32,
    sd: usize,
    n_corb: usize, n_rirb: usize, corb_wp: usize, rirb_rp: usize,
    rate: u32, fmt: u16,
    vol_l: u8, vol_r: u8,
    running: bool, wpos: usize, written: u64, consumed: u64, last: usize,
}
static mut ST: State = State {
    detected: false, ready: false, failed: false,
    bus: 0, dev: 0, func: 0,
    cad: 0, afg: 0, dac: 0, pin: 0,
    has_amp: false, amp_steps: 0, rates: 0,
    sd: 0,
    n_corb: 256, n_rirb: 256, corb_wp: 0, rirb_rp: 0,
    rate: 48000, fmt: 0x0011,
    vol_l: 80, vol_r: 80,
    running: false, wpos: 0, written: 0, consumed: 0, last: 0,
};
fn st() -> &'static mut State { unsafe { &mut *addr_of_mut!(ST) } }

// ---------------- codec verbs ----------------
fn v12(cad: u32, nid: u32, verb: u32, p: u32) -> u32 { (cad << 28) | (nid << 20) | (verb << 8) | (p & 0xFF) }
fn v4(cad: u32, nid: u32, verb: u32, p: u32) -> u32 { (cad << 28) | (nid << 20) | (verb << 16) | (p & 0xFFFF) }

static mut IMM: bool = false;

/// Immediate Command Interface: gui verb thang vao codec, khong qua CORB/RIRB.
unsafe fn cmd_imm(v: u32) -> Option<u32> {
    if !wait(100_000, || r16(0x68) & 1 == 0) { return None; }
    w32(0x60, v);
    w16(0x68, 0x0002); // xoa IRV (w1c)
    w16(0x68, 0x0001); // ICB = 1: gui
    if !wait(2_000_000, || r16(0x68) & 2 != 0) {
        log_hex("hda !! immediate timeout verb =", v as u64);
        return None;
    }
    Some(r32(0x64))
}

unsafe fn cmd(s: &mut State, v: u32) -> Option<u32> {
    if IMM { return cmd_imm(v); }
    let wp = (s.corb_wp + 1) % s.n_corb;
    write_volatile((addr_of_mut!(CORB) as *mut u32).add(wp), v);
    s.corb_wp = wp;
    w16(0x48, wp as u16);
    let last = s.rirb_rp;
    if !wait(2_000_000, || (r16(0x58) & 0xFF) as usize != last) {
        log("hda !! CORB/RIRB timeout -> chuyen sang Immediate Command");
        IMM = true;
        return cmd_imm(v);
    }
    let rp = (r16(0x58) & 0xFF) as usize;
    s.rirb_rp = rp;
    w8(0x5D, 0x05);
    let r = read_volatile((addr_of_mut!(RIRB) as *const u64).add(rp));
    Some(r as u32)
}
unsafe fn get(s: &mut State, nid: u32, verb: u32, p: u32) -> u32 { let c = s.cad; cmd(s, v12(c, nid, verb, p)).unwrap_or(0) }
unsafe fn param(s: &mut State, nid: u32, id: u32) -> u32 { get(s, nid, 0xF00, id) }
unsafe fn set12(s: &mut State, nid: u32, verb: u32, p: u32) { let c = s.cad; let _ = cmd(s, v12(c, nid, verb, p)); }
unsafe fn set4(s: &mut State, nid: u32, verb: u32, p: u32) { let c = s.cad; let _ = cmd(s, v4(c, nid, verb, p)); }

unsafe fn conn_list(s: &mut State, nid: u32, out: &mut [u8; 16]) -> usize {
    let len = param(s, nid, 0x0E);
    let n = ((len & 0x7F) as usize).min(16);
    let long = len & 0x80 != 0;
    let mut i = 0usize;
    while i < n {
        let r = get(s, nid, 0xF02, i as u32);
        if long {
            for k in 0..2usize { if i + k < n { out[i + k] = ((r >> (16 * k)) & 0x7F) as u8; } }
            i += 2;
        } else {
            for k in 0..4usize { if i + k < n { out[i + k] = ((r >> (8 * k)) & 0x7F) as u8; } }
            i += 4;
        }
    }
    n
}

/// Bat tieng (mute off, 0 dB) cho 1 widget tren duong ra.
unsafe fn unmute(s: &mut State, nid: u32, caps: u32, nconn: usize) {
    let src = if caps & 8 != 0 { nid } else { s.afg };
    if caps & 4 != 0 {
        let ac = param(s, src, 0x12);
        set4(s, nid, 0x3, 0xB000 | (ac & 0x7F));
    }
    if caps & 2 != 0 {
        let ac = param(s, src, 0x0D);
        let g = ac & 0x7F;
        let n = if nconn == 0 { 1 } else { nconn };
        for i in 0..n { set4(s, nid, 0x3, 0x7000 | ((i as u32) << 8) | g); }
    }
}

/// Tu `nid` (pin/mixer/selector) di nguoc ve DAC. Cau hinh cac widget tren duong tim duoc.
unsafe fn trace(s: &mut State, nid: u32, depth: u32) -> Option<u32> {
    let caps = param(s, nid, 0x09);
    let typ = (caps >> 20) & 0xF;
    if typ == 0 {
        set12(s, nid, 0x705, 0);
        unmute(s, nid, caps, 0);
        set12(s, nid, 0x706, TAG << 4);
        s.has_amp = caps & 4 != 0;
        if s.has_amp {
            let src = if caps & 8 != 0 { nid } else { s.afg };
            s.amp_steps = (param(s, src, 0x12) >> 8) & 0x7F;
        }
        return Some(nid);
    }
    if depth == 0 || !(typ == 2 || typ == 3 || typ == 4) { return None; }
    let mut list = [0u8; 16];
    let n = conn_list(s, nid, &mut list);
    for i in 0..n {
        if let Some(d) = trace(s, list[i] as u32, depth - 1) {
            set12(s, nid, 0x705, 0);
            if (typ == 3 || typ == 4) && n > 1 { set12(s, nid, 0x701, i as u32); }
            unmute(s, nid, caps, n);
            return Some(d);
        }
    }
    None
}

unsafe fn find_output(s: &mut State, start: u32, count: u32) -> bool {
    for want in [0u32, 1, 2] { // line out, speaker, headphone
        for nid in start..start + count {
            let caps = param(s, nid, 0x09);
            if (caps >> 20) & 0xF != 4 { continue; }
            let pcap = param(s, nid, 0x0C);
            if pcap & (1 << 4) == 0 { continue; }
            let cfg = get(s, nid, 0xF1C, 0);
            if cfg >> 30 == 1 || (cfg >> 20) & 0xF != want { continue; }
            if let Some(dac) = trace(s, nid, 4) {
                set12(s, nid, 0x707, 0x40 | if want == 2 { 0x80 } else { 0 });
                if pcap & (1 << 16) != 0 { set12(s, nid, 0x70C, 2); }
                s.pin = nid;
                s.dac = dac;
                return true;
            }
        }
    }
    false
}

// ---------------- volume / rate ----------------
fn gain_word(v: u8, steps: u32, lr: u32) -> u32 {
    let v = v.min(100) as u32;
    0x8000 | lr | (if v == 0 { 0x80 } else { 0 }) | (steps * v / 100)
}
unsafe fn apply_vol(s: &mut State) {
    if !s.ready || !s.has_amp { return; }
    let (d, steps, l, r) = (s.dac, s.amp_steps, s.vol_l, s.vol_r);
    set4(s, d, 0x3, gain_word(l, steps, 0x2000));
    set4(s, d, 0x3, gain_word(r, steps, 0x1000));
}

/// (gia tri SDnFMT, bit trong param 0x0A) cho tan so; 16-bit stereo.
fn fmt_of(r: u32) -> Option<(u16, u32)> {
    Some(match r {
        8000 => (0x0511, 0),
        11025 => (0x4311, 1),
        16000 => (0x0211, 2),
        22050 => (0x4111, 3),
        32000 => (0x0A11, 4),
        44100 => (0x4011, 5),
        48000 => (0x0011, 6),
        88200 => (0x4811, 7),
        96000 => (0x0811, 8),
        _ => return None,
    })
}

// ---------------- stream ----------------
unsafe fn chan_reset(s: &mut State) {
    let sd = s.sd;
    w8(sd, r8(sd) & !2);
    wait(100_000, || r8(sd) & 2 == 0);
    w8(sd, 1);
    wait(100_000, || r8(sd) & 1 != 0);
    w8(sd, 0);
    wait(100_000, || r8(sd) & 1 == 0);
    w8(sd + 2, (TAG << 4) as u8);
    w8(sd + 3, 0x1C);
    w32(sd + 8, RING as u32);
    w16(sd + 0x0C, (NBUF - 1) as u16);
    w16(sd + 0x12, s.fmt);
    let b = phys(addr_of_mut!(BDL) as *const Bdl);
    w32(sd + 0x18, b as u32);
    w32(sd + 0x1C, (b >> 32) as u32);
    write_bytes(ring(), 0, RING);
    s.running = false;
    s.wpos = 0;
    s.written = 0;
    s.consumed = 0;
    s.last = 0;
}

unsafe fn chan_start(s: &mut State) {
    let sd = s.sd;
    w8(sd + 3, 0x1C);
    s.consumed = 0;
    s.last = 0;
    w8(sd, r8(sd) | 2);
    s.running = true;
}

/// Cap nhat so byte da phat tu LPIB.
unsafe fn poll(s: &mut State) {
    if !s.running { return; }
    let lp = r32(s.sd + 4) as usize % RING;
    s.consumed += ((lp + RING - s.last) % RING) as u64;
    s.last = lp;
}

// ---------------- khoi tao phan cung ----------------
unsafe fn hw_init(s: &mut State) -> bool {
    log("hda: khoi tao controller");
    let (bus, dev, func) = (s.bus, s.dev, s.func);
    let id = pci_read(bus, dev, func, 0);
    log_hex("hda pci dev+ven =", id as u64);
    let bar0 = pci_read(bus, dev, func, 0x10);
    if bar0 & 1 != 0 { log("hda !! BAR0 khong phai MMIO"); return false; }
    let mut pa = (bar0 & 0xFFFF_FFF0) as u64;
    if (bar0 >> 1) & 3 == 2 { pa |= (pci_read(bus, dev, func, 0x14) as u64) << 32; }
    log_hex("hda BAR0 phys   =", pa);
    if pa == 0 { log("hda !! BAR0 chua duoc gan dia chi"); return false; }
    let cmdreg = pci_read(bus, dev, func, 4) & 0xFFFF;
    pci_write(bus, dev, func, 4, cmdreg | 0x6); // memory space + bus master
    if id & 0xFFFF == 0x8086 {
        let v = pci_read(bus, dev, func, 0x44);
        pci_write(bus, dev, func, 0x44, v & !7); // TCSEL = TC0
    }
    let m = ((*API).map_mmio)(pa, 0x1000);
    if m.is_null() { log("hda !! map_mmio that bai (kernel chua vá fix_mmio?)"); return false; }
    MMIO = m;

    // reset controller
    w32(0x08, 0);
    if !wait(100_000, || r32(0x08) & 1 == 0) { log("hda !! CRST khong xuong 0"); }
    delay_us(200);
    w32(0x08, 1);
    if !wait(100_000, || r32(0x08) & 1 != 0) { log("hda !! CRST khong len 1"); return false; }
    wait(300_000, || r16(0x0E) & 0x7FFF != 0);
    delay_us(1000);
    let ss = r16(0x0E) & 0x7FFF;
    log_hex("hda STATESTS    =", ss as u64);
    if ss == 0 { log("hda !! khong co codec"); return false; }
    s.cad = ss.trailing_zeros();
    let gcap = r16(0x00);
    log_hex("hda GCAP        =", gcap as u64);
    let iss = ((gcap >> 8) & 0xF) as usize;
    let oss = ((gcap >> 12) & 0xF) as usize;
    if oss == 0 { log("hda !! controller khong co output stream"); return false; }
    s.sd = 0x80 + iss * 0x20;
    w32(0x20, 0);      // tat ngat
    w16(0x0E, 0x7FFF); // xoa STATESTS

    // CORB / RIRB
    w8(0x4C, 0);
    w8(0x5C, 0);
    wait(100_000, || r8(0x4C) & 2 == 0);
    wait(100_000, || r8(0x5C) & 2 == 0);
    write_bytes(addr_of_mut!(CORB) as *mut u8, 0, 1024);
    write_bytes(addr_of_mut!(RIRB) as *mut u8, 0, 2048);
    let cc = r8(0x4E);
    if cc & 0x40 != 0 { w8(0x4E, 2); s.n_corb = 256; } else if cc & 0x20 != 0 { w8(0x4E, 1); s.n_corb = 16; } else { w8(0x4E, 0); s.n_corb = 2; }
    let rc = r8(0x5E);
    if rc & 0x40 != 0 { w8(0x5E, 2); s.n_rirb = 256; } else if rc & 0x20 != 0 { w8(0x5E, 1); s.n_rirb = 16; } else { w8(0x5E, 0); s.n_rirb = 2; }
    let cb = phys(addr_of_mut!(CORB) as *const Corb);
    let rb = phys(addr_of_mut!(RIRB) as *const Rirb);
    w32(0x40, cb as u32); w32(0x44, (cb >> 32) as u32);
    w32(0x50, rb as u32); w32(0x54, (rb >> 32) as u32);
    w16(0x4A, 0x8000);
    wait(100_000, || r16(0x4A) & 0x8000 != 0);
    w16(0x4A, 0);
    wait(100_000, || r16(0x4A) & 0x8000 == 0);
    w16(0x48, 0);
    s.corb_wp = 0;
    w16(0x58, 0x8000);
    s.rirb_rp = 0;
    w16(0x5A, 1);
    w8(0x5C, 2);
    w8(0x4C, 2);

    // codec
    let c = s.cad;
    let vid = match cmd(s, v12(c, 0, 0xF00, 0)) {
        Some(v) => v,
        None => { log("hda !! codec khong tra loi (CORB/RIRB)"); return false; }
    };
    log_hex("hda codec id    =", vid as u64);
    let sub = match cmd(s, v12(c, 0, 0xF00, 0x04)) {
        Some(v) => v,
        None => { log("hda !! TIMEOUT lenh param 0x04 cua root"); return false; }
    };
    log_hex("hda root sub     =", sub as u64);
    let mut afg = 0u32;
    for nid in ((sub >> 16) & 0xFF)..((sub >> 16) & 0xFF) + (sub & 0xFF) {
        let t = cmd(s, v12(c, nid, 0xF00, 0x05));
        log_hex("hda fg nid       =", nid as u64);
        match t {
            Some(v) => { log_hex("hda fg type      =", v as u64); if v & 0xFF == 1 { afg = nid; break; } }
            None => log("hda !! TIMEOUT lenh param 0x05"),
        }
    }
    if afg == 0 { log("hda !! khong co Audio Function Group"); return false; }
    s.afg = afg;
    set12(s, afg, 0x705, 0);
    delay_us(20000);
    let sub = param(s, afg, 0x04);
    if !find_output(s, (sub >> 16) & 0xFF, sub & 0xFF) { log("hda !! khong tim thay duong ra (pin -> DAC)"); return false; }
    log_hex("hda AFG nid     =", afg as u64);
    log_hex("hda pin nid     =", s.pin as u64);
    log_hex("hda DAC nid     =", s.dac as u64);

    let d = s.dac;
    let mut pcm = param(s, d, 0x0A);
    if pcm == 0 { pcm = param(s, afg, 0x0A); }
    s.rates = pcm & 0x1FF;
    if s.rates == 0 { s.rates = 1 << 6; }
    s.rates |= 1 << 6; // 48000 Hz luon duoc phep (codec ao co the khai bao thieu)
    log_hex("hda rates mask  =", s.rates as u64);
    s.rate = 48000;
    s.fmt = 0x0011;
    set4(s, d, 0x2, s.fmt as u32);

    for i in 0..NBUF {
        let b = ring().add(i * BUF_SZ);
        (*addr_of_mut!(BDL)).0[i] = Bd { addr: phys(b as *const u8), len: BUF_SZ as u32, flags: 0 };
    }
    log_hex("hda ring phys   =", phys(ring() as *const u8));
    log_hex("hda BDL phys    =", phys(addr_of_mut!(BDL) as *const Bdl));
    chan_reset(s);
    log("hda: san sang");
    true
}

unsafe fn ensure() -> bool {
    let s = st();
    if s.ready { return true; }
    if s.failed || !s.detected { return false; }
    if hw_init(s) {
        s.ready = true;
        apply_vol(s);
        true
    } else {
        s.failed = true;
        false
    }
}

// ---------------- AudioOps ----------------
extern "C" fn set_rate(r: u32) -> bool {
    unsafe {
        if !ensure() { return false; }
        let s = st();
        let (fmt, bit) = match fmt_of(r) { Some(x) => x, None => return false };
        if s.rates & (1 << bit) == 0 { return false; }
        s.rate = r;
        s.fmt = fmt;
        chan_reset(s);
        let d = s.dac;
        set4(s, d, 0x2, fmt as u32);
        true
    }
}
extern "C" fn get_rate() -> u32 { st().rate }
extern "C" fn set_volume(l: u8, r: u8) {
    unsafe {
        let s = st();
        s.vol_l = l;
        s.vol_r = r;
        apply_vol(s);
    }
}
extern "C" fn write(buf: *const u8, len: usize) -> usize {
    unsafe {
        if !ensure() { return 0; }
        let s = st();
        let len = len & !3;
        let mut done = 0usize;
        while done < len {
            poll(s);
            if s.running && s.consumed >= s.written { chan_reset(s); } // underrun: dung, khong lap rac
            let queued = (s.written - s.consumed.min(s.written)) as usize;
            let free = RING.saturating_sub(queued + MARGIN);
            let n = (len - done).min(free).min(RING - s.wpos) & !3;
            if n == 0 {
                if !s.running && s.written > 0 { chan_start(s); }
                break; // ring day: kernel yield roi goi lai
            }
            copy_nonoverlapping(buf.add(done), ring().add(s.wpos), n);
            s.wpos = (s.wpos + n) % RING;
            s.written += n as u64;
            done += n;
            if !s.running && s.written as usize >= START_BYTES { chan_start(s); }
        }
        done
    }
}
extern "C" fn drain() {
    unsafe {
        let s = st();
        if !s.ready { return; }
        if !s.running {
            if s.written == 0 { return; }
            chan_start(s);
        }
        let mut n = 0u32;
        loop {
            poll(s);
            if s.consumed >= s.written || n > 300_000_000 { break; }
            n += 1;
            core::hint::spin_loop();
        }
        delay_us(10000);
        chan_reset(s);
    }
}
extern "C" fn stop() {
    unsafe { let s = st(); if s.ready { chan_reset(s); } }
}
/// Panic: lap doan ngan ngay truoc cho audio. KHONG dung DMA / KHONG reset stream nen khong co
/// khoang lang: DMA cu chay tiep, ta ghi de ring o phia truoc vi tri phat de chuyen thang sang loop.
/// Chinh HOLD_MS / HOLD_AHEAD_MS o dau file.
const _: () = assert!(RING % 4 == 0 && HOLD_MAX % 4 == 0);
static mut HELD: bool = false;
#[repr(C, align(16))]
struct HoldBuf([u8; HOLD_MAX]);
static mut HOLD_BUF: HoldBuf = HoldBuf([0; HOLD_MAX]);

/// Do dai loop (byte): uoc luong tu HOLD_MS, lam tron ve uoc cua RING gan nhat (noi vong ring lien mach).
fn hold_len(rate: u32) -> usize {
    let want = (rate as u64 * 4 * HOLD_MS as u64 / 1000) as usize;
    let (mut best, mut bd) = (HOLD_MAX, usize::MAX);
    let mut k = 2usize; // RING/k <= RING/2 = HOLD_MAX
    while k <= RING / 4 {
        if RING % k == 0 && (RING / k) % 4 == 0 {
            let h = RING / k;
            let d = if h > want { h - want } else { want - h };
            if d < bd { bd = d; best = h; }
        }
        k += 1;
    }
    best
}

extern "C" fn hold() {
    unsafe {
        let s = st();
        if !(s.ready && s.running) { return; }
        let sd = s.sd;
        if !HELD {
            HELD = true;
            let hold = hold_len(s.rate);
            let ahead = ((s.rate as usize * 4 * HOLD_AHEAD_MS as usize / 1000) & !3).clamp(1024, RING / 4);
            // q = noi bat dau loop. Am thanh binh thuong van phat tiep tu vi tri hien tai toi q.
            let pos = (r32(sd + 4) as usize % RING) & !3;
            let q = (pos + ahead) % RING;
            let start = (q + RING - hold) % RING;
            let chunk = addr_of_mut!(HOLD_BUF) as *mut u8;
            // 1. chup `hold` byte ngay truoc q
            let mut i = 0usize;
            while i < hold {
                let v = read_volatile(ring().add((start + i) % RING) as *const u32);
                write_volatile(chunk.add(i) as *mut u32, v);
                i += 4;
            }
            // 2. ring[(q + i) % RING] = chunk[i % hold]. RING chia het cho hold nen noi vong lien mach;
            //    phan cuoi trung dung du lieu dang co nen an toan khi DMA dang doc.
            let (mut i, mut j) = (0usize, 0usize);
            while i < RING {
                let v = read_volatile(chunk.add(j) as *const u32);
                write_volatile(ring().add((q + i) % RING) as *mut u32, v);
                i += 4;
                j += 4;
                if j >= hold { j = 0; }
            }
        }
        if r8(sd) & 2 == 0 { w8(sd, r8(sd) | 2); } // chi bat lai RUN neu lo bi tat
    }
}

static OPS: AudioOps = AudioOps { set_rate, get_rate, set_volume, write, drain, stop, hold };

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    unsafe {
        API = api;
        if (*api).abi_version != DRIVER_ABI { return -1; }
        let (b, d, f) = match pci_find_hda() {
            Some(x) => x,
            None => { log("hda: khong thay Intel HDA (PCI class 0x0403)"); return -2; }
        };
        let s = st();
        s.bus = b; s.dev = d; s.func = f;
        s.detected = true;
        log("hda: phat hien controller, se khoi tao khi dung lan dau");
        let info = DriverInfo {
            name: name("hda"),
            kind: KIND_AUDIO,
            ops: &OPS as *const AudioOps as *const c_void,
        };
        ((*api).register)(&info)
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
