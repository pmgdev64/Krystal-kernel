//! Man hinh panic kieu log kernel. Ghi thang vao bootvid + COM1, khong di qua kprintln.
use crate::drivers;
use crate::idt::Regs;
use bootvid_abi::Color;
use core::arch::asm;
use core::fmt::{self, Write};
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

static PANICKING: AtomicBool = AtomicBool::new(false);

const BG: Color = Color::BLACK;
const FG: Color = Color::rgb(210, 210, 210);

fn raw_serial(b: u8) {
    unsafe {
        for _ in 0..100_000 {
            let s: u8;
            asm!("in al, dx", in("dx") 0x3FDu16, out("al") s, options(nomem, nostack));
            if s & 0x20 != 0 {
                break;
            }
        }
        asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
    }
}

struct Out;
impl Write for Out {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(v) = drivers::video_ops() {
            (v.write_str)(s.as_ptr(), s.len());
        }
        for b in s.bytes() {
            raw_serial(b);
        }
        Ok(())
    }
}

macro_rules! out {
    ($($a:tt)*) => {{ let _ = write!(Out, $($a)*); }};
}

fn color(fg: Color) {
    if let Some(v) = drivers::video_ops() {
        (v.set_colors)(fg, BG);
    }
}

/// Dung han: khong hlt khi co audio de hold() duoc goi lien tuc (dem gio tieng re roi tu cat).
fn halt() -> ! {
    unsafe { asm!("cli") };
    loop {
        match drivers::audio_ops() {
            Some(a) => {
                (a.hold)();
                for _ in 0..20000 {
                    core::hint::spin_loop();
                }
            }
            None => unsafe { asm!("hlt") },
        }
    }
}

/// Tat interrupt, chan panic long nhau, xoa man hinh.
fn begin() {
    unsafe { asm!("cli") };
    if PANICKING.swap(true, Ordering::SeqCst) {
        for b in b"\nKernel panic: recursive panic, halting\n" {
            raw_serial(*b);
        }
        halt();
    }
    if let Some(a) = drivers::audio_ops() {
        (a.hold)();
    }
    let frozen = freeze_frame();
    // diag-freeze
    for b in if frozen { b"\n[panic] frozen=1\n" as &[u8] } else { b"\n[panic] frozen=0 (no fb hoac bpp != 32)\n" as &[u8] } { raw_serial(*b); }
    if let Some(v) = drivers::video_ops() {
        (v.force_unlock)();
        if frozen {
            (v.set_scroll)(false);
            (v.set_transparent)(false);
            (v.set_cursor)(0, 0);
            (v.set_colors)(FG, BG);
        } else {
            if let Some(fb) = crate::video::saved() { (v.init)(&fb); }
            (v.set_colors)(FG, BG);
            (v.clear)();
        }
    }
}

/// In " name+0xoff" neu kdbg.sys da nap symbol. `caller` = true voi return address.
fn sym(addr: u64, caller: bool) {
    if let Some(d) = drivers::debug_ops() {
        let mut buf = [0u8; 160];
        let mut off = 0u64;
        let a = if caller { addr.wrapping_sub(1) } else { addr };
        if (d.resolve)(a, buf.as_mut_ptr(), buf.len(), &mut off) {
            let off = if caller { off + 1 } else { off };
            out!("  {}+{:#x}", drivers::cstr(&buf), off);
        }
    }
}

fn backtrace(rip: Option<u64>, mut rbp: u64, rsp: u64) {
    out!("\n-- backtrace --\n");
    let mut n = 0;
    if let Some(ip) = rip {
        out!("  #00  {:016x}", ip);
        sym(ip, false);
        out!("\n");
        n = 1;
    }
    let (lo, hi) = (rsp, rsp.wrapping_add(0x10000));
    while n < 16 {
        if rbp < lo || rbp >= hi || rbp & 7 != 0 {
            break;
        }
        let (next, ret) = unsafe { (*(rbp as *const u64), *(rbp as *const u64).add(1)) };
        if ret == 0 {
            break;
        }
        out!("  #{:02}  {:016x}", n, ret);
        sym(ret, true);
        out!("\n");
        n += 1;
        if next <= rbp {
            break;
        }
        rbp = next;
    }
    if drivers::debug_ops().is_none() {
        color(Color::GRAY);
        out!("\n(no kdbg.sys) addr2line -e kernel/target/x86_64-unknown-none/debug/kernel -f -C <addr>\n");
        color(FG);
    }
}

pub fn panic(info: &PanicInfo) -> ! {
    begin();
    color(Color::YELLOW);
    out!("== Krystal :: fatal error ==\n\nreason  {}\n", info.message());
    color(FG);
    if let Some(l) = info.location() {
        out!("where   {}:{}:{}\n", l.file(), l.line(), l.column());
    }
    out!("task    pid {}\n", crate::task::pid());
    let (rbp, rsp): (u64, u64);
    unsafe { asm!("mov {}, rbp", "mov {}, rsp", out(reg) rbp, out(reg) rsp) };
    backtrace(None, rbp, rsp);
    color(Color::YELLOW);
    out!("\n-- halted --\n");
    color(FG);
    halt()
}

