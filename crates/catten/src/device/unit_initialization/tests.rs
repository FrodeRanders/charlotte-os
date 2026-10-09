//! Serialized pre-driver evidence. Private prefix failure never changes a
//! hardware base; synthetic published rejection is not a hardware timeout.
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::memory::PHYSICAL_FRAME_ALLOCATOR;

static REJECT_PREFIX: AtomicUsize = AtomicUsize::new(0);
static REJECT_COMPLETE: AtomicBool = AtomicBool::new(false);
static PROBING: AtomicBool = AtomicBool::new(false);
static ENTRY_IRQ: AtomicBool = AtomicBool::new(false);
static PROBES: AtomicUsize = AtomicUsize::new(0);
static WAITS: AtomicUsize = AtomicUsize::new(0);
static PUBLICATIONS: AtomicUsize = AtomicUsize::new(0);

pub(in crate::device) fn reject_allocation() -> bool {
    REJECT_PREFIX.try_update(Ordering::AcqRel, Ordering::Relaxed, |n| n.checked_sub(1)) == Ok(1)
}

pub(in crate::device) fn reject_complete() -> bool {
    REJECT_COMPLETE.swap(false, Ordering::AcqRel)
}

pub(super) fn probe() {
    if !PROBING.load(Ordering::Acquire) {
        return;
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), ENTRY_IRQ.load(Ordering::Relaxed));
    crate::device::dma::test_assert_backend_available();
    for (available, name) in [
        (crate::memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(), "unit lifecycle"),
        (crate::device::DEVICES.try_lock().is_some(), "unit devices"),
        (crate::memory::ADDRESS_SPACE_TABLE.try_lock().is_some(), "unit root table"),
        (crate::memory::KERNEL_AS.try_lock().is_some(), "unit kernel table"),
        (PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(), "unit physical allocator"),
        (
            crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.try_lock().is_some(),
            "unit heap",
        ),
    ] {
        assert!(available, "initialization holds {}", name);
    }
    PROBES.fetch_add(1, Ordering::Relaxed);
}

pub(in crate::device) fn wait_boundary() {
    if PROBING.load(Ordering::Acquire) {
        probe();
        WAITS.fetch_add(1, Ordering::Relaxed);
    }
}

