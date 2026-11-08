OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(kmain)

PHDRS
{
    requests PT_LOAD;
    text     PT_LOAD;
    rodata   PT_LOAD;
    data     PT_LOAD;
    driver   PT_LOAD FLAGS(7);
}

SECTIONS
{
    . = 0xffffffff80000000;

    .requests : {
        KEEP(*(.requests_start_marker))
        KEEP(*(.requests))
        KEEP(*(.requests_end_marker))
    } :requests

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .text : { *(.text .text.*) } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .rodata : { *(.rodata .rodata.*) } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .data : { *(.data .data.*) *(.sdata .sdata.*) } :data
    .bss : { *(.bss .bss.*) *(.sbss .sbss.*) *(COMMON) } :data

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .driver_area : { KEEP(*(.driver_area)) } :driver

    /DISCARD/ : { *(.eh_frame*) *(.note .note.*) }
}
