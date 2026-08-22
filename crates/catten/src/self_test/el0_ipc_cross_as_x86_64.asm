.section .text.catten_el0_ipc_cross_as, "ax"
.balign 16

.global __catten_el0_ipc_cross_server_start
.global __catten_el0_ipc_cross_server_end
.global __catten_el0_ipc_cross_client_start
.global __catten_el0_ipc_cross_client_end

__catten_el0_ipc_cross_server_start:
    mov r15, 0x11000
    mov ebx, 1
    mfence
    mov dword ptr [r15], 0x5e5e

    mov eax, 27
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 8], eax
    mov dword ptr [r15 + 12], edi
    mov dword ptr [r15 + 16], esi
    mov dword ptr [r15 + 20], edx
    mov dword ptr [r15 + 24], r10d
    mov dword ptr [r15 + 28], r8d
    mov dword ptr [r15 + 32], r9d
    mov ebp, edx

    mov eax, 23
    mov edi, ebp
    mov esi, 0x6789
    syscall
    mov dword ptr [r15 + 36], eax

    mfence
    mov dword ptr [r15 + 4], 0x5e51
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_cross_server_end:

__catten_el0_ipc_cross_client_start:
    mov r15, 0x11000

1:
    cmp dword ptr [r15], 0x5e5e
    jne 1b
    lfence

    mov eax, 21
    mov edi, 1
    mov esi, 0x33
    mov edx, 0x99
    syscall
    mov dword ptr [r15 + 40], eax
    mov ebx, eax

2:
    mov eax, 24
    mov edi, ebx
    syscall
    test eax, eax
    jne 2b
    mov dword ptr [r15 + 44], eax
    mov dword ptr [r15 + 48], edi
    mov dword ptr [r15 + 52], esi

    mfence
    mov dword ptr [r15 + 56], 0xc1e1
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_cross_client_end:
