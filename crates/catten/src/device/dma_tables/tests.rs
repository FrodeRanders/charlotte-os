//! Serialized pre-driver fixtures; injected release rejection is not a hardware timeout.
use super::*;
use crate::memory::physical::Error as PhysicalError;

pub(super) fn run() {
    #[cfg(feature = "kernel_tests")]
    ledger::test_admission();
    ledger_before_physical();
    let mut pool = Pool::new(16);
    pool.reserve(12, Scope::Domain).unwrap();
    assert!(pool.reserve(1, Scope::Domain).is_err());
    assert_eq!(pool.total.used().pages, 12);
    pool.reserve(4, Scope::Unit).unwrap();
    assert!(pool.reserve(1, Scope::Unit).is_err());
    pool.release(12, Scope::Domain);
    pool.release(4, Scope::Unit);
    assert_eq!(pool.total.used().pages, 0);

    let baseline = used();
    let mut tables = Tables::new(Scope::Domain, false);
    assert_eq!(
        tables.allocate_with(1, PAGE, |_, _| Err(PhysicalError::OutOfFrames)),
        Err(Error::MapFailed)
    );
    assert_eq!(used(), baseline);
    let first = tables.allocate(2, PAGE * 2).unwrap();
    assert_eq!(u64::from(first) % (PAGE * 2) as u64, 0);
    assert!((0..PAGE * 2).all(|offset| unsafe { *first.into_hhdm_ptr::<u8>().add(offset) } == 0));
    tables.set_limit(2);
    assert_eq!(
        tables.allocate_with(1, PAGE, |_, _| panic!("table cap must precede allocation")),
        Err(Error::MapFailed)
    );
    tables.cancel_unpublished().unwrap();
    assert_eq!(used(), baseline);
    {
        let _pressure = ClientPressure::new();
        let mut domain = Tables::new(Scope::Domain, false);
        assert_eq!(
            domain.allocate_with(1, PAGE, |_, _| panic!("node cap must precede allocation")),
            Err(Error::MapFailed)
        );
        let mut unit = Tables::new(Scope::Unit, true);
        unit.allocate_frame().unwrap();
        unit.cancel_unpublished().unwrap();
    }
    assert_eq!(used(), baseline);

    let mut abandoned = Tables::new(Scope::Domain, false);
    abandoned.allocate_frame().unwrap();
    abandoned.publish();
    drop(abandoned);
    assert_eq!(used(), (baseline.0 + 1, baseline.1 + 1));
    let mut rejected = Tables::new(Scope::Domain, false);
    rejected.allocate(4, PAGE).unwrap();
    rejected.publish();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut calls = 0;
    assert_eq!(
        rejected.release_with(|frame| {
            calls += 1;
            if calls == 3 {
                Err(PhysicalError::InvalidPAddr)
            } else {
                PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame)
            }
        }),
        Err(Error::MapFailed)
    );
    assert_eq!(calls, 3);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free + 2);
    assert_eq!(
        rejected.release_with(|_| panic!("partially freed table walk must never retry")),
        Err(Error::MapFailed)
    );
    let metadata = crate::device::backend_registry::budget::snapshot();
    drop_under_guards(|| crate::device::backend_registry::budget::with_locked(|| drop(rejected)));
    assert_eq!(crate::device::backend_registry::budget::snapshot(), metadata);
    let retained = (baseline.0 + 5, baseline.1 + 5);
    assert_eq!(used(), retained);
    let mut successor = Tables::new(Scope::Domain, false);
    successor.allocate_frame().unwrap();
    successor.publish();
    successor.release().unwrap();
    successor.release().unwrap();
    drop(successor);
    assert_eq!(used(), retained);
    super::super::dma::test_table_admission();
    preparation_tests::run();
    detached_tests::run();
    let command_charges = used();
    let command_free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    super::super::dma::test_command_engines();
    assert_eq!(used(), command_charges);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), command_free);
    super::super::unit_initialization::test_admission();
    crate::logln!(
        "[IOMMU admission] node/domain/unit limits, private rollback, partial release fence and \
         cached sparse walkers passed; original release fixtures retain 5 charged pages/3 frames"
    );
}

mod detached_tests;
mod preparation_tests;

pub(super) fn drop_under_guards(action: impl FnOnce()) {
    preparation_tests::drop_under_guards(action);
}

fn ledger_before_physical() {
    use crate::{
        device::backend_registry::budget,
        klib::admission::{
            IommuNodes,
            Reservation,
        },
    };
    let before = budget::snapshot();
    let pages = used();
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut ordinary = Tables::new(Scope::Domain, false);
    ordinary.allocate_frame().unwrap();
    let ceiling = crate::resource_policy::POLICY.iommu_nodes.ordinary[0];
    let pressure =
        Reservation::<IommuNodes, 2>::reserve(false, [ceiling - budget::snapshot().1[0], 0])
            .unwrap();
    let full = budget::snapshot();
    let mut denied = Tables::new(Scope::Domain, false);
    {
        let _heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        assert_eq!(
            denied
                .allocate_with(1, PAGE, |_, _| panic!("ledger denial reached physical allocator")),
            Err(Error::MapFailed)
        );
    }
    assert_eq!(denied.pages(), 0);
    assert!(!denied.uncertain);
    assert_eq!(budget::snapshot(), full);
    // The admitted buffer still has three slots; quota pressure must not force
    // a pointless replacement or block use of already paid-for capacity.
    for _ in 0..3 {
        ordinary.allocate_frame().unwrap();
    }
    assert_eq!(budget::snapshot(), full);
    let charged = used();
    assert_eq!(
        ordinary.allocate_with(1, PAGE, |_, _| panic!("growth denial reached physical allocator")),
        Err(Error::MapFailed)
    );
    assert_eq!(used(), charged);
    assert_eq!(ordinary.regions.capacity(), 4);
    let mut trusted = Tables::new(Scope::Domain, true);
    trusted.allocate_frame().unwrap();
    assert_eq!(budget::snapshot().1, full.1);
    trusted.cancel_unpublished().unwrap();
    assert_eq!(budget::snapshot(), full);
    drop(pressure);
    ordinary.cancel_unpublished().unwrap();
    denied.cancel_unpublished().unwrap();
    assert_eq!(used(), pages);
    assert_eq!(budget::snapshot(), before);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    crate::logln!(
        "[IOMMU ledger physical admission] fresh/growth quota denial precedes physical \
         allocation; existing capacity and captured trusted class progress; page reservations, \
         buffers and charges recover exactly"
    );
}
