.section .text.catten_el0_ipc_memory, "ax"
.balign 4

.global __catten_el0_ipc_memory_server_start
.global __catten_el0_ipc_memory_server_end
.global __catten_el0_ipc_memory_client_start
.global __catten_el0_ipc_memory_client_end

__catten_el0_ipc_memory_server_start:
    // x9 = shared result page at 0x0001_1000.
    movz x9, #0x1000
    movk x9, #0x1, lsl #16

    // x8 = memory-object mapping VA at 0x0001_2000.
    movz x8, #0x2000
    movk x8, #0x1, lsl #16

    // The kernel pre-seeds endpoint cap 1 into the server AS.
    movz x19, #1
    dmb ish
    movz w10, #0x6d5e
    str w10, [x9]

    // Block until the client AS calls through its delegated connection cap.
    mov x1, x19
    svc #27
    str w0, [x9, #64]
    str w1, [x9, #68]
    str w2, [x9, #72]
    str w3, [x9, #76]
    str w7, [x9, #80]
    mov x20, x3
    mov x21, x7

    // Map the moved memory object writable and update its payload.
    mov x1, x21
    mov x2, x8
    movz x3, #1
    svc #29
    str w0, [x9, #84]

    ldr w10, [x8]
    str w10, [x9, #88]
    movz w10, #0x4d32
    movk w10, #0x4d45, lsl #16
    str w10, [x8]

    mov x1, x21
    svc #30
    str w0, [x9, #92]

    // Return the memory object to the caller with a scalar result.
    mov x1, x20
    mov x2, x21
    movz x3, #0x2468
    svc #34
    str w0, [x9, #96]

    dmb ish
    movz w10, #0x6d51
    str w10, [x9, #4]
1:
    nop
    b 1b

__catten_el0_ipc_memory_server_end:

__catten_el0_ipc_memory_client_start:
    // x9 = shared result page at 0x0001_1000.
    movz x9, #0x1000
    movk x9, #0x1, lsl #16

    // x8 = memory-object mapping VA at 0x0001_2000.
    movz x8, #0x2000
    movk x8, #0x1, lsl #16

2:
    ldr w10, [x9]
    movz w11, #0x6d5e
    cmp w10, w11
    b.ne 2b
    dmb ish

    // Allocate one page-backed memory object.
    movz x1, #1
    svc #28
    str w0, [x9, #12]
    mov x20, x0

    // Map it writable and seed the payload.
    mov x1, x20
    mov x2, x8
    movz x3, #1
    svc #29
    str w0, [x9, #16]

    movz w10, #0x4d31
    movk w10, #0x4d45, lsl #16
    str w10, [x8]

    mov x1, x20
    svc #30
    str w0, [x9, #20]

    // The kernel pre-seeds connection cap 1 into the client AS.
    movz x19, #1
    mov x1, x19
    movz x2, #0x44
    movz x3, #0xab
    mov x4, x20
    svc #33
    str w0, [x9, #24]
    mov x21, x0

    // The moved-from cap must no longer authorize mapping in the caller.
    mov x1, x20
    mov x2, x8
    movz x3, #0
    svc #29
    str w0, [x9, #28]

3:
    mov x1, x21
    svc #24
    cbnz x0, 3b
    str w0, [x9, #32]
    str w1, [x9, #36]
    str w2, [x9, #40]
    str w3, [x9, #44]
    mov x22, x3

    // Map the returned memory object read-only and verify the server's update.
    mov x1, x22
    mov x2, x8
    movz x3, #0
    svc #29
    str w0, [x9, #48]

    ldr w10, [x8]
    str w10, [x9, #52]

    mov x1, x22
    svc #30
    str w0, [x9, #56]

    mov x1, x22
    svc #31
    str w0, [x9, #60]

    dmb ish
    movz w10, #0xc6d1
    str w10, [x9, #8]
4:
    nop
    b 4b

__catten_el0_ipc_memory_client_end:
