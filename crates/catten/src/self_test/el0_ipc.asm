.pushsection .text.catten_el0_ipc, "ax"
.balign 4

.global __catten_el0_ipc_start
.global __catten_el0_ipc_end

__catten_el0_ipc_start:
    // x27 = result page at 0x0001_1000.
    movz x27, #0x1000
    movk x27, #0x1, lsl #16

    // endpoint = ipc_endpoint_create(interface=0x4950, version=1, capacity=4)
    movz x1, #0x4950
    movz x2, #1
    movz x3, #4
    svc #18
    str w0, [x27, #4]
    mov x19, x0

    // connection = ipc_connect(endpoint, SEND | CALL)
    mov x1, x19
    movz x2, #3
    movz x3, #0
    svc #19
    str w0, [x27, #8]
    mov x20, x0

    // ipc_scalar_send(connection, opcode=7, arg0=0x55)
    mov x1, x20
    movz x2, #7
    movz x3, #0x55
    svc #20
    str w0, [x27, #12]

    // block-receive the send message from endpoint.
    mov x1, x19
    svc #72
    str x9, [x27, #128]
    str x10, [x27, #136]
    str x11, [x27, #144]
    str w0, [x27, #16]
    str w1, [x27, #20]
    str w2, [x27, #24]

    // call_cap = ipc_scalar_call(connection, opcode=8, arg0=0x66)
    mov x1, x20
    movz x2, #8
    movz x3, #0x66
    svc #21
    str w0, [x27, #28]
    mov x21, x0

    // block-receive the call message and keep its reply token.
    mov x1, x19
    svc #72
    str w0, [x27, #32]
    str w1, [x27, #36]
    str w2, [x27, #40]
    str w3, [x27, #44]
    mov x22, x3

    // ipc_reply(reply_token, 0x1234)
    mov x1, x22
    movz x2, #0x1234
    svc #23
    str w0, [x27, #48]

    // ipc_reply_poll(call_cap)
    mov x1, x21
    svc #24
    str w0, [x27, #52]
    str w1, [x27, #56]
    str w2, [x27, #60]

    dmb ish
    movz w10, #0x1c50
    str w10, [x27]
    svc #8

__catten_el0_ipc_end:
.popsection
