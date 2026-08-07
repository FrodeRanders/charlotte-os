use core::{
    ptr::NonNull,
    sync::atomic::{
        AtomicPtr,
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
const HEAP_GROWTH_RESERVE: usize = mebibytes(192);
static ACQUIRE_COUNT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
#[global_allocator]
pub static PRIMARY_ALLOCATOR: TalcLock<MutexCore, ExtendOnOom> = TalcLock::new(ExtendOnOom::new());

pub fn init_primary_allocator() {
    let base = LA_MAP.get_region(KernelAllocatorArena).base;
    try_allocate_and_map_range(
        base,
        PageSize::Large,
        (INITIAL_HEAP_SIZE + HEAP_GROWTH_RESERVE) / PageSize::Large.num_bytes(),
    )
    .expect("Failed to allocate and map the kernel heap and its growth reserve");
    unsafe {
        let mut pa_lock = PRIMARY_ALLOCATOR.lock();
        let returned_ptr = pa_lock
            .claim(base.into_mut(), INITIAL_HEAP_SIZE)
            .expect("Talc failed to claim the initial kernel heap");
        pa_lock.source.heap_ptr.store(returned_ptr.as_ptr(), Ordering::Release);
        pa_lock.source.reserve_end.store(
            base.into_mut::<u8>().wrapping_add(INITIAL_HEAP_SIZE + HEAP_GROWTH_RESERVE),
            Ordering::Release,
        );
        let he = returned_ptr.as_ptr();
        let tag_now = he.wrapping_sub(1).read();
        let size_now = (he.wrapping_sub(8) as *const usize).read();
        // also probe a few physical aliases via HHDM and the heap mapping
        let mid = base.into_mut::<u8>().wrapping_add(0x100000);
        mid.write(0xab);
        let mid_read = mid.read();
        crate::early_logln!(
            "[HEAPDBG] claim base={:p} heap_end={:p} tag@-1={:#x} size@-8={:#x} \
             mid_write_read={:#x}",
            (base.into_mut::<u8>()),
            he,
            tag_now,
            size_now,
            mid_read
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
