.RECIPEPREFIX = >

TARGET   := x86_64-unknown-none
KERNEL   := kernel/target/$(TARGET)/debug/kernel
BOOTVID  := bootvid/target/$(TARGET)/debug/bootvid
SERIAL   := serial/target/$(TARGET)/debug/serial
KEYBOARD := keyboard/target/$(TARGET)/debug/keyboard
BEEP     := beep/target/$(TARGET)/debug/beep
AC97_    := ac97/target/$(TARGET)/debug/ac97
HDA_     := hda/target/$(TARGET)/debug/hda
GFX     := gfx/target/$(TARGET)/debug/gfx
MOUSE    := mouse/target/$(TARGET)/debug/mouse
E1000    := e1000/target/$(TARGET)/debug/e1000
NET      := -device e1000,netdev=n0 -netdev user,id=n0
TARFS    := tarfs/target/$(TARGET)/debug/tarfs
KDBG     := kdbg/target/$(TARGET)/debug/kdbg
PANICFS := panicfs/target/$(TARGET)/debug/panicfs

# ---------- Input Assets / Resource Paths ----------
RES_DIR     := res
BACKGROUND  := $(RES_DIR)/bootsplash.bmp

# ---------- Output paths ----------
DIST     := dist
ESP      := esp
ISO_ROOT := iso_root
ISO      := $(DIST)/Krystal-0.0.1-x64.iso

# ---------- Limine assets ----------
LIMINE_DIR      := limine
LIMINE_EXE      := $(LIMINE_DIR)/limine.exe
LIMINE_BOOTX64  := $(LIMINE_DIR)/BOOTX64.EFI
LIMINE_BIOS_CD  := $(LIMINE_DIR)/limine-bios-cd.bin
LIMINE_UEFI_CD  := $(LIMINE_DIR)/limine-uefi-cd.bin
LIMINE_BIOS_SYS := $(LIMINE_DIR)/limine-bios.sys

OVMF := /mingw64/share/qemu/edk2-x86_64-code.fd

# Hàm chuyển đổi đường dẫn sang dạng Windows (C:/path/to/file) cho QEMU, xorriso và limine.exe
winpath = $(shell cygpath -m $(1))

.PHONY: all build stage esp iso run run-uefi run-bios clean

all: iso

# ============================================================
# BUILD
# ============================================================
build:
> cd bootvid && cargo build --target $(TARGET)
> cd serial && cargo build --target $(TARGET)
> cd keyboard && cargo build --target $(TARGET)
> cd beep && cargo build --target $(TARGET)
> cd ac97 && cargo build --target $(TARGET)
> cd hda && cargo build --target $(TARGET)
> cd gfx && cargo build --target $(TARGET)
> cd mouse && cargo build --target $(TARGET)
> cd e1000 && cargo build --target $(TARGET)
> cd tarfs && cargo build --target $(TARGET)
> cd kdbg && cargo build --target $(TARGET)
# > cd panicfs && cargo build --target $(TARGET)
> cd kernel && cargo build --target $(TARGET)
# > cd panicfs && cargo build --target $(TARGET)
> bash apps/hello/build.sh
> bash apps/gfxdemo/build.sh
> bash apps/wapp/build.sh
> bash apps/term/build.sh
> bash apps/imgview/build.sh
> bash apps/monitor/build.sh
> bash apps/swm/build.sh
> bash apps/dm/build.sh
> bash apps/mediaplayer/build.sh
> bash apps/libstd/build.sh

# Staging chung cho initrd
stage: build
> mkdir -p initrd-root/apps
> nm -n -C --defined-only $(KERNEL) | grep -E ' [Tt] ' > initrd-root/kernel.sym
> cp apps/hello/hello.elf initrd-root/apps/hello.elf
> cp apps/gfxdemo/gfxdemo.elf initrd-root/apps/gfxdemo.elf
> cp apps/wapp/wapp.elf initrd-root/apps/wapp.elf
> cp apps/term/term.elf initrd-root/apps/term.elf
> cp apps/imgview/imgview.elf initrd-root/apps/imgview.elf
> cp apps/monitor/monitor.elf initrd-root/apps/monitor.elf
> cp apps/swm/swm.elf initrd-root/apps/swm.elf
> cp apps/dm/dm.elf initrd-root/apps/dm.elf
> cp apps/mediaplayer/mediaplayer.elf initrd-root/apps/mediaplayer.elf
> mkdir -p initrd-root/lib
> cp apps/libstd/libstd.so initrd-root/lib/libstd.so

# ============================================================
# ESP (FAT) — test nhanh UEFI (Không dùng cho ISO)
# ============================================================
esp: stage
> rm -rf $(ESP)
> mkdir -p $(ESP)/EFI/BOOT $(ESP)/boot $(ESP)/boot/res $(ESP)/boot/limine
> cp $(LIMINE_BOOTX64) $(ESP)/EFI/BOOT/BOOTX64.EFI
# Copy file cấu hình vào cả 3 vị trí chiến lược của ESP để chống lỗi quét hụt
> cp limine.conf $(ESP)/limine.conf
> cp limine.conf $(ESP)/boot/limine.conf
> cp limine.conf $(ESP)/boot/limine/limine.conf
# Sửa lỗi xuống dòng CRLF cho toàn bộ file config vừa copy
> sed -i 's/\r$$//' $(ESP)/limine.conf
> sed -i 's/\r$$//' $(ESP)/boot/limine.conf
> sed -i 's/\r$$//' $(ESP)/boot/limine/limine.conf
> cp $(BACKGROUND) $(ESP)/boot/res/bootsplash.bmp
> cp $(KERNEL)   $(ESP)/boot/kernel
> cp $(BOOTVID)  $(ESP)/boot/bootvid.sys
> cp $(SERIAL)   $(ESP)/boot/serial.sys
> cp $(KEYBOARD) $(ESP)/boot/keyboard.sys
> cp $(BEEP) $(ESP)/boot/beep.sys
> cp $(AC97_) $(ESP)/boot/ac97.sys
> cp $(HDA_) $(ESP)/boot/hda.sys
> cp $(GFX) $(ESP)/boot/gfx.sys
> cp $(MOUSE) $(ESP)/boot/mouse.sys
> cp $(E1000) $(ESP)/boot/e1000.sys
> cp $(KDBG)     $(ESP)/boot/kdbg.sys
> cp $(TARFS)    $(ESP)/boot/tarfs.sys
# > cp $(PANICFS)    $(ESP)/boot/panicfs.sys
> tar --format=ustar -cf $(ESP)/boot/initrd.tar -C initrd-root .

# ============================================================
# HYBRID ISO — BIOS + UEFI
# ============================================================
iso: stage
> rm -rf $(ISO_ROOT)
> mkdir -p $(ISO_ROOT)/boot $(ISO_ROOT)/boot/res $(ISO_ROOT)/boot/limine $(ISO_ROOT)/EFI/BOOT $(DIST)
> cp $(LIMINE_BOOTX64)  $(ISO_ROOT)/EFI/BOOT/BOOTX64.EFI
> cp $(LIMINE_BIOS_CD)  $(ISO_ROOT)/boot/limine-bios-cd.bin
> cp $(LIMINE_UEFI_CD)  $(ISO_ROOT)/boot/limine-uefi-cd.bin
> cp $(LIMINE_BIOS_SYS) $(ISO_ROOT)/boot/limine-bios.sys
# Copy file cấu hình vào cả 3 vị trí chiến lược của ISO để buộc Limine phải đọc bản mới nhất
> cp limine.conf        $(ISO_ROOT)/limine.conf
> cp limine.conf        $(ISO_ROOT)/boot/limine.conf
> cp limine.conf        $(ISO_ROOT)/boot/limine/limine.conf
# Thực hiện xóa sạch ký tự \r ẩn gây lỗi parse tệp tin trên Windows
> sed -i 's/\r$$//' $(ISO_ROOT)/limine.conf
> sed -i 's/\r$$//' $(ISO_ROOT)/boot/limine.conf
> sed -i 's/\r$$//' $(ISO_ROOT)/boot/limine/limine.conf
> cp $(BACKGROUND)     $(ISO_ROOT)/boot/res/bootsplash.bmp
> cp $(KERNEL)   $(ISO_ROOT)/boot/kernel
> cp $(BOOTVID)  $(ISO_ROOT)/boot/bootvid.sys
> cp $(SERIAL)   $(ISO_ROOT)/boot/serial.sys
> cp $(KEYBOARD) $(ISO_ROOT)/boot/keyboard.sys
> cp $(BEEP) $(ISO_ROOT)/boot/beep.sys
> cp $(AC97_) $(ISO_ROOT)/boot/ac97.sys
> cp $(HDA_) $(ISO_ROOT)/boot/hda.sys
> cp $(GFX) $(ISO_ROOT)/boot/gfx.sys
> cp $(MOUSE) $(ISO_ROOT)/boot/mouse.sys
> cp $(E1000) $(ISO_ROOT)/boot/e1000.sys
> cp $(KDBG)     $(ISO_ROOT)/boot/kdbg.sys
> cp $(TARFS)    $(ISO_ROOT)/boot/tarfs.sys
# > cp $(PANICFS)    $(ISO_ROOT)/boot/panicfs.sys
> tar --format=ustar -cf $(ISO_ROOT)/boot/initrd.tar -C initrd-root .
# Tạo El Torito + ESP + MBR bảo vệ (Dùng winpath để tránh lỗi MSYS2)
> xorriso -as mkisofs \
>   -b boot/limine-bios-cd.bin \
>   -no-emul-boot -boot-load-size 4 -boot-info-table \
>   --efi-boot boot/limine-uefi-cd.bin \
>   -efi-boot-part --efi-boot-image --protective-msdos-label \
>   "$(call winpath,$(ISO_ROOT))" -o "$(call winpath,$(ISO))"
# Ghi stage2 BIOS vào MBR của ISO (Gọi trực tiếp file exe)
> $(LIMINE_EXE) bios-install "$(call winpath,$(ISO))"

# ============================================================
# RUN
# ============================================================
run: run-uefi

run-uefi: iso
> qemu-system-x86_64 \
>   -machine q35 -m 4096M \
>   $(NET) \
>   -drive if=pflash,format=raw,readonly=on,file=$(call winpath,$(OVMF)) \
>   -cdrom "$(call winpath,$(ISO))" \
>   -audiodev dsound,id=audio0 -machine pcspk-audiodev=audio0 -device AC97,audiodev=audio0 \
>   -serial stdio

run-bios: iso
> qemu-system-x86_64 \
>   -machine q35 -m 4096M \
>   $(NET) \
>   -cdrom "$(call winpath,$(ISO))" \
>   -audiodev dsound,id=audio0 -machine pcspk-audiodev=audio0 -device AC97,audiodev=audio0 \
>   -serial stdio

# ============================================================
# CLEAN
# ============================================================
clean:
> cd kernel && cargo clean
> cd bootvid && cargo clean
> cd serial && cargo clean
> cd keyboard && cargo clean
> cd beep && cargo clean
> cd ac97 && cargo clean
> cd hda && cargo clean
> cd gfx && cargo clean
> cd mouse && cargo clean
> cd e1000 && cargo clean
> cd tarfs && cargo clean
> cd kdbg && cargo clean
# > cd panicfs && cargo clean
> cd apps/hello && cargo clean
> cd apps/gfxdemo && cargo clean
> cd apps/wapp && cargo clean
> cd apps/term && cargo clean
> cd apps/imgview && cargo clean
> cd apps/monitor && cargo clean
> cd apps/swm && cargo clean
> cd apps/dm && cargo clean
> cd apps/mediaplayer && cargo clean
> rm -rf $(ESP) $(ISO_ROOT) $(DIST) initrd-root/kernel.sym initrd-root/apps

# ---- chay thu Intel HD Audio tren QEMU (VirtualBox: Audio Controller = Intel HD Audio) ----
.PHONY: run-hda run-hda-bios
run-hda: iso
> qemu-system-x86_64 \
>   -machine q35 -m 4096M \
>   $(NET) \
>   -drive if=pflash,format=raw,readonly=on,file=$(call winpath,$(OVMF)) \
>   -cdrom "$(call winpath,$(ISO))" \
>   -audiodev dsound,id=audio0 -machine pcspk-audiodev=audio0 -device intel-hda -device hda-duplex,audiodev=audio0 \
>   -serial stdio

run-hda-bios: iso
> qemu-system-x86_64 \
>   -machine q35 -m 4096M \
>   $(NET) \
>   -cdrom "$(call winpath,$(ISO))" \
>   -audiodev dsound,id=audio0 -machine pcspk-audiodev=audio0 -device intel-hda -device hda-duplex,audiodev=audio0 \
>   -serial stdio
