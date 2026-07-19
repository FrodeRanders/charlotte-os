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

    // x28 = memory-object mapping VA at 0x0001_2000.
    movz x28, #0x2000
    movk x28, #0x1, lsl #16

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
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #84]

    ldr w10, [x28]
    str w10, [x9, #88]
    movz w10, #0x4d32
    movk w10, #0x4d45, lsl #16
    str w10, [x28]

    mov x1, x21
    svc #30
    str w0, [x9, #92]

    // Return the memory object to the caller with a scalar result.
    mov x1, x20
    mov x2, x21
    movz x3, #0x2468
    svc #34
    str w0, [x9, #96]

    // Receive a reply-bound read borrow. Writable map must fail; read-only map
    // must succeed until the reply revokes the borrowed cap.
    mov x1, x19
    svc #27
    str w0, [x9, #200]
    str w1, [x9, #204]
    str w2, [x9, #208]
    str w3, [x9, #212]
    str w7, [x9, #216]
    mov x20, x3
    mov x21, x7

    mov x1, x21
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #220]

    mov x1, x21
    mov x2, x28
    movz x3, #0
    svc #29
    str w0, [x9, #224]

    ldr w10, [x28]
    str w10, [x9, #228]

    mov x1, x21
    svc #30
    str w0, [x9, #232]

    mov x1, x20
    movz x2, #0x1357
    svc #23
    str w0, [x9, #236]

    // Receive a reply-bound write borrow and update it before replying.
    mov x1, x19
    svc #27
    str w0, [x9, #240]
    str w1, [x9, #244]
    str w2, [x9, #248]
    str w3, [x9, #252]
    str w7, [x9, #256]
    mov x20, x3
    mov x21, x7

    mov x1, x21
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #260]

    ldr w10, [x28]
    str w10, [x9, #264]
    movz w10, #0x5752
    movk w10, #0x4252, lsl #16
    str w10, [x28]

    mov x1, x21
    svc #30
    str w0, [x9, #268]

    mov x1, x20
    movz x2, #0x2469
    svc #23
    str w0, [x9, #272]

    dmb ish
    movz w10, #0x6d51
    str w10, [x9, #4]
    svc #8

__catten_el0_ipc_memory_server_end:

__catten_el0_ipc_memory_client_start:
    // x9 = shared result page at 0x0001_1000.
    movz x9, #0x1000
    movk x9, #0x1, lsl #16

    // x28 = memory-object mapping VA at 0x0001_2000.
    movz x28, #0x2000
    movk x28, #0x1, lsl #16

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
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #16]

    movz w10, #0x4d31
    movk w10, #0x4d45, lsl #16
    str w10, [x28]

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
    mov x2, x28
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
    mov x2, x28
    movz x3, #0
    svc #29
    str w0, [x9, #48]

    ldr w10, [x28]
    str w10, [x9, #52]

    mov x1, x22
    svc #30
    str w0, [x9, #56]

    mov x1, x22
    svc #31
    str w0, [x9, #60]

    // Read-borrow a separate memory object. The server may read but not write,
    // and a normal reply must revoke the server's borrowed cap.
    movz x1, #1
    svc #28
    str w0, [x9, #100]
    mov x23, x0

    mov x1, x23
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #104]

    movz w10, #0x5244
    movk w10, #0x4252, lsl #16
    str w10, [x28]

    mov x1, x23
    svc #30
    str w0, [x9, #108]

    mov x1, x19
    movz x2, #0x45
    movz x3, #0xbc
    mov x4, x23
    svc #35
    str w0, [x9, #112]
    mov x24, x0

5:
    mov x1, x24
    svc #24
    cbnz x0, 5b
    str w0, [x9, #116]
    str w1, [x9, #120]
    str w2, [x9, #124]
    str w3, [x9, #128]

    mov x1, x23
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #132]

    mov x1, x23
    svc #30
    str w0, [x9, #136]

    mov x1, x23
    svc #31
    str w0, [x9, #140]

    // Write-borrow another memory object. The server updates it and the client
    // observes the change after reply-bound revocation.
    movz x1, #1
    svc #28
    str w0, [x9, #144]
    mov x25, x0

    mov x1, x25
    mov x2, x28
    movz x3, #1
    svc #29
    str w0, [x9, #148]

    movz w10, #0x5752
    movk w10, #0x4257, lsl #16
    str w10, [x28]

    mov x1, x25
    svc #30
    str w0, [x9, #152]

    mov x1, x19
    movz x2, #0x46
    movz x3, #0xcd
    mov x4, x25
    svc #36
    str w0, [x9, #156]
    mov x26, x0

6:
    mov x1, x26
    svc #24
    cbnz x0, 6b
    str w0, [x9, #160]
    str w1, [x9, #164]
    str w2, [x9, #168]
    str w3, [x9, #172]

    mov x1, x25
    mov x2, x28
    movz x3, #0
    svc #29
    str w0, [x9, #176]

    ldr w10, [x28]
    str w10, [x9, #180]

    mov x1, x25
    svc #30
    str w0, [x9, #184]

    mov x1, x25
    svc #31
    str w0, [x9, #188]

    dmb ish
    movz w10, #0xc6d1
    str w10, [x9, #8]
    svc #8

__catten_el0_ipc_memory_client_end:
