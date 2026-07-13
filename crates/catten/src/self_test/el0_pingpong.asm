.section .rodata.el0_pingpong, "a"
.balign 4

.global __catten_el0_ping_start
.global __catten_el0_ping_end
.global __catten_el0_pong_start
.global __catten_el0_pong_end

__catten_el0_ping_start:
    // result page = 0x15000
    movz x9, #0x5000
    movk x9, #0x1, lsl #16
    ldr w20, [x9, #16]

    mov x0, x20
    mov x1, #0
    svc #1
    mov x19, x0

    mov x0, x20
    mov x1, #1
    svc #13
    mov x18, x0

    mov x0, x20
    mov x1, x18
    mov x2, x19
    svc #15

    mov x0, x20
    mov x1, x19
    movz x2, #60000
    svc #11
    cbz x0, 1f

    movz x9, #0x5000
    movk x9, #0x1, lsl #16
    movz w10, #0xdead
    str w10, [x9]
    b 2f

1:
    mov x11, x1
    nop
    nop
    nop
    nop

    movz x9, #0x5000
    movk x9, #0x1, lsl #16
    movz w10, #0x1500
    movk w10, #0x9100, lsl #16
    str w19, [x9, #4]
    str w11, [x9, #8]
    dmb ish
    str w10, [x9]

2:
    svc #8
__catten_el0_ping_end:

__catten_el0_pong_start:
    // result page = 0x15000
    movz x9, #0x5000
    movk x9, #0x1, lsl #16
    ldr w20, [x9, #16]

    mov x0, x20
    svc #14
    mov x18, x0

1:
    mov x0, x20
    mov x1, x18
    svc #16
    cbnz x1, 1b

    mov x19, x0

    mov x0, x20
    mov x1, #1
    movz x2, #0x6000
    movk x2, #0x1, lsl #16
    mov x3, #32
    svc #1
    mov x21, x0

    mov x0, x20
    mov x1, x21
    svc #4

    // CQ page = 0x14000. Spin until head != tail, then read entry[0].result.
    movz x9, #0x4000
    movk x9, #0x1, lsl #16
2:
    ldr w10, [x9]
    cbz w10, 2b
    ldr w11, [x9, #24]

    // buffer page = 0x16000
    movz x9, #0x6000
    movk x9, #0x1, lsl #16
    ldr w12, [x9]

    mov x0, x20
    mov x1, x19
    mov x2, #99
    svc #2

    movz x9, #0x5000
    movk x9, #0x1, lsl #16
    movz w10, #0x1000
    movk w10, #0x1000, lsl #16
    str w19, [x9, #20]
    str w11, [x9, #24]
    str w12, [x9, #28]
    dmb ish
    str w10, [x9, #16]

    svc #8
__catten_el0_pong_end:
