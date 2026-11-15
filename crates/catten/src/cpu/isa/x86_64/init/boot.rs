//! Raw Limine entries establish FP/SIMD state before any compiled Rust.

use core::arch::naked_asm;

#[cfg(any(target_feature = "avx", target_feature = "avx2", target_feature = "avx512f"))]
compile_error!("x86 kernel AVX code generation requires an extended-state ownership contract");

/// Initialize the x86-64 FP baseline before entering the kernel's SysV code.
///
/// # Safety
/// Call only from a raw boot entry with a valid aligned kernel stack, before
/// executing compiled Rust. This requires CPL0 and does not restore old state.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn x86_boot_fp_init() {
    naked_asm!(
        "push rbx",
        // CPUID leaves the AP's Limine argument in RDI intact. Unsupported
        // hardware halts before enabling FP, allocating or calling Rust.
        "mov eax, 1",
        "cpuid",
        "and edx, 0x05000000", // FXSR and SSE2
        "cmp edx, 0x05000000",
        "jne 3f",
        // Regression builds deliberately enter initialization with FP disabled.
        // The assembler removes this entire fixture from minimal images.
        ".if {poison}",
        "mov rax, cr0",
        "or rax, 0xc", // EM and TS
        "mov cr0, rax",
        "mov rax, cr4",
        "and rax, -1537", // clear OSFXSR and OSXMMEXCPT
        "btr rax, 18",
        "mov cr4, rax",
        ".endif",
        "mov rax, cr0",
        "and rax, -13", // clear inherited EM and TS
        "or rax, 0x22", // MP and NE
        "mov cr0, rax",
        "mov rax, cr4",
        "or rax, 0x600", // OSFXSR and OSXMMEXCPT
        "btr rax, 18", // no unsaved AVX/XSAVE state
        "mov cr4, rax",
        "fninit",
        "sub rsp, 16",
        "mov dword ptr [rsp], 0x1f80",
        "ldmxcsr [rsp]",
        "pxor xmm0, xmm0",
        "pxor xmm1, xmm1",
        "pxor xmm2, xmm2",
        "pxor xmm3, xmm3",
        "pxor xmm4, xmm4",
        "pxor xmm5, xmm5",
        "pxor xmm6, xmm6",
        "pxor xmm7, xmm7",
        "pxor xmm8, xmm8",
        "pxor xmm9, xmm9",
        "pxor xmm10, xmm10",
        "pxor xmm11, xmm11",
        "pxor xmm12, xmm12",
        "pxor xmm13, xmm13",
        "pxor xmm14, xmm14",
        "pxor xmm15, xmm15",
        "add rsp, 16",
        "pop rbx",
        "ret",
        "3:",
        "cli",
        "hlt",
        "jmp 3b",
        poison = const cfg!(feature = "kernel_tests") as usize,
    );
}

/// Limine AP callback, before any Rust prologue can use an XMM register.
///
/// # Safety
/// Limine must provide its valid MP argument and kernel stack exactly once.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn x86_ap_entry(_info: &limine::mp::MpInfo) -> ! {
    naked_asm!(
        "cli",
        "cld",
        "and rsp, -16",
        "call {fp_init}",
        "call {entry}",
        "ud2",
        fp_init = sym x86_boot_fp_init,
        entry = sym crate::ap_main,
    );
}
