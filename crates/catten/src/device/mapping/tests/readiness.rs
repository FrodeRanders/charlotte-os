//! Private fixed-slot probes borrow the real public root/claim and DMA pin.
//! No fixture slot or scalar result represents live hardware authority.
use super::*;
use crate::{
    cpu::{
        multiprocessor::interrupt_tracking::LocalInterruptMask,
        scheduler::{
            self,
            threads::{
                MASTER_THREAD_TABLE,
                ThreadState,
            },
        },
    },
    device::{
        domain_creation::readiness::{
            self,
            Source,
        },
        unit_initialization::{
            UnitSlot,
            UnitState,
        },
    },
    memory,
};

struct Unit {
    domain: Option<u64>,
    engine: Option<u64>,
    claims: usize,
}
fn ready(unit: &Unit) -> bool {
    unit.domain.is_some() && unit.engine.is_some()
}
fn slot() -> Mutex<UnitSlot<Unit>> {
    Mutex::new(UnitSlot::new(UnitState::Installed(Unit {
        domain: Some(u64::MAX),
        engine: None,
        claims: 0,
    })))
}
fn enable(slot: &Mutex<UnitSlot<Unit>>) {
    let mut state = slot.lock();
    state.installed().unwrap().engine = Some(1);
    state.signal_if(ready);
}
fn claim(
    slot: &mut UnitSlot<Unit>,
    owner: &mut DmaOperation,
) -> Result<MappingMaintenance<u64, u64>, dma::Error> {
    let unit = slot.installed()?;
    assert_eq!(unit.domain, Some(owner.id));
    unit.claims += 1;
    Ok(MappingMaintenance::new(
        Maintenance::new(unit.domain.take().unwrap(), unit.engine.take().unwrap()),
        owner.pending.take().unwrap(),
    ))
}
fn finish(slot: &Mutex<UnitSlot<Unit>>, held: MappingMaintenance<u64, u64>) {
    {
        let mut state = slot.lock();
        let unit = state.installed().unwrap();
        assert!(unit.domain.is_none() && unit.engine.is_none());
        unit.domain = Some(held.held.domain.into_inner());
        unit.engine = Some(held.held.commands.into_inner());
        state.signal_if(ready);
    }
    held.pending.release();
}
fn pinned(root: memory::AddressSpaceHandle, cap: u64, memory_cap: u64) -> DmaOperation {
    let mut owner = DmaOperation::begin(root.id(), cap).unwrap();
    owner.pending = Some(PendingPin::new(Some(
        object::pin_for_dma(root.id(), memory_cap, true, true, false).unwrap(),
    )));
    owner
}
fn retained(root: memory::AddressSpaceHandle, cap: u64, memory_cap: u64) {
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(close_cap(root.id(), cap), Err(DeviceError::OperationInFlight));
    assert_eq!(
        object::try_close_cap(root.id(), memory_cap),
        Err(object::MemoryObjectError::LendingActive)
    );
}

