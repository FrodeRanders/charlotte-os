//! Empty real buffers and borrowed scalar records; these own no hardware tables.
use super::*;
use crate::{
    device::backend_registry::budget,
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn allocate(regions: &mut Vec<Region>, capacity: usize) -> Result<(), Error> {
    regions.try_reserve_exact(capacity).map_err(|_| Error::MapFailed)
}
fn region(index: usize) -> Region {
    Region {
        base: crate::memory::physical::PAddr::from((index * 4096) as u64),
        pages: 1,
    }
}
fn buffers(platform: bool) -> Vec<Backing> {
    let bytes = if platform {
        crate::resource_policy::POLICY.iommu_nodes.total[0]
    } else {
        crate::resource_policy::POLICY.iommu_nodes.ordinary[0]
    };
    let mut buffers = Vec::with_capacity((bytes / 16384 + 256) as usize);
    for capacity in [1024, 4] {
        while let Ok(backing) = Backing::prepare(platform, capacity, allocate) {
            buffers.push(backing);
        }
    }
    buffers
}

pub(in crate::device::dma_tables) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let before = budget::snapshot();
    for platform in [false, true] {
        let mut ledger = Ledger::new();
        ledger.prepare(platform, 8).unwrap();
        for index in 0..4 {
            ledger.push(region(index));
        }
        let old = budget::snapshot();
        assert_eq!(old.0, [before.0[0] + 64, before.0[1]]);
        assert_eq!(
            ledger.prepare_with(
                platform,
                8,
                |_, _| {
                    assert_eq!(budget::snapshot().0, [before.0[0] + 192, before.0[1]]);
                    Err(Error::MapFailed)
                },
                || panic!("failed replacement freed old buffer")
            ),
            Err(Error::MapFailed)
        );
        assert_eq!(budget::snapshot(), old);
        ledger
            .prepare_with(
                platform,
                8,
                |vec, capacity| {
                    assert_eq!(budget::snapshot().0, [before.0[0] + 192, before.0[1]]);
                    allocate(vec, capacity)
                },
                || {
                    assert_eq!(budget::snapshot().0, [before.0[0] + 192, before.0[1]]);
                },
            )
            .unwrap();
        assert_eq!(budget::snapshot().0, [before.0[0] + 128, before.0[1]]);
        for (index, value) in ledger.iter().enumerate() {
            assert_eq!(u64::from(value.base), (index * 4096) as u64);
            assert_eq!(value.pages, 1);
        }
        for index in 4..8 {
            ledger.push(region(index));
        }
        assert_eq!(
            ledger.prepare_with(platform, 8, |_, _| panic!("owner bound reached allocator"), || {}),
            Err(Error::MapFailed)
        );
        ledger.release();
        assert_eq!(budget::snapshot(), before);
    }
    let ordinary = buffers(false);
    let full = budget::snapshot();
    assert_eq!(full.1[1], before.1[1]); // Buffers consume bytes, never node slots.
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(Ledger::new().prepare(false, 1024).is_err());
    }
    let trusted = Backing::prepare(true, 4, allocate).unwrap();
    assert_eq!(budget::snapshot().1, full.1);
    trusted.finish(|| {});
    let trusted = buffers(true);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(Ledger::new().prepare(false, 1024).is_err());
        assert!(Ledger::new().prepare(true, 1024).is_err());
    }
    let counts = (ordinary.len(), trusted.len());
    drop((ordinary, trusted));
    assert_eq!(budget::snapshot(), before);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[IOMMU ledger admission] real buffers ordinary={} trusted={}; byte-only limits, full \
         replacement overlap, allocator rejection, exact address transfer, bounded capacity, \
         heap-held quota denials and deallocation-before-refund passed",
        counts.0,
        counts.1
    );
}
