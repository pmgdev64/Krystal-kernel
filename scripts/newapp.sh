#!/bin/bash
# Dung: bash scripts/newapp.sh <ten>   -> tao apps/<ten> (cua so swm) va them vao Makefile
set -e
n=$1
[ -n "$n" ] || { echo "dung: newapp.sh <ten>"; exit 1; }
cd "$(dirname "$0")/.."
[ -d apps/wapp ] || { echo "thieu apps/wapp"; exit 1; }
if [ ! -d "apps/$n" ]; then
  mkdir -p "apps/$n/src"
  ( cd apps/wapp && for f in $(ls -A | grep -v -E '^(src|target|Cargo.lock)$|\.elf$|\.bak'); do cp -r "$f" "../$n/"; done )
  grep -rl wapp "apps/$n" 2>/dev/null | xargs -r sed -i "s/wapp/$n/g"
fi
if [ ! -f "apps/$n/src/main.rs" ]; then
cat > "apps/$n/src/main.rs" <<'EOT'
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
EOT
fi
if ! grep -q "apps/$n/build.sh" Makefile; then
  sed -i "\#^> bash apps/wapp/build.sh#a > bash apps/$n/build.sh" Makefile
  sed -i "\#^> cp apps/wapp/wapp.elf initrd-root/apps/wapp.elf#a > cp apps/$n/$n.elf initrd-root/apps/$n.elf" Makefile
  sed -i "\#^> cd apps/wapp && cargo clean#a > cd apps/$n \&\& cargo clean" Makefile
fi
echo "da tao apps/$n"
