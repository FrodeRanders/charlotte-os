/* Place _start at offset 0 in the binary so the kernel can jump to the
 * load address directly without computing an entry-point offset. */
SECTIONS
{
    . = 0;
    .text : {
        KEEP(*(.text._start .text._start.*))
        *(.text .text.*)
    }
    .rodata : { *(.rodata .rodata.*) }
    .data.rel.ro : { *(.data.rel.ro .data.rel.ro.*) }
    .data : { *(.data .data.*) }
    .bss : { *(.bss .bss.*) *(COMMON) }
    /DISCARD/ : { *(.eh_frame .eh_frame_hdr) }
}
