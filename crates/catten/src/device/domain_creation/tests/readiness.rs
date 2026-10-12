//! Private-slot fixtures use the production grant owner and readiness path;
//! no live hardware or backend recovery state is manufactured.
use alloc::sync::Arc;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    cpu::scheduler::{
        self,
        threads::{
            MASTER_THREAD_TABLE,
            ThreadState,
        },
    },
    klib::observer::{
        CallOnNotify,
        Observable,
        Observer,
        WaitSponsor,
        registration::RegistrationError,
        waiter_budget,
    },
};

struct Unit {
    engine: bool,
    cells: bool,
    work: usize,
}
fn available(unit: &Unit) -> bool {
    unit.engine && unit.cells
}
fn slot() -> Mutex<UnitSlot<Unit>> {
    Mutex::new(UnitSlot::new(UnitState::Installed(Unit {
        engine: false,
        cells: true,
        work: 0,
    })))
}
fn enable(slot: &Mutex<UnitSlot<Unit>>) {
    let mut state = slot.lock();
    let unit = state.installed().unwrap();
    unit.engine = true;
    unit.cells = true;
    state.signal_if(available);
}
fn grant(root: memory::AddressSpaceHandle) -> PreparedDmaDomain {
    let mut grant = PreparedDmaDomain::new(root.id(), |_| Ok(())).unwrap();
    grant.resources.creation.wait_platform = Some(grant.resources.admission.platform());
    let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
    grant.resources.admission.reserve(&lifecycle).unwrap();
    grant
}

pub(super) fn serialized() {
    use crate::cpu::scheduler::deferred;
    deferred::drain();
    let baseline = crate::klib::observer::list_budget::node_used();
    for case in 0..6 {
        let root = crate::service::loader::create_user_address_space_handle();
        let mut grant = grant(root);
        let slot = slot();
        let mut held = if case == 5 {
            let mut state = slot.lock();
            let UnitState::Installed(unit) =
                core::mem::replace(&mut state.state, UnitState::Claimed)
            else {
                unreachable!("fixture slot was installed");
            };
            Some(DetachedDomain::new(unit))
        } else {
            None
        };
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let observer: Arc<dyn Observer> = CallOnNotify::new(move || {
            assert!(DEVICES.try_lock().is_some());
            assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
            assert!(MASTER_THREAD_TABLE.try_write().is_some());
            count.fetch_add(1, Ordering::Relaxed);
        });
        let sponsor = WaitSponsor::new(false);
        let mut tokens = alloc::vec::Vec::new();
        let mut waits = 0;
        let result = Preparing::begin_with(
            &slot,
            &mut grant.resources.creation,
            available,
            |interest| {
                waits += 1;
                assert_eq!(
                    memory::close_user_address_space_handle(root),
                    Err(memory::AddressSpaceCloseError::OperationsInFlight)
                );
                assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 1);
                assert!(slot.try_lock().is_some());
                if case == 2 {
                    for _ in 0..waiter_budget::SOURCE_LIMIT {
                        tokens.push(
                            interest
                                .try_register_waiter(Arc::downgrade(&observer), &sponsor)
                                .unwrap(),
                        );
                    }
                    assert_eq!(
                        interest
                            .try_register_waiter(Arc::downgrade(&observer), &sponsor)
                            .unwrap_err(),
                        RegistrationError::ResourceLimit
                    );
                    return Err(Error::WaitRegistrationFailed);
                }
                tokens.push(
                    interest.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap(),
                );
                {
                    let mut state = slot.lock();
                    if let Some(owner) = held.take() {
                        state.state = UnitState::Installed(owner.into_inner());
                    }
                    let unit = state.installed().unwrap();
                    unit.engine = true;
                    unit.cells = false; // Engine restored, physical finalization active.
                    state.signal_if(available);
                    assert!(!interest.completed());
                    assert_eq!(state.readiness.as_ref().unwrap().registered(), 1);
                    state.installed().unwrap().cells = true;
                    state.signal_if(available);
                    assert!(interest.completed());
                    assert_eq!(hits.load(Ordering::Relaxed), waits - 1);
                    assert!(
                        interest
                            .try_register_waiter(Arc::downgrade(&observer), &sponsor)
                            .unwrap()
                            .is_ready()
                    );
                }
                deferred::drain();
                assert_eq!(hits.load(Ordering::Relaxed), waits);
                if case == 4 && waits == 1 {
                    // A competitor wins before the consumer's next claim.
                    // The completed epoch gives no permission to steal it.
                    slot.lock().installed().unwrap().engine = false;
                }
                Ok(())
            },
            |platform| {
                assert!(!platform);
                assert!(slot.try_lock().is_some());
                if case == 1 {
                    return Err(Error::WaitRegistrationFailed);
                }
                let source = super::super::readiness::Source::prepare(platform)?;
                if case == 3 {
                    enable(&slot);
                } // Producer won preparation race.
                Ok(source)
            },
        );
        if case == 1 || case == 2 {
            assert_eq!(result.err(), Some(Error::WaitRegistrationFailed));
        } else {
            result.unwrap_or_else(|_| panic!("readiness claim failed")).restore(available);
        }
        assert_eq!(
            waits,
            match case {
                1 | 3 => 0,
                4 => 2,
                _ => 1,
            }
        );
        drop(tokens);
        assert_eq!(sponsor.used(), 0);
        grant.cancel_unpublished().unwrap_or_else(|_| panic!("unarmed grant cancellation failed"));
        enable(&slot);
        deferred::drain();
        memory::close_user_address_space_handle(root).unwrap();
        assert_eq!(crate::klib::observer::list_budget::node_used(), baseline);
    }
    // Default production wait refuses a caller's enclosing mask, without
    // allocating a source or enabling IRQs. Admission rollback still completes.
    let root = crate::service::loader::create_user_address_space_handle();
    let mut grant = grant(root);
    let slot = slot();
    {
        let _mask = crate::cpu::multiprocessor::interrupt_tracking::LocalInterruptMask::new();
        assert_eq!(
            Preparing::begin(&slot, &mut grant.resources.creation, available).err(),
            Some(Error::WaitRegistrationFailed)
        );
        assert!(!crate::cpu::isa::lp::ops::get_int_state());
        assert!(slot.lock().readiness.is_none());
    }
    grant
        .cancel_unpublished()
        .unwrap_or_else(|_| panic!("masked rejection retained unarmed grant"));
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(crate::klib::observer::list_budget::node_used(), baseline);
    crate::logln!(
        "[DMA creation readiness] exact root/reservation retained; detached cell gates engine \
         wake; deferred notification, late registration, source rejection, saturation, \
         preparation race, competing epoch and masked rejection passed"
    );
    cancellation_serialized();
}

fn cancel_serial(creation: &mut DmaCreation) -> Result<(), Error> {
    use crate::cpu::scheduler::deferred;
    assert_eq!(creation.id, Some(DOMAIN));
    let root = ROOT.lock().unwrap();
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| {
        super::available(|| SLOT.try_lock().is_some(), "cancellation fixture slot");
        super::available(|| DEVICES.try_lock().is_some(), "cancellation device registry");
        super::available(
            || memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
            "cancellation lifecycle",
        );
        super::available(|| MASTER_THREAD_TABLE.try_write().is_some(), "cancellation scheduler");
    });
    let sponsor = WaitSponsor::new(false);
    let mut token = None;
    let _held = super::super::readiness::admit_with(
        &SLOT,
        creation,
        available,
        |slot| {
            let unit = slot.installed()?;
            unit.work += 1;
            unit.engine = false;
            unit.cells = false;
            Ok(DetachedDomain::new(())) // Empty fixture payload, no hardware.
        },
        |interest| {
            assert_eq!(
                memory::close_user_address_space_handle(root),
                Err(memory::AddressSpaceCloseError::OperationsInFlight)
            );
            assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 1);
            token =
                Some(interest.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
            let mut state = SLOT.lock();
            state.installed().unwrap().engine = true;
            state.installed().unwrap().cells = false;
            state.signal_if(available);
            assert!(!interest.completed());
            state.installed().unwrap().cells = true;
            state.signal_if(available);
            assert!(interest.completed());
            drop(state);
            deferred::drain();
            Ok(())
        },
        super::super::readiness::Source::prepare,
    )?;
    assert_eq!(SLOT.lock().installed().unwrap().work, 1);
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    enable(&SLOT);
    drop(token);
    assert_eq!(sponsor.used(), 0);
    Ok(())
}

