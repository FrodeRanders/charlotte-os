.section .text.catten_el0_ipc_memory_cancel, "ax"
.balign 4

.global __catten_el0_ipc_memory_cancel_server_start
.global __catten_el0_ipc_memory_cancel_server_end
.global __catten_el0_ipc_memory_cancel_client_start
.global __catten_el0_ipc_memory_cancel_client_end

__catten_el0_ipc_memory_cancel_server_start:
    // x9 = shared result page at 0x0001_1000.
    movz x9, #0x1000
    movk x9, #0x1, lsl #16

    // The kernel pre-seeds endpoint cap 1 into the server AS.
    movz x19, #1
    dmb ish
    movz w10, #0xca5e
    str w10, [x9]

1:
    ldr w10, [x9, #8]
    movz w11, #0xcad1
    cmp w10, w11
    b.ne 1b
    dmb ish

    // Both calls were queued and then cancelled by the client before receive.
    mov x1, x19
    svc #22
    str w0, [x9, #72]

    mov x1, x19
    svc #22
    str w0, [x9, #76]

    dmb ish
    movz w10, #0xca52
    str w10, [x9, #80]

    // Receive a write-borrow call, map and update it, then wait for the client
    // to cancel the already-delivered pending call.
    mov x1, x19
    svc #27
    str w0, [x9, #88]
    str w1, [x9, #92]
    str w3, [x9, #96]
    str w7, [x9, #100]
    mov x20, x3
    mov x21, x7

    movz x28, #0x2000
    movk x28, #0x1, lsl #16
    mov x1, x21
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #104]

    ldr w10, [x28]
    str w10, [x9, #108]
    movz w10, #0xd002
    str w10, [x28]

    dmb ish
    movz w10, #0xca53
    str w10, [x9, #84]

5:
    ldr w10, [x9, #120]
    movz w11, #0xca54
    cmp w10, w11
    b.ne 5b
    dmb ish

    // Cancellation should revoke the server's borrowed cap and reply token.
    mov x1, x21
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #112]

    mov x1, x20
    movz x2, #0xdead
    svc #23
    str w0, [x9, #116]

    dmb ish
    movz w10, #0xca51
    str w10, [x9, #4]
2:
    nop
    b 2b

__catten_el0_ipc_memory_cancel_server_end:

__catten_el0_ipc_memory_cancel_client_start:
    // x9 = shared result page at 0x0001_1000.
    movz x9, #0x1000
    movk x9, #0x1, lsl #16

    // x28 = memory-object mapping VA at 0x0001_2000.
    movz x28, #0x2000
    movk x28, #0x1, lsl #16

3:
    ldr w10, [x9]
    movz w11, #0xca5e
    cmp w10, w11
    b.ne 3b
    dmb ish

    // Queue a moved-memory call and cancel the pending-call cap before the
    // server receives it. The moved-from cap must remain consumed.
    movz x1, #1
    svc #28
    str w0, [x9, #12]
    mov x20, x0

    mov x1, x20
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #16]

    movz w10, #0xc001
    str w10, [x28]

    mov x1, x20
    svc #30
    str w0, [x9, #20]

    movz x19, #1
    mov x1, x19
    movz x2, #0x70
    movz x3, #0xa0
    mov x4, x20
    svc #33
    str w0, [x9, #24]
    mov x21, x0

    mov x1, x21
    svc #25
    str w0, [x9, #28]

    mov x1, x20
    mov x2, x28
    movz x3, #0
    svc #29
    str w0, [x9, #32]

    // Queue a write-borrow call and cancel it before delivery. Closing the
    // pending call must revoke the reply-bound borrow so the owner can map
    // writable again.
    movz x1, #1
    svc #28
    str w0, [x9, #36]
    mov x22, x0

    mov x1, x22
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #40]

    movz w10, #0xb001
    str w10, [x28]

    mov x1, x22
    svc #30
    str w0, [x9, #44]

    mov x1, x19
    movz x2, #0x71
    movz x3, #0xb0
    mov x4, x22
    svc #36
    str w0, [x9, #48]
    mov x23, x0

    mov x1, x23
    svc #25
    str w0, [x9, #52]

    mov x1, x22
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #56]

    ldr w10, [x28]
    str w10, [x9, #60]

    mov x1, x22
    svc #30
    str w0, [x9, #64]

    mov x1, x22
    svc #31
    str w0, [x9, #68]

    dmb ish
    movz w10, #0xcad1
    str w10, [x9, #8]

6:
    ldr w10, [x9, #80]
    movz w11, #0xca52
    cmp w10, w11
    b.ne 6b
    dmb ish

    // Deliver a write-borrow call, wait until the server has mapped and
    // updated it, then cancel the pending call.
    movz x1, #1
    svc #28
    str w0, [x9, #124]
    mov x24, x0

    mov x1, x24
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #128]

    movz w10, #0xd001
    str w10, [x28]

    mov x1, x24
    svc #30
    str w0, [x9, #132]

    mov x1, x19
    movz x2, #0x72
    movz x3, #0xc0
    mov x4, x24
    svc #36
    str w0, [x9, #136]
    mov x25, x0

7:
    ldr w10, [x9, #84]
    movz w11, #0xca53
    cmp w10, w11
    b.ne 7b
    dmb ish

    mov x1, x25
    svc #25
    str w0, [x9, #140]

    mov x1, x24
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #144]

    ldr w10, [x28]
    str w10, [x9, #148]

    mov x1, x24
    svc #30
    str w0, [x9, #152]

    mov x1, x24
    svc #31
    str w0, [x9, #156]

    dmb ish
    movz w10, #0xca54
    str w10, [x9, #120]
4:
    nop
    b 4b

__catten_el0_ipc_memory_cancel_client_end:
