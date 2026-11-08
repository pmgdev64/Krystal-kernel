#![no_std]
#![no_main]

use libswmapi::{rgb, Event, Window};

libswmapi::entry!(main);

fn main() {
    let mut win = match Window::create(320, 220, "App") {
        Ok(w) => w,
        Err(_) => libswmapi::exit(1),
    };
    {
        let mut c = win.canvas();
        c.clear(rgb(236, 236, 242));
        c.text(10, 10, "Hello", rgb(0, 0, 0), 2);
    }
    let _ = win.present_all();
    loop {
        while let Some(e) = win.poll_event() {
            if let Event::Close = e {
                return;
            }
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    libswmapi::exit(101)
}
