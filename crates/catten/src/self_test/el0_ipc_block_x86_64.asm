.section .text.catten_el0_ipc_block, "ax"
.balign 16

.global __catten_el0_ipc_block_server_start
.global __catten_el0_ipc_block_server_end
.global __catten_el0_ipc_block_client_start
.global __catten_el0_ipc_block_client_end

__catten_el0_ipc_block_server_start:
    mov r15, 0x14000

    mov eax, 18
    mov edi, 0x424c
    mov esi, 1
    mov edx, 4
    syscall
    mov dword ptr [r15 + 4], eax
    mov ebx, eax

    mov eax, 19
    mov edi, ebx
    mov esi, 2
    xor edx, edx
    syscall
    mov dword ptr [r15 + 8], eax

    mfence
    mov dword ptr [r15], 0x5150

    // This should park until the client posts a call.
    mov eax, 27
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 12], eax
    mov dword ptr [r15 + 16], edi
    mov dword ptr [r15 + 20], esi
    mov dword ptr [r15 + 24], edx
    mov ebp, edx

    mov eax, 23
    mov edi, ebp
    mov esi, 0x4567
    syscall
    mov dword ptr [r15 + 28], eax

    mfence
    mov dword ptr [r15 + 32], 0x1c51
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_block_server_end:

__catten_el0_ipc_block_client_start:
    mov r15, 0x14000

1:
    cmp dword ptr [r15], 0x5150
    jne 1b
    lfence

    mov ebx, dword ptr [r15 + 8]
    mov eax, 21
    mov edi, ebx
    mov esi, 9
    mov edx, 0x77
    syscall
    mov dword ptr [r15 + 36], eax
    mov ebp, eax

2:
    mov eax, 24
    mov edi, ebp
    syscall
    test eax, eax
    jne 2b
    mov dword ptr [r15 + 40], eax
    mov dword ptr [r15 + 44], edi
    mov dword ptr [r15 + 48], esi

    mfence
    mov dword ptr [r15 + 52], 0xc117
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_block_client_end:
