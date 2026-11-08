//! Driver mang Intel e1000 (82540EM / 82545EM / 82543GC). Polling, khong dung IRQ.
//! QEMU:       -device e1000,netdev=n0 -netdev user,id=n0
//! VirtualBox: adapter "Intel PRO/1000 MT Desktop" cung la 82540EM.
//!
//! Gia dinh: driver chay o ring 0 (can in/out port de quet PCI), kernel khong goi
//! cac ham trong NetOps dong thoi tu nhieu luong.
#![no_std]
#![no_main]

use core::arch::asm;
use core::ffi::c_void;
use core::panic::PanicInfo;
use core::ptr::{addr_of_mut, copy_nonoverlapping, null, null_mut, read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};
use driver_abi::*;

/// Sau khi init: gui ARP toi 10.0.2.2 (gateway cua QEMU user-net), log ket qua.
const SELFTEST: bool = true;

const RING: usize = 32; // so descriptor moi ring (RING * 16 phai chia het cho 128)
const BUF: usize = 2048; // moi buffer 2 KiB (khop RCTL.BSIZE = 00)
const MAX_FRAME: usize = 1514;
const MMIO_LEN: usize = 0x20000;

// ---- thanh ghi ----
const REG_CTRL: usize = 0x0000;
const REG_STATUS: usize = 0x0008;
const REG_EERD: usize = 0x0014;
const REG_ICR: usize = 0x00C0;
const REG_IMC: usize = 0x00D8;
const REG_RCTL: usize = 0x0100;
const REG_TCTL: usize = 0x0400;
const REG_TIPG: usize = 0x0410;
const REG_RDBAL: usize = 0x2800;
const REG_RDBAH: usize = 0x2804;
const REG_RDLEN: usize = 0x2808;
const REG_RDH: usize = 0x2810;
const REG_RDT: usize = 0x2818;
const REG_TDBAL: usize = 0x3800;
const REG_TDBAH: usize = 0x3804;
const REG_TDLEN: usize = 0x3808;
const REG_TDH: usize = 0x3810;
const REG_TDT: usize = 0x3818;
const REG_MTA: usize = 0x5200;
const REG_RAL0: usize = 0x5400;
const REG_RAH0: usize = 0x5404;

const CTRL_LRST: u32 = 1 << 3;
const CTRL_ASDE: u32 = 1 << 5;
const CTRL_SLU: u32 = 1 << 6;
const CTRL_RST: u32 = 1 << 26;
const CTRL_PHY_RST: u32 = 1 << 31;
const STATUS_LU: u32 = 1 << 1;
const RCTL_EN: u32 = 1 << 1;
const RCTL_BAM: u32 = 1 << 15;
const RCTL_SECRC: u32 = 1 << 26;
const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3;
const TXD_CMD: u8 = 0x0B; // EOP | IFCS | RS
const DD: u8 = 1 << 0;
const EOP: u8 = 1 << 1;

// ---- descriptor + vung DMA (tinh, 4 KiB-aligned de moi buffer nam gon trong 1 trang) ----
#[repr(C)]
#[derive(Clone, Copy)]
struct RxDesc {
    addr: u64,
    len: u16,
    csum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TxDesc {
    addr: u64,
    len: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

const RX0: RxDesc = RxDesc { addr: 0, len: 0, csum: 0, status: 0, errors: 0, special: 0 };
const TX0: TxDesc = TxDesc { addr: 0, len: 0, cso: 0, cmd: 0, status: 0, css: 0, special: 0 };

#[repr(C, align(4096))]
struct Rings {
    rx: [RxDesc; RING],
    tx: [TxDesc; RING],
}

#[repr(C, align(4096))]
struct Bufs {
    rx: [[u8; BUF]; RING],
    tx: [[u8; BUF]; RING],
}

static mut RINGS: Rings = Rings { rx: [RX0; RING], tx: [TX0; RING] };
static mut BUFS: Bufs = Bufs { rx: [[0; BUF]; RING], tx: [[0; BUF]; RING] };

struct Nic {
    mmio: *mut u8,
    phys: u64, // dia chi vat ly BAR0 (probe luc driver_entry, map luc init)
    mac: [u8; 6],
    rx_cur: usize,
    tx_tail: usize,
    ready: bool,
}

static mut NIC: Nic = Nic { mmio: null_mut(), phys: 0, mac: [0; 6], rx_cur: 0, tx_tail: 0, ready: false };

/// Con tro toi KernelApi (kernel giu static, song suot thoi gian chay).
static mut KAPI: *const KernelApi = null();

fn nic() -> &'static mut Nic {
    unsafe { &mut *addr_of_mut!(NIC) }
}

fn rx_desc(i: usize) -> *mut RxDesc {
    unsafe { addr_of_mut!((*addr_of_mut!(RINGS)).rx[i]) }
}
fn tx_desc(i: usize) -> *mut TxDesc {
    unsafe { addr_of_mut!((*addr_of_mut!(RINGS)).tx[i]) }
}
fn rx_buf(i: usize) -> *mut u8 {
    unsafe { addr_of_mut!((*addr_of_mut!(BUFS)).rx[i]) as *mut u8 }
}
fn tx_buf(i: usize) -> *mut u8 {
    unsafe { addr_of_mut!((*addr_of_mut!(BUFS)).tx[i]) as *mut u8 }
}
fn rx_ring() -> *const u8 {
    unsafe { addr_of_mut!((*addr_of_mut!(RINGS)).rx) as *const u8 }
}
fn tx_ring() -> *const u8 {
    unsafe { addr_of_mut!((*addr_of_mut!(RINGS)).tx) as *const u8 }
}

// ---- MMIO ----
fn rd(m: *mut u8, off: usize) -> u32 {
    unsafe { read_volatile(m.add(off) as *const u32) }
}
fn wr(m: *mut u8, off: usize, v: u32) {
    unsafe { write_volatile(m.add(off) as *mut u32, v) }
}

// ---- PCI config space (cong 0xCF8 / 0xCFC) ----
fn outl(port: u16, v: u32) {
    unsafe { asm!("out dx, eax", in("dx") port, in("eax") v, options(nomem, nostack, preserves_flags)) }
}
fn inl(port: u16) -> u32 {
    let v: u32;
    unsafe { asm!("in eax, dx", in("dx") port, out("eax") v, options(nomem, nostack, preserves_flags)) }
    v
}
fn pci_addr(bus: u8, dev: u8, func: u8, off: u8) -> u32 {
    0x8000_0000 | ((bus as u32) << 16) | ((dev as u32) << 11) | ((func as u32) << 8) | (off as u32 & 0xFC)
}
fn pci_read(bus: u8, dev: u8, func: u8, off: u8) -> u32 {
    outl(0xCF8, pci_addr(bus, dev, func, off));
    inl(0xCFC)
}
fn pci_write(bus: u8, dev: u8, func: u8, off: u8, v: u32) {
    outl(0xCF8, pci_addr(bus, dev, func, off));
    outl(0xCFC, v);
}

fn pci_find() -> Option<(u8, u8, u8)> {
    for bus in 0..16u8 {
        for dev in 0..32u8 {
            for func in 0..8u8 {
                let id = pci_read(bus, dev, func, 0);
                if id & 0xFFFF == 0xFFFF {
                    if func == 0 {
                        break;
                    }
                    continue;
                }
                if id & 0xFFFF == 0x8086 && matches!(id >> 16, 0x100E | 0x100F | 0x1004) {
                    return Some((bus, dev, func));
                }
                // khong phai thiet bi da chuc nang -> khoi quet func 1..7
                if func == 0 && (pci_read(bus, dev, 0, 0x0C) >> 16) & 0x80 == 0 {
                    break;
                }
            }
        }
    }
    None
}

fn eeprom_read(m: *mut u8, addr: u32) -> Option<u16> {
    wr(m, REG_EERD, 1 | (addr << 8));
    for _ in 0..100_000 {
        let v = rd(m, REG_EERD);
        if v & (1 << 4) != 0 {
            return Some((v >> 16) as u16);
        }
    }
    None
}

// ---- log ----
struct Msg {
    b: [u8; 112],
    n: usize,
}
impl Msg {
    fn new() -> Self {
        Msg { b: [0; 112], n: 0 }
    }
    fn c(&mut self, c: u8) {
        if self.n < self.b.len() {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
    fn s(&mut self, s: &str) -> &mut Self {
        for &c in s.as_bytes() {
            self.c(c);
        }
        self
    }
    fn mac(&mut self, m: &[u8]) -> &mut Self {
        const H: &[u8; 16] = b"0123456789abcdef";
        for (i, &x) in m.iter().enumerate() {
            if i > 0 {
                self.c(b':');
            }
            self.c(H[(x >> 4) as usize]);
            self.c(H[(x & 15) as usize]);
        }
        self
    }
    fn out(&self, api: &KernelApi) {
        (api.log)(self.b.as_ptr(), self.n);
    }
}

// ---- NetOps ----
extern "C" fn net_mac(out: *mut u8) {
    if !out.is_null() {
        unsafe { copy_nonoverlapping(nic().mac.as_ptr(), out, 6) };
    }
}

extern "C" fn net_link_up() -> bool {
    let n = nic();
    n.ready && rd(n.mmio, REG_STATUS) & STATUS_LU != 0
}

extern "C" fn net_send(buf: *const u8, len: usize) -> bool {
    if buf.is_null() || len < 14 || len > MAX_FRAME {
        return false;
    }
    let n = nic();
    if !n.ready {
        return false;
    }
    let i = n.tx_tail;
    let d = tx_desc(i);
    unsafe {
        if read_volatile(&(*d).status) & DD == 0 {
            return false; // ring day, thu lai sau
        }
        copy_nonoverlapping(buf, tx_buf(i), len);
        write_volatile(&mut (*d).len, len as u16);
        write_volatile(&mut (*d).cmd, TXD_CMD);
        write_volatile(&mut (*d).status, 0);
    }
    fence(Ordering::SeqCst);
    n.tx_tail = (i + 1) % RING;
    wr(n.mmio, REG_TDT, n.tx_tail as u32);
    true
}

/// Tra so byte da copy vao buf (toi da cap). 0 = chua co frame.
extern "C" fn net_recv(buf: *mut u8, cap: usize) -> usize {
    let n = nic();
    if !n.ready {
        return 0;
    }
    loop {
        let i = n.rx_cur;
        let d = rx_desc(i);
        let (st, len, err) = unsafe { (read_volatile(&(*d).status), read_volatile(&(*d).len) as usize, read_volatile(&(*d).errors)) };
        if st & DD == 0 {
            return 0;
        }
        fence(Ordering::Acquire);
        let ok = st & EOP != 0 && err == 0 && len > 0 && len <= BUF;
        let mut copied = 0;
        if ok && !buf.is_null() {
            copied = len.min(cap);
            unsafe { copy_nonoverlapping(rx_buf(i), buf, copied) };
        }
        unsafe { write_volatile(&mut (*d).status, 0) };
        fence(Ordering::SeqCst);
        n.rx_cur = (i + 1) % RING;
        wr(n.mmio, REG_RDT, i as u32); // tra descriptor lai cho card
        if ok {
            return copied;
        }
        // frame loi: bo, thu descriptor ke tiep
    }
}


// ---- khoi tao ----
/// Luc driver_entry: chi quet PCI + bat bus master + doc BAR0 (khong map MMIO,
/// vi luc nay kernel chua init pmm/paging).
fn probe() -> Result<(), &'static str> {
    let (bus, dev, func) = pci_find().ok_or("khong thay card e1000 tren PCI")?;

    // bat Memory Space + Bus Master (chi ghi 16 bit thap, tranh dong vao bit status W1C)
    let cmd = pci_read(bus, dev, func, 0x04);
    pci_write(bus, dev, func, 0x04, (cmd & 0xFFFF) | 0x6);

    let bar0 = pci_read(bus, dev, func, 0x10);
    if bar0 & 1 != 0 {
        return Err("BAR0 la I/O, can MMIO");
    }
    let mut phys = (bar0 & !0xF) as u64;
    if (bar0 >> 1) & 3 == 2 {
        phys |= (pci_read(bus, dev, func, 0x14) as u64) << 32;
    }
    nic().phys = phys;
    Ok(())
}

/// Kernel goi sau khi pmm/paging san sang: map MMIO, reset card, dung ring.
fn init_hw(api: &KernelApi) -> Result<(), &'static str> {
    let phys = nic().phys;
    if phys == 0 {
        return Err("chua probe");
    }
    let m = (api.map_mmio)(phys, MMIO_LEN);
    if m.is_null() {
        return Err("map_mmio that bai");
    }
    let n = nic();
    n.mmio = m;
    n.rx_cur = 0;
    n.tx_tail = 0;
    n.ready = false;

