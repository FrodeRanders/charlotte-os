// Each entry owns its interrupted x87/SSE image on its aligned kernel stack.
// Nested IRQs and suspended syscall handlers retain separate images. AVX is
// unavailable because init_lp_state leaves OSXSAVE disabled.
.macro catten_save_fp
    sub rsp, 528
    fxsave64 [rsp]
    fninit
    mov dword ptr [rsp + 512], 0x1f80
    ldmxcsr [rsp + 512]
.endm

.macro catten_restore_fp
    fxrstor64 [rsp]
    add rsp, 528
.endm
