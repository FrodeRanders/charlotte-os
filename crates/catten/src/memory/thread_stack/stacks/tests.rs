//! Serialized boot fixtures use never-dispatched roots and real stack ranges.
use super::*;
use crate::memory::{
    self,
    AddressSpace,
    DomainLimits,
};

mod preparation_tests;

fn domain(pages: usize, platform: bool) -> AddressSpaceHandle {
    let space = if platform {
        AddressSpace::try_new_platform_user()
    } else {
        AddressSpace::try_new_user()
    }
    .unwrap();
    let handle = memory::register_user_address_space(space).unwrap();
    if platform {
        memory::budget::mark_platform(handle);
    }
    memory::set_domain_limits(
        handle,
        DomainLimits {
            user_stack_pages: pages,
            max_threads: 1,
        },
    )
    .unwrap();
    handle
}

fn slots(handle: AddressSpaceHandle) -> u64 {
    ADDRESS_SPACE_TABLE.lock().get(handle.id()).unwrap().thread_stack_slots
}

fn free() -> usize {
    PHYSICAL_FRAME_ALLOCATOR.lock().free_frames()
}

pub(in crate::memory::thread_stack) fn run() {
    budget::test_pool();
    growth_and_drop();
    pressure();
    confirmed_constructor_failure();
    retained_failures();
    preparation_tests::run();
    crate::logln!(
        "[stack backing admission] maximum user/kernel reservation, growth/collision/retry, \
         success/Drop refund, rejection before allocation and platform progress passed; six \
         failed/abandoned owners retain original roots/slots/reservations (20 data frames \
         retained)"
    );
}

fn growth_and_drop() {
    let used = budget::used();
    let handle = domain(4, false);
    // Warm kernel branches independently of user-root final reclamation.
    let stacks = Stacks::user(handle, 4).unwrap();
    drop(stacks);
    let baseline = free();
    let mut stacks = Stacks::user(handle, 4).unwrap();
    assert_eq!(budget::used(), (used.0 + 20, used.1 + 20));
    assert_eq!(slots(handle), 1);
    assert!(Stacks::user(handle, 4).is_err());
    assert_eq!(free(), baseline - 17);
    let base = stacks.user_stack().unwrap().base_addr();
    assert_eq!(stacks.committed_pages(), 1);
    // A trusted foreign leaf forces growth to reject after one successful page.
    let foreign = PreparingUserFrame::allocate_zeroed().unwrap();
    let collision = VAddr::from(base + PAGE);
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(handle.id())
        .unwrap()
        .map_existing_page(MemoryMapping {
            vaddr: collision,
            paddr: foreign.frame(),
            page_type: PageType::UserData,
        })
        .unwrap();
    let data: *mut u8 = foreign.frame().into();
    unsafe {
        core::ptr::write_bytes(data, 0x5a, PAGE);
    }
    let before = free();
    assert_eq!(stacks.grow_user_stack(base), Some(base + 2 * PAGE));
    assert_eq!(stacks.committed_pages(), 2);
    assert_eq!(free(), before - 1);
    assert_eq!(budget::used(), (used.0 + 20, used.1 + 20));
    assert!(unsafe { core::slice::from_raw_parts(data, PAGE) }.iter().all(|&byte| byte == 0x5a));
    assert_eq!(
        ADDRESS_SPACE_TABLE.lock().get_mut(handle.id()).unwrap().unmap_page(collision).unwrap(),
        foreign.frame()
    );
    crate::cpu::isa::memory::tlb::try_inval_range_user(handle.id(), collision, 1).unwrap();
    foreign.release().unwrap();
    assert_eq!(stacks.grow_user_stack(base), Some(base));
    assert_eq!(stacks.committed_pages(), 4);
    assert_eq!(stacks.grow_user_stack(base - PAGE), None);
    assert_eq!(stacks.grow_user_stack(base), None);
    assert_eq!(free(), baseline - 20);
    drop(stacks);
    assert_eq!(free(), baseline);
    assert_eq!(budget::used(), used);
    assert_eq!(slots(handle), 0);
    memory::close_user_address_space_handle(handle).unwrap();
}

fn pressure() {
    let used = budget::used();
    let ordinary = domain(1, false);
    let platform = domain(1, true);
    let pressure = budget::OrdinaryPressure::new();
    let free = free();
    let mut allocated = false;
    assert!(
        PreparingStackPage::with_frame(ordinary, || {
            allocated = true;
            PreparingUserFrame::allocate_zeroed()
        })
        .is_err()
    );
    assert!(!allocated);
    assert_eq!(slots(ordinary), 0);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    let stacks = Stacks::user(platform, 1).unwrap();
    assert_eq!(budget::used(), (used.0 + 17, used.1));
    let kernel = Stacks::kernel().unwrap();
    assert_eq!(budget::used(), (used.0 + 33, used.1));
    drop(kernel);
    drop(stacks);
    assert_eq!(budget::used(), used);
    drop(pressure);
    drop(Stacks::user(ordinary, 1).unwrap());
    assert_eq!(budget::used(), used);
    memory::close_user_address_space_handle(ordinary).unwrap();
    memory::close_user_address_space_handle(platform).unwrap();
}

fn user_only(handle: AddressSpaceHandle) -> Stacks {
    let preparation = PreparingStackPage::reserve(handle).unwrap();
    let base = preparation.base();
    let slot = preparation.map(base).unwrap();
    let mut stacks = Stacks::default();
    stacks.user = Some(UserStack {
        slot,
        budget_pages: 1,
        committed_pages: 1,
    });
    stacks
}

fn confirmed_constructor_failure() {
    let used = budget::used();
    let handle = domain(1, false);
    assert!(PreparingStackPage::with_frame(handle, || None).is_err());
    assert_eq!(slots(handle), 0);
    assert_eq!(budget::used(), used);
    let mut stacks = user_only(handle);
    assert!(
        stacks
            .allocate_kernel_with(|_| Err(AllocationFailure {
                error: Error::InvalidStack,
                retained: false
            }))
            .is_err()
    );
    drop(stacks);
    assert_eq!(slots(handle), 0);
    assert_eq!(budget::used(), used);
    memory::close_user_address_space_handle(handle).unwrap();
}

fn retained_failures() {
    let progress = retirement_progress();
    for kind in 0..6 {
        let used = budget::used();
        let handle = domain(1, false);
        let initial_free;
        match kind {
            0 => {
                // Failed unpublished physical release consumes the slot too.
                initial_free = free();
                let mut preparation = PreparingStackPage::reserve(handle).unwrap();
                let mut attempts = 0;
                assert!(matches!(
                    preparation.rollback_with(|_| {
                        attempts += 1;
                        Err(memory::physical::Error::CannotDeallocateUnallocatedFrame)
                    }),
                    Err(PreparationError::Physical(
                        memory::physical::Error::CannotDeallocateUnallocatedFrame
                    ))
                ));
                assert_eq!(attempts, 1);
                drop(preparation);
                assert_eq!(free(), initial_free - 1);
            }
            1 => {
                // Completed user cleanup cannot discharge uncertain kernel work.
                let mut stacks = user_only(handle);
                initial_free = free();
                assert!(
                    stacks
                        .allocate_kernel_with(|_| Err(AllocationFailure {
                            error: Error::InvalidStack,
                            retained: true
                        }))
                        .is_err()
                );
                drop(stacks);
                assert_eq!(free(), initial_free + 1);
            }
            2 => {
                let mut stacks = user_only(handle);
                initial_free = free();
                let user = stacks.user.take().unwrap();
                assert!(
                    retire_user_with(
                        user,
                        |base, pages, handle| {
                            crate::cpu::isa::memory::tlb::try_inval_range_user(
                                handle.id(),
                                base,
                                pages,
                            )
                            .is_ok()
                        },
                        |_| Err(memory::physical::Error::CannotDeallocateUnallocatedFrame)
                    )
                    .is_none()
                );
                drop(stacks);
                assert_eq!(free(), initial_free);
            }
            3 => {
                let mut stacks = user_only(handle);
                initial_free = free();
                // The committed leaf is normally released, but an interrupted
                // growth preparation must prevent slot/node refund afterward.
                let mut preparation =
                    PreparingGrowthPage::allocate(stacks.user.as_mut().unwrap()).unwrap();
                preparation.mapping_started = true;
                drop(preparation);
                drop(stacks);
                assert_eq!(free(), initial_free);
            }
            4 => {
                // Reject cleanup of a real mapped kernel range after its user
                // leaf has been confirmed released. The complete reservation
                // and root lease must remain, along with all sixteen pages.
                let mut stacks = Stacks::user(handle, 1).unwrap();
                let base = stacks.kernel_base();
                initial_free = free();
                stacks.release_with(|_, _| Err(Error::InvalidStack));
                drop(stacks);
                assert_eq!(free(), initial_free + 1);
                assert!(memory::KERNEL_AS.lock().is_mapped(base).unwrap());
            }
            _ => {
                let mut stacks = user_only(handle);
                initial_free = free();
                let user = stacks.user.take().unwrap();
                assert!(
                    retire_user_with(
                        user,
                        |_, _, _| false,
                        |_| panic!("failed invalidation released stack backing")
                    )
                    .is_none()
                );
                drop(stacks);
                assert_eq!(free(), initial_free);
            }
        }
        assert_eq!(budget::used(), (used.0 + 17, used.1 + 17));
        assert_eq!(slots(handle), 1);
        assert!(memory::close_user_address_space_handle(handle).is_err());
        assert!(memory::address_space_handle_is_current(handle));
        assert!(StackSlot::reserve(handle).is_err());
    }
    let after = retirement_progress();
    assert_eq!(after[INVALIDATION_REJECTED], progress[INVALIDATION_REJECTED] + 1);
    assert_eq!(after[PHYSICAL_REJECTED], progress[PHYSICAL_REJECTED] + 1);
    crate::logln!(
        "[stack retirement diagnostics] distinct invalidation/physical rejections retain leases \
         and count once; counters={:?}",
        after
    );
}
