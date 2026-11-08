#!/usr/bin/env bash
# scripts/new-driver.sh <name> <KIND_CONST> [template]
#   vd: ./scripts/new-driver.sh ac97 KIND_AUDIO
#       ./scripts/new-driver.sh myfs KIND_FS tarfs
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")/.."

if [ $# -lt 2 ]; then
    sed -n 's/^# \{0,1\}//p' "$0" | head -n 6
    exit 1
fi

NAME="$1"; KIND="$2"; TEMPLATE="${3:-beep}"
UPPER=$(echo "$NAME" | tr '[:lower:]' '[:upper:]' | tr -c 'A-Z0-9' '_')

[ -d "$NAME" ] && { echo "✗ '$NAME/' đã tồn tại" >&2; exit 1; }
[ ! -f "$TEMPLATE/Cargo.toml" ] && { echo "✗ không có '$TEMPLATE/Cargo.toml'" >&2; exit 1; }

echo "→ scaffold '$NAME' từ '$TEMPLATE' (kind=$KIND)"
cp -r "$TEMPLATE" "$NAME"
rm -rf "$NAME/target"
sed -i "s/\"$TEMPLATE\"/\"$NAME\"/g" "$NAME/Cargo.toml"

cat > "$NAME/src/lib.rs" <<EOF
//! Driver $NAME (kind=$KIND).
#![no_std]

use core::ffi::c_void;
use core::ptr::null;
use driver_abi::*;

static mut API: *const KernelApi = null();

unsafe fn log(s: &str) {
    if !API.is_null() { ((*API).log)(s.as_ptr(), s.len()); }
}

unsafe fn hw_init() -> bool { true /* TODO */ }

#[no_mangle]
pub extern "C" fn driver_entry(api: *const KernelApi) -> i32 {
    unsafe {
        if (*api).abi_version != DRIVER_ABI { return -1; }
        API = api;
        if !hw_init() { log("$NAME: init failed"); return -2; }
        log("$NAME: ready");
        let info = DriverInfo {
            name: name("$NAME"),
            kind: $KIND,
            ops: null(), // TODO: &OPS as *const _ as *const c_void
        };
        ((*api).register)(&info)
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
EOF
echo "  ✓ $NAME/Cargo.toml"
echo "  ✓ $NAME/src/lib.rs"

PY=""
command -v python3 >/dev/null 2>&1 && PY=python3
[ -z "$PY" ] && command -v python >/dev/null 2>&1 && PY=python

if [ -n "$PY" ]; then
    cp Makefile Makefile.bak
    "$PY" - "$NAME" "$UPPER" <<'PYEOF'
import sys, re
name, upper = sys.argv[1], sys.argv[2]
raw = open('Makefile', 'rb').read()
crlf = b'\r\n' in raw
text = raw.decode('utf-8').replace('\r\n', '\n')
patches = [
    (r'(?m)^BEEP\s*:=\s*\S+$',
     f'{upper:<8} := {name}/target/$(TARGET)/debug/{name}'),
    (r'(?m)^> cd beep && cargo build --target \$\(TARGET\)$',
     f'> cd {name} && cargo build --target $(TARGET)'),
    (r'(?m)^> cp \$\(BEEP\) \$\(ESP\)/boot/beep\.sys$',
     f'> cp $({upper}) $(ESP)/boot/{name}.sys'),
    (r'(?m)^> cp \$\(BEEP\) \$\(ISO_ROOT\)/boot/beep\.sys$',
     f'> cp $({upper}) $(ISO_ROOT)/boot/{name}.sys'),
    (r'(?m)^> cd beep && cargo clean$',
     f'> cd {name} && cargo clean'),
]
for pat, add in patches:
    new = re.sub(pat, lambda m: m.group(0) + '\n' + add, text, count=1)
    if new == text:
        print(f'  ⚠ pattern không khớp: {pat}', file=sys.stderr)
    text = new
if crlf:
    text = text.replace('\n', '\r\n')
open('Makefile', 'wb').write(text.encode('utf-8'))
print('  ✓ Makefile patched (backup: Makefile.bak)')
PYEOF
else
    echo "  ⚠ không có Python — chèn tay:" >&2
    echo "      $UPPER := $NAME/target/\$(TARGET)/debug/$NAME" >&2
    echo "      > cd $NAME && cargo build --target \$(TARGET)" >&2
    echo "      > cp \$($UPPER) \$(ESP)/boot/$NAME.sys" >&2
    echo "      > cp \$($UPPER) \$(ISO_ROOT)/boot/$NAME.sys" >&2
    echo "      > cd $NAME && cargo clean" >&2
fi
