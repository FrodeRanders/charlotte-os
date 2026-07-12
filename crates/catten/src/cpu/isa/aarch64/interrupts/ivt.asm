// Define the optimized macros
.macro push_volatile_regs
    stp x30, xzr, [sp, #-16]!
    stp x18, xzr, [sp, #-16]!
    stp x16, x17, [sp, #-16]!
    stp x14, x15, [sp, #-16]!
    stp x12, x13, [sp, #-16]!
    stp x10, x11, [sp, #-16]!
    stp x8, x9, [sp, #-16]!
    stp x6, x7, [sp, #-16]!
    stp x4, x5, [sp, #-16]!
    stp x2, x3, [sp, #-16]!
    stp x0, x1, [sp, #-16]!
.endm

.macro pop_volatile_regs
    ldp x0, x1, [sp], #16
    ldp x2, x3, [sp], #16
    ldp x4, x5, [sp], #16
    ldp x6, x7, [sp], #16
    ldp x8, x9, [sp], #16
    ldp x10, x11, [sp], #16
    ldp x12, x13, [sp], #16
    ldp x14, x15, [sp], #16
    ldp x16, x17, [sp], #16
    ldp x18, xzr, [sp], #16
    ldp x30, xzr, [sp], #16
.endm

// Save/restore the exception-return state (ELR_EL1, SPSR_EL1, SP_EL0) onto the
// per-thread kernel stack. Used on the IRQ path, which may context-switch
// inside the dispatcher: while a preempted thread is descheduled, other
// threads' exceptions overwrite these banked system registers, so they must be
// preserved on the (stack-resident, per-thread) frame and restored on the way
// out. x9/x10 are scratch here and are already saved by push_volatile_regs.
.macro push_return_state
    mrs x9, elr_el1
    mrs x10, spsr_el1
    stp x9, x10, [sp, #-16]!
    mrs x9, sp_el0
    stp x9, xzr, [sp, #-16]!
.endm

.macro pop_return_state
    ldp x9, x10, [sp], #16
    msr sp_el0, x9
    ldp x9, x10, [sp], #16
    msr elr_el1, x9
    msr spsr_el1, x10
.endm

.text
.extern sync_dispatcher
.extern irq_dispatcher
.extern fiq_dispatcher
.extern serr_dispatcher
// Interrupt Vector Table
// Given the scheme we empoloy each used IVT entry is 22 instructions exactly while the ISA requires 32 instructions
// This means that we have 10 instructions of padding for each IVT entry
.balign 128
.global ivt
ivt:
// Exception from current EL while using SP_EL0 (EL1t).
// The kernel normally runs on SP_ELx, but a thread may be running in EL1t when
// an interrupt arrives, so these entries must dispatch just like the SP_ELx
// group rather than being left unused.
push_volatile_regs
// Pass the saved-register frame base (== current sp, which points at the
// saved x0/x1 pair) to sync_dispatcher as its first argument. Reading `sp`
// inside the compiled dispatcher would observe the value *after* its own
// prologue adjusts the stack, so the base must be captured here.
mov x0, sp
bl sync_dispatcher
pop_volatile_regs
eret
.balign 128
push_volatile_regs
b irq_common
.balign 128
push_volatile_regs
bl fiq_dispatcher
pop_volatile_regs
eret
.balign 128
push_volatile_regs
bl serr_dispatcher
pop_volatile_regs
eret
// Exception from current EL using SP_ELx
.balign 128
push_volatile_regs
// Pass the saved-register frame base (== current sp, which points at the
// saved x0/x1 pair) to sync_dispatcher as its first argument. Reading `sp`
// inside the compiled dispatcher would observe the value *after* its own
// prologue adjusts the stack, so the base must be captured here.
mov x0, sp
bl sync_dispatcher
pop_volatile_regs
eret

.balign 128
push_volatile_regs
b irq_common
.balign 128
push_volatile_regs
bl fiq_dispatcher
pop_volatile_regs
eret
.balign 128
push_volatile_regs
bl serr_dispatcher
pop_volatile_regs
eret
// Exception from a lower EL and at least one lower EL is AArch64
.balign 128
push_volatile_regs
// Pass the saved-register frame base (== current sp, which points at the
// saved x0/x1 pair) to sync_dispatcher as its first argument. Reading `sp`
// inside the compiled dispatcher would observe the value *after* its own
// prologue adjusts the stack, so the base must be captured here.
mov x0, sp
bl sync_dispatcher
pop_volatile_regs
eret
.balign 128
push_volatile_regs
b irq_common
.balign 128
push_volatile_regs
bl fiq_dispatcher
pop_volatile_regs
eret
.balign 128
push_volatile_regs
bl serr_dispatcher
pop_volatile_regs
eret
// Exception from a lower EL and all lower ELs are AArch32
// Unused because we don't support AArch32
.balign 128
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
nop
// End of IVT

// Shared IRQ handler, out of line so it is not constrained by the 128-byte
// vector slot. The three IRQ vectors tail-branch here after saving the
// volatile registers. Because `irq_dispatcher` may context-switch (the timer
// tick drives the scheduler), the exception-return state is saved on this
// thread's kernel stack and restored on the way out so a preempted thread —
// including an EL0 thread — resumes with the correct ELR/SPSR/SP_EL0.
irq_common:
push_return_state
bl irq_dispatcher
pop_return_state
pop_volatile_regs
eret