    // reset
    wr(m, REG_IMC, 0xFFFF_FFFF);
    wr(m, REG_RCTL, 0);
    wr(m, REG_TCTL, TCTL_PSP);
    let _ = rd(m, REG_STATUS);
    wr(m, REG_CTRL, rd(m, REG_CTRL) | CTRL_RST);
    let mut t = 0u32;
    while rd(m, REG_CTRL) & CTRL_RST != 0 {
        t += 1;
        if t > 1_000_000 {
            return Err("reset timeout");
        }
    }
    for _ in 0..20_000 {
        let _ = rd(m, REG_STATUS); // tre ~ vai ms tren card that
    }
    wr(m, REG_IMC, 0xFFFF_FFFF);
    let _ = rd(m, REG_ICR);
    let c = rd(m, REG_CTRL);
    wr(m, REG_CTRL, (c | CTRL_SLU | CTRL_ASDE) & !(CTRL_LRST | CTRL_PHY_RST));

    // MAC: uu tien RAL/RAH (BIOS/QEMU da nap), khong co thi doc EEPROM
    let ral = rd(m, REG_RAL0);
    let rah = rd(m, REG_RAH0);
    let mut mac = [0u8; 6];
    if ral != 0 || (rah & 0xFFFF) != 0 {
        mac[..4].copy_from_slice(&ral.to_le_bytes());
        mac[4..].copy_from_slice(&(rah as u16).to_le_bytes());
    } else {
        for i in 0..3 {
            let w = eeprom_read(m, i as u32).ok_or("doc EEPROM that bai")?;
            mac[i * 2] = w as u8;
            mac[i * 2 + 1] = (w >> 8) as u8;
        }
    }
    if mac == [0; 6] {
        return Err("MAC = 0");
    }
    wr(m, REG_RAL0, u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]));
    wr(m, REG_RAH0, (mac[4] as u32) | ((mac[5] as u32) << 8) | (1 << 31));
    n.mac = mac;
    for i in 0..128 {
        wr(m, REG_MTA + i * 4, 0);
    }

    // ring + buffer (dia chi vat ly lay tung buffer mot, khong gia dinh lien tuc)
    let v2p = api.virt_to_phys;
    for i in 0..RING {
        unsafe {
            let r = rx_desc(i);
            write_volatile(&mut (*r).addr, v2p(rx_buf(i)));
            write_volatile(&mut (*r).status, 0);
            let x = tx_desc(i);
            write_volatile(&mut (*x).addr, v2p(tx_buf(i)));
            write_volatile(&mut (*x).status, DD); // DD = descriptor ranh
        }
    }
    let rxp = v2p(rx_ring());
    wr(m, REG_RDBAL, rxp as u32);
    wr(m, REG_RDBAH, (rxp >> 32) as u32);
    wr(m, REG_RDLEN, (RING * 16) as u32);
    wr(m, REG_RDH, 0);
    wr(m, REG_RDT, (RING - 1) as u32);

    let txp = v2p(tx_ring());
    wr(m, REG_TDBAL, txp as u32);
    wr(m, REG_TDBAH, (txp >> 32) as u32);
    wr(m, REG_TDLEN, (RING * 16) as u32);
    wr(m, REG_TDH, 0);
    wr(m, REG_TDT, 0);

    fence(Ordering::SeqCst);
    wr(m, REG_TIPG, 0x0060_200A);
    wr(m, REG_TCTL, TCTL_EN | TCTL_PSP | (0x0F << 4) | (0x40 << 12));
    wr(m, REG_RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC);
    n.ready = true;

    let up = rd(m, REG_STATUS) & STATUS_LU != 0;
    Msg::new().s("e1000: mac ").mac(&mac).s(if up { ", link up\n" } else { ", link down\n" }).out(api);
    Ok(())
}

