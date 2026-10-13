.pushsection .text.catten_el0_ipc_cross_as, "ax"
.balign 4

.global __catten_el0_ipc_cross_server_start
.global __catten_el0_ipc_cross_server_end
.global __catten_el0_ipc_cross_client_start
.global __catten_el0_ipc_cross_client_end

__catten_el0_ipc_cross_server_start:
    // x27 = shared result page at 0x0001_1000.
    movz x27, #0x1000
    movk x27, #0x1, lsl #16

    // The kernel pre-seeds endpoint cap 1 into the server AS.
    movz x19, #1
    dmb ish
    movz w10, #0x5e5e
    str w10, [x27]

    // Block until the client AS calls through its delegated connection cap.
    mov x1, x19
    svc #72
    str w0, [x27, #8]
    str w1, [x27, #12]
    str w2, [x27, #16]
    str w3, [x27, #20]
    str w4, [x27, #24]
    str w5, [x27, #28]
    str w6, [x27, #32]
    mov x20, x3

    mov x1, x20
    movz x2, #0x6789
    svc #23
    str w0, [x27, #36]

    dmb ish
    movz w10, #0x5e51
    str w10, [x27, #4]
    svc #8

__catten_el0_ipc_cross_server_end:

__catten_el0_ipc_cross_client_start:
    // x27 = shared result page at 0x0001_1000.
    movz x27, #0x1000
    movk x27, #0x1, lsl #16

2:
    ldr w10, [x27]
    movz w11, #0x5e5e
    cmp w10, w11
    b.ne 2b
    dmb ish

    // The kernel pre-seeds connection cap 1 into the client AS.
    movz x19, #1
    mov x1, x19
    movz x2, #0x33
    movz x3, #0x99
    svc #21
    str w0, [x27, #40]
    mov x20, x0

3:
    mov x1, x20
    svc #24
    cbnz x0, 3b
    str w0, [x27, #44]
    str w1, [x27, #48]
    str w2, [x27, #52]

    dmb ish
    movz w10, #0xc1e1
    str w10, [x27, #56]
    svc #8

__catten_el0_ipc_cross_client_end:
.popsection
