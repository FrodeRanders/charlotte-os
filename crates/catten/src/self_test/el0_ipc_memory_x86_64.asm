.section .text.catten_el0_ipc_memory, "ax"
.balign 16

.global __catten_el0_ipc_memory_server_start
.global __catten_el0_ipc_memory_server_end
.global __catten_el0_ipc_memory_client_start
.global __catten_el0_ipc_memory_client_end

__catten_el0_ipc_memory_server_start:
    mov r15, 0x11000
    mov r14, 0x12000
    mov ebx, 1
    mfence
    mov dword ptr [r15], 0x6d5e

    // Receive the moved memory object.
    mov eax, 27
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 64], eax
    mov dword ptr [r15 + 68], edi
    mov dword ptr [r15 + 72], esi
    mov dword ptr [r15 + 76], edx
    mov dword ptr [r15 + 80], r11d
    mov ebp, edx
    mov r12d, r11d

    mov eax, 29
    mov edi, r12d
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 84], eax

    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 88], eax
    mov dword ptr [r14], 0x4d454d32

    mov eax, 30
    mov edi, r12d
    syscall
    mov dword ptr [r15 + 92], eax

    mov eax, 34
    mov edi, ebp
    mov esi, r12d
    mov edx, 0x2468
    syscall
    mov dword ptr [r15 + 96], eax

    // Receive a reply-bound read borrow.
    mov eax, 27
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 200], eax
    mov dword ptr [r15 + 204], edi
    mov dword ptr [r15 + 208], esi
    mov dword ptr [r15 + 212], edx
    mov dword ptr [r15 + 216], r11d
    mov ebp, edx
    mov r12d, r11d

    mov eax, 29
    mov edi, r12d
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 220], eax

    mov eax, 29
    mov edi, r12d
    mov rsi, r14
    xor edx, edx
    syscall
    mov dword ptr [r15 + 224], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 228], eax

    mov eax, 30
    mov edi, r12d
    syscall
    mov dword ptr [r15 + 232], eax

    mov eax, 23
    mov edi, ebp
    mov esi, 0x1357
    syscall
    mov dword ptr [r15 + 236], eax

    // Receive a reply-bound write borrow.
    mov eax, 27
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 240], eax
    mov dword ptr [r15 + 244], edi
    mov dword ptr [r15 + 248], esi
    mov dword ptr [r15 + 252], edx
    mov dword ptr [r15 + 256], r11d
    mov ebp, edx
    mov r12d, r11d

    mov eax, 29
    mov edi, r12d
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 260], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 264], eax
    mov dword ptr [r14], 0x42525752

    mov eax, 30
    mov edi, r12d
    syscall
    mov dword ptr [r15 + 268], eax

    mov eax, 23
    mov edi, ebp
    mov esi, 0x2469
    syscall
    mov dword ptr [r15 + 272], eax

    mfence
    mov dword ptr [r15 + 4], 0x6d51
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_memory_server_end:

__catten_el0_ipc_memory_client_start:
    mov r15, 0x11000
    mov r14, 0x12000

1:
    cmp dword ptr [r15], 0x6d5e
    jne 1b
    lfence

    // Allocate, map, and seed the object that will be moved.
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
    mov dword ptr [r14], 0x4d454d31

    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 20], eax

    mov eax, 33
    mov edi, 1
    mov esi, 0x44
    mov edx, 0xab
    mov r10d, ebx
    syscall
    mov dword ptr [r15 + 24], eax
    mov ebp, eax

    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    xor edx, edx
    syscall
    mov dword ptr [r15 + 28], eax

2:
    mov eax, 24
    mov edi, ebp
    syscall
    test eax, eax
    jne 2b
    mov dword ptr [r15 + 32], eax
    mov dword ptr [r15 + 36], edi
    mov dword ptr [r15 + 40], esi
    mov dword ptr [r15 + 44], edx
    mov r12d, edx

    mov eax, 29
    mov edi, r12d
    mov rsi, r14
    xor edx, edx
    syscall
    mov dword ptr [r15 + 48], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 52], eax

    mov eax, 30
    mov edi, r12d
    syscall
    mov dword ptr [r15 + 56], eax
    mov eax, 31
    mov edi, r12d
    syscall
    mov dword ptr [r15 + 60], eax

    // Read-borrow a separate object.
    mov eax, 28
    mov edi, 1
    syscall
    mov dword ptr [r15 + 100], eax
    mov ebx, eax

    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 104], eax
    mov dword ptr [r14], 0x42525244
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 108], eax

    mov eax, 35
    mov edi, 1
    mov esi, 0x45
    mov edx, 0xbc
    mov r10d, ebx
    syscall
    mov dword ptr [r15 + 112], eax
    mov ebp, eax

3:
    mov eax, 24
    mov edi, ebp
    syscall
    test eax, eax
    jne 3b
    mov dword ptr [r15 + 116], eax
    mov dword ptr [r15 + 120], edi
    mov dword ptr [r15 + 124], esi
    mov dword ptr [r15 + 128], edx

    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 132], eax
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 136], eax
    mov eax, 31
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 140], eax

    // Write-borrow another object.
    mov eax, 28
    mov edi, 1
    syscall
    mov dword ptr [r15 + 144], eax
    mov ebx, eax

    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    mov edx, 1
    syscall
    mov dword ptr [r15 + 148], eax
    mov dword ptr [r14], 0x42575752
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 152], eax

    mov eax, 36
    mov edi, 1
    mov esi, 0x46
    mov edx, 0xcd
    mov r10d, ebx
    syscall
    mov dword ptr [r15 + 156], eax
    mov ebp, eax

4:
    mov eax, 24
    mov edi, ebp
    syscall
    test eax, eax
    jne 4b
    mov dword ptr [r15 + 160], eax
    mov dword ptr [r15 + 164], edi
    mov dword ptr [r15 + 168], esi
    mov dword ptr [r15 + 172], edx

    mov eax, 29
    mov edi, ebx
    mov rsi, r14
    xor edx, edx
    syscall
    mov dword ptr [r15 + 176], eax
    mov eax, dword ptr [r14]
    mov dword ptr [r15 + 180], eax
    mov eax, 30
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 184], eax
    mov eax, 31
    mov edi, ebx
    syscall
    mov dword ptr [r15 + 188], eax

    mfence
    mov dword ptr [r15 + 8], 0xc6d1
    mov eax, 8
    syscall
    ud2

__catten_el0_ipc_memory_client_end:
