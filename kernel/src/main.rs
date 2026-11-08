#![no_std]
#![no_main]

mod app;
mod block;
mod drivers;
mod devfs;
mod idt;
mod loader;
mod linux;
mod paging;
mod pmm;
mod task;
mod time;
mod panic;
mod tty;
mod shell;
mod vfs;
#[macro_use]
mod video;
mod melody;

use bootvid_abi::{Color, FbInfo, ABI_VERSION};
use core::arch::asm;
use core::ptr::addr_of_mut;
use core::panic::PanicInfo;
use limine::request::{
    FramebufferRequest, HhdmRequest, MemoryMapRequest, ModuleRequest, RequestsEndMarker, RequestsStartMarker,
};
use limine::BaseRevision;
use limine::memory_map::EntryType;

#[used]
#[link_section = ".requests"]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[link_section = ".requests"]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[link_section = ".requests"]
static MODULE_REQUEST: ModuleRequest = ModuleRequest::new();

#[used]
#[link_section = ".requests"]
static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[link_section = ".requests"]
static MEMMAP_REQUEST: MemoryMapRequest = MemoryMapRequest::new();

#[used]
#[link_section = ".requests_start_marker"]
static _START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[link_section = ".requests_end_marker"]
static _END: RequestsEndMarker = RequestsEndMarker::new();

/// Đổi số này để test màn hình panic:
/// 0 = không test, 1 = panic!(), 2 = chia cho 0, 3 = page fault,
/// 4 = invalid opcode, 5 = int3 (chạy tiếp), 6 = #GP (địa chỉ non-canonical)
const PANIC_TEST: u32 = 0;

fn early_serial(s: &str) {
    for b in s.bytes() {
        unsafe {
            asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
        }
    }
}

fn halt() -> ! {
    loop {
        unsafe { asm!("hlt") };
    }
}

fn ls(path: &str) {
    kprintln!("ls {}", path);
    let mut i = 0;
    while let Some(e) = vfs::readdir(path, i) {
        i += 1;
        let slash = if e.is_dir { "/" } else { "" };
        kprintln!("  {}{}  {} bytes", drivers::cstr(&e.name), slash, e.size);
    }
}

fn cat(path: &str) {
    let mut buf = [0u8; 256];
    let n = vfs::read(path, 0, &mut buf);
    match core::str::from_utf8(&buf[..n]) {
        Ok(s) if n > 0 => kprintln!("{} -> {}", path, s.trim_end()),
        _ => kprintln!("{} -> (cannot read)", path),
    }
}

fn panic_test(n: u32) {
    unsafe {
        match n {
            1 => panic!("test panic: gia tri {} khong hop le", 42),
            2 => asm!(
                "xor edx, edx",
                "div ecx",
                in("ecx") 0u32,
                inout("eax") 1u32 => _,
                out("edx") _,
            ),
            3 => {
                (0xffff_a000_0000_0000u64 as *const u8).read_volatile();
            }
            4 => asm!("ud2"),
            5 => asm!("int3"),
            6 => {
                (0x8000_0000_0000_0000u64 as *const u8).read_volatile();
            }
            _ => {}
        }
    }
}

#[no_mangle]
unsafe extern "C" fn kmain() -> ! {
    assert!(BASE_REVISION.is_supported());
    idt::init();
    early_serial("kernel started\n");

    let modules = match MODULE_REQUEST.get_response() {
        Some(r) => r.modules(),
        None => {
            early_serial("no modules (check module_path in limine.conf)\n");
            halt()
        }
    };
    // Chon framebuffer dung duoc (lon nhat) tu Limine; UEFI GOP va BIOS VBE deu qua day.
    let fb_resp = match FRAMEBUFFER_REQUEST.get_response() {
        Some(r) => r,
        None => {
            early_serial("no framebuffer response from limine\n");
            halt()
        }
    };
    let mut best: Option<FbInfo> = None;
    let mut why = "no framebuffer";
    for f in fb_resp.framebuffers() {
        let i = FbInfo {
            addr: f.addr(),
            width: f.width() as usize,
            height: f.height() as usize,
            pitch: f.pitch() as usize,
            bpp: f.bpp() as usize,
            red_size: f.red_mask_size(),
            red_shift: f.red_mask_shift(),
            green_size: f.green_mask_size(),
            green_shift: f.green_mask_shift(),
            blue_size: f.blue_mask_size(),
            blue_shift: f.blue_mask_shift(),
        };
        match i.supported() {
            Ok(()) => {
                if best.map_or(true, |b| i.width * i.height > b.width * b.height) {
                    best = Some(i);
                }
            }
            Err(e) => why = e,
        }
    }
    let info = match best {
        Some(i) => i,
        None => {
            early_serial("bootvid: ");
            early_serial(why);
            early_serial("\n");
            halt()
        }
    };

    // Init video NGAY khi bootvid nap xong, de driver nap sau panic van co man hinh.
    // Yeu cau: bootvid.sys dung truoc cac driver khac trong limine.conf.
    if let Some(r) = HHDM_REQUEST.get_response() {
        pmm::set_hhdm(r.offset());
    }
    let mut vid_ready = false;
    for f in modules {
        let img: &'static [u8] = core::slice::from_raw_parts(f.addr(), f.size() as usize);
        let is_elf = img.len() >= 4 && &img[0..4] == b"\x7fELF";
        if is_elf {
            if let Err(e) = drivers::load(img) {
                early_serial("driver load failed: ");
                early_serial(e);
                if vid_ready {
                    kprintln!("driver load failed: {}", e);
                }
                early_serial("\n");
            }
            if !vid_ready {
                if let Some(v) = drivers::video_ops() {
                    if v.abi_version == ABI_VERSION {
                        vid_ready = video::init(&info);
                    }
                }
            }
        } else if block::add_ramdisk(img).is_none() {
            early_serial("cannot register ramdisk\n");
        }
    }
    if !vid_ready {
        early_serial("no usable video driver / bootvid init failed\n");
        halt();
    }

    video::set_fg(Color::GREEN);
    kprintln!("driver system OK");
    video::set_fg(Color::GRAY);
    let (cols, rows) = video::text_size();
    kprintln!("fb {}x{}, console {}x{}", info.width, info.height, cols, rows);
    kprintln!("fb {}bpp pitch={} r{}:{} g{}:{} b{}:{}", info.bpp, info.pitch, info.red_size, info.red_shift, info.green_size, info.green_shift, info.blue_size, info.blue_shift);
    kprintln!("loaded drivers:");
    drivers::list();

    // beep test v2 — ngũ cung cho đỡ "dell"
    time::init();

    const NOTE_MS: u32 = 300;   // giữ nốt
    const GAP_MS:  u32 = 200;   // nghỉ giữa nốt

    /*match drivers::beep_ops() {
        // trong match drivers::beep_ops():
        Some(b) => {
            kprintln!("♪ Bad Apple!! feat. SEKAI");
            let t0 = time::uptime_ms();
            melody::play(|f| (b.beep)(f));
            kprintln!("♪ done in {}ms", time::uptime_ms() - t0);
        }
        None => kprintln!("beep: driver not loaded"),
    }*/

    video::set_fg(Color::GREEN);
    kprintln!("mounted volumes:");
    video::set_fg(Color::GRAY);
    vfs::automount();
    ls("devices://");
    cat("devices://mem");
    cat("devices://mode");

    load_symbols();

    video::set_fg(Color::YELLOW);
    kprintln!("Welcome to Hyperion!");
    video::set_fg(Color::GRAY);

    run_app_demo();

    panic_test(PANIC_TEST);
    halt()
}

#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    panic::panic(info)
}

/// Buffer tĩnh chứa kernel.sym (chưa có allocator).
static mut SYMBUF: [u8; 1 << 20] = [0; 1 << 20];

fn load_symbols() {
    let d = match drivers::debug_ops() {
        Some(d) => d,
        None => return,
    };
    let buf = unsafe { &mut *addr_of_mut!(SYMBUF) };
    let n = vfs::read("rd0://kernel.sym", 0, buf);
    if n == 0 {
        kprintln!("kdbg: rd0://kernel.sym not found");
    } else if (d.init)(buf.as_ptr(), n) {
        kprintln!("kdbg: loaded {} bytes of symbols", n);
    }
}


const BUSYBOX: &str = "rd0://bin/busybox";

fn run_cmd(path: &str, argv: &[&str]) {
    kprint!("$");
    for a in argv {
        kprint!(" {}", a);
    }
    kprintln!();
    match app::run(path, argv) {
        Ok(code) if code < 0 => kprintln!("[crashed: exception vector {}]", -code - 1),
        Ok(code) => kprintln!("[exit {}]", code),
        Err(e) => kprintln!("[load failed: {}]", e),
    }
}

/// Khoi tao pmm/paging/syscall roi chay thu hello + vai applet busybox.
fn run_app_demo() {
    let hhdm = match HHDM_REQUEST.get_response() {
        Some(r) => r.offset(),
        None => {
            kprintln!("app: no HHDM response");
            return;
        }
    };
    let mm = match MEMMAP_REQUEST.get_response() {
        Some(r) => r,
        None => {
            kprintln!("app: no memory map response");
            return;
        }
    };
    unsafe {
        pmm::init(
            hhdm,
            mm.entries()
                .iter()
                .filter(|e| e.entry_type == EntryType::USABLE)
                .map(|e| (e.base, e.length)),
        );
        paging::init();
        drivers::net_init();
        app::init();
        time::start_tick(500);
        kprintln!("preempt: timer 500Hz (user-mode)");
        let total: u64 = mm
            .entries()
            .iter()
            .filter(|e| {
                e.entry_type != EntryType::RESERVED
                    && e.entry_type != EntryType::BAD_MEMORY
                    && e.entry_type != EntryType::FRAMEBUFFER
                    && e.entry_type != EntryType::ACPI_NVS
            })
            .map(|e| e.length)
            .sum();
        pmm::set_total(total);
        mem_report();
    }

        shell::run_sh();
}

/// In kích thước dạng "4.00 GiB" / "512.25 MiB" / "64 KiB" (làm tròn, 2 chữ số thập phân).
struct Sz(u64);
impl core::fmt::Display for Sz {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let b = self.0;
        if b >= 1 << 30 {
            let c = (b * 100 + (1 << 29)) >> 30; // GiB * 100, làm tròn
            write!(f, "{}.{:02} GiB", c / 100, c % 100)
        } else if b >= 1 << 20 {
            let c = (b * 100 + (1 << 19)) >> 20;
            write!(f, "{}.{:02} MiB", c / 100, c % 100)
        } else {
            write!(f, "{} KiB", (b + 512) >> 10)
        }
    }
}

fn fmt_size(b: u64) -> Sz {
    Sz(b)
}

pub fn mem_report() {
    let t = pmm::total_bytes();
    let t_round = (t + (16 << 20) - 1) & !((16u64 << 20) - 1); // lên bội số 16 MiB
    let p = pmm::pool_bytes();
    let f = pmm::free_bytes();
    let u = p - f;
    let pct = if p > 0 { (f * 1000 / p) as u64 } else { 0 };
    kprintln!("ram: {} installed ({} usable by kernel)", fmt_size(t_round), fmt_size(t));
    kprintln!(
        "     pool {}  used {}  free {} ({}.{}%)",
        fmt_size(p),
        fmt_size(u),
        fmt_size(f),
        pct / 10,
        pct % 10
    );
}