const NAMES: [&str; 32] = [
    "Divide Error (#DE)",
    "Debug (#DB)",
    "Non-Maskable Interrupt",
    "Breakpoint (#BP)",
    "Overflow (#OF)",
    "BOUND Range Exceeded (#BR)",
    "Invalid Opcode (#UD)",
    "Device Not Available (#NM)",
    "Double Fault (#DF)",
    "Coprocessor Segment Overrun",
    "Invalid TSS (#TS)",
    "Segment Not Present (#NP)",
    "Stack-Segment Fault (#SS)",
    "General Protection Fault (#GP)",
    "Page Fault (#PF)",
    "Reserved",
    "x87 Floating-Point (#MF)",
    "Alignment Check (#AC)",
    "Machine Check (#MC)",
    "SIMD Floating-Point (#XM)",
    "Virtualization (#VE)",
    "Control Protection (#CP)",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Hypervisor Injection (#HV)",
    "VMM Communication (#VC)",
    "Security Exception (#SX)",
    "Reserved",
];

fn cr(n: u8) -> u64 {
    let v: u64;
    unsafe {
        match n {
            0 => asm!("mov {}, cr0", out(reg) v),
            2 => asm!("mov {}, cr2", out(reg) v),
            3 => asm!("mov {}, cr3", out(reg) v),
            _ => asm!("mov {}, cr4", out(reg) v),
        }
    }
    v
}

pub fn exception(r: &Regs) -> ! {
    begin();
    let v = r.vector as usize;
    color(Color::YELLOW);
    out!("== HYPERION :: cpu exception ==\n\nfault   {}  vec={} err={:#x}\n", NAMES.get(v).copied().unwrap_or("Unknown"), v, r.error);
    color(FG);

    if v == 14 {
        let e = r.error;
        out!(
            "access  {:#018x}  {} {} {}{}{}\n",
            cr(2),
            if e & 1 != 0 { "protection-violation" } else { "page-not-present" },
            if e & 2 != 0 { "on-write" } else { "on-read" },
            if e & 4 != 0 { "user" } else { "supervisor" },
            if e & 8 != 0 { " reserved-bit-set" } else { "" },
            if e & 16 != 0 { " instruction-fetch" } else { "" },
        );
    }
    if v == 8 {
        out!("Double fault: stack corrupted or exception handler failed.\n");
    }

    out!("task    pid {}\n", crate::task::pid());
    out!("rip  {:04x}:{:016x} flags {:08x}\n", r.cs, r.rip, r.rflags);
    out!("rsp  {:04x}:{:016x}\n", r.ss, r.rsp);
    out!("rax {:016x} rbx {:016x} rcx {:016x}\n", r.rax, r.rbx, r.rcx);
    out!("rdx {:016x} rsi {:016x} rdi {:016x}\n", r.rdx, r.rsi, r.rdi);
    out!("rbp {:016x} r8  {:016x} r9  {:016x}\n", r.rbp, r.r8, r.r9);
    out!("r10 {:016x} r11 {:016x} r12 {:016x}\n", r.r10, r.r11, r.r12);
    out!("r13 {:016x} r14 {:016x} r15 {:016x}\n", r.r13, r.r14, r.r15);
    out!("cr0 {:016x} cr2 {:016x}\n", cr(0), cr(2));
    out!("cr3 {:016x} cr4 {:016x}\n", cr(3), cr(4));

    backtrace(Some(r.rip), r.rbp, r.rsp);
    color(Color::YELLOW);
    out!("\n-- halted --\n");
    color(FG);
    halt()
}

/// Dong bang khung hinh cuoi: thang xam + scanline (kieu VirtualBox pause). true neu da ap dung.
fn freeze_frame() -> bool {
    let fb = match crate::video::saved() {
        Some(f) => f,
        None => return false,
    };
    if fb.bpp != 32 {
        return false;
    }
    let (rs, gs, bs) = (fb.red_shift as u32, fb.green_shift as u32, fb.blue_shift as u32);
    for y in 0..fb.height {
        let row = unsafe { (fb.addr as *mut u8).add(y * fb.pitch) as *mut u32 };
        let dark = y & 1 == 0; // dong chan toi hon -> scanline
        for x in 0..fb.width {
            unsafe {
                let p = row.add(x);
                let v = core::ptr::read_volatile(p);
                let r = (v >> rs) & 255;
                let g = (v >> gs) & 255;
                let b = (v >> bs) & 255;
                let mut l = (r * 77 + g * 150 + b * 29) >> 8; // luma
                l = if dark { l * 5 / 10 } else { l * 8 / 10 };
                core::ptr::write_volatile(p, (l << rs) | (l << gs) | (l << bs));
            }
        }
    }
    true
}