pub(in crate::device::mapping) fn serialized() {
    for case in 0..4 {
        let root = crate::service::loader::create_user_address_space_handle();
        let cap = grant(root);
        let memory_cap = object::allocate(root.id(), 2).unwrap();
        let mut owner = pinned(root, cap, memory_cap);
        let slot = slot();
        let mut waits = 0;
        let result = if case == 3 {
            let mask = LocalInterruptMask::new();
            let result = readiness::admit(&slot, &mut owner, ready, claim);
            assert!(!crate::cpu::isa::lp::ops::get_int_state());
            drop(mask);
            result
        } else {
            readiness::admit_with(
                &slot,
                &mut owner,
                ready,
                claim,
                |interest| {
                    waits += 1;
                    retained(root, cap, memory_cap);
                    assert!(slot.try_lock().is_some());
                    if case == 1 {
                        return Err(dma::Error::WaitRegistrationFailed);
                    }
                    {
                        let mut state = slot.lock();
                        let unit = state.installed().unwrap();
                        unit.engine = Some(1);
                        unit.domain = None;
                        state.signal_if(ready);
                    }
                    assert!(!interest.completed()); // Detached cell is not readiness.
                    {
                        let mut state = slot.lock();
                        state.installed().unwrap().domain = Some(u64::MAX);
                        state.signal_if(ready);
                    }
                    assert!(interest.completed()); // Producer precedes registration.
                    Ok(())
                },
                |platform| {
                    assert!(!platform);
                    assert!(slot.try_lock().is_some());
                    if case == 0 {
                        Err(dma::Error::WaitRegistrationFailed)
                    } else {
                        Source::prepare(platform)
                    }
                },
            )
        };
        match result {
            Ok(held) => {
                assert_eq!(case, 2);
                assert_eq!(waits, 1);
                retained(root, cap, memory_cap);
                finish(&slot, held);
                assert_eq!(slot.lock().installed().unwrap().claims, 1);
            }
            Err(error) => {
                assert_eq!(error, dma::Error::WaitRegistrationFailed);
                assert_eq!(slot.lock().installed().unwrap().claims, 0);
                retained(root, cap, memory_cap);
                owner.pending.take().unwrap().release();
                enable(&slot);
            }
        }
        owner.complete().unwrap();
        crate::cpu::scheduler::deferred::drain();
        object::try_close_cap(root.id(), memory_cap).unwrap();
        close_with(root.id(), cap, || {}, destroy_fake).unwrap();
        memory::close_user_address_space_handle(root).unwrap();
    }
    crate::logln!(
        "[DMA map readiness] source/wait rejection, captured ordinary sponsorship, \
         pre-registration wake, detached-cell fence, masked rejection and exactly one admitted \
         claim passed"
    );
}

pub(in crate::device::mapping) fn abandon(root: memory::AddressSpaceHandle) {
    // Reuse the existing abandonment fixture root; no new retirement family.
    let cap = grant(root);
    let memory_cap = object::allocate(root.id(), 2).unwrap();
    let mut owner = pinned(root, cap, memory_cap);
    let source = Source::prepare(false).unwrap();
    *owner.interest = Some(source.interest());
    crate::device::dma_tables::test_drop_under_guards(|| {
        let _devices = DEVICES.lock();
        drop(owner);
    });
    source.finish();
    crate::cpu::scheduler::deferred::drain();
    retained(root, cap, memory_cap);
    crate::logln!(
        "[DMA map readiness] pre-claim Drop under guards retains original root, public claim, \
         two-page pin and interest; source closure is no cleanup proof"
    );
}

static SLOT: Mutex<UnitSlot<Unit>> = Mutex::new(UnitSlot::new(UnitState::Installed(Unit {
    domain: Some(u64::MAX),
    engine: None,
    claims: 0,
})));
static INPUT: Mutex<Option<(memory::AddressSpaceHandle, u64, u64)>> = Mutex::new(None);
const CLIENT: memory::AddressSpaceId = 0x5d08;
extern "C" fn worker() {
    let (root, cap, memory_cap) = INPUT.lock().unwrap();
    assert_eq!(
        map_with(root.id(), cap, |_, owner| {
            owner.pending = Some(PendingPin::new(Some(
                object::pin_for_dma(root.id(), memory_cap, true, true, false).unwrap(),
            )));
            let held = readiness::admit(&SLOT, owner, ready, claim)?;
            retained(root, cap, memory_cap);
            finish(&SLOT, held);
            Ok(0x1000) // Metadata-only result, never an actual IOVA grant.
        }),
        Ok(0x1000)
    );
}
pub(in crate::device::mapping) fn scheduled() {
    let root = crate::service::loader::create_user_address_space_handle();
    let cap = grant(root);
    let memory_cap = object::allocate(root.id(), 2).unwrap();
    *INPUT.lock() = Some((root, cap, memory_cap));
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
        assert!(scheduler::monotonic_millis() < deadline, "map waiter never registered");
        // Bounded fixture-only registration observation has no producer signal.
        // The resource wait being tested parks on its actual source.
        scheduler::sleep_millis(1);
    }
    assert!(matches!(MASTER_THREAD_TABLE.read().get(tid).unwrap().state, ThreadState::Blocked(_)));
    retained(root, cap, memory_cap);
    assert_eq!(SLOT.lock().installed().unwrap().claims, 0);
    enable(&SLOT);
    assert!(crate::completion::wait_timeout(CLIENT, watched, 5_000).unwrap());
    assert_eq!(SLOT.lock().installed().unwrap().claims, 1);
    object::try_close_cap(root.id(), memory_cap).unwrap();
    close_with(root.id(), cap, || {}, destroy_fake).unwrap();
    *INPUT.lock() = None;
    memory::close_user_address_space_handle(root).unwrap();
    crate::completion::close(CLIENT, watched).unwrap();
    crate::completion::close_address_space(CLIENT);
    crate::capability::close_address_space(CLIENT);
    crate::logln!(
        "[DMA map readiness] scheduled map parked with original root/claim/two-page pin, woke on \
         unit restoration, ran once and released original authority/backing"
    );
}

