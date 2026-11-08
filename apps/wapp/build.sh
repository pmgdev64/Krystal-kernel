#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --target x86_64-unknown-none
cp target/x86_64-unknown-none/release/wapp wapp.elf
echo "OK -> apps/wapp/wapp.elf"
