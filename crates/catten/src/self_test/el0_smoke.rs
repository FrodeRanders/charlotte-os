//! Self-test: load a real x86_64 Rust ELF (the `smoke` service) into ring 3
//! and verify it runs the full `catten-rt` startup path.
//!
//! Unlike the hand-written assembly stub in [`el0`](crate::self_test::el0),
//! this test loads a Rust-compiled service image through the shared ELF
//! [`loader`](crate::service::loader), so it exercises the same path the
//! service supervisor uses on AArch64: signature verification, `PT_LOAD`
//! segment mapping, runtime-page setup, and the `catten-rt` entry trampoline.
//!
//! The image is the `smoke` service (`crates/catten-services/src/bin/smoke.rs`),
//! which writes a sentinel to the status page, emits one kernel log line
//! through the syscall ABI, and exits. The verifier polls the status page via
//! its HHDM alias and reports success once the sentinel appears.

use crate::{
    cpu::scheduler::spawn_thread,
    logln,
    service::loader::load_domain,
};

const SMOKE_ELF: &[u8] = include_bytes!(env!("CATTEN_X86_64_SMOKE_ELF"));

const SMOKE_SENTINEL: u32 = 0xdead_beef;

pub fn test_el0_smoke() {
    logln!("Testing EL0 x86_64 Rust ELF round-trip (smoke service)...");

    let loaded = load_domain(SMOKE_ELF);
    logln!("[el0 smoke] loaded asid={} entry={:#x}", loaded.asid, loaded.entry_vaddr);

    let entry: extern "C" fn() =
        unsafe { core::mem::transmute::<usize, extern "C" fn()>(loaded.entry_vaddr) };
    let tid = spawn_thread(loaded.asid, entry);
    logln!("[el0 smoke] spawned tid={} asid={}", tid, loaded.asid);

    // Poll the status page (via its HHDM alias) until the service writes the
    // sentinel, then tear the payload down. Uses cooperative yields rather than
    // a blocking timer wait so it does not add a waiter to the boot path.
    let deadline = crate::self_test::results::Deadline::after_millis(10_000);
    let status_ptr: *mut u8 = loaded.status_frame.into();
    let status = status_ptr as *const u32;
    loop {
        let sentinel = unsafe { core::ptr::read_volatile(status) };
        if sentinel == SMOKE_SENTINEL {
            logln!("[el0 smoke] SUCCESS: x86_64 Rust ELF ran at ring 3 and wrote the sentinel.");
            let _ =
                crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER.read().abort_thread(tid);
            return;
        }
        deadline.assert_pending("el0 smoke status sentinel");
        crate::cpu::scheduler::yield_lp();
    }
}
