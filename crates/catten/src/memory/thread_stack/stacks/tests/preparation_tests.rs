//! Boot-only failure adapters. Every retained root/slot/reservation is real;
//! publication interruption is modeled without panic unwinding or adoption.
use super::*;

pub(super) fn run() {
    cancellation();
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
        action();
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
        PreparingGrowthPage::allocate(stacks.user.as_mut().unwrap())
            .unwrap()
            .cancel_unpublished()
            .unwrap();
        assert!(!stacks.user.as_ref().unwrap().slot.uncertain);
        assert_eq!(free(), before);
        assert!(
            PreparingGrowthPage::allocate_with(stacks.user.as_mut().unwrap(), || None).is_none()
        );
        assert!(!stacks.user.as_ref().unwrap().slot.uncertain);
        assert_eq!(free(), before);
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
            let mut preparation = if kind == 0 {
                PreparingGrowthPage {
                    stack: user,
                    frame: None,
                    mapping_started: false,
                    finished: false,
                }
            } else {
                PreparingGrowthPage::allocate(user).unwrap()
            };
            if kind == 2 {
                preparation.mapping_started = true;
            } else if kind == 3 {
                let mut attempts = 0;
                assert!(matches!(
                    preparation.rollback_with(|_| {
                        attempts += 1;
                        Err(memory::physical::Error::CannotDeallocateUnallocatedFrame)
                    }),
                    Err(memory::physical::Error::CannotDeallocateUnallocatedFrame)
                ));
                assert_eq!(attempts, 1);
            }
            drop_under_guards(|| drop(preparation));
            assert!(stacks.user.as_ref().unwrap().slot.uncertain);
            assert_eq!(stacks.grow_user_stack(stacks.user_stack().unwrap().base_addr()), None);
            // Run published pair retirement outside the probe guards: successful
            // committed-page cleanup cannot discharge abandoned growth admission.
            drop(stacks);
            retained(handle, used, platform);
            assert_eq!(free(), before + usize::from(kind == 0));
        }
    }
}
