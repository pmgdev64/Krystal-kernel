#![no_std]
#![no_main]

use core::arch::asm;
use core::fmt::Write;
use core::panic::PanicInfo;

const SYS_READ: u64 = 0;
const SYS_WRITE: u64 = 1;
const SYS_OPEN: u64 = 2;
const SYS_CLOSE: u64 = 3;
const SYS_FORK: u64 = 57;
const SYS_EXECVE: u64 = 59;
const SYS_EXIT: u64 = 60;
const SYS_WAIT4: u64 = 61;

/// true: dùng ESC[2J ESC[H để xóa màn hình. Nếu console của bạn hiện rác thì đổi thành false.
const ANSI_CLEAR: bool = true;

const RESET: &str = "\x1b[0m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const WHITE: &str = "\x1b[37m";

unsafe fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    asm!(
        "syscall",
        inlateout("rax") nr => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack),
    );
    ret
}

unsafe fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    syscall4(nr, a1, a2, a3, 0)
}

fn write(s: &str) {
    unsafe { syscall3(SYS_WRITE, 1, s.as_ptr() as u64, s.len() as u64) };
}

struct Out;
impl Write for Out {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write(s);
        Ok(())
    }
}

macro_rules! print {
    ($($a:tt)*) => {{ let _ = write!(Out, $($a)*); }};
}

macro_rules! println {
    ($($a:tt)*) => {{ let _ = writeln!(Out, $($a)*); }};
}

fn exit(code: u64) -> ! {
    unsafe { syscall3(SYS_EXIT, code, 0, 0) };
    loop {}
}

/// `path` phải kết thúc bằng NUL.
fn read_file(path: &[u8], buf: &mut [u8]) -> Option<usize> {
    unsafe {
        let fd = {
            let mut r = syscall3(SYS_OPEN, path.as_ptr() as u64, 0, 0);
            if (r as i64) < 0 && path[0] == b'/' && path.len() + 5 <= 96 {
                let mut alt = [0u8; 96];
                alt[..5].copy_from_slice(b"rd0:/");
                alt[5..5 + path.len()].copy_from_slice(path);
                r = syscall3(SYS_OPEN, alt.as_ptr() as u64, 0, 0);
            }
            r
        };
        if (fd as i64) < 0 {
            return None;
        }
        let n = syscall3(SYS_READ, fd, buf.as_mut_ptr() as u64, buf.len() as u64);
        syscall3(SYS_CLOSE, fd, 0, 0);
        if (n as i64) < 0 {
            None
        } else {
            Some(n as usize)
        }
    }
}

/// Đọc 1 dòng từ stdin (tty canonical), bỏ '\n'. None nếu EOF (Ctrl+D).
fn read_line(buf: &mut [u8]) -> Option<usize> {
    let n = unsafe { syscall3(SYS_READ, 0, buf.as_mut_ptr() as u64, buf.len() as u64) } as i64;
    if n <= 0 {
        return None;
    }
    let mut n = n as usize;
    while n > 0 && (buf[n - 1] == b'\n' || buf[n - 1] == b'\r') {
        n -= 1;
    }
    Some(n)
}

fn clear() {
    if ANSI_CLEAR {
        write("\x1b[2J\x1b[H");
    } else {
        for _ in 0..45 {
            write("\n");
        }
    }
}

fn banner() {
    clear();
    print!("{}", BLUE);
    println!("  +------------------------------------------------+");
    println!("  |                                                |");
    print!("  |   {}M Y O S   D I S P L A Y   M A N A G E R{}      |\n", YELLOW, BLUE);
    println!("  |                                                |");
    println!("  +------------------------------------------------+");
    print!("{}\n", RESET);
}

/// fork + execve + wait4. Trả về status của wait4 (hoặc -1 nếu fork lỗi).
fn spawn(path: &[u8], args: &[&[u8]], envp: &[u64]) -> i64 {
    let mut argv = [0u64; 8];
    for (i, a) in args.iter().enumerate().take(7) {
        argv[i] = a.as_ptr() as u64;
    }
    unsafe {
        let pid = syscall3(SYS_FORK, 0, 0, 0) as i64;
        if pid == 0 {
            syscall3(SYS_EXECVE, path.as_ptr() as u64, argv.as_ptr() as u64, envp.as_ptr() as u64);
            if path[0] == b'/' && path.len() + 5 <= 96 {
                let mut alt = [0u8; 96];
                alt[..5].copy_from_slice(b"rd0:/");
                alt[5..5 + path.len()].copy_from_slice(path);
                syscall3(SYS_EXECVE, alt.as_ptr() as u64, argv.as_ptr() as u64, envp.as_ptr() as u64);
            }
            exit(127);
        }
        if pid < 0 {
            return -1;
        }
        let mut status: i32 = 0;
        let r = syscall4(SYS_WAIT4, pid as u64, &mut status as *mut i32 as u64, 0, 0) as i64;
        if r < 0 {
            return -2;
        }
        core::ptr::read_volatile(&status) as i64
    }
}

fn report(status: i64) {
    if status == -1 {
        println!("{}[fork failed]{}", RED, RESET);
    } else if status == -2 {
        println!("{}[wait4 failed]{}", RED, RESET);
    } else if status & 0x7f != 0 {
        println!("{}[session killed by signal {}]{}", RED, status & 0x7f, RESET);
    } else {
        let code = (status >> 8) & 0xff;
        if code == 127 {
            println!("{}[cannot execute program (exit 127)]{}", RED, RESET);
        } else if code != 0 {
            println!("{}[session exited with code {}]{}", YELLOW, code, RESET);
        }
    }
}

fn pause() {
    print!("\n{}Enter de quay lai menu...{}", WHITE, RESET);
    let mut b = [0u8; 8];
    let _ = read_line(&mut b);
}

fn main() {
    let mut user = [0u8; 48];
    // "USER=" + tên + NUL
    user[..5].copy_from_slice(b"USER=");

    loop {
        // ---------- login ----------
        banner();
        print!("{}login:{} ", GREEN, RESET);
        let mut name = [0u8; 32];
        let n = match read_line(&mut name) {
            Some(n) => n,
            None => continue,
        };
        let nm: &[u8] = if n == 0 { b"root" } else { &name[..n] };
        let nm = if nm.len() > 30 { &nm[..30] } else { nm };
        user[5..5 + nm.len()].copy_from_slice(nm);
        user[5 + nm.len()] = 0;
        let uname = core::str::from_utf8(nm).unwrap_or("?");

        let envp: [u64; 6] = [
            user.as_ptr() as u64,
            b"HOME=/\0".as_ptr() as u64,
            b"PATH=/bin:/apps:/usr/bin\0".as_ptr() as u64,
            b"TERM=linux\0".as_ptr() as u64,
            b"PS1=\\u@myos:\\w# \0".as_ptr() as u64,
            0,
        ];

        // ---------- menu ----------
        loop {
            banner();
            println!("  Xin chao, {}{}{}!\n", YELLOW, uname, RESET);
            println!("  {}1{}) Shell (busybox sh)", GREEN, RESET);
            println!("  {}2{}) Chay hello", GREEN, RESET);
            println!("  {}3{}) Doc /hello.txt", GREEN, RESET);
            println!("  {}4{}) Graphics demo (libgfx)", GREEN, RESET);
            println!("  {}5{}) Log out\n", GREEN, RESET);
            print!("  chon [1-5]: ");

            let mut c = [0u8; 8];
            let k = match read_line(&mut c) {
                Some(k) if k > 0 => c[0],
                Some(_) => continue,
                None => break,
            };

            match k {
                b'1' => {
                    clear();
                    let st = spawn(b"/bin/busybox\0", &[b"busybox\0", b"sh\0"], &envp);
                    if st < 0 || st & 0x7f != 0 || ((st >> 8) & 0xff) == 127 {
                        report(st);
                    }
                    pause();
                }
                b'2' => {
                    println!();
                    let st = spawn(b"/apps/hello.elf\0", &[b"hello\0"], &envp);
                    report(st);
                    pause();
                }
                b'3' => {
                    println!();
                    let mut buf = [0u8; 200];
                    match read_file(b"/hello.txt\0", &mut buf) {
                        Some(n) => println!(
                            "  hello.txt ({} bytes): {}",
                            n,
                            core::str::from_utf8(&buf[..n]).unwrap_or("?").trim_end()
                        ),
                        None => println!("  {}cannot read /hello.txt{}", RED, RESET),
                    }
                    pause();
                }
                b'4' => {
                    clear();
                    let st = spawn(b"/apps/gfxdemo.elf\0", &[b"gfxdemo\0"], &envp);
                    report(st);
                    pause();
                }
                b'5' => break,
                _ => {
                    println!("  {}lua chon khong hop le{}", RED, RESET);
                    pause();
                }
            }
        }
        println!("\n{}Logged out.{}", GREEN, RESET);
    }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    exit(101)
}

core::arch::global_asm!(
    ".globl _start",
    "_start:",
    "xor ebp, ebp",
    "mov rdi, rsp",
    "and rsp, -16",
    "call __myos_start",
    "ud2",
);

#[no_mangle]
extern "C" fn __myos_start(_sp: *const u64) -> ! {
    main();
    exit(0)
}
