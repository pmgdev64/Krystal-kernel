//! TTY console: stdin lấy từ keyboard.sys (và COM1), line discipline kiểu canonical.
use crate::drivers;
use crate::paging::{self, Space};
use core::arch::asm;
use core::ptr::addr_of_mut;

const LN: usize = 256;
static mut LINE: [u8; LN] = [0; LN];
static mut LEN: usize = 0;
static mut POS: usize = 0;
pub static mut ICANON: bool = true;

unsafe fn inb(p: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", in("dx") p, out("al") v, options(nomem, nostack));
    v
}

/// Chờ 1 ký tự (polling): bàn phím PS/2 hoặc COM1.
fn getc() -> u8 {
    loop {
        if let Some(m) = drivers::mouse_ops() {
            (m.poll)();
        }
        if let (Some(k), true) = (drivers::keyboard_ops(), kbd_ready()) {
            let c = (k.poll)();
            if c >= 0 {
                return c as u8;
            }
        }
        unsafe {
            if inb(0x3FD) & 1 != 0 {
                return match inb(0x3F8) {
                    b'\r' => b'\n',
                    0x7f => 0x08,
                    b => b,
                };
            }
        }
        crate::task::sleep(1);
        core::hint::spin_loop();
    }
}

/// read(0, buf, count): trả về khi đủ 1 dòng. Ctrl+D ở đầu dòng = EOF (trả 0).
pub unsafe fn read(sp: &Space, buf: u64, count: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    let line = &mut *addr_of_mut!(LINE);
    if !ICANON {
        let c = getc();
        if !paging::copy_to_user(sp, buf, &[c], true) {
            return (-14i64) as u64;
        }
        return 1;
    }
    if POS >= LEN {
        LEN = 0;
        POS = 0;
        loop {
            let c = getc();
            match c {
                b'\n' => {
                    crate::kprint!("\n");
                    line[LEN] = b'\n';
                    LEN += 1;
                    break;
                }
                0x08 => {
                    if LEN > 0 {
                        LEN -= 1;
                        crate::kprint!("\x08 \x08");
                    }
                }
                4 => {
                    if LEN == 0 {
                        return 0;
                    }
                    break;
                }
                3 => {
                    crate::kprint!("^C\n");
                    LEN = 0;
                }
                0x20..=0x7e | b'\t' => {
                    if LEN < LN - 1 {
                        line[LEN] = c;
                        LEN += 1;
                        crate::kprint!("{}", c as char);
                    }
                }
                _ => {}
            }
        }
    }
    let n = (count as usize).min(LEN - POS);
    if !paging::copy_to_user(sp, buf, &line[POS..POS + n], true) {
        return (-14i64) as u64; // EFAULT
    }
    POS += n;
    n as u64
}

/// Đọc 1 dòng (echo, Backspace, Ctrl+C hủy dòng) cho shell của kernel.
pub fn read_line(out: &mut [u8]) -> usize {
    let mut n = 0;
    loop {
        let c = getc();
        match c {
            b'\n' => {
                crate::kprint!("\n");
                return n;
            }
            0x08 => {
                if n > 0 {
                    n -= 1;
                    crate::kprint!("\x08 \x08");
                }
            }
            3 => {
                crate::kprint!("^C\n");
                return 0;
            }
            0x20..=0x7e => {
                if n < out.len() {
                    out[n] = c;
                    n += 1;
                    crate::kprint!("{}", c as char);
                }
            }
            _ => {}
        }
    }
}

fn kbd_ready() -> bool {
    unsafe { inb(0x64) & 0x21 == 0x01 }
}
