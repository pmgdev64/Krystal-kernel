//! GDT + TSS + IDT. Exception nào cũng đi qua isr_common -> exception_handler.
use core::arch::{asm, global_asm};
use core::ptr::{addr_of, addr_of_mut};

/// Layout khớp đúng thứ tự push trong isr_common.
#[repr(C)]
pub struct Regs {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

global_asm!(
    r#"
.macro ISR_NOERR n
isr_stub_\n:
    push 0
    push \n
    jmp isr_common
.endm

.macro ISR_ERR n
isr_stub_\n:
    push \n
    jmp isr_common
.endm

ISR_NOERR 0
ISR_NOERR 1
ISR_NOERR 2
ISR_NOERR 3
ISR_NOERR 4
ISR_NOERR 5
ISR_NOERR 6
ISR_NOERR 7
ISR_ERR   8
ISR_NOERR 9
ISR_ERR   10
ISR_ERR   11
ISR_ERR   12
ISR_ERR   13
ISR_ERR   14
ISR_NOERR 15
ISR_NOERR 16
ISR_ERR   17
ISR_NOERR 18
ISR_NOERR 19
ISR_NOERR 20
ISR_ERR   21
ISR_NOERR 22
ISR_NOERR 23
ISR_NOERR 24
ISR_NOERR 25
ISR_NOERR 26
ISR_NOERR 27
ISR_NOERR 28
ISR_ERR   29
ISR_ERR   30
ISR_NOERR 31
ISR_NOERR 32

isr_common:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov rdi, rsp
    cld
    call exception_handler
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    add rsp, 16
    iretq

.pushsection .rodata
.global isr_table
isr_table:
    .quad isr_stub_0,  isr_stub_1,  isr_stub_2,  isr_stub_3
    .quad isr_stub_4,  isr_stub_5,  isr_stub_6,  isr_stub_7
    .quad isr_stub_8,  isr_stub_9,  isr_stub_10, isr_stub_11
    .quad isr_stub_12, isr_stub_13, isr_stub_14, isr_stub_15
    .quad isr_stub_16, isr_stub_17, isr_stub_18, isr_stub_19
    .quad isr_stub_20, isr_stub_21, isr_stub_22, isr_stub_23
    .quad isr_stub_24, isr_stub_25, isr_stub_26, isr_stub_27
    .quad isr_stub_28, isr_stub_29, isr_stub_30, isr_stub_31
    .quad isr_stub_32
.popsection
"#
);

extern "C" {
    static isr_table: [u64; 33];
}

#[no_mangle]
extern "C" fn exception_handler(regs: &mut Regs) {
    if regs.vector == 32 {
        // timer IRQ0: EOI truoc, chi preempt khi dang o ring 3
        crate::time::tick_eoi();
        if regs.cs & 3 == 3 {
            crate::task::yield_now();
        }
        return;
    }
    if regs.cs & 3 == 3 {
        crate::app::fault(regs);
    }
    if regs.vector == 3 {
        crate::kprintln!("[#BP] breakpoint at {:#x}, continuing", regs.rip);
        return;
    }
    crate::panic::exception(regs);
}

// ---------- IDT ----------

#[repr(C)]
#[derive(Clone, Copy)]
struct Entry {
    off_lo: u16,
    sel: u16,
    ist: u8,
    attr: u8,
    off_mid: u16,
    off_hi: u32,
    zero: u32,
}

impl Entry {
    const MISSING: Entry = Entry { off_lo: 0, sel: 0, ist: 0, attr: 0, off_mid: 0, off_hi: 0, zero: 0 };

    fn new(handler: u64, ist: u8) -> Entry {
        Entry {
            off_lo: handler as u16,
            sel: 0x08,
            ist,
            attr: 0x8E, // present, ring0, interrupt gate
            off_mid: (handler >> 16) as u16,
            off_hi: (handler >> 32) as u32,
            zero: 0,
        }
    }
}

static mut IDT: [Entry; 256] = [Entry::MISSING; 256];

// ---------- TSS + GDT ----------

#[repr(C, packed(4))]
struct Tss {
    _r0: u32,
    rsp0: u64,
    rsp1: u64,
    rsp2: u64,
    _r1: u64,
    ist1: u64,
    ist_rest: [u64; 6],
    _r2: u64,
    _r3: u16,
    iomap: u16,
}

static mut TSS: Tss = Tss {
    _r0: 0,
    rsp0: 0,
    rsp1: 0,
    rsp2: 0,
    _r1: 0,
    ist1: 0,
    ist_rest: [0; 6],
    _r2: 0,
    _r3: 0,
    iomap: 0,
};

const DF_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct Stack([u8; DF_SIZE]);

static mut DF_STACK: Stack = Stack([0; DF_SIZE]);

static mut RING0_STACK: Stack = Stack([0; DF_SIZE]);

static mut GDT: [u64; 7] = [0; 7];

#[repr(C, packed)]
struct DescPtr {
    limit: u16,
    base: u64,
}

pub unsafe fn init() {
    // TSS: IST1 = stack riêng cho double fault
    let tss = addr_of_mut!(TSS);
    (*tss).ist1 = addr_of!(DF_STACK) as u64 + DF_SIZE as u64;
    (*tss).rsp0 = addr_of!(RING0_STACK) as u64 + DF_SIZE as u64;
    (*tss).iomap = 104;

    let base = tss as u64;
    let limit = 103u64;
    let low = (limit & 0xFFFF)
        | ((base & 0xFF_FFFF) << 16)
        | (0x89u64 << 40)
        | (((limit >> 16) & 0xF) << 48)
        | (((base >> 24) & 0xFF) << 56);
    let high = base >> 32;

    *addr_of_mut!(GDT) = [
        0,
        0x00AF_9A00_0000_FFFF, // 0x08 kernel code
        0x00CF_9200_0000_FFFF, // 0x10 kernel data
        0x00CF_F200_0000_FFFF, // 0x18 user data (0x1B)
        0x00AF_FA00_0000_FFFF, // 0x20 user code (0x23)
        low,                   // 0x28 TSS
        high,
    ];

    let gp = DescPtr { limit: (7 * 8 - 1) as u16, base: addr_of!(GDT) as u64 };
    asm!(
        "lgdt [{gp}]",
        "push 0x08",
        "lea {t}, [rip + 2f]",
        "push {t}",
        "retfq",
        "2:",
        "mov ax, 0x10",
        "mov ds, ax",
        "mov es, ax",
        "mov ss, ax",
        "mov ax, 0x28",
        "ltr ax",
        gp = in(reg) &gp,
        t = out(reg) _,
        out("ax") _,
    );

    // IDT
    let idt = &mut *addr_of_mut!(IDT);
    for i in 0..32 {
        idt[i] = Entry::new(isr_table[i], if i == 8 { 1 } else { 0 });
    }
    idt[32] = Entry::new(isr_table[32], 0);
    let ip = DescPtr { limit: (256 * 16 - 1) as u16, base: addr_of!(IDT) as u64 };
    asm!("lidt [{}]", in(reg) &ip);
}

/// Kernel stack dung khi ngat/exception tu ring 3 = stack cua thread hien tai.
pub unsafe fn set_rsp0(v: u64) {
    (*addr_of_mut!(TSS)).rsp0 = v;
}
