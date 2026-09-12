use core::{
    ptr::NonNull,
    sync::atomic::{
        AtomicPtr,
        AtomicUsize,
        Ordering,
    },
};

use talc::{
    base::{
        Talc,
        binning::Binning,
    },
    source::Source,
    *,
};

use crate::{
    cpu::{
        isa::interface::memory::address::VirtualAddress,
        multiprocessor::spin::mutex::MutexCore,
    },
    klib::size::mebibytes,
    memory::{
        PHYSICAL_FRAME_ALLOCATOR,
        allocators::memory::{
            PageSize,
            try_allocate_and_map_range,
        },
        linear::address_map::{
            LA_MAP,
            RegionType::KernelAllocatorArena,
        },
    },
};

// Store-backed service images are cached on first use. A deploy-test boot can
// therefore retain roughly 2.4 MiB of signed ELF data in addition to the
// kernel's ordinary boot allocations. Claim enough arena up front that this
// expected working set does not force heap growth in the middle of the highly
// concurrent service-start storm.
const INITIAL_HEAP_SIZE: usize = mebibytes(8);
// A pre-mapped growth reserve immediately after the initial heap. The
// ExtendOnOom acquire extends the talc's range *within* this reserve, so it
// never takes the kernel address-space or frame-allocator locks while the
// talc lock is held — the lock-ordering deadlock that a concurrent
// map-while-allocating could otherwise trigger (the reserve maps at boot,
// before any concurrency exists).
//
// The reserve scales with discovered RAM: one sixty-fourth of usable memory,
// clamped between the historical 64 MiB floor and a 256 MiB ceiling, and
// rounded up to the 2 MiB large-page granularity the arena is mapped with.
pub(crate) const HEAP_RESERVE_FRACTION_DIVISOR: u64 = 64;
pub(crate) const MIN_HEAP_GROWTH_RESERVE: usize = mebibytes(64);
pub(crate) const MAX_HEAP_GROWTH_RESERVE: usize = mebibytes(256);
pub(crate) static HEAP_GROWTH_RESERVE_BYTES: AtomicUsize =
    AtomicUsize::new(MIN_HEAP_GROWTH_RESERVE);
#[global_allocator]
pub static PRIMARY_ALLOCATOR: TalcLock<MutexCore, ExtendOnOom> = TalcLock::new(ExtendOnOom::new());

fn heap_growth_reserve_bytes(usable_bytes: u64) -> usize {
    let scaled =
        usize::try_from(usable_bytes / HEAP_RESERVE_FRACTION_DIVISOR).unwrap_or(usize::MAX);
    let page = PageSize::Large.num_bytes();
    let aligned = scaled.div_ceil(page) * page;
    aligned.clamp(MIN_HEAP_GROWTH_RESERVE, MAX_HEAP_GROWTH_RESERVE)
}

pub fn init_primary_allocator() {
    let usable_bytes = {
        let allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        allocator.usable_bytes()
    };
    let growth_reserve = heap_growth_reserve_bytes(usable_bytes);
    HEAP_GROWTH_RESERVE_BYTES.store(growth_reserve, Ordering::Relaxed);
    crate::early_logln!(
        "Kernel heap: usable={} MiB initial={} MiB reserve={} MiB total={} MiB",
        usable_bytes / mebibytes(1) as u64,
        INITIAL_HEAP_SIZE / mebibytes(1),
        growth_reserve / mebibytes(1),
        (INITIAL_HEAP_SIZE + growth_reserve) / mebibytes(1)
    );
    let base = LA_MAP.get_region(KernelAllocatorArena).base;
    try_allocate_and_map_range(
        base,
        PageSize::Large,
        (INITIAL_HEAP_SIZE + growth_reserve) / PageSize::Large.num_bytes(),
    )
    .expect("Failed to allocate and map the kernel heap and its growth reserve");
    unsafe {
        let mut pa_lock = PRIMARY_ALLOCATOR.lock();
        let returned_ptr = pa_lock
            .claim(base.into_mut(), INITIAL_HEAP_SIZE)
            .expect("Talc failed to claim the initial kernel heap");
        pa_lock.source.heap_ptr.store(returned_ptr.as_ptr(), Ordering::Release);
        pa_lock.source.reserve_end.store(
            base.into_mut::<u8>().wrapping_add(INITIAL_HEAP_SIZE + growth_reserve),
            Ordering::Release,
        );
    }
}

#[derive(Debug)]
pub struct ExtendOnOom {
    heap_ptr: AtomicPtr<u8>,
    /// One past the last byte of the pre-mapped growth reserve.
    reserve_end: AtomicPtr<u8>,
}

unsafe impl Sync for ExtendOnOom {}
unsafe impl Send for ExtendOnOom {}

impl ExtendOnOom {
    const fn new() -> Self {
        ExtendOnOom {
            heap_ptr: AtomicPtr::new(core::ptr::null_mut()),
            reserve_end: AtomicPtr::new(core::ptr::null_mut()),
        }
    }
}

unsafe impl Source for ExtendOnOom {
    fn acquire<B: Binning>(
        talc: &mut Talc<Self, B>,
        layout: core::alloc::Layout,
    ) -> Result<(), ()> {
        let curr_end = talc.source.heap_ptr.load(Ordering::Acquire);
        let reserve_end = talc.source.reserve_end.load(Ordering::Acquire);
        // The growth region is already mapped (the boot-time reserve); the
        // acquire only extends the talc's range, so no kernel address-space
        // or frame-allocator lock is taken while the talc lock is held.
        let new_region_end = curr_end.wrapping_add(PageSize::Large.num_bytes());
        if new_region_end > reserve_end {
            crate::early_logln!(
                "[HEAPDBG] acquire out-of-reserve: curr_end={:p} reserve_end={:p} req={}",
                curr_end,
                reserve_end,
                layout.size()
            );
            return Err(());
        }
        unsafe {
            talc.extend(
                NonNull::new(curr_end).expect("Passed null pointer to the constructor of NonNull"),
                new_region_end,
            );
        }
        talc.source.heap_ptr.store(new_region_end, Ordering::Release);
        let _ = layout;
        Ok(())
    }
}