/// ARP request toi 10.0.2.2, cho ARP reply. Chi de kiem tra nhanh TX + RX.
fn selftest(api: &KernelApi) {
    if !net_link_up() {
        Msg::new().s("e1000: selftest bo qua (link down)\n").out(api);
        return;
    }
    let mac = nic().mac;
    let mut f = [0u8; 42];
    f[0..6].fill(0xFF);
    f[6..12].copy_from_slice(&mac);
    f[12] = 0x08;
    f[13] = 0x06; // ethertype ARP
    f[14..22].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]); // htype, ptype, hlen, plen, op=request
    f[22..28].copy_from_slice(&mac);
    f[28..32].copy_from_slice(&[10, 0, 2, 15]);
    f[38..42].copy_from_slice(&[10, 0, 2, 2]);
    if !net_send(f.as_ptr(), f.len()) {
        Msg::new().s("e1000: selftest: send that bai\n").out(api);
        return;
    }
    let mut rb = [0u8; 1600];
    for _ in 0..100_000_000u32 {
        let len = net_recv(rb.as_mut_ptr(), rb.len());
        if len >= 42 && rb[12] == 0x08 && rb[13] == 0x06 && rb[20] == 0 && rb[21] == 2 && rb[28..32] == [10, 0, 2, 2] {
            Msg::new().s("e1000: selftest OK, ARP reply 10.0.2.2 = ").mac(&rb[22..28]).s("\n").out(api);
            return;
        }
    }
    Msg::new().s("e1000: selftest: khong co ARP reply\n").out(api);
}

extern "C" fn net_init() -> bool {
    let api = unsafe { KAPI };
    if api.is_null() {
        return false;
    }
    let api = unsafe { &*api };
    if nic().ready {
        return true;
    }
    match init_hw(api) {
        Ok(()) => {
            if SELFTEST {
                selftest(api);
            }
            true
        }
        Err(e) => {
            Msg::new().s("e1000: ").s(e).s("\n").out(api);
            false
        }
    }
}

static OPS: NetOps = NetOps { init: net_init, mac: net_mac, link_up: net_link_up, send: net_send, recv: net_recv };

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    if api.is_null() {
        return -1;
    }
    let api = unsafe { &*api };
    if api.abi_version != DRIVER_ABI {
        return -1;
    }
    unsafe { KAPI = api as *const KernelApi };
    if let Err(e) = probe() {
        Msg::new().s("e1000: ").s(e).s("\n").out(api);
        return -2;
    }
    let info = DriverInfo { name: name("e1000"), kind: KIND_NET, ops: &OPS as *const NetOps as *const c_void };
    let r = (api.register)(&info);
    if r != 0 {
        Msg::new().s("e1000: kernel tu choi register (chua ho tro KIND_NET?)\n").out(api);
    }
    r
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}
