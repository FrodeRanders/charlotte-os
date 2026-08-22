.section .text.catten_el0_ipc, "ax"
.balign 16

.global __catten_el0_ipc_start
.global __catten_el0_ipc_end

__catten_el0_ipc_start:
    mov r15, 0x11000

    // endpoint = ipc_endpoint_create(interface=0x4950, version=1, capacity=4)
    mov eax, 18
    mov edi, 0x4950
    mov esi, 1
    mov edx, 4
    syscall
    mov dword ptr [r15 + 4], eax
    mov ebx, eax

    // connection = ipc_connect(endpoint, SEND | CALL)
    mov eax, 19
    mov edi, ebx
    mov esi, 3
    xor edx, edx
    syscall
    mov dword ptr [r15 + 8], eax
    mov ebp, eax

    // ipc_scalar_send(connection, opcode=7, arg0=0x55)
    mov eax, 20
    mov edi, ebp
    mov esi, 7
    mov edx, 0x55
    syscall
    mov dword ptr [r15 + 12], eax

    // block-receive the send message from endpoint.
    mov eax, 27
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 16], eax
    mov dword ptr [r15 + 20], edi
    mov dword ptr [r15 + 24], esi

    // call_cap = ipc_scalar_call(connection, opcode=8, arg0=0x66)
    mov eax, 21
    mov edi, ebp
    mov esi, 8
    mov edx, 0x66
    syscall
    mov dword ptr [r15 + 28], eax
    mov r12d, eax

    // block-receive the call message and keep its reply token.
    mov eax, 27
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 32], eax
    mov dword ptr [r15 + 36], edi
    mov dword ptr [r15 + 40], esi
    mov dword ptr [r15 + 44], edx
    mov r13d, edx

    // ipc_reply(reply_token, 0x1234)
    mov eax, 23
    mov edi, r13d
    mov esi, 0x1234
    syscall
    mov dword ptr [r15 + 48], eax

    // ipc_reply_poll(call_cap)
    mov eax, 24
    mov edi, r12d
    syscall
    mov dword ptr [r15 + 52], eax
    mov dword ptr [r15 + 56], edi
    mov dword ptr [r15 + 60], esi

    mfence
    mov dword ptr [r15], 0x1c50
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_end:
