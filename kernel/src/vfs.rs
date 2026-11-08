//! VFS: mount table + đường dẫn kiểu <loại><số>://path (ví dụ rd0://hello.txt).
use crate::{block, drivers};
use core::ffi::c_void;
use core::ptr::{addr_of, addr_of_mut};
use driver_abi::{DirEntry, FsOps, FsStat};

#[derive(Clone, Copy)]
struct Mount {
    kind: &'static str,
    index: usize,
    fs: &'static FsOps,
    inst: *mut c_void,
}

const MAX_MOUNTS: usize = 8;
static mut MOUNTS: [Option<Mount>; MAX_MOUNTS] = [None; MAX_MOUNTS];

/// Thử mount mọi block device bằng các driver FS đã nạp.
pub fn automount() {
    let mut i = 0;
    while let Some(d) = block::get(i) {
        i += 1;
        let mut done = false;
        let mut n = 0;
        while let Some(fs) = drivers::fs_ops(n) {
            n += 1;
            if !(fs.probe)(&d.dev) {
                continue;
            }
            let inst = (fs.mount)(&d.dev);
            if inst.is_null() {
                continue;
            }
            unsafe {
                let t = &mut *addr_of_mut!(MOUNTS);
                for s in t.iter_mut() {
                    if s.is_none() {
                        *s = Some(Mount { kind: d.kind, index: d.index, fs, inst });
                        break;
                    }
                }
            }
            crate::kprintln!("  {}{}://  ->  {}", d.kind, d.index, drivers::cstr(&fs.fs_name));
            done = true;
            break;
        }
        if !done {
            crate::kprintln!("  {}{}://  ->  no filesystem recognized", d.kind, d.index);
        }
    }
}

/// "rd0://a/b" -> ("rd", 0, "a/b")
fn split(path: &str) -> Option<(&str, usize, &str)> {
    let p = path.find("://")?;
    let drive = &path[..p];
    let rest = &path[p + 3..];
    let d = drive.find(|c: char| c.is_ascii_digit())?;
    let (kind, num) = drive.split_at(d);
    if kind.is_empty() {
        return None;
    }
    Some((kind, num.parse::<usize>().ok()?, rest))
}

fn resolve(path: &str) -> Option<(Mount, &str)> {
    let (kind, index, rest) = split(path)?;
    unsafe {
        let t = &*addr_of!(MOUNTS);
        for s in t.iter() {
            if let Some(m) = s {
                if m.kind == kind && m.index == index {
                    return Some((*m, rest));
                }
            }
        }
    }
    None
}

pub fn stat(path: &str) -> Option<FsStat> {
    if let Some(n) = devname(path) {
        return crate::devfs::stat(n);
    }
    let (m, rest) = resolve(path)?;
    let mut st = FsStat { size: 0, is_dir: false };
    if (m.fs.stat)(m.inst, rest.as_ptr(), rest.len(), &mut st) {
        Some(st)
    } else {
        None
    }
}

pub fn read(path: &str, offset: u64, buf: &mut [u8]) -> usize {
    if let Some(n) = devname(path) {
        return crate::devfs::read(n, offset, buf);
    }
    match resolve(path) {
        Some((m, rest)) => {
            (m.fs.read)(m.inst, rest.as_ptr(), rest.len(), offset, buf.as_mut_ptr(), buf.len())
        }
        None => 0,
    }
}

pub fn readdir(path: &str, idx: usize) -> Option<DirEntry> {
    if let Some(n) = devname(path) {
        return crate::devfs::readdir(n, idx);
    }
    let (m, rest) = resolve(path)?;
    let mut e = DirEntry::empty();
    if (m.fs.readdir)(m.inst, rest.as_ptr(), rest.len(), idx, &mut e) {
        Some(e)
    } else {
        None
    }
}

/// Tên ổ thứ i đã mount ("rd0", "ata0"...) ghi vào out, trả độ dài; None nếu hết.
pub fn drive_name(i: usize, out: &mut [u8; 16]) -> Option<usize> {
    unsafe {
        let t = &*addr_of!(MOUNTS);
        let m = t.iter().flatten().nth(i)?;
        let mut s = [0u8; 16];
        let mut n = 0;
        for &b in m.kind.as_bytes() {
            s[n] = b;
            n += 1;
        }
        let mut digits = [0u8; 8];
        let mut d = 0;
        let mut v = m.index;
        loop {
            digits[d] = b'0' + (v % 10) as u8;
            d += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while d > 0 {
            d -= 1;
            s[n] = digits[d];
            n += 1;
        }
        *out = s;
        Some(n)
    }
}

pub fn drive_exists(name: &str) -> bool {
    let mut b = [0u8; 16];
    let mut i = 0;
    while let Some(n) = drive_name(i, &mut b) {
        if &b[..n] == name.as_bytes() {
            return true;
        }
        i += 1;
    }
    false
}

/// "devices://fb0" -> Some("fb0"); đường dẫn khác -> None.
pub fn devname(path: &str) -> Option<&str> {
    path.strip_prefix("devices://")
}

/// Ghi file. Hiện chỉ hỗ trợ devices://; volume rd0 là read-only.
pub fn write(path: &str, offset: u64, data: &[u8]) -> usize {
    match devname(path) {
        Some(n) => crate::devfs::write(n, offset, data),
        None => 0,
    }
}