static REAL_INPUT: Mutex<Option<(memory::AddressSpaceHandle, u64, u64, bool)>> = Mutex::new(None);
static REAL_RESULT: Mutex<Option<Result<u64, MapError>>> = Mutex::new(None);
extern "C" fn real_worker() {
    let (root, cap, memory_cap, exclusive) = REAL_INPUT.lock().unwrap();
    *REAL_RESULT.lock() =
        Some(super::super::super::dma_map_scheduled(root.id(), cap, memory_cap, 3, exclusive));
}

pub(in crate::device::mapping) fn real(root: memory::AddressSpaceHandle, cap: DeviceCap) {
    assert!(crate::cpu::isa::lp::ops::get_int_state());
    const REAL_CLIENT: memory::AddressSpaceId = 0x5d09;
    for exclusive in [false, true] {
        // The pre-driver caller owns this real domain. Keep its exact root
        // while a complete-unit fixture claim prevents first map admission.
        let root_owner = AddressSpaceOperation::acquire(root).unwrap();
        let memory_cap = object::allocate(root.id(), 2).unwrap();
        *REAL_INPUT.lock() = Some((root, cap, memory_cap, exclusive));
        crate::completion::open_address_space(REAL_CLIENT, 4);
        let mut watched = None;
        dma::test_with_claimed_unit(|| {
            let tid = scheduler::spawn_thread_on_lp(
                memory::KERNEL_ASID,
                real_worker,
                crate::cpu::isa::lp::ops::get_lp_id(),
            );
            let generation = MASTER_THREAD_TABLE.read().get(tid).unwrap().generation;
            watched = Some(
                crate::completion::observe_thread_exit_with_generation(
                    REAL_CLIENT,
                    tid,
                    Some(generation),
                )
                .unwrap(),
            );
            let deadline = scheduler::monotonic_millis().saturating_add(5_000);
            loop {
                if dma::test_registered_unit_waiters() == 1 {
                    break;
                }
                assert!(
                    scheduler::monotonic_millis() < deadline,
                    "real map waiter never registered"
                );
                scheduler::sleep_millis(1); // Bounded fixture registration observation only.
            }
            assert!(matches!(
                MASTER_THREAD_TABLE.read().get(tid).unwrap().state,
                ThreadState::Blocked(_)
            ));
            assert!(REAL_RESULT.lock().is_none());
            retained(root, cap, memory_cap);
        }); // Restore the same actual unit and publish readiness after guards.
        let watched = watched.unwrap();
        assert!(crate::completion::wait_timeout(REAL_CLIENT, watched, 5_000).unwrap());
        let iova = REAL_RESULT.lock().take().unwrap().unwrap();
        assert_ne!(iova, 0);
        // No retry: confirmed real backend unmap precedes physical memory release.
        super::super::super::dma_unmap(root.id(), cap, iova).unwrap();
        object::try_close_cap(root.id(), memory_cap).unwrap();
        *REAL_INPUT.lock() = None;
        root_owner.release().unwrap();
        crate::completion::close(REAL_CLIENT, watched).unwrap();
        crate::completion::close_address_space(REAL_CLIENT);
        crate::capability::close_address_space(REAL_CLIENT);
    }
    crate::logln!(
        "[DMA map readiness] real map and map-exclusive parked behind complete unit ownership, \
         resumed original pin/root/cap once, and confirmed unmap before backing release"
    );
}
