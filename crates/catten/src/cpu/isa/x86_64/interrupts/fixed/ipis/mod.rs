core::arch::global_asm!(include_str!("ipis.asm"));

unsafe extern "custom" {
    pub fn isr_asynchronous_ipi();
    pub fn isr_synchronous_ipi();
}

#[unsafe(no_mangle)]
pub extern "C" fn ih_asynchronous_ipi() {
    // Drain pending IPI RPCs queued for this LP. The architecture-independent
    // handler dispatches TLB maintenance, scheduler wakeups, and typed
    // Closures (ShardMailbox).
    crate::cpu::multiprocessor::ipi::drain_local_ipi_queue();
}

#[unsafe(no_mangle)]
pub extern "C" fn ih_synchronous_ipi() {
    // Placeholder: the synchronous IPI mechanism (broadcast + barrier) is not
    // yet used by any kernel subsystem. When cross-LP synchronisation is
    // needed (e.g. TLB shootdown requiring all targets to acknowledge), this
    // handler will implement the rendezvous protocol.
    //
    // The assembly stub at ipis.asm:42-66 already has the barrier/decrement
    // logic wired up; once the kernel calls send_sync_ipi(), this handler
    // should decrement the barrier and the last LP to arrive signals the
    // initiator.
}