fn cancellation_serialized() {
    let root = crate::service::loader::create_user_address_space_handle();
    *ROOT.lock() = Some(root);
    let lists = crate::klib::observer::list_budget::node_used();
    let mut prepared = grant(root);
    prepared.resources.destroy = DmaDestruction::Prepared(cancel_serial);
    prepared.resources.creation.record(DOMAIN);
    prepared.cancel_unpublished().unwrap_or_else(|_| panic!("armed fixture cancellation failed"));
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 0);
    memory::close_user_address_space_handle(root).unwrap();
    *ROOT.lock() = None;
    assert_eq!(crate::klib::observer::list_budget::node_used(), lists);
    let mut state = SLOT.lock();
    let unit = state.installed().unwrap();
    unit.engine = false;
    unit.work = 0;
    crate::logln!(
        "[DMA grant cancellation readiness] armed grant kept exact root/reservation through \
         pre-destruction wait; engine-only return did not wake; one admitted cleanup and explicit \
         refund/root completion passed"
    );
}

pub(in crate::device) static REJECTED_CALLS: AtomicUsize = AtomicUsize::new(0);

pub(in crate::device) fn reject_cancellation(creation: &mut DmaCreation) -> Result<(), Error> {
    REJECTED_CALLS.fetch_add(1, Ordering::Relaxed);
    assert!(creation.id.is_some());
    super::super::readiness::admit_with(
        &SLOT,
        creation,
        available,
        |_| panic!("rejected cancellation reached descriptor work"),
        |_| Err(Error::WaitRegistrationFailed),
        super::super::readiness::Source::prepare,
    )
}

pub(in crate::device) fn finish_rejected_source() {
    enable(&SLOT);
    crate::cpu::scheduler::deferred::drain();
    SLOT.lock().installed().unwrap().engine = false;
}

static SLOT: Mutex<UnitSlot<Unit>> = Mutex::new(UnitSlot::new(UnitState::Installed(Unit {
    engine: false,
    cells: true,
    work: 0,
})));
static ROOT: Mutex<Option<memory::AddressSpaceHandle>> = Mutex::new(None);
static RESULT: Mutex<Option<DeviceCap>> = Mutex::new(None);
const CLIENT: memory::AddressSpaceId = 0x5d07;
const DOMAIN: u64 = 0xfeed;

extern "C" fn worker() {
    let root = ROOT.lock().unwrap();
    let cap = grant_dma_domain_with_policy(
        root.id(),
        true,
        |creation| {
            prepare(&SLOT, creation, available, |unit, creation| {
                unit.work += 1;
                creation.record(DOMAIN); // Fixture backend; never hardware authority.
                Ok(())
            })
            .map_err(|error| {
                DeviceError::DmaCreationFailed(creation_failure::Failure::backend(
                    creation_failure::Stage::Backend,
                    error,
                ))
            })
        },
        |_| Ok(()),
    )
    .expect("scheduled fixture grant failed");
    *RESULT.lock() = Some(cap);
}

pub(in crate::device) fn scheduled() {
    let root = crate::service::loader::create_user_address_space_handle();
    *ROOT.lock() = Some(root);
    crate::completion::open_address_space(CLIENT, 4);
    let tid = scheduler::spawn_thread_on_lp(
        memory::KERNEL_ASID,
        worker,
        crate::cpu::isa::lp::ops::get_lp_id(),
    );
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    let watched =
        crate::completion::observe_thread_exit_with_generation(CLIENT, tid, Some(generation))
            .unwrap();
    let deadline = scheduler::monotonic_millis().saturating_add(5_000);
    loop {
        if SLOT.lock().readiness.as_ref().is_some_and(|source| source.registered() == 1) {
            break;
        }
        assert!(scheduler::monotonic_millis() < deadline, "DMA grant never registered");
        scheduler::sleep_millis(1); // Bounded fixture observation only.
    }
    assert!(matches!(MASTER_THREAD_TABLE.read().get(tid).unwrap().state, ThreadState::Blocked(_)));
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 1);
    assert_eq!(SLOT.lock().installed().unwrap().work, 0);
    enable(&SLOT);
    assert!(crate::completion::wait_timeout(CLIENT, watched, 5_000).unwrap());
    assert_eq!(SLOT.lock().installed().unwrap().work, 1);
    let cap = RESULT.lock().take().unwrap();
    super::super::super::mapping::close_with(
        root.id(),
        cap,
        || {},
        |id| {
            assert_eq!(id, DOMAIN);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 0);
    *ROOT.lock() = None;
    memory::close_user_address_space_handle(root).unwrap();
    crate::completion::close(CLIENT, watched).unwrap();
    crate::completion::close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    crate::logln!(
        "[DMA creation readiness] scheduled grant parked with exact root/reservation, woke on \
         unit availability, created once and closed its original fixture authority"
    );
    scheduled_cancellation();
}

fn cancel_scheduled(creation: &mut DmaCreation) -> Result<(), Error> {
    assert_eq!(creation.id, Some(DOMAIN));
    let _held = super::super::readiness::admit(&SLOT, creation, available, |slot| {
        let unit = slot.installed()?;
        unit.work += 1;
        unit.engine = false;
        unit.cells = false;
        Ok(DetachedDomain::new(()))
    })?;
    enable(&SLOT);
    Ok(())
}

extern "C" fn cancellation_worker() {
    let root = ROOT.lock().unwrap();
    let mut prepared = grant(root);
    prepared.resources.destroy = DmaDestruction::Prepared(cancel_scheduled);
    prepared.resources.creation.record(DOMAIN);
    prepared
        .cancel_unpublished()
        .unwrap_or_else(|_| panic!("scheduled grant cancellation rejected"));
}

fn scheduled_cancellation() {
    let root = crate::service::loader::create_user_address_space_handle();
    *ROOT.lock() = Some(root);
    {
        let mut slot = SLOT.lock();
        let unit = slot.installed().unwrap();
        unit.engine = false;
        unit.work = 0;
    }
    crate::completion::open_address_space(CLIENT, 4);
    let tid = scheduler::spawn_thread_on_lp(
        memory::KERNEL_ASID,
        cancellation_worker,
        crate::cpu::isa::lp::ops::get_lp_id(),
    );
    let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
    let watched =
        crate::completion::observe_thread_exit_with_generation(CLIENT, tid, Some(generation))
            .unwrap();
    let deadline = scheduler::monotonic_millis().saturating_add(5_000);
    loop {
        if SLOT.lock().readiness.as_ref().is_some_and(|source| source.registered() == 1) {
            break;
        }
        assert!(scheduler::monotonic_millis() < deadline, "grant cancellation never registered");
        scheduler::sleep_millis(1); // Bounded fixture observation, not cleanup polling.
    }
    assert!(matches!(MASTER_THREAD_TABLE.read().get(tid).unwrap().state, ThreadState::Blocked(_)));
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 1);
    assert_eq!(SLOT.lock().installed().unwrap().work, 0);
    enable(&SLOT);
    assert!(crate::completion::wait_timeout(CLIENT, watched, 5_000).unwrap());
    assert_eq!(SLOT.lock().installed().unwrap().work, 1);
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 0);
    *ROOT.lock() = None;
    memory::close_user_address_space_handle(root).unwrap();
    crate::completion::close(CLIENT, watched).unwrap();
    crate::completion::close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    crate::logln!(
        "[DMA grant cancellation readiness] scheduled armed grant parked before descriptor work; \
         original reservation/root retained, one cleanup admitted and exact root closed"
    );
}
