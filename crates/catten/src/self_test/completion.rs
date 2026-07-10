//! Self-tests for the completion-capability subsystem (Option C prototype).
//!
//! These are whitebox integration tests of the kernel side of the async syscall
//! ABI (`docs/async-syscall-abi.md`). They validate the submission-side
//! semantics that exist today — the capability table, the buffer-ownership /
//! deferred-reclaim contract, the observer-signal path that [`wait`] relies on,
//! and submission backpressure — without needing the (not-yet-existing) EL0
//! syscall entry or a running scheduler.
//!
//! [`wait`](crate::completion::wait) itself is not exercised here because it
//! blocks the calling thread, which requires the scheduler to be yielding;
//! self-tests run before the BSP yields. The signal path `wait` depends on is
//! validated directly via [`observe`](crate::completion::observe).

use alloc::sync::{Arc, Weak};
use alloc::vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::completion::{self, CancelState, OpCode, OpResult, SubmitError};
use crate::klib::observer::{CallOnNotify, Observer};
use crate::logln;

pub fn test_completion_caps() {
    logln!("Testing completion-capability subsystem...");

    // A test address space id distinct from the kernel's (ASID 0). Capacity 2
    // so the backpressure case is reachable.
    let asid = 0xC0FFEE;
    completion::open_address_space(asid, 2);

    // --- submit transfers buffer ownership to the kernel ---------------------
    let cap = completion::submit(asid, OpCode::Read, Some(vec![0u8; 4])).unwrap();
    assert!(completion::holds_buffer(asid, cap).unwrap());

    // --- the observer-signal path (the mechanism `wait` uses) fires ----------
    let fired = Arc::new(AtomicBool::new(false));
    let flag = fired.clone();
    let observer = CallOnNotify::new(move || flag.store(true, Ordering::SeqCst));
    completion::observe(asid, cap, Arc::downgrade(&observer) as Weak<dyn Observer>).unwrap();

    // --- complete posts the result, wakes observers, hands the buffer back ---
    completion::complete(asid, cap, OpResult::Ok(4)).unwrap();
    assert!(fired.load(Ordering::SeqCst), "completion must notify observers");
    assert!(!completion::holds_buffer(asid, cap).unwrap());

    let done = completion::poll(asid, cap).unwrap().expect("must be complete");
    assert!(matches!(done.result, OpResult::Ok(4)));
    assert_eq!(done.buffer.as_deref(), Some(&[0u8; 4][..]));

    // --- close frees the slot ------------------------------------------------
    completion::close(asid, cap).unwrap();
    assert!(completion::poll(asid, cap).is_err());

    // --- cancel retains the buffer until the terminal completion -------------
    let cap2 = completion::submit(asid, OpCode::Write, Some(vec![1u8, 2, 3])).unwrap();
    assert_eq!(
        completion::cancel(asid, cap2).unwrap(),
        CancelState::CancelRequested
    );
    // Deferred reclaim: the kernel still owns the buffer while cancellation is
    // in flight — it may still touch it until the terminal completion.
    assert!(completion::holds_buffer(asid, cap2).unwrap());

    completion::complete(asid, cap2, OpResult::Ok(3)).unwrap();
    let done = completion::poll(asid, cap2).unwrap().expect("must be complete");
    assert!(matches!(done.result, OpResult::Cancelled));
    assert_eq!(done.buffer.as_deref(), Some(&[1u8, 2, 3][..]));
    completion::close(asid, cap2).unwrap();

    // --- submission backpressure (capacity 2) --------------------------------
    let _a = completion::submit(asid, OpCode::Nop, None).unwrap();
    let _b = completion::submit(asid, OpCode::Nop, None).unwrap();
    assert_eq!(
        completion::submit(asid, OpCode::Nop, None),
        Err(SubmitError::WouldBlock)
    );

    completion::close_address_space(asid);
    logln!("Completion-capability subsystem tests passed.");
}
