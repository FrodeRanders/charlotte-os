//! Kernel fixtures for scheduler event admission and allocation-free queue publication.

use alloc::{
    boxed::Box,
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::{
    PreparedEvent,
    TimerEvent,
    TimerEventKey,
    budget::{
        self,
        SchedulerSponsor,
    },
    queue::{
        Events,
        Node,
    },
};
use crate::{
    cpu::{
        isa::lp::ops::get_int_state,
        multiprocessor::interrupt_tracking::LocalInterruptMask,
        scheduler::{
            block_until,
            monotonic_millis,
            sleep_millis,
            system_scheduler::get_thread_id,
            threads::{
                MASTER_THREAD_TABLE,
                ThreadGeneration,
                ThreadId,
                ThreadState,
            },
        },
    },
    klib::{
        observer::{
            CallOnNotify,
            Observer,
            waiter_source::WaiterSource,
        },
        time::duration::ExtDuration,
    },
};

pub(crate) fn test_admission() {
    let baseline = budget::node_used();
    let sponsor = SchedulerSponsor::new(false);
    let mut events = Events::default();
    for deadline in (1..=budget::MAX_DOMAIN_TIMERS).rev() {
        let mut event = TimerEvent::from(deadline as u64);
        event._charge = Some(sponsor.reserve().unwrap());
        events.insert_prepared(Box::try_new(Node::new(event)).unwrap());
    }
    assert_eq!(sponsor.used(), 1024);
    assert!(sponsor.reserve().is_err());
    let mut quantum =
        TimerEvent::keyed(ExtDuration::from_millis(1), TimerEventKey::SchedulerQuantum);
    quantum.deadline = 2;
    events.insert_quantum(quantum);
    assert_eq!(events.len(), 1025);
    assert!(events.iter().zip(events.iter().skip(1)).all(|(a, b)| a.deadline <= b.deadline));
    assert_eq!(events.pop_front().unwrap().deadline, 1);
    assert_eq!(sponsor.used(), 1023);
    let charge = sponsor.reserve().unwrap();
    drop(charge);
    assert_eq!(events.pop_front().unwrap().key, Some(TimerEventKey::SchedulerQuantum));
    events.retain(|event| event.deadline % 2 == 0);
    assert_eq!(sponsor.used(), 512);
    drop(events); // Iterative reclamation of a real linked footprint.
    assert_eq!(sponsor.used(), 0);

    let entry_irq = get_int_state();
    let (event, handle) =
        TimerEvent::charged(ExtDuration::from_millis(60_000), sponsor.reserve().unwrap()).unwrap();
    assert!(PreparedEvent::new_with(event, |_| Err(())).is_err());
    assert_eq!(sponsor.used(), 0);
    assert_eq!(get_int_state(), entry_irq);
    drop(handle);
    let (event, handle) =
        TimerEvent::charged(ExtDuration::from_millis(60_000), sponsor.reserve().unwrap()).unwrap();
    let prepared = PreparedEvent::new(event).unwrap();
    assert_eq!(get_int_state(), entry_irq);
    assert_eq!(sponsor.used(), 1);
    drop(prepared);
    assert_eq!(sponsor.used(), 0);
    assert_eq!(get_int_state(), entry_irq);
    drop(handle);

    let retained = sponsor.reserve().unwrap();
    sponsor.retire();
    assert!(sponsor.reserve().is_err());
    let replacement = SchedulerSponsor::new(false);
    let fresh = replacement.reserve().unwrap();
    drop(retained);
    assert_eq!(replacement.used(), 1);
    drop(fresh);
    let handle = crate::service::loader::create_user_address_space_handle();
    let old = crate::memory::budget::timer_sponsor(handle.id());
    let retained = old.reserve().unwrap();
    crate::memory::budget::retire(handle);
    assert!(old.reserve().is_err());
    crate::memory::close_user_address_space_handle(handle).unwrap();
    let replacement = crate::service::loader::create_user_address_space_handle();
    assert_eq!(handle.id(), replacement.id());
    assert_ne!(handle.generation(), replacement.generation());
    let new = crate::memory::budget::timer_sponsor(replacement.id());
    let fresh = new.reserve().unwrap();
    drop(retained);
    assert_eq!(old.used(), 0);
    assert_eq!(new.used(), 1);
    drop(fresh);
    crate::memory::close_user_address_space_handle(replacement).unwrap();
    let promoted = SchedulerSponsor::new(false);
    let ordinary_before = budget::node_used().1;
    let old = promoted.reserve().unwrap();
    promoted.mark_platform();
    let platform = promoted.reserve().unwrap();
    assert_eq!(budget::node_used().1, ordinary_before + 1);
    drop((old, platform));

    // Shared completion/scheduler node pool and reserve; maximum node pressure
    // is counter-only, not an allocation of its full backing footprint.
    let domains: Vec<_> = (0..6).map(|_| SchedulerSponsor::new(false)).collect();
    let mut charges = Vec::new();
    for domain in &domains {
        for _ in 0..1024 {
            charges.push(domain.reserve().unwrap());
        }
    }
    assert!(SchedulerSponsor::new(false).reserve().is_err());
    let completion = budget::DomainBudget::new(1);
    assert!(budget::reserve(&completion, false).is_err());
    let reserved: Vec<_> = (0..2).map(|_| SchedulerSponsor::new(true)).collect();
    for domain in &reserved {
        for _ in 0..1024 {
            charges.push(domain.reserve().unwrap());
        }
    }
    assert!(budget::reserve(&completion, true).is_err());
    drop(charges);
    assert_eq!(budget::node_used(), baseline);
    crate::logln!(
        "[timer events] SUCCESS: scheduler/domain/node admission, shared reserve, prepared-node \
         rollback, sorted/iterative reclamation, inline quantum and retirement"
    );
}

/// Kernel fixture only: substitute this executing thread's sponsor to force
/// deterministic rejection without consuming other threads' shared allowance.
struct SponsorFixture {
    tid: ThreadId,
    generation: ThreadGeneration,
    original: SchedulerSponsor,
}
impl SponsorFixture {
    fn install(sponsor: SchedulerSponsor) -> Self {
        let tid = get_thread_id().unwrap();
        let mut table = MASTER_THREAD_TABLE.write();
        let thread = table.get_mut(tid).unwrap();
        Self {
            tid,
            generation: thread.generation,
            original: core::mem::replace(&mut thread.timer_sponsor, sponsor),
        }
    }
}
impl Drop for SponsorFixture {
    fn drop(&mut self) {
        let mut table = MASTER_THREAD_TABLE.write();
        let thread = table.get_mut(self.tid).unwrap();
        assert_eq!(thread.generation, self.generation);
        thread.timer_sponsor = self.original.clone();
    }
}

pub(crate) fn test_scheduled_cleanup() {
    let sponsor = SchedulerSponsor::new(true);
    let fixture = SponsorFixture::install(sponsor.clone());
    let mut charges = Vec::new();
    for _ in 0..1024 {
        charges.push(sponsor.reserve().unwrap());
    }
    let source = WaiterSource::new();
    let constraints = MASTER_THREAD_TABLE.read().get(fixture.tid).unwrap().migration_constraints;
    assert!(!block_until(&source, 1, || false));
    assert_eq!(source.registered(), 0);
    // Synthetic completion namespace / trap frame are kernel ABI fixtures,
    // not a real EL0 domain-saturation test. No backing memory is delegated.
    let asid = 0xfa11;
    crate::completion::open_address_space(asid, 2);
    let cap = crate::completion::submit_timer(asid, 60_000).unwrap();
    let mut frame = crate::syscall::TrapFrame {
        regs: [0; 19],
        elr_el1: 0,
        spsr_el1: 0,
        sp_el0: 0,
        lp_id: crate::cpu::isa::lp::ops::get_lp_id(),
        asid,
    };
    frame.regs[1] = cap;
    frame.regs[2] = 1;
    crate::syscall::syscall_dispatch(&mut frame, crate::syscall::call_no::COMPLETION_WAIT_TIMEOUT);
    assert_eq!(frame.regs[0], catten_syscall::completion_status::WAIT_ADMISSION_FAILED);
    assert!(crate::completion::poll(asid, cap).unwrap().is_none());
    assert!(!crate::completion::wait_on_cq_timeout(asid, 0, 1, 1));
    crate::completion::cancel(asid, cap).unwrap();
    crate::completion::close(asid, cap).unwrap();
    crate::completion::close_address_space(asid);
    let started = monotonic_millis();
    sleep_millis(2);
    assert!(monotonic_millis().saturating_sub(started) >= 2);
    {
        let table = MASTER_THREAD_TABLE.read();
        let thread = table.get(fixture.tid).unwrap();
        assert!(matches!(thread.state, ThreadState::Running(_)));
        assert_eq!(thread.migration_constraints, constraints);
    }
    drop(charges);
    // Real publication/cancellation under a busy local queue retains the
    // charge, then owner-side reconciliation removes the flagged node.
    let hits = Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    let (prepared, handle) =
        super::prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
    prepared.event().register_observer(Arc::downgrade(&observer));
    prepared.enqueue();
    {
        let _mask = LocalInterruptMask::new();
        let queue = super::TIMER_QUEUES.try_get_mut().unwrap();
        assert!(!super::cancel_event(handle));
        assert_eq!(sponsor.used(), 1);
        drop(queue);
    }
    super::process_local_events();
    assert_eq!(sponsor.used(), 0);
    assert_eq!(hits.load(Ordering::Relaxed), 0);
    let (prepared, handle) =
        super::prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
    let lp = crate::cpu::isa::lp::ops::get_lp_id();
    // Simulate preparation before relocation; publication must set the actual
    // queue owner visible to every clone of the cancellation state.
    handle.state.owner_lp.store(lp.wrapping_add(1), Ordering::Release);
    prepared.enqueue();
    assert_eq!(handle.state.owner_lp.load(Ordering::Acquire), lp);
    assert!(super::cancel_event(handle));
    assert_eq!(sponsor.used(), 0);
    let (prepared, handle) =
        super::prepare_watchdog(ExtDuration::from_millis(60_000), &sponsor).unwrap();
    assert!(!super::cancel_event(handle)); // Not yet published.
    assert_eq!(sponsor.used(), 1);
    prepared.enqueue(); // Discards the cancelled prepared node, without a wake.
    assert_eq!(sponsor.used(), 0);
    for _ in 0..64 {
        sleep_millis(1);
        assert_eq!(sponsor.used(), 0);
    }
    for _ in 0..64 {
        assert!(!block_until(&source, 1, || false));
        assert_eq!(sponsor.used(), 0);
    }
    drop(fixture);
    crate::logln!(
        "[timer events] SUCCESS: pre-park rejection, sleep fallback, deferred queue reclamation \
         and sleep/watchdog accounting"
    );
}