pub(in crate::device) fn publication_boundary() {
    if PROBING.load(Ordering::Acquire) {
        probe();
        PUBLICATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

pub(in crate::device) fn test_real<T: UnitBacking>(
    slot: &Mutex<UnitState<T>>,
    prepare: fn() -> Result<DetachedDomain<T>, Rejected<T>>,
    prefixes: usize,
    waits: usize,
) {
    assert!(matches!(*slot.lock(), UnitState::Vacant));
    let baseline = crate::device::dma_tables::used();
    ENTRY_IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
    PROBING.store(true, Ordering::Release);
    for prefix in 1..=prefixes + 1 {
        PROBES.store(0, Ordering::Relaxed);
        if prefix <= prefixes {
            REJECT_PREFIX.store(prefix, Ordering::Release);
        } else {
            REJECT_COMPLETE.store(true, Ordering::Release);
        }
        let result = initialize(slot, prepare, |_| true);
        if prefix == 1 && result == Err(Error::Unsupported) {
            REJECT_PREFIX.store(0, Ordering::Release);
            PROBING.store(false, Ordering::Release);
            assert_eq!(crate::device::dma_tables::used(), baseline);
            crate::logln!(
                "[IOMMU unit initialization] unsupported platform; prefix fixture skipped"
            );
            return;
        }
        assert_eq!(result, Err(Error::MapFailed));
        assert_eq!(REJECT_PREFIX.load(Ordering::Acquire), 0);
        assert!(!REJECT_COMPLETE.load(Ordering::Acquire));
        assert_eq!(PROBES.load(Ordering::Relaxed), 3);
        assert_eq!(WAITS.load(Ordering::Relaxed), 0);
        assert_eq!(PUBLICATIONS.load(Ordering::Relaxed), 0);
        assert!(matches!(*slot.lock(), UnitState::Vacant));
        assert_eq!(crate::device::dma_tables::used(), baseline);
    }
    initialize(slot, prepare, |_| true).unwrap();
    assert_eq!(WAITS.load(Ordering::Relaxed), waits);
    assert_eq!(PUBLICATIONS.load(Ordering::Relaxed), 1);
    assert!(matches!(*slot.lock(), UnitState::Installed(_)));
    initialize(slot, || panic!("installed unit was initialized twice"), |_| true).unwrap();
    PROBING.store(false, Ordering::Release);
    crate::logln!(
        "[IOMMU unit initialization] {} allocated region prefixes plus complete private \
         preparation refunded; {} real waits and publication outside \
         backend/lifecycle/device/table guards; installed once, caller IRQ policy preserved",
        prefixes,
        waits
    );
}

static DROPS: AtomicUsize = AtomicUsize::new(0);
struct Metadata;
impl Drop for Metadata {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }
}
struct Payload {
    tables: Tables,
    _metadata: alloc::vec::Vec<Metadata>,
}
impl UnitBacking for Payload {
    fn tables(&mut self) -> &mut Tables {
        &mut self.tables
    }
}
fn payload(pages: usize) -> DetachedDomain<Payload> {
    let mut owner = DetachedDomain::new(Payload {
        tables: Tables::new(super::super::dma_tables::Scope::Unit),
        _metadata: alloc::vec![Metadata],
    });
    for _ in 0..pages {
        owner.value_mut().tables.allocate_frame().unwrap();
    }
    owner
}
fn fenced(slot: &Mutex<UnitState<Payload>>) {
    assert!(matches!(*slot.lock(), UnitState::Claimed));
    assert_eq!(slot.lock().installed().err(), Some(Error::OperationInFlight));
    assert_eq!(
        initialize(slot, || panic!("fenced initialization retried"), |_| true),
        Err(Error::OperationInFlight)
    );
}

pub(super) fn run() {
    let baseline = super::super::dma_tables::used();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let drops = DROPS.load(Ordering::Relaxed);
    let slot = Mutex::new(UnitState::<Payload>::Vacant);
    assert_eq!(slot.lock().installed().err(), Some(Error::Unsupported));
    assert_eq!(
        initialize(&slot, || Err(Rejected::before_backing(Error::Unsupported)), |_| true),
        Err(Error::Unsupported)
    );
    assert!(matches!(*slot.lock(), UnitState::Vacant));
    assert_eq!(
        initialize(
            &slot,
            || {
                fenced(&slot);
                Err(Rejected::with_owner(Error::MapFailed, payload(2)))
            },
            |_| true
        ),
        Err(Error::MapFailed)
    );
    assert!(matches!(*slot.lock(), UnitState::Vacant));
    assert_eq!(DROPS.load(Ordering::Relaxed), drops + 1);
    assert_eq!(super::super::dma_tables::used(), baseline);

    // An abandoned empty claim cannot be reopened merely because no backing
    // was allocated. Hold its own slot as well as the existing global guards.
    let empty = Mutex::new(UnitState::<Payload>::Claimed);
    let claim = Claim {
        slot: &empty,
        owner: None,
    };
    super::super::dma_tables::test_drop_under_guards(|| {
        let _slot = empty.lock();
        drop(claim);
    });
    fenced(&empty);

    let abandoned = Mutex::new(UnitState::Claimed);
    let claim = Claim {
        slot: &abandoned,
        owner: Some(payload(1)),
    };
    super::super::dma_tables::test_drop_under_guards(|| {
        let _slot = abandoned.lock();
        drop(claim);
    });
    fenced(&abandoned);
    assert_eq!(DROPS.load(Ordering::Relaxed), drops + 1);

    let failed = Mutex::new(UnitState::Claimed);
    let claim = Claim {
        slot: &failed,
        owner: Some(payload(2)),
    };
    let mut calls = 0;
    claim.cancel_private(|tables| {
        tables.cancel_private_with(|frame| {
            calls += 1;
            if calls == 2 {
                return Err(crate::memory::physical::Error::InvalidPAddr);
            }
            PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame)
        })
    });
    assert_eq!(calls, 2);
    fenced(&failed);
    assert_eq!(DROPS.load(Ordering::Relaxed), drops + 1);

    // A started control write can be uncertain even before a new base was
    // published. Private backing alone is not permission to reopen the unit.
    let uncertain = Mutex::new(UnitState::Vacant);
    assert_eq!(
        initialize(
            &uncertain,
            || Err(Rejected::retain_owner(Error::HardwareTimeout, payload(1))),
            |_| true
        ),
        Err(Error::HardwareTimeout)
    );
    fenced(&uncertain);
    assert_eq!(DROPS.load(Ordering::Relaxed), drops + 1);

    let published = Mutex::new(UnitState::Vacant);
    let mut owner = payload(1);
    owner.value_mut().tables.publish(); // Synthetic state, no hardware descriptor.
    assert_eq!(
        initialize(
            &published,
            || Err(Rejected::retain_owner(Error::HardwareTimeout, owner)),
            |_| true
        ),
        Err(Error::HardwareTimeout)
    );
    fenced(&published);
    assert_eq!(DROPS.load(Ordering::Relaxed), drops + 1);
    assert_eq!(super::super::dma_tables::used(), (baseline.0 + 5, baseline.1));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 4);
    crate::logln!(
        "[IOMMU unit retention] empty/private abandonment under guards, partial release without \
         retry and uncertain control/published rejection remain fenced; 5 original unit charges/4 \
         frames and complete metadata retained"
    );
}
