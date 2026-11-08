#!/bin/bash
set -e
cd "$(dirname "$0")"
cargo build --release --target x86_64-unknown-none
SYS=$(rustc --print sysroot)
HOST=$(rustc -vV | sed -n 's/host: //p')
LLD="$SYS/lib/rustlib/$HOST/bin/rust-lld"
"$LLD" -flavor gnu -shared -soname=libstd.so --hash-style=both --gc-sections \
  --whole-archive target/x86_64-unknown-none/release/libstdlib.a --no-whole-archive \
  -o libstd.so
echo "OK -> apps/libstd/libstd.so ($(nm -D libstd.so 2>/dev/null | grep -c ' T ') ham export)"
