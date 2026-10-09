//! Serialized pre-driver fixtures; injected release rejection is not a hardware timeout.
use super::*;
use crate::memory::physical::Error as PhysicalError;

pub(super) fn run() {
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
    let mut tables = Tables::new(Scope::Domain);
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
        let mut domain = Tables::new(Scope::Domain);
        assert_eq!(
            domain.allocate_with(1, PAGE, |_, _| panic!("node cap must precede allocation")),
            Err(Error::MapFailed)
        );
        let mut unit = Tables::new(Scope::Unit);
        unit.allocate_frame().unwrap();
        unit.cancel_unpublished().unwrap();
    }
    assert_eq!(used(), baseline);

    let mut abandoned = Tables::new(Scope::Domain);
    abandoned.allocate_frame().unwrap();
    abandoned.publish();
    drop(abandoned);
    assert_eq!(used(), (baseline.0 + 1, baseline.1 + 1));
    let mut rejected = Tables::new(Scope::Domain);
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
    drop(rejected);
    let retained = (baseline.0 + 5, baseline.1 + 5);
    assert_eq!(used(), retained);
    let mut successor = Tables::new(Scope::Domain);
    successor.allocate_frame().unwrap();
    successor.publish();
    successor.release().unwrap();
    successor.release().unwrap();
    drop(successor);
    assert_eq!(used(), retained);
    super::super::dma::test_table_admission();
    preparation_tests::run();
    detached_tests::run();
    crate::logln!(
        "[IOMMU admission] node/domain/unit limits, private rollback, partial release fence and \
         cached sparse walkers passed; original release fixtures retain 5 charged pages/3 frames"
    );
}

mod detached_tests;
mod preparation_tests;
