//! Private preparation and guarded abandonment; no hardware publication.
use super::*;

pub(super) fn run() {
    ordinary_rejection();
    let baseline = used();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for scope in [Scope::Domain, Scope::Unit] {
        table_abandonment(scope);
        region_abandonment(scope);
    }
    assert_eq!(used(), (baseline.0 + 14, baseline.1 + 7));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 10);
    for scope in [Scope::Domain, Scope::Unit] {
        let (tables, _) = Tables::prepare_unpublished(scope, Tables::allocate_frame).unwrap();
        tables.cancel_unpublished().unwrap();
        assert_eq!(used(), (baseline.0 + 14, baseline.1 + 7));
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 10);
    }
    crate::logln!(
        "[IOMMU preparation abandonment] explicit metadata/allocation/prefix rollback and Drop \
         under backend/lifecycle/table/heap/physical/pool guards passed; 14 original charges (7 \
         domain), 10 frames and table ledgers retained"
    );
}

fn drop_under_guards(action: impl FnOnce()) {
    let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
    crate::device::dma::test_with_backend_locked(|| {
        let _table = crate::memory::ADDRESS_SPACE_TABLE.lock();
        let _kernel = crate::memory::KERNEL_AS.lock();
        let pool = POOL.lock();
        let physical = PHYSICAL_FRAME_ALLOCATOR.lock();
        let _heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let free = physical.free_frames();
        let charged = (pool.total.used().pages, pool.clients.used().pages);
        action();
        assert_eq!(physical.free_frames(), free);
        assert_eq!((pool.total.used().pages, pool.clients.used().pages), charged);
    });
}

fn ordinary_rejection() {
    for scope in [Scope::Domain, Scope::Unit] {
        let baseline = used();
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let mut tables = Tables::new(scope);
        assert_eq!(
            tables.allocate_prepared_with(
                1,
                PAGE,
                |_| Err(Error::MapFailed),
                |_, _| panic!("metadata rejection reached physical allocation")
            ),
            Err(Error::MapFailed)
        );
        assert_eq!(tables.pages(), 0);
        assert!(!tables.uncertain);
        assert_eq!(used(), baseline);
        assert_eq!(
            tables.allocate_with(1, PAGE, |_, _| Err(PhysicalError::OutOfFrames)),
            Err(Error::MapFailed)
        );
        assert!(!tables.uncertain);
        assert_eq!(used(), baseline);
        // A known-unused region explicitly refunds only its own reservation.
        PreparingRegion::reserve(&mut tables, 1).unwrap().cancel_unpublished().unwrap();
        assert_eq!(tables.pages(), 0);
        // Explicit allocated-region cancellation releases backing before refund.
        let mut region = PreparingRegion::reserve(&mut tables, 2).unwrap();
        region.frame = Some(PHYSICAL_FRAME_ALLOCATOR.lock().allocate_contiguous(2, PAGE).unwrap());
        region.cancel_unpublished().unwrap();
        assert!(!tables.uncertain);
        assert_eq!(used(), baseline);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        tables.cancel_unpublished().unwrap();
        // Every initialized prefix is still private when ordinary preparation
        // rejects; the shared constructor helper explicitly cancels it.
        for prefix in 1..=3 {
            assert!(
                Tables::prepare_unpublished(scope, |tables| {
                    for _ in 0..prefix {
                        tables.allocate_frame()?;
                    }
                    Err::<(), _>(Error::MapFailed)
                })
                .is_err()
            );
            assert_eq!(used(), baseline);
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        }
        let (tables, _) =
            Tables::prepare_unpublished(scope, |tables| tables.allocate_frame()).unwrap();
        tables.cancel_unpublished().unwrap();
        assert_eq!(used(), baseline);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    }
}

fn table_abandonment(scope: Scope) {
    for published in [false, true] {
        let mut tables = Tables::new(scope);
        tables.allocate_frame().unwrap();
        assert!(!tables.regions.is_empty() && tables.regions.capacity() != 0);
        if published {
            tables.publish();
        }
        if published {
            drop_under_guards(|| assert_eq!(tables.cancel_unpublished(), Err(Error::MapFailed)));
        } else {
            drop_under_guards(|| drop(tables));
        }
    }
}

fn region_abandonment(scope: Scope) {
    for kind in 0..4 {
        let mut tables = Tables::new(scope);
        let pages = if kind == 3 {
            2
        } else {
            1
        };
        let mut region = PreparingRegion::reserve(&mut tables, pages).unwrap();
        if kind != 0 {
            region.frame =
                Some(PHYSICAL_FRAME_ALLOCATOR.lock().allocate_contiguous(pages, PAGE).unwrap());
        }
        if kind == 2 {
            // Transfer armed, before ledger insertion: no ordinary rollback.
            region.tables.uncertain = true;
            region.finished = true;
        }
        if kind == 3 {
            let mut calls = 0;
            assert_eq!(
                region.rollback_with(|frame| {
                    calls += 1;
                    if calls == 2 {
                        Err(PhysicalError::InvalidPAddr)
                    } else {
                        PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame)
                    }
                }),
                Err(Error::MapFailed)
            );
            assert_eq!(calls, 2);
        }
        drop_under_guards(|| drop(region));
        assert!(tables.uncertain);
        assert_eq!(tables.pages(), pages as u64);
        assert_eq!(
            tables.allocate_with(1, PAGE, |_, _| panic!("uncertain parent allocated")),
            Err(Error::MapFailed)
        );
        assert_eq!(
            tables.release_with(|_| panic!("uncertain parent retried")),
            Err(Error::MapFailed)
        );
        assert_eq!(tables.release_with(|_| panic!("frozen parent retried")), Err(Error::MapFailed));
        drop_under_guards(|| drop(tables));
    }
}
