.section .text.catten_el0_demo, "ax"
.balign 16

.global __catten_el0_demo_coord_start
.global __catten_el0_demo_coord_end
.global __catten_el0_demo_worker_start
.global __catten_el0_demo_worker_end

__catten_el0_demo_coord_start:
    mov r15, 0x13000

    // COMPLETION_SUBMIT(Nop) -> cap in RAX.
    mov eax, 1
    xor edi, edi
    xor esi, esi
    xor edx, edx
    syscall
    mov ebx, eax
    mov dword ptr [r15 + 4], ebx

    // Spawn worker@0x11000 on LP1.
    mov eax, 7
    mov edi, 0x11000
    mov esi, 1
    syscall

    // Wait for the worker to complete the submitted operation.
    mov eax, 4
    mov edi, ebx
    syscall

    // Drain entry zero from the shared completion queue.
    mov r14, 0x12000
1:
    cmp dword ptr [r14], 0
    je 1b
    lfence
    mov r12d, dword ptr [r14 + 40]

    mov dword ptr [r15 + 4], ebx
    mov dword ptr [r15 + 8], r12d
    mfence
    mov dword ptr [r15], 0xc0de
    mov eax, 8
    syscall
    ud2

__catten_el0_demo_coord_end:

__catten_el0_demo_worker_start:
    mov r15, 0x13000
    mov edi, dword ptr [r15 + 4]
    mov esi, 42
    mov eax, 2
    syscall
    mov eax, 8
    syscall
    ud2

__catten_el0_demo_worker_end:
