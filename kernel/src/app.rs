//! App loader: nap ELF64 vao address space rieng, chay o ring 3 (iretq).
//! App goi kernel bang SYSCALL theo Linux x86_64 ABI (xem linux.rs).
//! Moi lan chi 1 app; khi app exit hoac bi exception thi `run` tra ve.
use crate::linux;
use crate::paging::{self, Space};
use crate::{pmm, vfs};
use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::ptr::read_unaligned;

const PAGE: u64 = 4096;
/// Load bias cho ELF ET_DYN (PIE). ET_EXEC dung thang vaddr trong file.
const USER_BASE: u64 = 0x40_0000;
const STACK_TOP: u64 = 0x0000_7FFF_FFFF_F000;
const STACK_PAGES: u64 = 128; // 512 KiB

const AT_NULL: u64 = 0;
const AT_PHDR: u64 = 3;
const AT_PHENT: u64 = 4;
const AT_PHNUM: u64 = 5;
const AT_PAGESZ: u64 = 6;
const AT_ENTRY: u64 = 9;
const AT_SECURE: u64 = 23;
const AT_RANDOM: u64 = 25;

#[allow(dead_code)]
#[repr(C)]
struct Ehdr {
    ident: [u8; 16],
    e_type: u16,
    machine: u16,
    version: u32,
    entry: u64,
    phoff: u64,
    shoff: u64,
    flags: u32,
    ehsize: u16,
    phentsize: u16,
    phnum: u16,
    shentsize: u16,
    shnum: u16,
    shstrndx: u16,
}

#[allow(dead_code)]
#[repr(C)]
struct Phdr {
    p_type: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
}

struct Loaded {
    entry: u64,
    phdr: u64,
    phnum: u64,
    brk: u64,
    bias: u64,
    dyn_va: Option<u64>,
}

#[no_mangle]
static mut USER_RSP_TMP: u64 = 0;
#[no_mangle]
pub static mut CUR_KSTACK_TOP: u64 = 0;

// Frame tren kernel stack (dia chi tang dan): r10 r9 r8 rdx rsi rdi r15 r14 r13 r12 rbp rbx r11 rcx user_rsp
// GDT: 0x08 kcode, 0x10 kdata, 0x18 udata (0x1B), 0x20 ucode (0x23).
global_asm!(
    r#"
.global syscall_entry
syscall_entry:
    mov [rip + USER_RSP_TMP], rsp
    mov rsp, [rip + CUR_KSTACK_TOP]
    push qword ptr [rip + USER_RSP_TMP]
    push rcx
    push r11
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    push rdi
    push rsi
    push rdx
    push r8
    push r9
    push r10
    sub rsp, 8
    mov r9, r8
    mov r8, r10
    mov rcx, rdx
    mov rdx, rsi
    mov rsi, rdi
    mov rdi, rax
    cld
    call syscall_dispatch
    jmp ret_to_user

.global ret_to_user
ret_to_user:
    add rsp, 8
    pop r10
    pop r9
    pop r8
    pop rdx
    pop rsi
    pop rdi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    pop r11
    pop rcx
    pop rsp
    sysretq
"#
);

extern "C" {
    fn syscall_entry();
}

/// Bat SSE cho user + SYSCALL/SYSRET. Goi sau paging::init.
pub unsafe fn init() {
    // musl/gcc sinh lenh SSE: CR0.EM=0, CR0.MP=1, CR4.OSFXSR | OSXMMEXCPT
    asm!(
        "mov {t}, cr0",
        "and {t}, -5",
        "or {t}, 2",
        "mov cr0, {t}",
        "mov {t}, cr4",
        "or {t}, 0x600",
        "mov cr4, {t}",
        "fninit",
        t = out(reg) _,
        options(nostack),
    );
    let mxcsr: u32 = 0x1F80;
    asm!("ldmxcsr [{}]", in(reg) &mxcsr, options(nostack, readonly));

    paging::wrmsr(0xC000_0080, paging::rdmsr(0xC000_0080) | 1 | (1 << 11)); // EFER.SCE + NXE
    paging::wrmsr(0xC000_0081, (0x10u64 << 48) | (0x08u64 << 32)); // STAR
    paging::wrmsr(0xC000_0082, syscall_entry as usize as u64); // LSTAR
    paging::wrmsr(0xC000_0084, 0x700); // SFMASK: xoa TF | IF | DF
    crate::task::init();
}

/// exit/exit_group: ket thuc process hien tai.
pub fn exit_to_kernel(code: u64) -> ! {
    crate::task::exit_current((code & 0xff) as i64)
}
 
/// exit (syscall 60): chi ket thuc thread hien tai; thread cuoi cung thi thoat ca process.
pub fn exit_thread(code: u64) -> ! {
    crate::task::exit_thread((code & 0xff) as i64)
}

/// Goi tu exception_handler khi exception xay ra o ring 3: giet process hien tai.
pub fn fault(regs: &crate::idt::Regs) -> ! {
    let cr2: u64;
    unsafe { asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)) };
    crate::kprintln!(
        "[app] killed by exception: vector={} error={:#x} rip={:#x} cr2={:#x}",
        regs.vector,
        regs.error,
        regs.rip,
        cr2
    );
    crate::task::exit_current(-(regs.vector as i64) - 1)
}

#[no_mangle]
extern "C" fn syscall_dispatch(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    unsafe { linux::dispatch(nr, a1, a2, a3, a4, a5) }
}

fn read_exact(path: &str, off: u64, buf: &mut [u8]) -> bool {
    vfs::read(path, off, buf) == buf.len()
}

/// Nap 1 PT_LOAD, tra ve dia chi ket thuc (user) cua segment.
unsafe fn load_segment(path: &str, sp: &Space, ph: &Phdr, bias: u64) -> Result<u64, &'static str> {
    if ph.filesz > ph.memsz {
        return Err("bad segment size");
    }
    let start = bias.checked_add(ph.vaddr).ok_or("bad vaddr")?;
    let end = start.checked_add(ph.memsz).ok_or("bad vaddr")?;
    if start < 0x1000 || end > STACK_TOP - STACK_PAGES * PAGE {
        return Err("segment outside user range");
    }
    let (w, x) = (ph.flags & 2 != 0, ph.flags & 1 != 0);
    let mut pg = start & !0xfff;
    while pg < end {
        paging::ensure_user_page(sp, pg, w, x).ok_or("out of memory")?;
        pg += PAGE;
    }
    // chep phan co trong file thang vao frame (qua HHDM); phan con lai (.bss) da la 0
    let mut done = 0u64;
    while done < ph.filesz {
        let va = start + done;
        let n = (ph.filesz - done).min(PAGE - (va & 0xfff));
        let (pa, _) = paging::translate(sp, va).ok_or("internal: page missing")?;
        let dst = core::slice::from_raw_parts_mut(pmm::virt(pa), n as usize);
        if !read_exact(path, ph.offset + done, dst) {
            return Err("cannot read segment");
        }
        done += n;
    }
    Ok(end)
}

unsafe fn rd64(sp: &Space, va: u64) -> Option<u64> {
    let mut b = [0u8; 8];
    if paging::copy_from_user(sp, va, &mut b) {
        Some(u64::from_le_bytes(b))
    } else {
        None
    }
}

/// Ap dung R_X86_64_RELATIVE cho ELF ET_DYN.
unsafe fn relocate(sp: &Space, dyn_va: u64, bias: u64) -> Result<(), &'static str> {
    let (mut rela, mut relasz) = (0u64, 0u64);
    let mut k = 0u64;
    loop {
        if k > 512 {
            return Err("dynamic section too long");
        }
        let tag = rd64(sp, dyn_va + k * 16).ok_or("bad dynamic")?;
        let val = rd64(sp, dyn_va + k * 16 + 8).ok_or("bad dynamic")?;
        match tag {
            0 => break,
            7 => rela = val,
            8 => relasz = val,
            _ => {}
        }
        k += 1;
    }
    for i in 0..relasz / 24 {
        let at = bias + rela + i * 24;
        let off = rd64(sp, at).ok_or("bad rela")?;
        let info = rd64(sp, at + 8).ok_or("bad rela")?;
        let add = rd64(sp, at + 16).ok_or("bad rela")? as i64;
        match info & 0xffff_ffff {
            0 => {}
            8 => {
                let v = (bias as i64).wrapping_add(add) as u64;
                if !paging::copy_to_user(sp, bias + off, &v.to_le_bytes(), false) {
                    return Err("bad relocation target");
                }
            }
            _ => return Err("unsupported relocation (only R_X86_64_RELATIVE)"),
        }
    }
    Ok(())
}

/// Nap ELF vao `sp`.
unsafe fn load_elf(path: &str, sp: &Space, bias_hint: Option<u64>) -> Result<Loaded, &'static str> {
    let mut hb = [0u8; size_of::<Ehdr>()];
    if !read_exact(path, 0, &mut hb) {
        return Err("cannot read file");
    }
    if &hb[0..4] != b"\x7fELF" {
        return Err("not an ELF");
    }
    let eh = read_unaligned(hb.as_ptr() as *const Ehdr);
    if eh.ident[4] != 2 || eh.ident[5] != 1 || eh.machine != 0x3E || (eh.e_type != 2 && eh.e_type != 3) {
        return Err("need x86_64 ELF64 (ET_EXEC or ET_DYN)");
    }
    if (eh.phentsize as usize) < size_of::<Phdr>() || eh.phnum == 0 || eh.phnum > 32 {
        return Err("bad program headers");
    }
    let bias = bias_hint.unwrap_or(if eh.e_type == 3 { USER_BASE } else { 0 });
    let mut dyn_va = None;
    let mut brk = 0u64;
    let mut phdr = 0u64;
    for i in 0..eh.phnum as u64 {
        let mut pb = [0u8; size_of::<Phdr>()];
        if !read_exact(path, eh.phoff + i * eh.phentsize as u64, &mut pb) {
            return Err("cannot read program header");
        }
        let ph = read_unaligned(pb.as_ptr() as *const Phdr);
        match ph.p_type {
            1 => {
                let end = load_segment(path, sp, &ph, bias)?;
                brk = brk.max(end);
                // program header table nam trong segment nao thi AT_PHDR tro vao do
                if ph.offset <= eh.phoff && eh.phoff < ph.offset + ph.filesz {
                    phdr = bias + ph.vaddr + (eh.phoff - ph.offset);
                }
            }
            2 => dyn_va = Some(bias + ph.vaddr),
            _ => {}
        }
    }
    let entry = bias.checked_add(eh.entry).ok_or("bad entry")?;
    if entry < 0x1000 || entry >= STACK_TOP {
        return Err("bad entry");
    }
    Ok(Loaded { entry, phdr, phnum: eh.phnum as u64, brk: (brk + PAGE - 1) & !(PAGE - 1), bias, dyn_va })
}

unsafe fn push_str(sp: &Space, top: &mut u64, s: &[u8]) -> Option<u64> {
    *top = top.checked_sub(s.len() as u64 + 1)?;
    if !paging::copy_to_user(sp, *top, s, false) || !paging::copy_to_user(sp, *top + s.len() as u64, &[0], false) {
        return None;
    }
    Some(*top)
}

/// Dung stack ban dau kieu Linux: argc, argv[], NULL, envp[], NULL, auxv[]. Tra ve rsp.
unsafe fn build_stack(sp: &Space, argv: &[&str], ld: &Loaded) -> Option<u64> {
    const ENV: [&str; 3] = ["PATH=/apps:/bin", "HOME=/", "TERM=linux"];
    if argv.is_empty() || argv.len() > 16 {
        return None;
    }
    let mut top = STACK_TOP;

    // 16 byte ngau nhien cho AT_RANDOM
    let t = linux::rdtsc();
    let mut rnd = [0u8; 16];
    for (i, b) in rnd.iter_mut().enumerate() {
        *b = ((t >> ((i % 8) * 8)) as u8) ^ (i as u8).wrapping_mul(0x9d);
    }
    top -= 16;
    if !paging::copy_to_user(sp, top, &rnd, false) {
        return None;
    }
    let rnd_addr = top;

    let mut av = [0u64; 16];
    for (i, s) in argv.iter().enumerate() {
        av[i] = push_str(sp, &mut top, s.as_bytes())?;
    }
    let mut ev = [0u64; 3];
    for (i, s) in ENV.iter().enumerate() {
        ev[i] = push_str(sp, &mut top, s.as_bytes())?;
    }
    top &= !15;

    let aux: [(u64, u64); 8] = [
        (AT_PHDR, ld.phdr),
        (AT_PHENT, size_of::<Phdr>() as u64),
        (AT_PHNUM, ld.phnum),
        (AT_PAGESZ, PAGE),
        (AT_ENTRY, ld.entry),
        (AT_SECURE, 0),
        (AT_RANDOM, rnd_addr),
        (AT_NULL, 0),
    ];
    let argc = argv.len() as u64;
    let nq = 1 + (argc + 1) + (ENV.len() as u64 + 1) + 2 * aux.len() as u64;
    let rsp = (top - nq * 8) & !15; // rsp tro vao argc, can 16

    let mut p = rsp;
    let mut put = |v: u64| -> Option<()> {
        if paging::copy_to_user(sp, p, &v.to_le_bytes(), false) {
            p += 8;
            Some(())
        } else {
            None
        }
    };
    put(argc)?;
    for i in 0..argv.len() {
        put(av[i])?;
    }
    put(0)?;
    for i in 0..ENV.len() {
        put(ev[i])?;
    }
    put(0)?;
    for (k, v) in aux.iter() {
        put(*k)?;
        put(*v)?;
    }
    Some(rsp)
}

/// Tra ve (entry, rsp ban dau, brk ban dau).
unsafe fn build(path: &str, sp: &Space, argv: &[&str]) -> Result<(u64, u64, u64), &'static str> {
    let ld = load_elf(path, sp, None)?;
    link(sp, &ld)?;
    for i in 0..STACK_PAGES {
        paging::ensure_user_page(sp, STACK_TOP - (i + 1) * PAGE, true, false).ok_or("out of memory")?;
    }
    let rsp = build_stack(sp, argv, &ld).ok_or("cannot build stack")?;
    Ok((ld.entry, rsp, ld.brk))
}

/// Dung address space + stack cho `path`; chua chay.
pub unsafe fn prepare(path: &str, argv: &[&str]) -> Result<(Space, u64, u64, u64), &'static str> {
    let sp = paging::new_space().ok_or("out of memory")?;
    match build(path, &sp, argv) {
        Ok((e, r, b)) => Ok((sp, e, r, b)),
        Err(e) => {
            paging::destroy(sp);
            Err(e)
        }
    }
}

/// Chay app (kernel main cho den khi no thoat). Ok(code): >=0 ma exit, <0 = bi exception (vector = -code-1).
pub fn run(path: &str, argv: &[&str]) -> Result<i64, &'static str> {
    unsafe {
        let (sp, entry, rsp, brk) = prepare(path, argv)?;
        match crate::task::spawn(sp, entry, rsp, brk) {
            Some(pid) => Ok(crate::task::wait_child(pid)),
            None => {
                paging::destroy(sp);
                Err("process table full")
            }
        }
    }
}

// ================= dynamic linker (user-space shared libraries) =================
const LIB_BASE: u64 = 0x1000_0000;
const LIB_STRIDE: u64 = 0x100_0000;
const MAX_OBJ: usize = 8;

#[derive(Clone, Copy)]
struct Obj {
    bias: u64,
    symtab: u64,
    strtab: u64,
    nsyms: u64,
    rela: u64,
    relasz: u64,
    jmprel: u64,
    pltrelsz: u64,
}

impl Obj {
    const ZERO: Obj = Obj { bias: 0, symtab: 0, strtab: 0, nsyms: 0, rela: 0, relasz: 0, jmprel: 0, pltrelsz: 0 };
}

unsafe fn read_cstr(sp: &Space, va: u64, out: &mut [u8]) -> Option<usize> {
    for i in 0..out.len() {
        let mut b = [0u8; 1];
        if !paging::copy_from_user(sp, va + i as u64, &mut b) {
            return None;
        }
        if b[0] == 0 {
            return Some(i);
        }
        out[i] = b[0];
    }
    None
}

unsafe fn parse_dyn(sp: &Space, dyn_va: u64, bias: u64) -> Result<(Obj, [u64; 8], usize), &'static str> {
    let mut o = Obj::ZERO;
    o.bias = bias;
    let mut need = [0u64; 8];
    let mut nn = 0;
    let mut k = 0u64;
    loop {
        if k > 512 {
            return Err("dynamic section too long");
        }
        let tag = rd64(sp, dyn_va + k * 16).ok_or("bad dynamic")?;
        let val = rd64(sp, dyn_va + k * 16 + 8).ok_or("bad dynamic")?;
        match tag {
            0 => break,
            1 => {
                if nn < 8 {
                    need[nn] = val;
                    nn += 1;
                }
            }
            2 => o.pltrelsz = val,
            5 => o.strtab = bias + val,
            6 => o.symtab = bias + val,
            7 => o.rela = val,
            8 => o.relasz = val,
            23 => o.jmprel = val,
            _ => {}
        }
        k += 1;
    }
    if o.strtab > o.symtab && o.symtab != 0 {
        o.nsyms = (o.strtab - o.symtab) / 24;
    }
    Ok((o, need, nn))
}

/// (name_off, info, shndx, value)
unsafe fn rd_sym(sp: &Space, o: &Obj, i: u64) -> Option<(u32, u8, u16, u64)> {
    let mut b = [0u8; 24];
    if !paging::copy_from_user(sp, o.symtab + i * 24, &mut b) {
        return None;
    }
    Some((
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        b[4],
        u16::from_le_bytes([b[6], b[7]]),
        u64::from_le_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]),
    ))
}

unsafe fn lookup(sp: &Space, objs: &[Obj], name: &[u8]) -> Option<u64> {
    let mut buf = [0u8; 64];
    for o in objs {
        for i in 1..o.nsyms {
            let (no, info, shndx, val) = match rd_sym(sp, o, i) {
                Some(x) => x,
                None => continue,
            };
            let bind = info >> 4;
            if shndx == 0 || (bind != 1 && bind != 2) {
                continue;
            }
            if let Some(l) = read_cstr(sp, o.strtab + no as u64, &mut buf) {
                if &buf[..l] == name {
                    return Some(o.bias + val);
                }
            }
        }
    }
    None
}

unsafe fn resolve_sym(sp: &Space, objs: &[Obj], o: &Obj, si: u64) -> Result<u64, &'static str> {
    let (no, info, _, _) = rd_sym(sp, o, si).ok_or("bad symbol index")?;
    let mut buf = [0u8; 64];
    let l = read_cstr(sp, o.strtab + no as u64, &mut buf).ok_or("symbol name too long")?;
    if let Some(v) = lookup(sp, objs, &buf[..l]) {
        return Ok(v);
    }
    if info >> 4 == 2 {
        return Ok(0); // weak, undefined
    }
    crate::kprintln!("[ld] undefined symbol: {}", core::str::from_utf8(&buf[..l]).unwrap_or("?"));
    Err("undefined symbol")
}

unsafe fn reloc_table(sp: &Space, objs: &[Obj], o: &Obj, tab: u64, sz: u64) -> Result<(), &'static str> {
    if tab == 0 {
        return Ok(());
    }
    for i in 0..sz / 24 {
        let at = o.bias + tab + i * 24;
        let off = rd64(sp, at).ok_or("bad rela")?;
        let info = rd64(sp, at + 8).ok_or("bad rela")?;
        let add = rd64(sp, at + 16).ok_or("bad rela")? as i64;
        let ty = info & 0xffff_ffff;
        let si = info >> 32;
        let val = match ty {
            0 => continue,
            8 => (o.bias as i64).wrapping_add(add) as u64,
            1 | 6 | 7 => {
                let sv = if si == 0 { 0 } else { resolve_sym(sp, objs, o, si)? };
                if ty == 1 { (sv as i64).wrapping_add(add) as u64 } else { sv }
            }
            _ => return Err("unsupported relocation type"),
        };
        if !paging::copy_to_user(sp, o.bias + off, &val.to_le_bytes(), false) {
            return Err("bad relocation target");
        }
    }
    Ok(())
}

/// Nap thu vien DT_NEEDED (rd0://lib/<ten>) va relocate toan bo.
unsafe fn link(sp: &Space, ld: &Loaded) -> Result<(), &'static str> {
    let d = match ld.dyn_va {
        Some(d) => d,
        None => return Ok(()), // app link tinh
    };
    let mut objs = [Obj::ZERO; MAX_OBJ];
    let mut needs = [[0u64; 8]; MAX_OBJ];
    let mut nn = [0usize; MAX_OBJ];
    let mut names = [[0u8; 32]; MAX_OBJ];
    let mut nlen = [0usize; MAX_OBJ];

    let (o, nd, nc) = parse_dyn(sp, d, ld.bias)?;
    objs[0] = o;
    needs[0] = nd;
    nn[0] = nc;
    let mut count = 1usize;
    let mut i = 0usize;
    while i < count {
        for k in 0..nn[i] {
            let mut nm = [0u8; 32];
            let l = read_cstr(sp, objs[i].strtab + needs[i][k], &mut nm).ok_or("bad DT_NEEDED")?;
            let mut found = false;
            for j in 1..count {
                if nlen[j] == l && names[j][..l] == nm[..l] {
                    found = true;
                    break;
                }
            }
            if found {
                continue;
            }
            if count >= MAX_OBJ {
                return Err("too many libraries");
            }
            let mut path = [0u8; 48];
            let pre = b"rd0://lib/";
            path[..pre.len()].copy_from_slice(pre);
            path[pre.len()..pre.len() + l].copy_from_slice(&nm[..l]);
            let ps = core::str::from_utf8(&path[..pre.len() + l]).map_err(|_| "bad library name")?;
            let lb = LIB_BASE + (count as u64 - 1) * LIB_STRIDE;
            let lo = match load_elf(ps, sp, Some(lb)) {
                Ok(x) => x,
                Err(e) => {
                    crate::kprintln!("[ld] cannot load {}: {}", ps, e);
                    return Err("library load failed");
                }
            };
            let ldyn = lo.dyn_va.ok_or("library has no dynamic section")?;
            let (o2, n2, c2) = parse_dyn(sp, ldyn, lo.bias)?;
            objs[count] = o2;
            needs[count] = n2;
            nn[count] = c2;
            names[count] = nm;
            nlen[count] = l;
            count += 1;
        }
        i += 1;
    }
    for j in 0..count {
        let o = objs[j];
        reloc_table(sp, &objs[..count], &o, o.rela, o.relasz)?;
        reloc_table(sp, &objs[..count], &o, o.jmprel, o.pltrelsz)?;
    }
    Ok(())
}
