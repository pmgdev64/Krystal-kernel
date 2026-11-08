//! Lop tuong thich Linux x86_64 syscall ABI, du de chay busybox static (musl).
//! Moi lan chi 1 process nen fd table / brk / mmap / cwd la trang thai toan cuc.
//! Filesystem chi doc (tarfs). Chua co: fork/exec, signal, ban phim, thoi gian that.
//! Tham so thu 6 (r9) chua duoc truyen vao.
use crate::drivers::cstr;
use crate::paging::{self, Space};
use crate::{pmm, vfs};
use core::ptr::addr_of_mut;

const ENOENT: i64 = 2;
const EBADF: i64 = 9;
const ENOMEM: i64 = 12;
const EFAULT: i64 = 14;
const ENODEV: i64 = 19;
const ENOTDIR: i64 = 20;
const EISDIR: i64 = 21;
const EINVAL: i64 = 22;
const EMFILE: i64 = 24;
const ENOTTY: i64 = 25;
const ESPIPE: i64 = 29;
const EROFS: i64 = 30;
const ERANGE: i64 = 34;
const ENOSYS: i64 = 38;

fn err(e: i64) -> u64 {
    (-e) as u64
}

fn is_err(r: u64) -> bool {
    let v = r as i64;
    v < 0 && v > -4096
}

pub fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    unsafe { core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack)) };
    ((hi as u64) << 32) | lo as u64
}

// ---------------- trang thai process ----------------

const PMAX: usize = 160;
const NFD: usize = 32;
const MMAP_BASE: u64 = 0x0000_2000_0000_0000;
const BRK_MAX: u64 = 64 << 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Free,
    Console,
    Null,
    File,
    PipeR,
    PipeW,
}

#[derive(Clone, Copy)]
struct Fd {
    kind: Kind,
    is_dir: bool,
    size: u64,
    off: u64,
    plen: usize,
    path: [u8; PMAX],
}

const FREE_FD: Fd = Fd { kind: Kind::Free, is_dir: false, size: 0, off: 0, plen: 0, path: [0; PMAX] };

static mut FDS: [Fd; NFD] = [FREE_FD; NFD];
static mut SPACE: u64 = 0;
static mut BRK_START: u64 = 0;
static mut BRK_CUR: u64 = 0;
static mut MMAP_NEXT: u64 = MMAP_BASE;
static mut CWD: [u8; 64] = [0; 64];
static mut CWD_LEN: usize = 1;

unsafe fn fds() -> &'static mut [Fd; NFD] {
    &mut *addr_of_mut!(FDS)
}

unsafe fn fd_get(fd: u64) -> Option<Fd> {
    if fd < NFD as u64 {
        let f = fds()[fd as usize];
        if f.kind != Kind::Free {
            return Some(f);
        }
    }
    None
}

unsafe fn fd_set_off(fd: u64, off: u64) {
    fds()[fd as usize].off = off;
}

/// Goi truoc khi chay app moi.
pub unsafe fn reset(sp: &Space, brk: u64) {
    SPACE = sp.pml4;
    BRK_START = brk;
    BRK_CUR = brk;
    MMAP_NEXT = MMAP_BASE;
    let f = fds();
    for s in f.iter_mut() {
        *s = FREE_FD;
    }
    for i in 0..3 {
        f[i].kind = Kind::Console;
    }
    let cwd = &mut *addr_of_mut!(CWD);
    cwd[0] = b'/';
    CWD_LEN = 1;
    paging::wrmsr(0xC000_0100, 0); // FS base
}

// ---------------- dispatch ----------------

pub unsafe fn dispatch(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    let sp = Space { pml4: SPACE };
    match nr {
        0 => sys_read(&sp, a1, a2, a3),
        1 => sys_write(&sp, a1, a2, a3),
        2 => do_open(&sp, a1, a2),
        3 => sys_close(a1),
        4 | 6 => sys_stat(&sp, a1, a2),
        5 => sys_fstat(&sp, a1, a2),
        8 => sys_lseek(a1, a2, a3),
        9 => sys_mmap(&sp, a1, a2, a3, a4, a5),
        10 => sys_mprotect(&sp, a1, a2, a3),
        11 => sys_munmap(&sp, a1, a2),
        12 => sys_brk(&sp, a1),
        13 | 14 => 0, // rt_sigaction / rt_sigprocmask
        16 => sys_ioctl(&sp, a1, a2, a3),
        20 => sys_writev(&sp, a1, a2, a3),
        21 => sys_access(&sp, a1),
        28 => 0, // madvise
        24 => { crate::task::yield_now(); 0 } // sched_yield
        39 => crate::task::pid() as u64, // getpid (= thread group id)
        186 => crate::task::tid() as u64, // gettid
        202 => crate::task::futex(&sp, a1, a2, a3, a4, a5) as u64,
        200 | 234 | 273 => 0, // tkill / tgkill / set_robust_list (chua co signal)
        40 => err(ENOSYS), // sendfile: busybox tu fallback
        35 => sys_nanosleep(&sp, a1),
        60 => crate::app::exit_thread(a1),     // exit: chi thread nay
        231 => crate::app::exit_to_kernel(a1), // exit_group: ca process
        63 => sys_uname(&sp, a1),
        72 => sys_fcntl(a1, a2, a3),
        79 => sys_getcwd(&sp, a1, a2),
        80 => sys_chdir(&sp, a1),
        89 => err(EINVAL), // readlink: chua co symlink
        102 | 104 | 107 | 108 => 0,
        110 => crate::task::ppid() as u64, // uid/gid/ppid
        62 | 109 | 112 => 0, // kill / setpgid / setsid
        7 | 271 => sys_poll(&sp, a1, a2), // poll / ppoll
        111 | 121 | 124 => 1, // getpgrp / getpgid / getsid
        32 => sys_dupfd(a1, 0),
        33 | 292 => sys_dup2(a1, a2),
        158 => sys_arch_prctl(&sp, a1, a2),
        217 => sys_getdents64(&sp, a1, a2, a3),
        218 => crate::task::set_tid_address(a1) as u64,
        228 => sys_clock_gettime(&sp, a2),
        257 => do_open(&sp, a2, a3),
        262 => sys_newfstatat(&sp, a1, a2, a3, a4),
        318 => sys_getrandom(&sp, a1, a2),
        22 | 293 => sys_pipe(&sp, a1),
        56 => {
            if a1 & crate::task::CLONE_THREAD != 0 {
                crate::task::clone_thread(&sp, a1, a2, a3, a4, a5) as u64
            } else {
                crate::task::fork(a2) as u64
            }
        }
        57 | 58 => crate::task::fork(0) as u64,
        59 => sys_execve(&sp, a1, a2),
        61 => crate::task::wait4(&sp, a1 as i32 as i64, a2, a3) as u64,
        95 => 18, // umask
        _ => {
            let _ = a5;
            crate::kprintln!("[sys] chua ho tro syscall #{}", nr);
            err(ENOSYS)
        }
    }
}

// ---------------- duong dan ----------------

struct Path {
    buf: [u8; PMAX],
    len: usize,
}

impl Path {
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

/// Chuan hoa duong dan Linux -> "<o>://a/b". "/" = rd0://, "/mnt/<o>/..." = "<o>://...".
unsafe fn make_path(user: &[u8]) -> Option<Path> {
    let cwd_all = &*addr_of_mut!(CWD);
    let cwd = &cwd_all[..CWD_LEN];
    // 1. gom cac thanh phan (xu ly . va ..) thanh duong dan tuyet doi
    let mut comps: [[u8; 48]; 20] = [[0; 48]; 20];
    let mut lens = [0usize; 20];
    let mut depth = 0usize;
    let base: &[u8] = if user.first() == Some(&b'/') { b"" } else { cwd };
    for chunk in [base, user] {
        for comp in chunk.split(|&c| c == b'/') {
            match comp {
                b"" | b"." => {}
                b".." => depth = depth.saturating_sub(1),
                c => {
                    if depth >= 20 || c.len() > 48 {
                        return None;
                    }
                    comps[depth][..c.len()].copy_from_slice(c);
                    lens[depth] = c.len();
                    depth += 1;
                }
            }
        }
    }
    // 2. anh xa: /mnt/<o>/rest -> <o>://rest ; con lai -> rd0://...
    let mut out = [0u8; PMAX];
    let mut len;
    let mut first = 0usize;
    let is_devs = depth >= 1 && (&comps[0][..lens[0]] == b"devices:" || &comps[0][..lens[0]] == b"procfs:");
    let is_devdir = depth >= 2
        && &comps[0][..lens[0]] == b"dev"
        && matches!(&comps[1][..lens[1]], b"fb0" | b"mode" | b"mouse" | b"kbd" | b"mem" | b"uptime" | b"beep" | b"audio0" | b"net0" | b"zero");
    if is_devs || is_devdir {
        out[..10].copy_from_slice(b"devices://");
        len = 10;
        first = 1;
    } else if depth >= 2 && &comps[0][..lens[0]] == b"mnt" {
        let dn = &comps[1][..lens[1]];
        if 3 + dn.len() > PMAX {
            return None;
        }
        out[..dn.len()].copy_from_slice(dn);
        out[dn.len()..dn.len() + 3].copy_from_slice(b"://");
        len = dn.len() + 3;
        first = 2;
    } else {
        out[..6].copy_from_slice(b"rd0://");
        len = 6;
    }
    for i in first..depth {
        let c = &comps[i][..lens[i]];
        if len + 1 + c.len() > PMAX {
            return None;
        }
        if i > first {
            out[len] = b'/';
            len += 1;
        }
        out[len..len + c.len()].copy_from_slice(c);
        len += c.len();
    }
    core::str::from_utf8(&out[..len]).ok()?;
    Some(Path { buf: out, len })
}

/// "/mnt" ao: thu muc chua cac o dang mount.
fn is_mnt(path: &[u8]) -> bool {
    path == b"rd0://mnt"
}

fn drive_of(s: &str) -> Option<&str> {
    let i = s.find("://")?;
    Some(&s[..i])
}

unsafe fn copy_cstr(sp: &Space, ptr: u64, out: &mut [u8]) -> Option<usize> {
    for i in 0..out.len() {
        let mut b = [0u8; 1];
        if !paging::copy_from_user(sp, ptr.wrapping_add(i as u64), &mut b) {
            return None;
        }
        if b[0] == 0 {
            return Some(i);
        }
        out[i] = b[0];
    }
    None
}

unsafe fn user_path(sp: &Space, ptr: u64) -> Result<Path, u64> {
    let mut raw = [0u8; PMAX];
    let n = copy_cstr(sp, ptr, &mut raw).ok_or(err(EFAULT))?;
    if n == 0 {
        return Err(err(ENOENT));
    }
    make_path(&raw[..n]).ok_or(err(ENOENT))
}

fn is_dev(s: &str) -> bool {
    s == "rd0://dev/null" || s == "rd0://dev/tty" || s == "rd0://dev/console"
}

/// (is_dir, size)
fn path_stat(s: &str) -> Option<(bool, u64)> {
    if is_mnt(s.as_bytes()) {
        return Some((true, 0));
    }
    if s.ends_with("://") {
        return match drive_of(s) {
            Some(d) if d == "devices" || vfs::drive_exists(d) => Some((true, 0)),
            _ => None,
        };
    }
    if let Some(st) = vfs::stat(s) {
        return Some((st.is_dir, st.size));
    }
    if vfs::readdir(s, 0).is_some() {
        return Some((true, 0));
    }
    // /bin/<ten>: applet busybox ao
    if let Some(n) = s.strip_prefix("rd0://bin/") {
        if !n.is_empty() && !n.contains('/') && vfs::stat(BUSYBOX).is_some() {
            return Some((false, 1));
        }
    }
    None
}

// ---------------- file ----------------

unsafe fn do_open(sp: &Space, path_ptr: u64, flags: u64) -> u64 {
    let p = match user_path(sp, path_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let s = p.as_str();
    let slot = match (3..NFD).find(|&i| fds()[i].kind == Kind::Free) {
        Some(i) => i,
        None => return err(EMFILE),
    };
    let mut f = FREE_FD;
    if s == "rd0://dev/null" {
        f.kind = Kind::Null;
    } else if s == "rd0://dev/tty" || s == "rd0://dev/console" {
        f.kind = Kind::Console;
    } else {
        // O_WRONLY | O_RDWR | O_CREAT | O_TRUNC: filesystem chi doc
        if !s.starts_with("devices://") && (flags & 3 != 0 || flags & 0x240 != 0) {
            return err(EROFS);
        }
        let (is_dir, size) = match path_stat(s) {
            Some(x) => x,
            None => return err(ENOENT),
        };
        f.kind = Kind::File;
        f.is_dir = is_dir;
        f.size = size;
        f.plen = p.len;
        f.path[..p.len].copy_from_slice(&p.buf[..p.len]);
    }
    fds()[slot] = f;
    slot as u64
}

unsafe fn sys_close(fd: u64) -> u64 {
    if fd_get(fd).is_none() {
        return err(EBADF);
    }
    let old = fds()[fd as usize];
    fd_unref(&old);
    fds()[fd as usize] = FREE_FD;
    0
}

unsafe fn sys_read(sp: &Space, fd: u64, buf: u64, count: u64) -> u64 {
    let f = match fd_get(fd) {
        Some(f) => f,
        None => return err(EBADF),
    };
    match f.kind {
        Kind::Null => return 0,
        Kind::Console => return crate::tty::read(sp, buf, count),
        Kind::File => {}
        Kind::PipeR => return pipe_read(sp, f.off as usize, buf, count),
        Kind::PipeW => return err(EBADF),
        Kind::Free => return err(EBADF),
    }
    if f.is_dir {
        return err(EISDIR);
    }
    let path = core::str::from_utf8(&f.path[..f.plen]).unwrap_or("");
    if path == "devices://net0" {
        // 1 lan read = 1 frame Ethernet (0 = chua co frame)
        let mut fr = [0u8; 1536];
        let cap = (count as usize).min(fr.len());
        let n = vfs::read(path, 0, &mut fr[..cap]);
        if n == 0 {
            return 0;
        }
        if !paging::copy_to_user(sp, buf, &fr[..n], true) {
            return err(EFAULT);
        }
        return n as u64;
    }
    let mut off = f.off;
    let mut done = 0u64;
    let mut chunk = [0u8; 1024];
    while done < count {
        let want = (count - done).min(chunk.len() as u64) as usize;
        let n = vfs::read(path, off, &mut chunk[..want]);
        if n == 0 {
            break;
        }
        if !paging::copy_to_user(sp, buf.wrapping_add(done), &chunk[..n], true) {
            return err(EFAULT);
        }
        off += n as u64;
        done += n as u64;
        if n < want {
            break;
        }
    }
    fd_set_off(fd, off);
    done
}

unsafe fn console_write(sp: &Space, ptr: u64, len: u64) -> u64 {
    let len = len.min(1 << 20);
    let mut buf = [0u8; 128];
    let mut done = 0u64;
    while done < len {
        let n = (len - done).min(buf.len() as u64) as usize;
        if !paging::copy_from_user(sp, ptr.wrapping_add(done), &mut buf[..n]) {
            return if done > 0 { done } else { err(EFAULT) };
        }
        // chi cho ASCII in duoc (font 8x8 chi co ASCII, va tranh UTF-8 hong)
        for b in buf[..n].iter_mut() {
            let ok = matches!(*b, b'\n' | b'\r' | b'\t' | 0x08 | 0x1b) || (0x20..0x7f).contains(&*b);
            if !ok {
                *b = b'?';
            }
        }
        emit(&buf[..n]);
        done += n as u64;
    }
    done
}

unsafe fn sys_write(sp: &Space, fd: u64, ptr: u64, len: u64) -> u64 {
    match fd_get(fd) {
        Some(f) if f.kind == Kind::Console => console_write(sp, ptr, len),
        Some(f) if f.kind == Kind::PipeW => pipe_write(sp, f.off as usize, ptr, len),
        Some(f) if f.kind == Kind::Null => len,
        Some(f) if f.kind == Kind::File => file_write(sp, fd, &f, ptr, len),
        Some(_) => err(EBADF),
        None => err(EBADF),
    }
}

unsafe fn sys_writev(sp: &Space, fd: u64, iov: u64, cnt: u64) -> u64 {
    if cnt > 16 {
        return err(EINVAL);
    }
    let mut total = 0u64;
    for i in 0..cnt {
        let mut b = [0u8; 16];
        if !paging::copy_from_user(sp, iov.wrapping_add(i * 16), &mut b) {
            return if total > 0 { total } else { err(EFAULT) };
        }
        let base = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        let len = u64::from_le_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]);
        if len == 0 {
            continue;
        }
        let r = sys_write(sp, fd, base, len);
        if is_err(r) {
            return if total > 0 { total } else { r };
        }
        total += r;
        if r < len {
            break;
        }
    }
    total
}

unsafe fn sys_lseek(fd: u64, off: u64, whence: u64) -> u64 {
    let f = match fd_get(fd) {
        Some(f) => f,
        None => return err(EBADF),
    };
    match f.kind {
        Kind::File => {}
        Kind::Null => return 0,
        _ => return err(ESPIPE),
    }
    let base = match whence {
        0 => 0i64,
        1 => f.off as i64,
        2 => f.size as i64,
        _ => return err(EINVAL),
    };
    let n = base.wrapping_add(off as i64);
    if n < 0 {
        return err(EINVAL);
    }
    fd_set_off(fd, n as u64);
    n as u64
}

// ---------------- stat / thu muc ----------------

fn hash(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &c in b {
        h ^= c as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn mode_of(is_dir: bool) -> u32 {
    if is_dir {
        0o040755
    } else {
        0o100755
    }
}

unsafe fn put_stat(sp: &Space, buf: u64, mode: u32, size: u64, ino: u64) -> u64 {
    let mut b = [0u8; 144];
    b[0..8].copy_from_slice(&1u64.to_le_bytes()); // st_dev
    b[8..16].copy_from_slice(&ino.to_le_bytes());
    b[16..24].copy_from_slice(&1u64.to_le_bytes()); // st_nlink
    b[24..28].copy_from_slice(&mode.to_le_bytes());
    b[48..56].copy_from_slice(&size.to_le_bytes());
    b[56..64].copy_from_slice(&4096u64.to_le_bytes()); // st_blksize
    b[64..72].copy_from_slice(&((size + 511) / 512).to_le_bytes()); // st_blocks
    if paging::copy_to_user(sp, buf, &b, true) {
        0
    } else {
        err(EFAULT)
    }
}

unsafe fn sys_stat(sp: &Space, path_ptr: u64, buf: u64) -> u64 {
    let p = match user_path(sp, path_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let s = p.as_str();
    if is_dev(s) {
        return put_stat(sp, buf, 0o020666, 0, 2);
    }
    match path_stat(s) {
        Some((d, size)) => put_stat(sp, buf, mode_of(d), size, hash(s.as_bytes())),
        None => err(ENOENT),
    }
}

unsafe fn sys_fstat(sp: &Space, fd: u64, buf: u64) -> u64 {
    let f = match fd_get(fd) {
        Some(f) => f,
        None => return err(EBADF),
    };
    match f.kind {
        Kind::Console => put_stat(sp, buf, 0o020620, 0, 1),
        Kind::Null => put_stat(sp, buf, 0o020666, 0, 2),
        _ => put_stat(sp, buf, mode_of(f.is_dir), f.size, hash(&f.path[..f.plen])),
    }
}

unsafe fn sys_newfstatat(sp: &Space, dirfd: u64, path_ptr: u64, buf: u64, flags: u64) -> u64 {
    if flags & 0x1000 != 0 {
        // AT_EMPTY_PATH
        let mut b = [0u8; 1];
        if paging::copy_from_user(sp, path_ptr, &mut b) && b[0] == 0 {
            return sys_fstat(sp, dirfd, buf);
        }
    }
    sys_stat(sp, path_ptr, buf)
}

unsafe fn sys_access(sp: &Space, path_ptr: u64) -> u64 {
    let p = match user_path(sp, path_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let s = p.as_str();
    if is_dev(s) || path_stat(s).is_some() {
        0
    } else {
        err(ENOENT)
    }
}

unsafe fn sys_getdents64(sp: &Space, fd: u64, dirp: u64, count: u64) -> u64 {
    let f = match fd_get(fd) {
        Some(f) if f.kind == Kind::File => f,
        _ => return err(EBADF),
    };
    if !f.is_dir {
        return err(ENOTDIR);
    }
    let path = core::str::from_utf8(&f.path[..f.plen]).unwrap_or("");
    let mut idx = f.off;
    let mut written = 0u64;
    loop {
        let e = if is_mnt(path.as_bytes()) {
            let mut nb = [0u8; 16];
            match vfs::drive_name(idx as usize, &mut nb) {
                Some(n) => {
                    let mut de = driver_abi::DirEntry::empty();
                    de.name[..n].copy_from_slice(&nb[..n]);
                    de.is_dir = true;
                    de
                }
                None => break,
            }
        } else {
            match vfs::readdir(path, idx as usize) {
                Some(e) => e,
                None => break,
            }
        };
        let name = cstr(&e.name).as_bytes();
        let reclen = (19 + name.len() + 1 + 7) & !7;
        if written + reclen as u64 > count {
            if written == 0 {
                return err(EINVAL);
            }
            break;
        }
        let mut b = [0u8; 128];
        b[0..8].copy_from_slice(&(idx + 1).to_le_bytes()); // d_ino
        b[8..16].copy_from_slice(&(idx + 1).to_le_bytes()); // d_off
        b[16..18].copy_from_slice(&(reclen as u16).to_le_bytes());
        b[18] = if e.is_dir { 4 } else { 8 }; // DT_DIR / DT_REG
        b[19..19 + name.len()].copy_from_slice(name);
        if !paging::copy_to_user(sp, dirp.wrapping_add(written), &b[..reclen], true) {
            return err(EFAULT);
        }
        written += reclen as u64;
        idx += 1;
    }
    fd_set_off(fd, idx);
    written
}

unsafe fn sys_getcwd(sp: &Space, buf: u64, size: u64) -> u64 {
    let cwd = &*addr_of_mut!(CWD);
    let n = CWD_LEN;
    if size < (n + 1) as u64 {
        return err(ERANGE);
    }
    let mut b = [0u8; 65];
    b[..n].copy_from_slice(&cwd[..n]);
    if paging::copy_to_user(sp, buf, &b[..n + 1], true) {
        (n + 1) as u64
    } else {
        err(EFAULT)
    }
}

unsafe fn sys_chdir(sp: &Space, path_ptr: u64) -> u64 {
    let p = match user_path(sp, path_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match path_stat(p.as_str()) {
        Some((true, _)) => {}
        Some(_) => return err(ENOTDIR),
        None => return err(ENOENT),
    }
    let mut lin = [0u8; 64];
    let mut ll = 0usize;
    let ps = p.as_str();
    if is_mnt(ps.as_bytes()) {
        lin[..4].copy_from_slice(b"/mnt");
        ll = 4;
    } else if let Some(i) = ps.find("://") {
        let (dr, rest0) = (&ps[..i], &ps[i + 3..]);
        if dr == "rd0" {
            lin[0] = b'/';
            ll = 1;
        } else {
            let pre = b"/mnt/";
            if pre.len() + dr.len() + 1 > 64 {
                return err(ERANGE);
            }
            lin[..5].copy_from_slice(pre);
            lin[5..5 + dr.len()].copy_from_slice(dr.as_bytes());
            ll = 5 + dr.len();
            if !rest0.is_empty() {
                lin[ll] = b'/';
                ll += 1;
            }
        }
        if ll + rest0.len() > 64 {
            return err(ERANGE);
        }
        lin[ll..ll + rest0.len()].copy_from_slice(rest0.as_bytes());
        ll += rest0.len();
    }
    let rest = &lin[..ll];
    let cwd = &mut *addr_of_mut!(CWD);
    if rest.len() + 1 > cwd.len() {
        return err(ERANGE);
    }
    cwd[..rest.len()].copy_from_slice(rest);
    CWD_LEN = rest.len();
    0
}

// ---------------- bo nho ----------------

unsafe fn sys_brk(sp: &Space, addr: u64) -> u64 {
    if addr < BRK_START || addr > BRK_START + BRK_MAX {
        return BRK_CUR; // addr = 0 la truy van
    }
    let cur_end = (BRK_CUR + 4095) & !4095;
    let new_end = (addr + 4095) & !4095;
    let mut p = cur_end;
    while p < new_end {
        if paging::ensure_user_page(sp, p, true, false).is_none() {
            return BRK_CUR;
        }
        p += 4096;
    }
    BRK_CUR = addr;
    addr
}

unsafe fn sys_mmap(sp: &Space, addr: u64, len: u64, prot: u64, flags: u64, fd: u64) -> u64 {
    if flags & 0x20 == 0 {
        return mmap_fb(sp, len, fd);
    }
    if len == 0 {
        return err(EINVAL);
    }
    if len > (1 << 32) {
        return err(ENOMEM);
    }
    let len = (len + 4095) & !4095;
    let fixed = flags & 0x10 != 0 && addr & 0xfff == 0 && addr >= 0x1000;
    let base = if fixed {
        addr // MAP_FIXED
    } else {
        let a = MMAP_NEXT;
        MMAP_NEXT += len;
        a
    };
    if base.checked_add(len).map_or(true, |e| e > paging::USER_END) {
        return err(ENOMEM);
    }
    let (w, x) = (prot & 2 != 0, prot & 4 != 0);
    let mut p = base;
    while p < base + len {
        match paging::ensure_user_page(sp, p, w, x) {
            // trang moi da duoc pmm::alloc xoa 0; chi xoa lai khi MAP_FIXED de len trang cu
            Some(pa) => {
                if fixed {
                    core::ptr::write_bytes(pmm::virt(pa), 0, 4096)
                }
            }
            None => {
                let mut q = base;
                while q < p {
                    paging::unmap_user_page(sp, q);
                    q += 4096;
                }
                paging::flush();
                return err(ENOMEM);
            }
        }
        p += 4096;
    }
    paging::flush();
    base
}

unsafe fn sys_munmap(sp: &Space, addr: u64, len: u64) -> u64 {
    if addr & 0xfff != 0 || len == 0 || len > (1 << 32) {
        return err(EINVAL);
    }
    let end = addr.saturating_add(len).min(paging::USER_END);
    let mut p = addr;
    while p < end {
        paging::unmap_user_page(sp, p);
        p += 4096;
    }
    paging::flush();
    0
}

unsafe fn sys_mprotect(sp: &Space, addr: u64, len: u64, prot: u64) -> u64 {
    if addr & 0xfff != 0 || len > (1 << 32) {
        return err(EINVAL);
    }
    let end = addr.saturating_add(len).min(paging::USER_END);
    let mut p = addr;
    while p < end {
        paging::protect_user_page(sp, p, prot & 2 != 0, prot & 4 != 0);
        p += 4096;
    }
    paging::flush();
    0
}

// ---------------- khac ----------------

unsafe fn sys_arch_prctl(sp: &Space, code: u64, addr: u64) -> u64 {
    match code {
        0x1002 => {
            // ARCH_SET_FS
            if addr >= paging::USER_END {
                return err(EINVAL);
            }
            paging::wrmsr(0xC000_0100, addr);
            0
        }
        0x1003 => {
            // ARCH_GET_FS
            let v = paging::rdmsr(0xC000_0100);
            if paging::copy_to_user(sp, addr, &v.to_le_bytes(), true) {
                0
            } else {
                err(EFAULT)
            }
        }
        _ => err(EINVAL),
    }
}

unsafe fn sys_uname(sp: &Space, buf: u64) -> u64 {
    let mut b = [0u8; 65 * 6];
    let fields: [&[u8]; 6] = [b"myos", b"localhost", b"0.1", b"#1", b"x86_64", b"(none)"];
    for (i, s) in fields.iter().enumerate() {
        b[i * 65..i * 65 + s.len()].copy_from_slice(s);
    }
    if paging::copy_to_user(sp, buf, &b, true) {
        0
    } else {
        err(EFAULT)
    }
}

unsafe fn sys_clock_gettime(sp: &Space, ts: u64) -> u64 {
    // gia su TSC ~2 GHz; chi de co thoi gian tang dan
    let ns_total = rdtsc() / 2;
    let mut b = [0u8; 16];
    b[0..8].copy_from_slice(&(ns_total / 1_000_000_000).to_le_bytes());
    b[8..16].copy_from_slice(&(ns_total % 1_000_000_000).to_le_bytes());
    if paging::copy_to_user(sp, ts, &b, true) {
        0
    } else {
        err(EFAULT)
    }
}

unsafe fn sys_getrandom(sp: &Space, buf: u64, len: u64) -> u64 {
    let len = len.min(256) as usize;
    let mut b = [0u8; 256];
    let mut x = rdtsc() | 1;
    for v in b[..len].iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *v = x as u8;
    }
    if paging::copy_to_user(sp, buf, &b[..len], true) {
        len as u64
    } else {
        err(EFAULT)
    }
}

unsafe fn sys_ioctl(sp: &Space, fd: u64, req: u64, arg: u64) -> u64 {
    match fd_get(fd) {
        Some(f) if f.kind == Kind::Console => {}
        Some(f) if f.kind == Kind::File => {
            let p = core::str::from_utf8(&f.path[..f.plen]).unwrap_or("");
            return match p.strip_prefix("devices://") {
                Some(name) => crate::devfs::ioctl(name, req, arg) as u64,
                None => err(ENOTTY),
            };
        }
        _ => return err(ENOTTY),
    }
    match req {
        0x5401 => {
            // TCGETS: termios 36 byte
            let mut b = [0u8; 36];
            b[0..4].copy_from_slice(&0x100u32.to_le_bytes()); // ICRNL
            b[4..8].copy_from_slice(&0x5u32.to_le_bytes()); // OPOST|ONLCR
            b[8..12].copy_from_slice(&0xB0u32.to_le_bytes()); // CS8|CREAD
            let l: u32 = if crate::tty::ICANON { 0x1B } else { 0x1 };
            b[12..16].copy_from_slice(&l.to_le_bytes()); // ISIG|ICANON|ECHO|ECHOE
            b[17] = 3; // VINTR
            b[21] = 4; // VEOF
            b[23] = 1; // VMIN
            if paging::copy_to_user(sp, arg, &b, true) { 0 } else { err(EFAULT) }
        }
        0x5402 | 0x5403 | 0x5404 => {
            // TCSETS / TCSETSW / TCSETSF
            let mut b = [0u8; 36];
            if !paging::copy_from_user(sp, arg, &mut b) {
                return err(EFAULT);
            }
            let l = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
            crate::tty::ICANON = l & 2 != 0;
            0
        }
        0x5413 => {
            // TIOCGWINSZ
            let (c, r) = crate::video::text_size();
            let mut b = [0u8; 8];
            b[0..2].copy_from_slice(&(r as u16).to_le_bytes());
            b[2..4].copy_from_slice(&(c as u16).to_le_bytes());
            if paging::copy_to_user(sp, arg, &b, true) { 0 } else { err(EFAULT) }
        }
        0x540F => {
            // TIOCGPGRP
            if paging::copy_to_user(sp, arg, &1u32.to_le_bytes(), true) { 0 } else { err(EFAULT) }
        }
        0x5410 => 0, // TIOCSPGRP
        _ => err(ENOTTY),
    }
}

unsafe fn sys_dupfd(fd: u64, min: u64) -> u64 {
    let f = match fd_get(fd) {
        Some(f) => f,
        None => return err(EBADF),
    };
    let start = min.min(NFD as u64) as usize;
    match (start..NFD).find(|&i| fds()[i].kind == Kind::Free) {
        Some(i) => {
            fd_ref(&f);
            fds()[i] = f;
            i as u64
        }
        None => err(EMFILE),
    }
}

unsafe fn sys_dup2(old: u64, new: u64) -> u64 {
    let f = match fd_get(old) {
        Some(f) => f,
        None => return err(EBADF),
    };
    if new >= NFD as u64 {
        return err(EBADF);
    }
    if old != new {
        let t = fds()[new as usize];
        if t.kind != Kind::Free {
            fd_unref(&t);
        }
        fd_ref(&f);
        fds()[new as usize] = f;
    }
    new
}

unsafe fn sys_fcntl(fd: u64, cmd: u64, arg: u64) -> u64 {
    match cmd {
        0 | 1030 => sys_dupfd(fd, arg), // F_DUPFD / F_DUPFD_CLOEXEC
        _ => {
            if fd_get(fd).is_none() {
                err(EBADF)
            } else {
                0 // F_GETFD / F_SETFD / F_GETFL / F_SETFL
            }
        }
    }
}

/// poll/ppoll: bao san sang ngay cho fd hop le; read() tu cho phim.
unsafe fn sys_poll(sp: &Space, ptr: u64, n: u64) -> u64 {
    if n > 16 {
        return err(EINVAL);
    }
    let mut ready = 0u64;
    for i in 0..n {
        let at = ptr.wrapping_add(i * 8);
        let mut b = [0u8; 8];
        if !paging::copy_from_user(sp, at, &mut b) {
            return err(EFAULT);
        }
        let fd = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let ev = i16::from_le_bytes([b[4], b[5]]);
        let rev: i16 = if fd < 0 {
            0
        } else if let Some(f) = fd_get(fd as u64) {
            if f.kind == Kind::PipeR {
                let q = &pipes()[f.off as usize];
                if q.len > 0 {
                    ev & 1 // POLLIN
                } else if q.writers == 0 {
                    0x10 // POLLHUP
                } else {
                    0
                }
            } else {
                ev & 0x5 // POLLIN | POLLOUT
            }
        } else {
            0x20 // POLLNVAL
        };
        if !paging::copy_to_user(sp, at + 6, &rev.to_le_bytes(), true) {
            return err(EFAULT);
        }
        if rev != 0 {
            ready += 1;
        }
    }
    ready
}

// ---- loc ESC[J / ESC[K thanh " \b" (xoa ky tu duoi con tro) ----
static mut ESCBUF: [u8; 8] = [0; 8];
static mut ESCLEN: usize = 0;

unsafe fn out_bytes(b: &[u8]) {
    if let Ok(s) = core::str::from_utf8(b) {
        crate::kprint!("{}", s);
    }
}

unsafe fn emit(data: &[u8]) {
    let eb = &mut *addr_of_mut!(ESCBUF);
    for &c in data {
        if ESCLEN == 0 {
            if c == 0x1b {
                eb[0] = c;
                ESCLEN = 1;
            } else {
                out_bytes(&[c]);
            }
            continue;
        }
        if ESCLEN == 1 {
            if c == b'[' {
                eb[1] = c;
                ESCLEN = 2;
            } else {
                out_bytes(&[0x1b]);
                ESCLEN = 0;
                out_bytes(&[c]);
            }
            continue;
        }
        if c.is_ascii_digit() || c == b';' {
            if ESCLEN < 8 {
                eb[ESCLEN] = c;
                ESCLEN += 1;
            } else {
                out_bytes(&eb[..ESCLEN]);
                out_bytes(&[c]);
                ESCLEN = 0;
            }
            continue;
        }
        if (c == b'J' || c == b'K') && (ESCLEN == 2 || (ESCLEN == 3 && eb[2] == b'0')) {
            out_bytes(b" \x08");
        } else {
            out_bytes(&eb[..ESCLEN]);
            out_bytes(&[c]);
        }
        ESCLEN = 0;
    }
}


// ================= process state / pipe / execve =================
const EPIPE: i64 = 32;
const ENOEXEC: i64 = 8;
const BUSYBOX: &str = "rd0://bin/busybox";

pub struct PState {
    fds: [Fd; NFD],
    space: u64,
    brk_start: u64,
    brk_cur: u64,
    mmap_next: u64,
    cwd: [u8; 64],
    cwd_len: usize,
    fs_base: u64,
}

impl PState {
    pub const EMPTY: PState = PState {
        fds: [FREE_FD; NFD],
        space: 0,
        brk_start: 0,
        brk_cur: 0,
        mmap_next: 0,
        cwd: [0; 64],
        cwd_len: 0,
        fs_base: 0,
    };
}

pub unsafe fn save_state(p: &mut PState) {
    core::ptr::copy_nonoverlapping(fds().as_ptr(), p.fds.as_mut_ptr(), NFD);
    p.space = SPACE;
    p.brk_start = BRK_START;
    p.brk_cur = BRK_CUR;
    p.mmap_next = MMAP_NEXT;
    p.cwd = *addr_of_mut!(CWD);
    p.cwd_len = CWD_LEN;
    p.fs_base = paging::rdmsr(0xC000_0100);
}

pub unsafe fn load_state(p: &PState) {
    core::ptr::copy_nonoverlapping(p.fds.as_ptr(), fds().as_mut_ptr(), NFD);
    SPACE = p.space;
    BRK_START = p.brk_start;
    BRK_CUR = p.brk_cur;
    MMAP_NEXT = p.mmap_next;
    *addr_of_mut!(CWD) = p.cwd;
    CWD_LEN = p.cwd_len;
    paging::wrmsr(0xC000_0100, p.fs_base);
}

pub unsafe fn init_fresh(p: &mut PState, space: u64, brk: u64) {
    for i in 0..NFD {
        p.fds[i] = FREE_FD;
    }
    for i in 0..3 {
        p.fds[i].kind = Kind::Console;
    }
    p.space = space;
    p.brk_start = brk;
    p.brk_cur = brk;
    p.mmap_next = MMAP_BASE;
    p.cwd = [0; 64];
    p.cwd[0] = b'/';
    p.cwd_len = 1;
    p.fs_base = 0;
}

pub unsafe fn after_fork(p: &mut PState, space: u64) {
    p.space = space;
    for i in 0..NFD {
        let f = p.fds[i];
        if f.kind != Kind::Free {
            fd_ref(&f);
        }
    }
}

pub unsafe fn exec_reset(sp: &Space, brk: u64) {
    SPACE = sp.pml4;
    BRK_START = brk;
    BRK_CUR = brk;
    MMAP_NEXT = MMAP_BASE;
    paging::wrmsr(0xC000_0100, 0);
}

pub unsafe fn close_all() {
    for i in 0..NFD {
        let f = fds()[i];
        if f.kind != Kind::Free {
            fd_unref(&f);
            fds()[i] = FREE_FD;
        }
    }
}

// ---------------- pipe ----------------
const PBUF: usize = 32 * 1024;
const NPIPE: usize = 16;

struct Pipe {
    used: bool,
    head: usize,
    len: usize,
    readers: u32,
    writers: u32,
    buf: [u8; PBUF],
}

const P0: Pipe = Pipe { used: false, head: 0, len: 0, readers: 0, writers: 0, buf: [0; PBUF] };
static mut PIPES: [Pipe; NPIPE] = [P0; NPIPE];

unsafe fn pipes() -> &'static mut [Pipe; NPIPE] {
    &mut *addr_of_mut!(PIPES)
}

unsafe fn fd_ref(f: &Fd) {
    match f.kind {
        Kind::PipeR => pipes()[f.off as usize].readers += 1,
        Kind::PipeW => pipes()[f.off as usize].writers += 1,
        _ => {}
    }
}

unsafe fn fd_unref(f: &Fd) {
    let id = f.off as usize;
    match f.kind {
        Kind::PipeR => pipes()[id].readers -= 1,
        Kind::PipeW => pipes()[id].writers -= 1,
        _ => return,
    }
    let q = &mut pipes()[id];
    if q.readers == 0 && q.writers == 0 {
        q.used = false;
    }
}

unsafe fn sys_pipe(sp: &Space, ptr: u64) -> u64 {
    let mut id = NPIPE;
    for i in 0..NPIPE {
        if !pipes()[i].used {
            id = i;
            break;
        }
    }
    if id == NPIPE {
        return err(EMFILE);
    }
    let r = match (0..NFD).find(|&i| fds()[i].kind == Kind::Free) {
        Some(i) => i,
        None => return err(EMFILE),
    };
    let w = match (r + 1..NFD).find(|&i| fds()[i].kind == Kind::Free) {
        Some(i) => i,
        None => return err(EMFILE),
    };
    let mut b = [0u8; 8];
    b[0..4].copy_from_slice(&(r as i32).to_le_bytes());
    b[4..8].copy_from_slice(&(w as i32).to_le_bytes());
    if !paging::copy_to_user(sp, ptr, &b, true) {
        return err(EFAULT);
    }
    let q = &mut pipes()[id];
    q.used = true;
    q.head = 0;
    q.len = 0;
    q.readers = 1;
    q.writers = 1;
    let mut fr = FREE_FD;
    fr.kind = Kind::PipeR;
    fr.off = id as u64;
    let mut fw = FREE_FD;
    fw.kind = Kind::PipeW;
    fw.off = id as u64;
    fds()[r] = fr;
    fds()[w] = fw;
    0
}

unsafe fn pipe_read(sp: &Space, id: usize, buf: u64, count: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    loop {
        let q = &mut pipes()[id];
        if q.len > 0 {
            // doan lien ke tu head toi cuoi vong, copy thang vao user
            let n = (count as usize).min(q.len).min(PBUF - q.head);
            if !paging::copy_to_user(sp, buf, &q.buf[q.head..q.head + n], true) {
                return err(EFAULT);
            }
            q.head = (q.head + n) % PBUF;
            q.len -= n;
            return n as u64;
        }
        if q.writers == 0 {
            return 0;
        }
        crate::task::yield_now(); // khong sleep(1): tranh tre >= 1ms moi lan pipe rong
    }
}

unsafe fn pipe_write(sp: &Space, id: usize, ptr: u64, len: u64) -> u64 {
    let mut done = 0u64;
    while done < len {
        let q = &mut pipes()[id];
        if q.readers == 0 {
            return if done > 0 { done } else { err(EPIPE) };
        }
        let room = PBUF - q.len;
        if room == 0 {
            crate::task::yield_now();
            continue;
        }
        let tail = (q.head + q.len) % PBUF;
        let n = (room as u64).min(len - done).min((PBUF - tail) as u64) as usize;
        if !paging::copy_from_user(sp, ptr.wrapping_add(done), &mut q.buf[tail..tail + n]) {
            return if done > 0 { done } else { err(EFAULT) };
        }
        q.len += n;
        done += n as u64;
    }
    done
}

// ---------------- execve ----------------
unsafe fn sys_execve(sp: &Space, path_ptr: u64, argv_ptr: u64) -> u64 {
    if !crate::task::can_exec() {
        return err(11); // EAGAIN: execve tu thread phu khi con thread khac chua ho tro
    }
    let p = match user_path(sp, path_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let mut bufs = [[0u8; 128]; 16];
    let mut lens = [0usize; 16];
    let mut argc = 0usize;
    if argv_ptr != 0 {
        while argc < 16 {
            let mut b = [0u8; 8];
            if !paging::copy_from_user(sp, argv_ptr + argc as u64 * 8, &mut b) {
                return err(EFAULT);
            }
            let a = u64::from_le_bytes(b);
            if a == 0 {
                break;
            }
            match copy_cstr(sp, a, &mut bufs[argc]) {
                Some(n) => lens[argc] = n,
                None => return err(EFAULT),
            }
            argc += 1;
        }
    }
    let mut av: [&str; 16] = [""; 16];
    for i in 0..argc {
        av[i] = core::str::from_utf8(&bufs[i][..lens[i]]).unwrap_or("");
    }
    let ps = p.as_str();
    if argc == 0 {
        av[0] = ps;
        argc = 1;
    }
    let exists = matches!(vfs::stat(ps), Some(st) if !st.is_dir);
    let target: &str = if exists {
        ps
    } else {
        let applet = match ps.strip_prefix("rd0://bin/") {
            Some(n) => !n.is_empty() && !n.contains('/'),
            None => false,
        };
        if applet && vfs::stat(BUSYBOX).is_some() {
            BUSYBOX
        } else {
            return err(ENOENT);
        }
    };
    match crate::app::prepare(target, &av[..argc]) {
        Ok((nsp, entry, rsp, brk)) => {
            crate::task::exec_commit(nsp, entry, rsp, brk);
            0
        }
        Err(e) => {
            crate::kprintln!("[exec] {}: {}", ps, e);
            err(ENOEXEC)
        }
    }
}

/// write() vào file: chỉ devices:// ghi được (qua vfs::write), còn lại read-only.
unsafe fn file_write(sp: &Space, fd: u64, f: &Fd, ptr: u64, len: u64) -> u64 {
    if f.is_dir {
        return err(EISDIR);
    }
    let path = core::str::from_utf8(&f.path[..f.plen]).unwrap_or("");
    if !path.starts_with("devices://") {
        return err(EROFS);
    }
    let mut off = f.off;
    let mut done = 0u64;
    let mut chunk = [0u8; 2048];
    while done < len {
        let want = (len - done).min(chunk.len() as u64) as usize;
        if !paging::copy_from_user(sp, ptr.wrapping_add(done), &mut chunk[..want]) {
            return if done > 0 { done } else { err(EFAULT) };
        }
        let n = vfs::write(path, off, &chunk[..want]);
        if n == 0 {
            break;
        }
        off += n as u64;
        done += n as u64;
        if n < want {
            break;
        }
    }
    fd_set_off(fd, off);
    if done == 0 && len > 0 {
        err(EINVAL)
    } else {
        done
    }
}

/// mmap(fd = devices://fb0): map thang framebuffer vao user (dung cho libgfx present).
unsafe fn mmap_fb(sp: &Space, len: u64, fd: u64) -> u64 {
    let f = match fd_get(fd) {
        Some(f) if f.kind == Kind::File => f,
        _ => return err(ENODEV),
    };
    let path = core::str::from_utf8(&f.path[..f.plen]).unwrap_or("");
    if path != "devices://fb0" {
        return err(ENODEV);
    }
    let fb = match crate::video::saved() {
        Some(x) => x,
        None => return err(ENODEV),
    };
    let size = (fb.pitch * fb.height) as u64;
    let len = (len + 4095) & !4095;
    if len == 0 || len > ((size + 4095) & !4095) {
        return err(EINVAL);
    }
    let phys = (fb.addr as usize as u64).wrapping_sub(pmm::virt(0) as u64);
    let base = MMAP_NEXT;
    if base.checked_add(len).map_or(true, |e| e > paging::USER_END) {
        return err(ENOMEM);
    }
    MMAP_NEXT += len;
    let mut off = 0u64;
    while off < len {
        if !paging::map_user_phys(sp, base + off, (phys & !0xfff) + off, true) {
            let mut q = base;
            while q < base + off {
                paging::unmap_user_page(sp, q);
                q += 4096;
            }
            paging::flush();
            return err(ENOMEM);
        }
        off += 4096;
    }
    paging::flush();
    crate::video::GFX_OWNER.store(crate::task::pid(), core::sync::atomic::Ordering::Relaxed);
    base
}

unsafe fn sys_nanosleep(sp: &Space, req: u64) -> u64 {
    let mut b = [0u8; 16];
    if !paging::copy_from_user(sp, req, &mut b) {
        return err(EFAULT);
    }
    let s = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
    let ns = u64::from_le_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]);
    let mut ms = s.saturating_mul(1000).saturating_add(ns / 1_000_000);
    if ms == 0 {
        ms = 1;
    }
    crate::task::sleep(ms.min(60_000));
    0
}
