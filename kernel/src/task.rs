//! Bang process/thread + scheduler hop tac. Moi thread co kernel stack rieng;
//! cho (wait4, pipe, doc phim, futex) = vong lap `yield_now()`.
//!
//! Thread group: cac thread cung 1 process chung address space + fd table + brk/mmap/cwd.
//! Trang thai chung do linux.rs giu o bien toan cuc; khi doi giua 2 thread CUNG nhom thi
//! khong save/load lai, chi doi FS base (TLS) + FPU. Ban sao "chinh thuc" cua trang thai
//! chung nam o `pst` cua slot leader (khi doi sang nhom khac thi save vao do).
use crate::paging::{self, Space};
use crate::{app, linux};
use core::arch::{asm, global_asm};
use core::ptr::{addr_of, addr_of_mut, copy_nonoverlapping, write_bytes};

pub const MAXP: usize = 48; // slot 0 = kernel main; con lai la process + thread
const KSTACK: usize = 32 * 1024;
const MSR_FS_BASE: u32 = 0xC000_0100;

// clone flags (Linux x86_64)
pub const CLONE_VM: u64 = 0x100;
pub const CLONE_FILES: u64 = 0x400;
pub const CLONE_THREAD: u64 = 0x1_0000;
const CLONE_SETTLS: u64 = 0x8_0000;
const CLONE_PARENT_SETTID: u64 = 0x10_0000;
const CLONE_CHILD_CLEARTID: u64 = 0x20_0000;
const CLONE_CHILD_SETTID: u64 = 0x0100_0000;

/// Khop dung thu tu push trong syscall_entry (app.rs).
#[repr(C)]
pub struct Frame {
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub r11: u64,
    pub rcx: u64,
    pub rsp: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Free,
    Ready,
    Sleep,
    Zombie,
    /// Leader da thoat (SYS_exit) nhung nhom con thread song: giu slot vi `pst` chung nam o day.
    Held,
}

#[repr(C, align(16))]
struct Fx([u8; 512]);

struct Proc {
    fx: Fx,
    state: St,
    pid: i64,  // tid (= tgid voi leader)
    tgid: i64, // pid cua process (thread group id)
    leader: usize, // slot cua leader trong nhom
    nthr: usize,   // (chi o slot leader) so thread con song cua nhom
    ppid: i64,     // thread phu: -1
    code: i64,
    kwait: bool,
    wake: u64,
    space: u64,
    ksp: u64,
    fsb: u64,       // FS base (TLS) cua rieng thread nay
    clear_tid: u64, // CLONE_CHILD_CLEARTID / set_tid_address
    fkey: u64,      // dia chi futex dang cho (0 = khong cho)
    pst: linux::PState, // chi dung o slot leader
}

#[repr(C, align(16))]
struct KS([u8; KSTACK]);

static mut KSTACKS: [KS; MAXP] = [const { KS([0; KSTACK]) }; MAXP];
static mut PROCS: [Proc; MAXP] = [const {
    Proc {
        fx: Fx([0; 512]),
        state: St::Free,
        pid: 0,
        tgid: 0,
        leader: 0,
        nthr: 0,
        ppid: 0,
        code: 0,
        kwait: false,
        wake: 0,
        space: 0,
        ksp: 0,
        fsb: 0,
        clear_tid: 0,
        fkey: 0,
        pst: linux::PState::EMPTY,
    }
}; MAXP];

static mut CUR: usize = 0;
static mut NEXT_PID: i64 = 1;
static mut INITED: bool = false;

global_asm!(
    r#"
.global ctx_switch
ctx_switch:
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    mov [rdi], rsp
    mov rsp, rsi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    ret

.global child_start
child_start:
    xor eax, eax
    jmp ret_to_user
"#
);

extern "C" {
    fn ctx_switch(old: *mut u64, new: u64);
    fn child_start();
}

fn p(i: usize) -> &'static mut Proc {
    unsafe { &mut *addr_of_mut!(PROCS[i]) }
}

fn ktop(i: usize) -> u64 {
    unsafe { addr_of!(KSTACKS[i]) as u64 + KSTACK as u64 }
}

pub fn init() {
    unsafe {
        let q = p(0);
        q.state = St::Ready;
        q.pid = 0;
        q.tgid = 0;
        q.leader = 0;
        q.nthr = 1;
        q.ppid = 0;
        q.kwait = false;
        CUR = 0;
        INITED = true;
    }
}

/// pid cua process hien tai (= thread group id, giong getpid()).
pub fn pid() -> i64 {
    unsafe { p(CUR).tgid }
}

/// tid cua thread hien tai (gettid()).
pub fn tid() -> i64 {
    unsafe { p(CUR).pid }
}

pub fn ppid() -> i64 {
    unsafe { p(p(CUR).leader).ppid }
}

/// Frame user cua syscall dang chay (tren kernel stack cua thread hien tai).
pub unsafe fn frame() -> *mut Frame {
    (app::CUR_KSTACK_TOP - 120) as *mut Frame
}

fn alloc_slot() -> Option<usize> {
    (1..MAXP).find(|&i| p(i).state == St::Free)
}

fn next_ready(cur: usize) -> Option<usize> {
    let now = crate::time::uptime_ms();
    for k in 1..MAXP {
        let i = (cur + k) % MAXP;
        let q = p(i);
        if q.state == St::Sleep && now >= q.wake {
            q.state = St::Ready;
        }
        if q.state == St::Ready {
            return Some(i);
        }
    }
    None
}

unsafe fn fxsave(q: *mut u8) {
    asm!("fxsave [{}]", in(reg) q, options(nostack));
}

unsafe fn fxrstor(q: *const u8) {
    asm!("fxrstor [{}]", in(reg) q, options(nostack));
}

unsafe fn switch_to(next: usize) {
    let cur = CUR;
    if cur == next {
        return;
    }
    fxsave(p(cur).fx.0.as_mut_ptr());
    p(cur).fsb = paging::rdmsr(MSR_FS_BASE);
    let (gc, gn) = (p(cur).leader, p(next).leader);
    // cung nhom: fd/brk/mmap/cwd da nam san trong bien toan cuc, khong dung toi
    if gc != gn {
        linux::save_state(&mut p(gc).pst);
    }
    CUR = next;
    if gc != gn {
        linux::load_state(&p(gn).pst);
    }
    paging::wrmsr(MSR_FS_BASE, p(next).fsb); // TLS cua rieng thread
    fxrstor(p(next).fx.0.as_ptr());
    app::CUR_KSTACK_TOP = ktop(next);
    crate::idt::set_rsp0(ktop(next));
    let cr3 = if next == 0 { paging::kernel_cr3() } else { p(next).space };
    if cr3 != paging::read_cr3() {
        paging::write_cr3(cr3);
    }
    ctx_switch(&mut p(cur).ksp, p(next).ksp);
}

pub fn yield_now() {
    unsafe {
        if !INITED {
            return;
        }
        if let Some(n) = next_ready(CUR) {
            switch_to(n);
        }
    }
}

/// Dung kernel stack cho thread moi: [6 slot callee-saved][ret=child_start][pad][frame 120B].
unsafe fn setup_kstack(slot: usize) {
    let top = ktop(slot);
    *((top - 128) as *mut u64) = 0;
    *((top - 136) as *mut u64) = child_start as *const () as usize as u64;
    for k in 0..6u64 {
        *((top - 184 + k * 8) as *mut u64) = 0;
    }
    p(slot).ksp = top - 184;
}

/// Khoi tao cac truong chung cho 1 slot moi (leader cua nhom moi).
unsafe fn init_leader(slot: usize, parent: i64) {
    let q = p(slot);
    q.pid = NEXT_PID;
    NEXT_PID += 1;
    q.tgid = q.pid;
    q.leader = slot;
    q.nthr = 1;
    q.ppid = parent;
    q.code = 0;
    q.wake = 0;
    q.clear_tid = 0;
    q.fkey = 0;
}

/// Tao process moi (dung boi app::run, cha = kernel main).
pub unsafe fn spawn(sp: Space, entry: u64, rsp: u64, brk: u64) -> Option<i64> {
    let slot = alloc_slot()?;
    let parent = pid();
    init_leader(slot, parent);
    let q = p(slot);
    q.kwait = parent == 0;
    q.space = sp.pml4;
    q.fsb = 0;
    linux::init_fresh(&mut q.pst, sp.pml4, brk);
    write_bytes(q.fx.0.as_mut_ptr(), 0, 512);
    q.fx.0[0] = 0x7F;
    q.fx.0[1] = 0x03; // FCW = 0x037F
    q.fx.0[24] = 0x80;
    q.fx.0[25] = 0x1F; // MXCSR = 0x1F80
    let f = (ktop(slot) - 120) as *mut Frame;
    write_bytes(f, 0, 1);
    (*f).rcx = entry;
    (*f).r11 = 0x202;
    (*f).rsp = rsp;
    setup_kstack(slot);
    q.state = St::Ready;
    Some(q.pid)
}

/// fork/vfork/clone khong CLONE_THREAD: copy address space, child tra ve 0.
/// Chi thread goi fork duoc nhan ban (giong Linux).
pub unsafe fn fork(child_stack: u64) -> i64 {
    let slot = match alloc_slot() {
        Some(s) => s,
        None => return -11, // EAGAIN
    };
    let cur = CUR;
    let sp = Space { pml4: p(cur).space };
    let nsp = match paging::clone_space(&sp) {
        Some(s) => s,
        None => return -12, // ENOMEM
    };
    let parent_pid = p(cur).tgid;
    init_leader(slot, parent_pid);
    let q = p(slot);
    q.kwait = false;
    q.space = nsp.pml4;
    q.fsb = paging::rdmsr(MSR_FS_BASE);
    linux::save_state(&mut q.pst);
    linux::after_fork(&mut q.pst, nsp.pml4);
    fxsave(q.fx.0.as_mut_ptr());
    let cf = (ktop(slot) - 120) as *mut u8;
    copy_nonoverlapping(frame() as *const u8, cf, 120);
    if child_stack != 0 {
        (*(cf as *mut Frame)).rsp = child_stack;
    }
    setup_kstack(slot);
    q.state = St::Ready;
    q.pid
}

/// clone(CLONE_VM | CLONE_FILES | CLONE_THREAD | ...): tao thread moi trong cung process.
/// Tra ve tid cua thread moi (thread moi chay tiep sau syscall voi rax = 0, rsp = `stack`).
pub unsafe fn clone_thread(sp: &Space, flags: u64, stack: u64, ptid: u64, ctid: u64, tls: u64) -> i64 {
    if flags & CLONE_VM == 0 || flags & CLONE_FILES == 0 || stack == 0 {
        return -22; // EINVAL
    }
    let slot = match alloc_slot() {
        Some(s) => s,
        None => return -11, // EAGAIN
    };
    let cur = CUR;
    let l = p(cur).leader;
    let tid = NEXT_PID;
    let tb = (tid as u32).to_le_bytes();
    if flags & CLONE_PARENT_SETTID != 0 && !paging::copy_to_user(sp, ptid, &tb, true) {
        return -14; // EFAULT
    }
    if flags & CLONE_CHILD_SETTID != 0 && !paging::copy_to_user(sp, ctid, &tb, true) {
        return -14;
    }
    NEXT_PID += 1;

    let q = p(slot);
    q.pid = tid;
    q.tgid = p(cur).tgid;
    q.leader = l;
    q.nthr = 0;
    q.ppid = -1; // thread phu khong phai con cua ai (wait4 bo qua)
    q.code = 0;
    q.kwait = false;
    q.wake = 0;
    q.space = p(cur).space;
    q.fsb = if flags & CLONE_SETTLS != 0 { tls } else { paging::rdmsr(MSR_FS_BASE) };
    q.clear_tid = if flags & CLONE_CHILD_CLEARTID != 0 { ctid } else { 0 };
    q.fkey = 0;
    fxsave(q.fx.0.as_mut_ptr());
    let cf = (ktop(slot) - 120) as *mut u8;
    copy_nonoverlapping(frame() as *const u8, cf, 120);
    (*(cf as *mut Frame)).rsp = stack;
    setup_kstack(slot);
    p(l).nthr += 1;
    q.state = St::Ready;
    tid
}

/// set_tid_address(): dia chi duoc ghi 0 + futex_wake khi thread thoat. Tra ve tid.
pub fn set_tid_address(addr: u64) -> i64 {
    unsafe {
        p(CUR).clear_tid = addr;
        p(CUR).pid
    }
}

/// Giet moi thread khac trong nhom `l` (tru `keep` va leader `l`).
unsafe fn reap_threads(l: usize, keep: usize) {
    for i in 1..MAXP {
        if i == keep || i == l {
            continue;
        }
        let c = p(i);
        if c.state != St::Free && c.leader == l {
            c.state = St::Free;
            c.space = 0;
            c.fkey = 0;
        }
    }
}

/// execve chi chay duoc khi thread hien tai la thread duy nhat con song cua nhom.
/// (execve tu thread phu khi con thread khac chua duoc ho tro.)
pub fn can_exec() -> bool {
    unsafe {
        let l = p(CUR).leader;
        CUR == l || p(l).nthr <= 1
    }
}

/// execve thanh cong: thay address space cua process hien tai, doi frame de nhay vao entry moi.
pub unsafe fn exec_commit(newsp: Space, entry: u64, rsp: u64, brk: u64) {
    let l = p(CUR).leader;
    reap_threads(l, CUR); // exec giet cac thread con lai
    p(l).nthr = 1;
    p(CUR).clear_tid = 0;
    let old = Space { pml4: p(CUR).space };
    p(CUR).space = newsp.pml4;
    linux::exec_reset(&newsp, brk);
    paging::write_cr3(newsp.pml4);
    paging::destroy(old);
    let f = frame();
    write_bytes(f, 0, 1);
    (*f).rcx = entry;
    (*f).r11 = 0x202;
    (*f).rsp = rsp;
}

/// exit_group / exception: ket thuc CA process (moi thread).
pub fn exit_current(code: i64) -> ! {
    unsafe {
        let cur = CUR;
        let l = p(cur).leader;
        reap_threads(l, cur);
        linux::close_all();
        paging::write_cr3(paging::kernel_cr3());
        paging::destroy(Space { pml4: p(cur).space });
        let me = p(cur).tgid;
        if crate::video::GFX_OWNER.load(core::sync::atomic::Ordering::Relaxed) == me {
            crate::video::GFX_OWNER.store(-1, core::sync::atomic::Ordering::Relaxed);
        }
        for i in 1..MAXP {
            let c = p(i);
            if i != cur && c.state != St::Free && c.ppid == me {
                c.ppid = 0; // mo coi: kernel nhan
                if c.state == St::Zombie {
                    c.state = St::Free;
                }
            }
        }
        // ket qua thoat luon ghi o slot leader (wait4 tim theo pid cua leader)
        let q = p(l);
        q.code = code;
        q.space = 0;
        q.nthr = 0;
        q.state = if q.ppid == 0 && !q.kwait { St::Free } else { St::Zombie };
        if cur != l {
            let c = p(cur);
            c.space = 0;
            c.state = St::Free;
        }
        loop {
            let n = next_ready(cur).unwrap_or(0);
            switch_to(n);
        }
    }
}

/// SYS_exit: ket thuc 1 thread. Thread cuoi cung cua nhom = thoat ca process.
pub fn exit_thread(code: i64) -> ! {
    unsafe {
        let cur = CUR;
        let l = p(cur).leader;
        let ct = p(cur).clear_tid;
        if ct != 0 {
            // pthread_join cho gia tri nay ve 0
            let sp = Space { pml4: p(cur).space };
            let _ = paging::copy_to_user(&sp, ct, &0u32.to_le_bytes(), true);
            futex_wake_key(sp.pml4, ct, 1);
        }
        if p(l).nthr <= 1 {
            exit_current(code);
        }
        p(l).nthr -= 1;
        p(cur).fkey = 0;
        // leader thoat truoc: giu slot (pst chung nam o do); thread phu: giai phong luon
        p(cur).state = if cur == l { St::Held } else { St::Free };
        loop {
            let n = next_ready(cur).unwrap_or(0);
            switch_to(n);
        }
    }
}

/// Kernel main cho process `pid` ket thuc, tra ma thoat (<0 = bi exception).
pub fn wait_child(pid: i64) -> i64 {
    loop {
        for i in 1..MAXP {
            let c = p(i);
            if c.pid == pid && c.state == St::Zombie {
                c.state = St::Free;
                return c.code;
            }
        }
        sleep(1);
    }
}

fn sig_of(code: i64) -> i32 {
    match -code - 1 {
        0 => 8,
        6 => 4,
        _ => 11,
    }
}

/// wait4 cho process user. -ECHILD neu khong co con.
pub unsafe fn wait4(sp: &Space, want: i64, status: u64, options: u64) -> i64 {
    let me = pid();
    loop {
        let mut any = false;
        for i in 1..MAXP {
            let c = p(i);
            if c.state == St::Free || c.ppid != me {
                continue;
            }
            if !(want <= 0 || want == c.pid) {
                continue;
            }
            any = true;
            if c.state == St::Zombie {
                let st: i32 = if c.code >= 0 { ((c.code & 0xff) as i32) << 8 } else { sig_of(c.code) };
                let cpid = c.pid;
                c.state = St::Free;
                if status != 0 && !paging::copy_to_user(sp, status, &st.to_le_bytes(), true) {
                    return -14;
                }
                return cpid;
            }
        }
        if !any {
            return -10; // ECHILD
        }
        if options & 1 != 0 {
            return 0; // WNOHANG
        }
        yield_now();
    }
}

/// Ngu `ms` mili giay: tien trinh khong bi chon lai cho den khi het han.
pub fn sleep(ms: u64) {
    unsafe {
        if !INITED {
            return;
        }
        let cur = CUR;
        let wake = crate::time::uptime_ms() + ms;
        p(cur).wake = wake;
        p(cur).state = St::Sleep;
        loop {
            if crate::time::uptime_ms() >= wake {
                p(cur).state = St::Ready;
                return;
            }
            match next_ready(cur) {
                Some(n) => switch_to(n),
                None => core::hint::spin_loop(),
            }
        }
    }
}

// ================= futex =================
// Khoa = (address space, dia chi user). Chi futex "private" (cung process).

fn le64(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_le_bytes(a)
}

/// Danh thuc toi da `n` thread dang cho (space, uaddr). Tra ve so thread da danh thuc.
unsafe fn futex_wake_key(space: u64, uaddr: u64, n: u64) -> i64 {
    let mut woke = 0u64;
    for i in 1..MAXP {
        if woke >= n {
            break;
        }
        let q = p(i);
        if q.state == St::Sleep && q.fkey == uaddr && q.space == space {
            q.fkey = 0;
            q.state = St::Ready;
            woke += 1;
        }
    }
    woke as i64
}

/// futex(uaddr, op, val, timeout|val2, uaddr2). Ho tro WAIT, WAKE, REQUEUE (+ bien the BITSET don gian).
pub unsafe fn futex(sp: &Space, uaddr: u64, op: u64, val: u64, a4: u64, uaddr2: u64) -> i64 {
    if uaddr & 3 != 0 {
        return -22; // EINVAL
    }
    let cmd = op & 0x7f; // bo FUTEX_PRIVATE_FLAG / CLOCK_REALTIME
    match cmd {
        0 | 9 => {
            // FUTEX_WAIT / FUTEX_WAIT_BITSET (bitset: chi ho tro khi khong co timeout tuyet doi)
            if cmd == 9 && a4 != 0 {
                return -38; // ENOSYS
            }
            let mut cur = [0u8; 4];
            if !paging::copy_from_user(sp, uaddr, &mut cur) {
                return -14; // EFAULT
            }
            if u32::from_le_bytes(cur) != val as u32 {
                return -11; // EAGAIN: gia tri da doi
            }
            let mut deadline = u64::MAX;
            if a4 != 0 {
                let mut b = [0u8; 16];
                if !paging::copy_from_user(sp, a4, &mut b) {
                    return -14;
                }
                let ms = le64(&b[0..8]).saturating_mul(1000).saturating_add(le64(&b[8..16]) / 1_000_000).max(1);
                deadline = crate::time::uptime_ms().saturating_add(ms);
            }
            let me = CUR;
            p(me).fkey = uaddr;
            p(me).wake = deadline;
            p(me).state = St::Sleep;
            loop {
                if p(me).fkey == 0 {
                    return 0; // bi futex_wake danh thuc
                }
                if crate::time::uptime_ms() >= deadline {
                    p(me).fkey = 0;
                    p(me).state = St::Ready;
                    return -110; // ETIMEDOUT
                }
                match next_ready(me) {
                    Some(n) => switch_to(n),
                    None => core::hint::spin_loop(),
                }
            }
        }
        1 | 10 => futex_wake_key(sp.pml4, uaddr, val as u32 as u64), // FUTEX_WAKE / WAKE_BITSET
        3 => {
            // FUTEX_REQUEUE: danh thuc `val`, chuyen toi da `a4` thread sang uaddr2
            let woke = futex_wake_key(sp.pml4, uaddr, val as u32 as u64);
            let max = a4 as u32 as u64;
            let mut moved = 0u64;
            for i in 1..MAXP {
                if moved >= max {
                    break;
                }
                let q = p(i);
                if q.state == St::Sleep && q.fkey == uaddr && q.space == sp.pml4 {
                    q.fkey = uaddr2;
                    moved += 1;
                }
            }
            woke + moved as i64
        }
        _ => -38, // ENOSYS
    }
}