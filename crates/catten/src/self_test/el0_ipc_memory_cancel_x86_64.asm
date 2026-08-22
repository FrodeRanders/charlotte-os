.section .text.catten_el0_ipc_memory_cancel, "ax"
.balign 16

.global __catten_el0_ipc_memory_cancel_server_start
.global __catten_el0_ipc_memory_cancel_server_end
.global __catten_el0_ipc_memory_cancel_client_start
.global __catten_el0_ipc_memory_cancel_client_end

__catten_el0_ipc_memory_cancel_server_start:
    mov r15, 0x11000
    mov r14, 0x12000
    mov ebx, 1
    mfence
    mov dword ptr [r15], 0xca5e

1:
    cmp dword ptr [r15 + 8], 0xcad1
    jne 1b
    lfence

    // Both queued calls were cancelled before receive.
    mov eax, 22
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 72], eax
    mov eax, 22
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 76], eax

    mfence
    mov dword ptr [r15 + 80], 0xca52

    // Receive, map, and update the delivered write borrow.
    mov eax, 27
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 88], eax
    mov dword ptr [r15 + 92], edi
    mov dword ptr [r15 + 96], edx
    mov dword ptr [r15 + 100], r11d
    mov ebp, edx
    mov r12d, r11d

    mov eax, 29
    mov edi, r12d
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 104], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 108], eax
    mov dword ptr [r14], 0xd002

    mfence
    mov dword ptr [r15 + 84], 0xca53

2:
    cmp dword ptr [r15 + 120], 0xca54
    jne 2b
    lfence

    mov eax, 29
    mov edi, r12d
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 112], eax

    mov eax, 23
    mov edi, ebp
    mov esi, 0xdead
    syscall
    mov dword ptr [r15 + 116], eax

    mfence
    mov dword ptr [r15 + 4], 0xca51
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_memory_cancel_server_end:

__catten_el0_ipc_memory_cancel_client_start:
    mov r15, 0x11000
    mov r14, 0x12000

1:
    cmp dword ptr [r15], 0xca5e
    jne 1b
    lfence

    // Queue and cancel a moved-memory call.
    mov eax, 28
    mov edi, 1
    syscall
    mov dword ptr [r15 + 12], eax
    mov ebx, eax

    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 16], eax
    mov dword ptr [r14], 0xc001
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 20], eax

    mov eax, 33
    mov edi, 1
    mov esi, 0x70
    mov edx, 0xa0
    mov r10d, ebx
    syscall
    mov dword ptr [r15 + 24], eax
    mov ebp, eax

    mov eax, 25
    mov edi, ebp
    syscall
    mov dword ptr [r15 + 28], eax
    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    xor edx, edx
    syscall
    mov dword ptr [r15 + 32], eax

    // Queue and cancel a write-borrow call before delivery.
    mov eax, 28
    mov edi, 1
    syscall
    mov dword ptr [r15 + 36], eax
    mov ebx, eax
    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 40], eax
    mov dword ptr [r14], 0xb001
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 44], eax

    mov eax, 36
    mov edi, 1
    mov esi, 0x71
    mov edx, 0xb0
    mov r10d, ebx
    syscall
    mov dword ptr [r15 + 48], eax
    mov ebp, eax
    mov eax, 25
    mov edi, ebp
    syscall
    mov dword ptr [r15 + 52], eax

    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 56], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 60], eax
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 64], eax
    mov eax, 31
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 68], eax

    mfence
    mov dword ptr [r15 + 8], 0xcad1

2:
    cmp dword ptr [r15 + 80], 0xca52
    jne 2b
    lfence

    // Deliver a write borrow, then cancel it after the server updates it.
    mov eax, 28
    mov edi, 1
    syscall
    mov dword ptr [r15 + 124], eax
    mov ebx, eax
    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 128], eax
    mov dword ptr [r14], 0xd001
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 132], eax

    mov eax, 36
    mov edi, 1
    mov esi, 0x72
    mov edx, 0xc0
    mov r10d, ebx
    syscall
    mov dword ptr [r15 + 136], eax
    mov ebp, eax

3:
    cmp dword ptr [r15 + 84], 0xca53
    jne 3b
    lfence

    mov eax, 25
    mov edi, ebp
    syscall
    mov dword ptr [r15 + 140], eax
    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 144], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 148], eax
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 152], eax
    mov eax, 31
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 156], eax

    mfence
    mov dword ptr [r15 + 120], 0xca54
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_memory_cancel_client_end:
