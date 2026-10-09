//! Serialized QEMU probes and synthetic complete-unit/grant retention.
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    device::*,
    memory::{
        self,
        object,
    },
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static ROOT: Mutex<Option<memory::AddressSpaceHandle>> = Mutex::new(None);
static MEMORY: AtomicUsize = AtomicUsize::new(0);
static IRQ: AtomicBool = AtomicBool::new(false);
static COUNTS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];

fn available(mut probe: impl FnMut() -> bool, name: &'static str) {
    let deadline = crate::self_test::results::Deadline::after_millis(1000);
    while !probe() {
        deadline.assert_pending(name);
        core::hint::spin_loop();
    }
}

pub(super) fn boundary(phase: Phase) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let root = ROOT.lock().unwrap();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), IRQ.load(Ordering::Relaxed));
    dma::test_assert_backend_available();
    available(|| memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(), "creation lifecycle");
    available(|| DEVICES.try_lock().is_some(), "creation device registry");
    available(|| memory::ADDRESS_SPACE_TABLE.try_lock().is_some(), "creation root table");
    available(|| memory::KERNEL_AS.try_lock().is_some(), "creation kernel table");
    available(
        || memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(),
        "creation physical allocator",
    );
    available(
        || memory::allocators::global_allocator::PRIMARY_ALLOCATOR.try_lock().is_some(),
        "creation heap",
    );
    assert_eq!(memory::current_address_space_handle(root.id()), Some(root));
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(dma::initialize_early(), Err(Error::OperationInFlight));
    assert_eq!(dma::destroy_domain(u64::MAX), Err(Error::OperationInFlight));
    assert_eq!(dma::unmap(u64::MAX, 0), Err(Error::OperationInFlight));
    assert_eq!(
        dma::create_domain_with_reset(u32::MAX, None, &mut DmaCreation::new(), |_, _| panic!(
            "creation claim reached reset"
        )),
        Err(Error::OperationInFlight)
    );
    let cap = MEMORY.load(Ordering::Relaxed) as u64;
    if cap != 0 {
        assert_eq!(
            dma::map(u64::MAX, root.id(), cap, dma::Direction::from_bits(3).unwrap(), false),
            Err(Error::OperationInFlight)
        );
    }
    COUNTS[phase as usize].fetch_add(1, Ordering::Relaxed);
}

pub(in crate::device) fn with_real<T>(
    root: memory::AddressSpaceHandle,
    memory: Option<u64>,
    work: impl FnOnce() -> T,
) -> T {
    assert!(!ACTIVE.load(Ordering::Acquire));
    *ROOT.lock() = Some(root);
    MEMORY.store(memory.unwrap_or(0) as usize, Ordering::Relaxed);
    IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
    for counter in &COUNTS {
        counter.store(0, Ordering::Relaxed);
    }
    ACTIVE.store(true, Ordering::Release);
    let result = work();
    ACTIVE.store(false, Ordering::Release);
    *ROOT.lock() = None;
    for counter in &COUNTS {
        assert_eq!(counter.load(Ordering::Acquire), 1);
    }
    crate::logln!(
        "[DMA creation phases] complete installed-unit claim, allocation, initial configuration \
         and exact restoration outside backend/lifecycle/device/table/allocator guards; preserved \
         IRQ policy, exact-root busy close and nested mutation/reset exclusion passed"
    );
    result
}

static DROPS: AtomicUsize = AtomicUsize::new(0);
struct Metadata;
impl Drop for Metadata {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }
}
struct Domain {
    _tables: super::super::dma_tables::Tables,
    _pin: object::DmaPin,
    _metadata: alloc::vec::Vec<Metadata>,
}
struct Payload {
    commands: Option<alloc::vec::Vec<u64>>,
    domains: alloc::collections::BTreeMap<u64, Option<Domain>>,
    _tables: super::super::dma_tables::Tables,
    _sources: alloc::collections::BTreeMap<u16, u64>,
}
fn ready(payload: &Payload) -> bool {
    payload.commands.is_some() && payload.domains.values().all(Option::is_some)
}
fn payload() -> Payload {
    use super::super::dma_tables::{
        Scope,
        Tables,
    };
    Payload {
        commands: Some(alloc::vec![0x1234]),
        domains: alloc::collections::BTreeMap::new(),
        _tables: Tables::new(Scope::Unit),
        _sources: alloc::collections::BTreeMap::new(),
    }
}

#[allow(clippy::drop_non_drop)] // Exercise the grant's implicit ManuallyDrop fallback under guards.
pub(super) fn run() {
    let slot = Mutex::new(UnitState::Installed(payload()));
    let mut grant = DmaCreation::new();
    for fails in [false, true] {
        assert_eq!(
            prepare(&slot, &mut grant, ready, |unit, _| {
                assert!(matches!(*slot.lock(), UnitState::Claimed));
                assert_eq!(slot.lock().installed().err(), Some(Error::OperationInFlight));
                assert!(Preparing::begin(&slot, &mut DmaCreation::new(), ready).is_err());
                unit.commands.as_mut().unwrap().push(0xbeef);
                if fails {
                    Err(Error::HardwareTimeout)
                } else {
                    Ok(())
                }
            }),
            if fails {
                Err(Error::HardwareTimeout)
            } else {
                Ok(())
            }
        );
    }
    {
        let mut state = slot.lock();
        let unit = state.installed().unwrap();
        assert_eq!(unit.commands.as_ref().unwrap(), &[0x1234, 0xbeef, 0xbeef]);
        unit.domains.insert(1, None);
    }
    assert_eq!(
        prepare(&slot, &mut grant, ready, |_, _| panic!("physical-finalization cell stolen")),
        Err(Error::OperationInFlight)
    );
    {
        let mut state = slot.lock();
        let unit = state.installed().unwrap();
        unit.domains.remove(&1);
        unit.commands = None;
    }
    assert_eq!(
        prepare(&slot, &mut grant, ready, |_, _| panic!("maintenance engine stolen")),
        Err(Error::OperationInFlight)
    );
    drop(slot);

    // The complete installed unit carries both table scopes, command/source/
    // domain metadata and a real memory pin beside the exact grant admission.
    use super::super::dma_tables::{
        self,
        Scope,
        Tables,
    };
    let baseline = dma_tables::used();
    let root = crate::service::loader::create_user_address_space_handle();
    let memory = object::allocate(root.id(), 2).unwrap();
    let mut grant =
        PreparedDmaDomain::new(root.id(), |_| panic!("abandoned creation replayed")).unwrap();
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        grant.resources.admission.resources.reservation = Some(
            crate::capability::reserve_in_lifecycle(
                root.id(),
                crate::capability::ObjectKind::Device,
                &lifecycle,
            )
            .unwrap(),
        );
    }
    let mut value = payload();
    value._tables.allocate_frame().unwrap();
    value._tables.publish(); // Synthetic state, no hardware base.
    let mut tables = Tables::new(Scope::Domain);
    tables.allocate_frame().unwrap();
    tables.publish();
    value.domains.insert(
        1,
        Some(Domain {
            _tables: tables,
            _pin: object::pin_for_dma(root.id(), memory, true, true, false).unwrap(),
            _metadata: alloc::vec![Metadata],
        }),
    );
    value._sources.insert(0x1234, 1);
    let slot = Mutex::new(UnitState::Installed(value));
    let drops = DROPS.load(Ordering::Relaxed);
    dma_tables::test_drop_under_guards(|| {
        crate::capability::admission_tests::test_with_registry_locked(|| {
            let owner = Preparing::begin(&slot, &mut grant.resources.creation, ready).unwrap();
            let _slot = slot.lock();
            let _devices = DEVICES.lock();
            drop(owner);
            drop(grant);
        });
    });
    assert_eq!(DROPS.load(Ordering::Relaxed), drops);
    assert_eq!(dma_tables::used(), (baseline.0 + 2, baseline.1 + 1));
    assert!(matches!(*slot.lock(), UnitState::Claimed));
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(
        object::try_close_cap(root.id(), memory),
        Err(object::MemoryObjectError::LendingActive)
    );
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 2);
    let fresh = crate::service::loader::create_user_address_space_handle();
    assert_ne!(fresh.id(), root.id());
    memory::close_user_address_space_handle(fresh).unwrap();
    crate::logln!(
        "[DMA creation ownership] exact unit/engine state restored on success/error; \
         detached-domain and absent-engine claims reject before work; complete-unit/grant Drop \
         under all guards retains one unit and one domain table, two data frames, metadata, exact \
         root and original authority reservation"
    );
}
