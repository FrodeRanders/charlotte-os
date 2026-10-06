# User faults and initial CPU state

User execution begins with an isolated architectural state. The kernel sets
only the entry point, stack pointer and required privilege/control state;
application-visible general and SIMD register payloads start at zero. This
contract is enforced by the architecture trampolines before the ELF or runtime
executes, including `SPAWN_THREAD`.

ARM first entry clears x0–x30, v0–v31, FPCR and FPSR after programming the banked
return entry/SP. x86 first entry clears all GPRs except RSP. Its pinned context
owns a 16-byte-aligned FX image plus user FS and GS bases. Eager switching saves
and restores x87/MMX/SSE state and both TLS bases; a new context has zero register
payload, empty x87 tags, default x87 control `0x037f` and MXCSR `0x1f80`.

x86 LP initialization requires FXSR/SSE2, enables OSFXSR/OSXMMEXCPT, clears
CR0.EM/TS and leaves OSXSAVE disabled. AVX and other XSAVE extensions require a
larger owned context and validation before enablement. The kernel still builds
with software float; interrupt/syscall Rust must not introduce unowned SIMD use.
Active kernel GS remains the per-LP base. The hidden user GS base follows its
thread, including switches through kernel workers and migration.

Ordinary x86 exception stubs pass the hardware-saved CS to one fault handler.
Only ring-3 origin qualifies for application containment. Valid not-present
data heap/stack growth retries; unrecovered user faults abort the domain using
the translation context active at exception entry. The handler enables interrupts
after trusted frame/GS setup, before locking for growth or retirement. Stubs
mask them again before GS restoration. Kernel faults, double fault, machine
check and unsolicited NMI remain fatal; requested watchdog NMIs are separate.
Never classify a kernel exception as a tenant fault using only page-fault flags
or scheduler TID lookup.

The EL0 verifier snapshots machine state before runtime startup. x86 also
checks state preservation through a timer wait and another domain's first entry,
and executes invalid opcode, divide-by-zero, privileged instruction, unmapped
access, NX fetch and rejected stack-growth cases. These tests accompany
[SEC-23–25](../reports/audits/2026-10-05-security-follow-up.md).
