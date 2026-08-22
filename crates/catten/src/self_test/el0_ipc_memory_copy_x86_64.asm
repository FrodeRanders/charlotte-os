.section .text.catten_el0_ipc_memory_copy, "ax"
.balign 16

.global __catten_el0_ipc_memory_copy_server_start
.global __catten_el0_ipc_memory_copy_server_end
.global __catten_el0_ipc_memory_copy_client_start
.global __catten_el0_ipc_memory_copy_client_end

__catten_el0_ipc_memory_copy_server_start:
    mov r15, 0x11000
    mov r14, 0x12000
    mov ebx, 1
    mfence
    mov dword ptr [r15], 0xc05e

    mov eax, 27
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 48], eax
    mov dword ptr [r15 + 52], edi
    mov dword ptr [r15 + 56], esi
    mov dword ptr [r15 + 60], edx
    mov dword ptr [r15 + 64], r11d
    mov ebp, edx
    mov r12d, r11d

    mov eax, 29
    mov edi, r12d
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 68], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 72], eax
    mov dword ptr [r14], 0xc092

    mov eax, 30
    mov edi, r12d
    syscall
    mov dword ptr [r15 + 76], eax
    mov eax, 31
    mov edi, r12d
    syscall
    mov dword ptr [r15 + 80], eax

    mov eax, 23
    mov edi, ebp
    mov esi, 0xc0
    syscall
    mov dword ptr [r15 + 84], eax

    mfence
    mov dword ptr [r15 + 4], 0xc051
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_memory_copy_server_end:

__catten_el0_ipc_memory_copy_client_start:
    mov r15, 0x11000
    mov r14, 0x12000

1:
    cmp dword ptr [r15], 0xc05e
    jne 1b
    lfence

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
    mov dword ptr [r14], 0xc091
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 20], eax

    mov eax, 38
    mov edi, 1
    mov esi, 0x90
    mov edx, 0xc9
    mov r10d, ebx
    syscall
    mov dword ptr [r15 + 24], eax
    mov ebp, eax

    // The source remains mappable immediately after call-copy.
    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    xor edx, edx
    syscall
    mov dword ptr [r15 + 28], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 32], eax
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 36], eax

2:
    mov eax, 24
    mov edi, ebp
    syscall
    test eax, eax
    jne 2b
    mov dword ptr [r15 + 88], eax
    mov dword ptr [r15 + 92], edi
    mov dword ptr [r15 + 96], esi
    mov dword ptr [r15 + 100], edx

    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    xor edx, edx
    syscall
    mov dword ptr [r15 + 104], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 108], eax
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 112], eax
    mov eax, 31
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 116], eax

    mfence
    mov dword ptr [r15 + 8], 0xc0d1
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_memory_copy_client_end:
