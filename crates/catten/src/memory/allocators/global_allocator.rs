use core::mem::MaybeUninit;
use core::ptr::NonNull;

use spin::Mutex;
use talc::base::Talc;
use talc::base::binning::Binning;
use talc::source::Source;
use talc::*;

use crate::cpu::isa::interface::memory::address::{Address, VirtualAddress};
use crate::cpu::isa::memory::paging::PAGE_SIZE;
use crate::klib::size::mebibytes;
use crate::memory::VAddr;
use crate::memory::allocators::memory::try_allocate_and_map_range;
use crate::memory::linear::address_map::LA_MAP;
use crate::memory::linear::address_map::RegionType::KernelAllocatorArena;

const INITIAL_HEAP_SIZE: usize = mebibytes(2);
#[global_allocator]
pub static PRIMARY_ALLOCATOR: TalcLock<Mutex<()>, ExtendOnOom> = TalcLock::new(ExtendOnOom::new());

pub fn init_primary_allocator() {
    let base = LA_MAP.get_region(KernelAllocatorArena).base;
    try_allocate_and_map_range(base, INITIAL_HEAP_SIZE / PAGE_SIZE)
        .expect("Failed to allocate and map initial kernel heap memory");
    unsafe {
        let mut pa_lock = PRIMARY_ALLOCATOR.lock();
        let returned_ptr = pa_lock
            .claim(base.into_mut(), INITIAL_HEAP_SIZE)
            .expect("Talc failed to claim the initial kernel heap");
        pa_lock.source.heap_ptr.lock().write(returned_ptr);
    }
}

#[derive(Debug)]
pub struct ExtendOnOom {
    heap_ptr: Mutex<MaybeUninit<NonNull<u8>>>,
}

unsafe impl Sync for ExtendOnOom {}
unsafe impl Send for ExtendOnOom {}

impl ExtendOnOom {
    const fn new() -> Self {
        ExtendOnOom {
            heap_ptr: Mutex::new(MaybeUninit::uninit()),
        }
    }
}

unsafe impl Source for ExtendOnOom {
    fn acquire<B: Binning>(
        talc: &mut Talc<Self, B>,
        layout: core::alloc::Layout,
    ) -> Result<(), ()> {
        let curr_end = unsafe { talc.source.heap_ptr.lock().assume_init() };
        let new_region_start =
            VAddr::from(curr_end.as_ptr() as usize).next_aligned_to(layout.align());
        let new_region_end = new_region_start + layout.size();
        unsafe {
            talc.extend(curr_end, new_region_end.into_mut());
        }
        Ok(())
    }
}
