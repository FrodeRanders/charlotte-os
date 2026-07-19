.section .text.catten_el0_ipc_block, "ax"
.balign 4

.global __catten_el0_ipc_block_server_start
.global __catten_el0_ipc_block_server_end
.global __catten_el0_ipc_block_client_start
.global __catten_el0_ipc_block_client_end

__catten_el0_ipc_block_server_start:
    // x9 = result page at 0x0001_4000.
    movz x9, #0x4000
    movk x9, #0x1, lsl #16

    // endpoint = ipc_endpoint_create(interface=0x424c, version=1, capacity=4)
    movz x1, #0x424c
    movz x2, #1
    movz x3, #4
    svc #18
    str w0, [x9, #4]
    mov x19, x0

    // connection = ipc_connect(endpoint, CALL)
    mov x1, x19
    movz x2, #2
    movz x3, #0
    svc #19
    str w0, [x9, #8]

    dmb ish
    movz w10, #0x5150
    str w10, [x9]

    // This should park until the client posts a call.
    mov x1, x19
    svc #27
    str w0, [x9, #12]
    str w1, [x9, #16]
    str w2, [x9, #20]
    str w3, [x9, #24]
    mov x20, x3

    mov x1, x20
    movz x2, #0x4567
    svc #23
    str w0, [x9, #28]

    dmb ish
    movz w10, #0x1c51
    str w10, [x9, #32]
    svc #8

__catten_el0_ipc_block_server_end:

__catten_el0_ipc_block_client_start:
    // x9 = result page at 0x0001_4000.
    movz x9, #0x4000
    movk x9, #0x1, lsl #16

2:
    ldr w10, [x9]
    movz w11, #0x5150
    cmp w10, w11
    b.ne 2b
    dmb ish

    ldr w19, [x9, #8]
    mov x1, x19
    movz x2, #9
    movz x3, #0x77
    svc #21
    str w0, [x9, #36]
    mov x20, x0

3:
    mov x1, x20
    svc #24
    cbnz x0, 3b
    str w0, [x9, #40]
    str w1, [x9, #44]
    str w2, [x9, #48]

    dmb ish
    movz w10, #0xc117
    str w10, [x9, #52]
    svc #8

__catten_el0_ipc_block_client_end:
