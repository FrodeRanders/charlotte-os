//! Boot-only failure adapters. Every retained root/slot/reservation is real;
//! publication interruption is modeled without panic unwinding or adoption.
use super::*;

pub(super) fn run() {
    cancellation();
    published_pair_abandonment();
    let used = budget::used();
    initial_abandonment();
    growth_abandonment();
    assert_eq!(budget::used(), (used.0 + 272, used.1 + 136));
    crate::logln!(
        "[stack preparation abandonment] explicit cancellation/rejection and Drop under \
         lifecycle/table/allocator/pool guards passed; 16 original roots/slots retain 272 \
         reservation pages (136 ordinary) and 10 provisional frames"
    );
}

fn drop_under_guards(action: impl FnOnce()) {
    let _lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
    budget::test_with_pool_locked(|| {
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _kernel = memory::KERNEL_AS.lock();
        let physical = PHYSICAL_FRAME_ALLOCATOR.lock();
        let before = physical.free_frames();
        memory::usage::with_principal_history_locked(action);
        assert_eq!(physical.free_frames(), before, "Drop released stack backing");
    });
}

fn retained(handle: AddressSpaceHandle, before: (u64, u64), platform: bool) {
    assert_eq!(
        budget::used(),
        (
            before.0 + 17,
            before.1
                + if platform {
                    0
                } else {
                    17
                }
        )
    );
    assert_eq!(slots(handle), 1);
    assert!(matches!(
        memory::close_user_address_space_handle(handle),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    ));
    assert!(memory::address_space_handle_is_current(handle));
    assert!(StackSlot::reserve(handle).is_err());
}

fn cancellation() {
    for platform in [false, true] {
        let handle = domain(2, platform);
        let used = budget::used();
        let before = free();
        StackSlot::reserve(handle).unwrap().cancel_unpublished().unwrap();
        let preparation = PreparingStackPage::reserve(handle).unwrap();
        assert_eq!(free(), before - 1);
        preparation.cancel_unpublished().unwrap();
        assert_eq!(slots(handle), 0);
        assert_eq!(budget::used(), used);
        assert_eq!(free(), before);
        let preparation = PreparingStackPage::reserve(handle).unwrap();
        let invalid = preparation.base() - PAGE;
        assert!(preparation.map(invalid).is_err());
        assert_eq!(slots(handle), 0);
        assert_eq!(budget::used(), used);
        assert_eq!(free(), before);
        assert!(PreparingStackPage::with_frame(handle, || None).is_err());
        assert_eq!(budget::used(), used);
        assert_eq!(slots(handle), 0);

        let mut stacks = Stacks::user(handle, 2).unwrap();
        let before = free();
        let address = stacks.user_stack().unwrap().base_addr();
        let mut growth = stacks.begin_growth(address).unwrap();
        growth.run_with(
            PreparingUserFrame::allocate_zeroed,
            |_, _, _| false,
            |frame| PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame),
        );
        assert!(
            stacks
                .finish_growth(growth)
                .unwrap_or_else(|_| panic!("growth cancellation"))
                .is_none()
        );
        assert!(!stacks.user.as_ref().unwrap().slot.uncertain);
        assert_eq!(free(), before);
        let mut growth = stacks.begin_growth(address).unwrap();
        growth.run_with(
            || None,
            |_, _, _| panic!("rejected allocation mapped"),
            |_| panic!("rejected allocation released"),
        );
        assert!(
            stacks.finish_growth(growth).unwrap_or_else(|_| panic!("growth rejection")).is_none()
        );
        assert!(!stacks.user.as_ref().unwrap().slot.uncertain);
        stacks.release().unwrap();
        drop(stacks);
        assert_eq!(budget::used(), used);
        assert_eq!(slots(handle), 0);
        memory::close_user_address_space_handle(handle).unwrap();
    }
}

fn initial_abandonment() {
    for platform in [false, true] {
        for kind in 0..4 {
            let handle = domain(1, platform);
            let used = budget::used();
            let before = free();
            if kind == 0 {
                let slot = StackSlot::reserve(handle).unwrap();
                drop_under_guards(|| drop(slot));
            } else {
                let mut preparation = if kind == 1 {
                    PreparingStackPage {
                        frame: None,
                        slot: Some(StackSlot::reserve(handle).unwrap()),
                        mapping_started: false,
                    }
                } else {
                    PreparingStackPage::reserve(handle).unwrap()
                };
                if kind == 3 {
                    preparation.mapping_started = true;
                }
                drop_under_guards(|| drop(preparation));
            }
            retained(handle, used, platform);
            assert_eq!(free(), before - usize::from(kind >= 2));
        }
    }
}

fn growth_abandonment() {
    for platform in [false, true] {
        for kind in 0..4 {
            let handle = domain(1, platform);
            let used = budget::used();
            let mut stacks = user_only(handle);
            let before = free();
            let user = stacks.user.as_mut().unwrap();
            let mut preparation = staged_growth(user, kind != 0 && kind != 3);
            if kind == 2 {
                preparation.mapping_started = true;
            }
            if kind == 3 {
                let mut attempts = 0;
                preparation.run_with(
                    PreparingUserFrame::allocate_zeroed,
                    |_, _, _| false,
                    |_| {
                        attempts += 1;
                        Err(memory::physical::Error::CannotDeallocateUnallocatedFrame)
                    },
                );
                assert_eq!(attempts, 1);
                assert!(preparation.uncertain);
                // Completed failure handback has no frame/allocator/root work
                // and must preserve the containing slot's original fence.
                drop_under_guards(|| {
                    assert!(
                        stacks
                            .finish_growth(preparation)
                            .unwrap_or_else(|_| panic!("failed growth handback"))
                            .is_none()
                    );
                });
            } else {
                drop_under_guards(|| drop(preparation));
            }
            assert!(stacks.user.as_ref().unwrap().slot.uncertain);
            assert_eq!(stacks.grow_user_stack(stacks.user_stack().unwrap().base_addr()), None);
            // Run published pair retirement outside the probe guards: successful
            // committed-page cleanup cannot discharge abandoned growth admission.
            assert!(stacks.release().is_err());
            drop(stacks);
            retained(handle, used, platform);
            assert_eq!(free(), before + usize::from(kind == 0));
        }
    }
}

fn published_pair_abandonment() {
    for platform in [false, true] {
        let handle = domain(1, platform);
        let used = budget::used();
        let stacks = Stacks::user(handle, 1).unwrap();
        let base = stacks.kernel_base();
        let user_low = stacks.user.as_ref().unwrap().committed_low();
        let before = free();
        let progress = retirement_progress();
        drop_under_guards(|| drop(stacks));
        assert_eq!(retirement_progress(), progress);
        assert_eq!(free(), before);
        assert!(memory::KERNEL_AS.lock().is_mapped(base).unwrap());
        assert!(
            ADDRESS_SPACE_TABLE
                .lock()
                .get_mut(handle.id())
                .unwrap()
                .is_mapped(VAddr::from(user_low))
                .unwrap()
        );
        retained(handle, used, platform);
    }
    crate::logln!(
        "[published stack abandonment] complete pair Drop under lifecycle/table/allocator/pool \
         guards retains two roots/slots, 34 reservation/data pages"
    );
}
