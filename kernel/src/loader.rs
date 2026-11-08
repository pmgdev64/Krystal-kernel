use core::mem::size_of;
use core::ptr::{addr_of_mut, read_unaligned, write_unaligned};

const AREA_SIZE: usize = 2 * 1024 * 1024;

#[repr(C, align(4096))]
struct Area([u8; AREA_SIZE]);

#[used]
#[link_section = ".driver_area"]
static mut AREA: Area = Area([0; AREA_SIZE]);

/// Con trỏ bump: offset trống tiếp theo trong AREA.
static mut NEXT: usize = 0;

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

#[repr(C)]
struct Dyn {
    tag: i64,
    val: u64,
}

#[repr(C)]
struct Rela {
    offset: u64,
    info: u64,
    addend: i64,
}

unsafe fn phdr(img: &[u8], eh: &Ehdr, i: usize) -> Result<Phdr, &'static str> {
    let off = eh.phoff as usize + i * eh.phentsize as usize;
    if off + size_of::<Phdr>() > img.len() {
        return Err("bad phdr");
    }
    Ok(read_unaligned(img.as_ptr().add(off) as *const Phdr))
}

/// Load ELF PIE vào vùng driver, trả về địa chỉ entry.
pub unsafe fn load(img: &[u8]) -> Result<usize, &'static str> {
    if img.len() < size_of::<Ehdr>() || &img[0..4] != b"\x7fELF" {
        return Err("not an ELF");
    }
    let eh = read_unaligned(img.as_ptr() as *const Ehdr);
    if eh.ident[4] != 2 || eh.machine != 0x3E || eh.e_type != 3 {
        return Err("need x86_64 ET_DYN ELF64");
    }

    // pass 1: tính tổng kích thước cần
    let mut size = 0usize;
    for i in 0..eh.phnum as usize {
        let ph = phdr(img, &eh, i)?;
        if ph.p_type == 1 {
            size = size.max((ph.vaddr + ph.memsz) as usize);
        }
    }
    if size == 0 {
        return Err("no PT_LOAD");
    }
    if eh.entry == 0 || eh.entry as usize >= size {
        return Err("bad e_entry (0 or outside image): link driver with --entry=driver_entry");
    }

    // cấp phát (bump, align 4096)
    let start = (NEXT + 4095) & !4095;
    if start + size > AREA_SIZE {
        return Err("driver area full");
    }
    NEXT = start + size;
    let base = (addr_of_mut!(AREA) as *mut u8).add(start);

    // pass 2: copy segment + tìm PT_DYNAMIC
    let mut dyn_vaddr: Option<u64> = None;
    for i in 0..eh.phnum as usize {
        let ph = phdr(img, &eh, i)?;
        match ph.p_type {
            1 => {
                let (va, fs, fo) = (ph.vaddr as usize, ph.filesz as usize, ph.offset as usize);
                if fo + fs > img.len() {
                    return Err("segment out of file");
                }
                for j in 0..fs {
                    base.add(va + j).write(img[fo + j]);
                }
            }
            2 => dyn_vaddr = Some(ph.vaddr),
            _ => {}
        }
    }

    // relocation
    if let Some(dv) = dyn_vaddr {
        let (mut rela, mut relasz) = (0u64, 0u64);
        let mut p = base.add(dv as usize) as *const Dyn;
        loop {
            let d = read_unaligned(p);
            match d.tag {
                0 => break,
                7 => rela = d.val,
                8 => relasz = d.val,
                _ => {}
            }
            p = p.add(1);
        }
        let n = relasz as usize / size_of::<Rela>();
        for i in 0..n {
            let r = read_unaligned((base.add(rela as usize) as *const Rela).add(i));
            if (r.info & 0xffff_ffff) != 8 {
                return Err("unsupported relocation type (only R_X86_64_RELATIVE)");
            }
            let target = base.add(r.offset as usize) as *mut u64;
            write_unaligned(target, (base as i64 + r.addend) as u64);
        }
    }

    Ok(base as usize + eh.entry as usize)
}
