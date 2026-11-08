#!/usr/bin/env bash
set -e
cd "$(dirname "$0")"
RUSTFLAGS="-C link-arg=-Tlinker.ld -C relocation-model=static -C code-model=small" \
    cargo build --release --target x86_64-unknown-none
cp target/x86_64-unknown-none/release/dm dm.elf
echo "OK -> apps/dm/dm.elf"
