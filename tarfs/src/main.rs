#![no_std]
#![no_main]

use core::ffi::c_void;
use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, null_mut};
use driver_abi::*;

const MAX_INST: usize = 4;
static mut INST: [Option<BlockDev>; MAX_INST] = [None; MAX_INST];

fn eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for i in 0..a.len() {
        if a[i] != b[i] {
            return false;
        }
    }
    true
}

fn octal(b: &[u8]) -> u64 {
    let mut v = 0u64;
    for &c in b {
        match c {
            b'0'..=b'7' => v = v * 8 + (c - b'0') as u64,
            b' ' if v == 0 => {}
            _ => break,
        }
    }
    v
}

fn read_hdr(dev: &BlockDev, pos: u64, h: &mut [u8; 512]) -> bool {
    if pos + 512 > dev.size {
        return false;
    }
    (dev.read)(dev.ctx, pos, h.as_mut_ptr(), 512)
}

struct Ent {
    name: [u8; 256],
    nlen: usize,
    size: u64,
    data_off: u64,
    hdr: u64,
    is_dir: bool,
}

impl Ent {
    fn name(&self) -> &[u8] {
        &self.name[..self.nlen]
    }
}

/// Đọc entry kế tiếp (file hoặc thư mục), bỏ qua pax header và entry root.
fn next(dev: &BlockDev, pos: &mut u64) -> Option<Ent> {
    loop {
        let hdr = *pos;
        let mut h = [0u8; 512];
        if !read_hdr(dev, hdr, &mut h) || h[0] == 0 {
            return None;
        }
        let size = octal(&h[124..136]);
        let kind = h[156];
        let data_off = hdr + 512;
        *pos = data_off + ((size + 511) & !511);
        if kind != 0 && kind != b'0' && kind != b'5' {
            continue;
        }

        let mut e = Ent {
            name: [0; 256],
            nlen: 0,
            size,
            data_off,
            hdr,
            is_dir: kind == b'5',
        };

        // ghép prefix (ustar) + name
        let mut n = 0;
        if eq(&h[257..262], b"ustar") {
            let mut pl = 0;
            while pl < 155 && h[345 + pl] != 0 {
                pl += 1;
            }
            if pl > 0 {
                for i in 0..pl {
                    e.name[n] = h[345 + i];
                    n += 1;
                }
                e.name[n] = b'/';
                n += 1;
            }
        }
        let mut nl = 0;
        while nl < 100 && h[nl] != 0 {
            nl += 1;
        }
        for i in 0..nl {
            e.name[n] = h[i];
            n += 1;
        }

        // chuẩn hoá: bỏ "./" đầu, bỏ "/" cuối
        let mut s = 0;
        if n >= 2 && e.name[0] == b'.' && e.name[1] == b'/' {
            s = 2;
        }
        let mut end = n;
        while end > s && e.name[end - 1] == b'/' {
            end -= 1;
            e.is_dir = true;
        }
        if end == s || (end - s == 1 && e.name[s] == b'.') {
            continue; // entry root "./"
        }
        let len = end - s;
        for i in 0..len {
            e.name[i] = e.name[s + i];
        }
        e.nlen = len;
        return Some(e);
    }
}

fn norm(p: &[u8]) -> &[u8] {
    let mut s = 0;
    let mut e = p.len();
    while s < e && p[s] == b'/' {
        s += 1;
    }
    while e > s && p[e - 1] == b'/' {
        e -= 1;
    }
    &p[s..e]
}

/// `name` nằm bên dưới thư mục `dir`?
fn under(name: &[u8], dir: &[u8]) -> bool {
    name.len() > dir.len() && eq(&name[..dir.len()], dir) && name[dir.len()] == b'/'
}

/// Thành phần con trực tiếp của `dir` mà entry `name` thuộc về, kèm cờ "con này là thư mục".
fn child<'a>(name: &'a [u8], dir: &[u8]) -> Option<(&'a [u8], bool)> {
    let rest = if dir.is_empty() {
        name
    } else if under(name, dir) {
        &name[dir.len() + 1..]
    } else {
        return None;
    };
    if rest.is_empty() {
        return None;
    }
    let mut i = 0;
    while i < rest.len() && rest[i] != b'/' {
        i += 1;
    }
    Some((&rest[..i], i < rest.len()))
}

fn seen_before(dev: &BlockDev, dir: &[u8], limit: u64, c: &[u8]) -> bool {
    let mut p = 0u64;
    while let Some(e) = next(dev, &mut p) {
        if e.hdr >= limit {
            break;
        }
        if let Some((n, _)) = child(e.name(), dir) {
            if eq(n, c) {
                return true;
            }
        }
    }
    false
}

fn dev_of(h: *mut c_void) -> Option<BlockDev> {
    let i = h as usize;
    if i == 0 || i > MAX_INST {
        return None;
    }
    unsafe { (*addr_of!(INST))[i - 1] }
}

unsafe fn path<'a>(p: *const u8, l: usize) -> &'a [u8] {
    norm(core::slice::from_raw_parts(p, l))
}

extern "C" fn op_probe(dev: *const BlockDev) -> bool {
    let d = unsafe { &*dev };
    let mut h = [0u8; 512];
    read_hdr(d, 0, &mut h) && eq(&h[257..262], b"ustar")
}

extern "C" fn op_mount(dev: *const BlockDev) -> *mut c_void {
    unsafe {
        let t = &mut *addr_of_mut!(INST);
        for (i, s) in t.iter_mut().enumerate() {
            if s.is_none() {
                *s = Some(*dev);
                return (i + 1) as *mut c_void;
            }
        }
    }
    null_mut()
}

extern "C" fn op_unmount(h: *mut c_void) {
    let i = h as usize;
    if i >= 1 && i <= MAX_INST {
        unsafe { (*addr_of_mut!(INST))[i - 1] = None };
    }
}

extern "C" fn op_stat(h: *mut c_void, p: *const u8, l: usize, out: *mut FsStat) -> bool {
    let dev = match dev_of(h) {
        Some(d) => d,
        None => return false,
    };
    let path = unsafe { path(p, l) };
    unsafe {
        if path.is_empty() {
            *out = FsStat { size: 0, is_dir: true };
            return true;
        }
        let mut pos = 0u64;
        let mut implicit = false;
        while let Some(e) = next(&dev, &mut pos) {
            if eq(e.name(), path) {
                *out = FsStat { size: e.size, is_dir: e.is_dir };
                return true;
            }
            if under(e.name(), path) {
                implicit = true;
            }
        }
        if implicit {
            *out = FsStat { size: 0, is_dir: true };
            return true;
        }
    }
    false
}

extern "C" fn op_read(
    h: *mut c_void,
    p: *const u8,
    l: usize,
    off: u64,
    buf: *mut u8,
    len: usize,
) -> usize {
    let dev = match dev_of(h) {
        Some(d) => d,
        None => return 0,
    };
    let path = unsafe { path(p, l) };
    let mut pos = 0u64;
    while let Some(e) = next(&dev, &mut pos) {
        if !e.is_dir && eq(e.name(), path) {
            if off >= e.size {
                return 0;
            }
            let n = core::cmp::min(len as u64, e.size - off) as usize;
            if (dev.read)(dev.ctx, e.data_off + off, buf, n) {
                return n;
            }
            return 0;
        }
    }
    0
}

extern "C" fn op_readdir(
    h: *mut c_void,
    p: *const u8,
    l: usize,
    idx: usize,
    out: *mut DirEntry,
) -> bool {
    let dev = match dev_of(h) {
        Some(d) => d,
        None => return false,
    };
    let dir = unsafe { path(p, l) };
    let mut pos = 0u64;
    let mut count = 0usize;
    while let Some(e) = next(&dev, &mut pos) {
        let (c, deeper) = match child(e.name(), dir) {
            Some(x) => x,
            None => continue,
        };
        if seen_before(&dev, dir, e.hdr, c) {
            continue;
        }
        if count == idx {
            let o = unsafe { &mut *out };
            for i in 0..100 {
                o.name[i] = 0;
            }
            let n = if c.len() < 99 { c.len() } else { 99 };
            for i in 0..n {
                o.name[i] = c[i];
            }
            o.is_dir = deeper || e.is_dir;
            o.size = if o.is_dir { 0 } else { e.size };
            return true;
        }
        count += 1;
    }
    false
}

static OPS: FsOps = FsOps {
    fs_name: name("tarfs"),
    probe: op_probe,
    mount: op_mount,
    unmount: op_unmount,
    stat: op_stat,
    read: op_read,
    readdir: op_readdir,
};

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    let api = unsafe { &*api };
    if api.abi_version != DRIVER_ABI {
        return -1;
    }
    let info = DriverInfo {
        name: name("tarfs"),
        kind: KIND_FS,
        ops: &OPS as *const FsOps as *const c_void,
    };
    (api.register)(&info)
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
