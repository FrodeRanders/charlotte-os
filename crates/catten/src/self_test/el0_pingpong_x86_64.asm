.section .text.catten_el0_pingpong, "ax"
.balign 16

.global __catten_el0_ping_start
.global __catten_el0_ping_end
.global __catten_el0_pong_start
.global __catten_el0_pong_end

__catten_el0_ping_start:
    mov r15, 0x15000

    // SUBMIT(Nop) -> completion cap.
    mov eax, 1
    xor edi, edi
    xor esi, esi
    xor edx, edx
    syscall
    mov ebx, eax

    // Open a mailbox sender to LP1 and transfer the completion cap.
    mov eax, 13
    mov edi, 1
    syscall
    mov ebp, eax
    mov eax, 15
    mov edi, ebp
    mov esi, ebx
    syscall

    // Wait up to 60 seconds. The completion result is returned in RDI.
    mov eax, 11
    mov edi, ebx
    mov esi, 60000
    syscall
    test eax, eax
    jne 1f
    mov r12d, edi

    mov dword ptr [r15 + 4], ebx
    mov dword ptr [r15 + 8], r12d
    mfence
    mov dword ptr [r15], 0x91001500
    jmp 2f

1:
    mov dword ptr [r15], 0xdead

2:
    mov eax, 8
    syscall
    ud2

__catten_el0_ping_end:

__catten_el0_pong_start:
    mov r15, 0x15000

    mov eax, 14
    syscall
    mov ebp, eax

1:
    mov eax, 16
    mov edi, ebp
    syscall
    test edi, edi
    jne 1b
    mov ebx, eax

    // SUBMIT(Read, buffer@0x16000, 32) and wait for it.
    mov eax, 1
    mov edi, 1
    mov esi, 0x16000
    mov edx, 32
    syscall
    mov ebp, eax
    mov eax, 4
    mov edi, ebp
    syscall

    // Drain entry zero from the shared CQ.
    mov r14, 0x14000
2:
    cmp dword ptr [r14], 0
    je 2b
    lfence
    mov r12d, dword ptr [r14 + 40]
    mov r13d, dword ptr [0x16000]

    // Complete Ping's transferred cap with result 99.
    mov eax, 2
    mov edi, ebx
    mov esi, 99
    syscall

    mov dword ptr [r15 + 20], ebx
    mov dword ptr [r15 + 24], r12d
    mov dword ptr [r15 + 28], r13d
    mfence
    mov dword ptr [r15 + 16], 0x10001000
    mov eax, 8
    syscall
    ud2

__catten_el0_pong_end:
