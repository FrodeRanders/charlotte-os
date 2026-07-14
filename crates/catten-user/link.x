/* Keep user permissions page-granular: executable text, read-only constants,
 * and writable data/BSS must not share a 4 KiB page. The CharlotteOS loader
 * consumes PT_LOAD program headers directly and rejects overlapping LOAD pages
 * instead of manufacturing RWX mappings. */
PHDRS
{
    text PT_LOAD FLAGS(5);   /* PF_R | PF_X */
    rodata PT_LOAD FLAGS(4); /* PF_R */
    data PT_LOAD FLAGS(6);   /* PF_R | PF_W */
}

SECTIONS
{
    . = 0x20000;
    .text : {
        KEEP(*(.text._start .text._start.*))
        *(.text .text.*)
    } :text

    . = ALIGN(0x1000);
    .rodata : { *(.rodata .rodata.*) } :rodata
    .data.rel.ro : { *(.data.rel.ro .data.rel.ro.*) } :rodata

    . = ALIGN(0x1000);
    .data : { *(.data .data.*) } :data
    .bss : { *(.bss .bss.*) *(COMMON) } :data
    /DISCARD/ : { *(.eh_frame .eh_frame_hdr) }
}
