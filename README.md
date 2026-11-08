# Krystal-kernel

Krystal-kernel is a high-performance, bare-metal monolithic kernel written in Rust, built with modern systems programming principles and a kernel-level Zero-Trust security architecture.

## Architecture & Features

* **Zero-Trust Security Model:** Implements a granular capability-based access control system with a strict `deny-by-default` policy. Security configurations enforced via `etcetera/policy` ensure that newly spawned processes cannot elevate privileges past their parent or file definitions.
* **Vector Font & Graphics Engine:** Integrated custom TrueType font parser (`libttf`) and window management interface (`libswmapi`) handling UI rendering directly from the system layers.
* **Modular Driver Framework:** Decoupled driver architecture with interfaces for network adapters (`e1000`), audio controllers (`ac97`, `hda`), and standard HID peripherals (keyboard, mouse).
* **VFS & Storage:** In-kernel support for `tarfs` to load initial boot resources and a dedicated `panicfs` for failure containment and diagnostic logging.
* **Boot Pipeline:** Powered by the Limine Bootloader, leveraging UEFI/BIOS hybrid initialization pipelines.

## Project Structure

* `kernel/` - Core kernel logic (memory management, scheduling, IPC, process lifecycle).
* `drivers/` - Modular hardware drivers (ac97, hda, e1000, keyboard, mouse, serial, beep).
* `libgfx/` & `libttf/` - Vector graphics layout and font rendering engine.
* `libswmapi/` & `apps/` - Window manager interfaces and userspace binaries (SWM, DM, monitor, taskmgr, term, mediaplayer, etc.).
* `etcetera/policy` - Capabilities and permission management configurations.

## Building and Running

### Prerequisites

* Rust nightly toolchain with `x86_64-unknown-none` target setup.
* GNU Make.
* QEMU for emulation (requires `edk2-x86_64-code.fd` for UEFI targets).
* MSYS2 / Cygwin environment (if building on Windows, required for `cygpath` and bash staging scripts).

### Commands

```bash
# Clean workspace build artifacts
make clean

# Compile kernel, sys-drivers, and build userspace applications into initrd
make

# Launch QEMU configurations:
make run           # Default target (alias for run-uefi)
make run-uefi      # Boot using UEFI mode with standard AC97 audio emulation
make run-bios      # Boot using Legacy BIOS mode with standard AC97 audio emulation
make run-hda       # Boot using UEFI mode with Intel HD Audio emulation
make run-hda-bios  # Boot using Legacy BIOS mode with Intel HD Audio emulation
```

Each execution target automatically configures user-mode networking via an e1000 adapter, routes PC speaker audio, maps standard system diagnostics to standard I/O (`stdio`), and mounts the built filesystem workspace.
