.section .text.catten_el0_ipc_memory_copy, "ax"
.balign 4

.global __catten_el0_ipc_memory_copy_server_start
.global __catten_el0_ipc_memory_copy_server_end
.global __catten_el0_ipc_memory_copy_client_start
.global __catten_el0_ipc_memory_copy_client_end

__catten_el0_ipc_memory_copy_server_start:
    // x9 = shared result page at 0x0001_1000.
    movz x9, #0x1000
    movk x9, #0x1, lsl #16

    // x8 = memory-object mapping VA at 0x0001_2000.
    movz x8, #0x2000
    movk x8, #0x1, lsl #16

    // The kernel pre-seeds endpoint cap 1 into the server AS.
    movz x19, #1
    dmb ish
    movz w10, #0xc05e
    str w10, [x9]

    mov x1, x19
    svc #27
    str w0, [x9, #48]
    str w1, [x9, #52]
    str w2, [x9, #56]
    str w3, [x9, #60]
    str w7, [x9, #64]
    mov x20, x3
    mov x21, x7

    mov x1, x21
    mov x2, x8
    movz x3, #1
    svc #29
    str w0, [x9, #68]

    ldr w10, [x8]
    str w10, [x9, #72]
    movz w10, #0xc092
    str w10, [x8]

    mov x1, x21
    svc #30
    str w0, [x9, #76]

    mov x1, x21
    svc #31
    str w0, [x9, #80]

    mov x1, x20
    movz x2, #0xc0
    svc #23
    str w0, [x9, #84]

    dmb ish
    movz w10, #0xc051
    str w10, [x9, #4]
1:
    nop
    b 1b

__catten_el0_ipc_memory_copy_server_end:

__catten_el0_ipc_memory_copy_client_start:
    // x9 = shared result page at 0x0001_1000.
    movz x9, #0x1000
    movk x9, #0x1, lsl #16

    // x8 = memory-object mapping VA at 0x0001_2000.
    movz x8, #0x2000
    movk x8, #0x1, lsl #16

2:
    ldr w10, [x9]
    movz w11, #0xc05e
    cmp w10, w11
    b.ne 2b
    dmb ish

    movz x1, #1
    svc #28
    str w0, [x9, #12]
    mov x20, x0

    mov x1, x20
    mov x2, x8
    movz x3, #1
    svc #29
    str w0, [x9, #16]

    movz w10, #0xc091
    str w10, [x8]

    mov x1, x20
    svc #30
    str w0, [x9, #20]

    // The kernel pre-seeds connection cap 1 into the client AS.
    movz x19, #1
    mov x1, x19
    movz x2, #0x90
    movz x3, #0xc9
    mov x4, x20
    svc #38
    str w0, [x9, #24]
    mov x21, x0

    // The source remains owned and mappable immediately after call-copy.
    mov x1, x20
    mov x2, x8
    movz x3, #0
    svc #29
    str w0, [x9, #28]

    ldr w10, [x8]
    str w10, [x9, #32]

    mov x1, x20
    svc #30
    str w0, [x9, #36]

3:
    mov x1, x21
    svc #24
    cbnz x0, 3b
    str w0, [x9, #88]
    str w1, [x9, #92]
    str w2, [x9, #96]
    str w3, [x9, #100]

    // Server wrote to its copy. The client original must remain unchanged.
    mov x1, x20
    mov x2, x8
    movz x3, #0
    svc #29
    str w0, [x9, #104]

    ldr w10, [x8]
    str w10, [x9, #108]

    mov x1, x20
    svc #30
    str w0, [x9, #112]

    mov x1, x20
    svc #31
    str w0, [x9, #116]

    dmb ish
    movz w10, #0xc0d1
    str w10, [x9, #8]
4:
    nop
    b 4b

__catten_el0_ipc_memory_copy_client_end:
