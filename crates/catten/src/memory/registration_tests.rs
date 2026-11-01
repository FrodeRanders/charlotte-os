//! Unregistered-root ownership on slot metadata preparation rejection.

use super::{
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    AddressSpace,
    AddressSpaceRegistrationError,
    DOMAIN_LIMITS,
    PHYSICAL_FRAME_ALLOCATOR,
};
use crate::klib::collections::id_table::Error;

// Rejected roots stay inline: boxing would allocate on the failure path.
#[allow(clippy::result_large_err)]
pub(crate) fn test_publication_rejection() {
    super::registration::test_ipc_preparation();
    super::registration::metadata::test_preparation();
    sponsor_rejection();
    capacity_conflict();
    super::budget::tests::run();
    super::registration::test_abandonment();
    let existing =
        super::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    let previous =
        super::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    super::close_user_address_space_handle(previous).unwrap();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let capabilities = crate::capability::node_admission_used();
    let tables = (
        super::translation::account::test_used_pages(),
        super::translation::account::test_ordinary_pages(),
    );
    let limits = DOMAIN_LIMITS.lock().iter().count();
    let occupied = ADDRESS_SPACE_TABLE.lock().iter().filter(|entry| entry.is_some()).count();

    for _ in 0..64 {
        let space = AddressSpace::try_new_user().unwrap();
        assert_eq!(
            super::register_user_address_space_with(
                space,
                |_, _, _| Err(Error::AllocationFailed),
                |space| {
                    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                    assert!(super::KERNEL_AS.try_lock().is_some());
                    assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
                    drop(space);
                },
            ),
            Err(AddressSpaceRegistrationError::TableAllocationFailed)
        );
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        assert_eq!(crate::capability::node_admission_used(), capabilities);
        assert_eq!(
            (
                super::translation::account::test_used_pages(),
                super::translation::account::test_ordinary_pages(),
            ),
            tables
        );
        assert_eq!(DOMAIN_LIMITS.lock().iter().count(), limits);
        assert_eq!(
            ADDRESS_SPACE_TABLE.lock().iter().filter(|entry| entry.is_some()).count(),
            occupied
        );
        assert!(super::address_space_handle_is_current(existing));
        assert!(!super::address_space_handle_is_current(previous));
    }

    let replacement = super::register_user_address_space(AddressSpace::try_new_user().unwrap())
        .expect("rejection must leave publication capacity recoverable");
    assert_eq!(replacement.id(), previous.id());
    assert_eq!(replacement.generation(), previous.generation() + 1);
    super::close_user_address_space_handle(replacement).unwrap();
    super::close_user_address_space_handle(existing).unwrap();
    crate::logln!(
        "[slot publication] repeated rejection releases unpublished roots outside guards, \
         preserves existing namespaces and recovers the exact slot generation"
    );
}

// Rejected roots stay inline; rejection must not allocate another owner.
#[allow(clippy::result_large_err)]
fn sponsor_rejection() {
    for _ in 0..64 {
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let space = AddressSpace::try_new_user().unwrap();
        let before = ADDRESS_SPACE_TABLE.lock().iter().filter(|entry| entry.is_some()).count();
        assert_eq!(
            super::registration::register(
                space,
                |_, _, _| panic!("sponsor rejection published a root"),
                |root| {
                    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                    assert!(super::budget::tests::ledger_available());
                    drop(root);
                },
                || Err(core::alloc::AllocError),
                || super::registration::metadata::PreparingMetadata::try_new(false),
            ),
            Err(AddressSpaceRegistrationError::MemorySponsorAllocationFailed)
        );
        assert_eq!(
            ADDRESS_SPACE_TABLE.lock().iter().filter(|entry| entry.is_some()).count(),
            before
        );
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    }
    crate::logln!(
        "[memory sponsor registration] 64 preparation rejections precede root publication and \
         release original roots outside local guards"
    );
}

fn capacity_conflict() {
    let before = crate::memory::registration::table_backing::snapshot();
    let mut attempts = 0;
    let root = super::register_user_address_space_with(
        AddressSpace::try_new_user().unwrap(),
        |table, owner, growth| {
            attempts += 1;
            if attempts == 1 {
                Err(Error::CapacityChanged)
            } else {
                table.try_insert_from(owner, growth)
            }
        },
        drop,
    )
    .unwrap();
    assert_eq!(attempts, 2);
    super::close_user_address_space_handle(root).unwrap();
    let roots = ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count();
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut attempts = 0;
    assert_eq!(
        super::register_user_address_space_with(
            AddressSpace::try_new_user().unwrap(),
            |_, _, _| {
                attempts += 1;
                Err(Error::CapacityChanged)
            },
            |root| {
                assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                drop(root);
            },
        ),
        Err(AddressSpaceRegistrationError::TableAllocationFailed)
    );
    assert_eq!(attempts, 8);
    assert_eq!(ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count(), roots);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    assert_eq!(crate::memory::registration::table_backing::snapshot(), before);
    crate::logln!(
        "[root table conflict] unstarted rejection preserves root/namespaces through one retry; \
         eight-attempt exhaustion releases original ownership after guards with exact \
         frame/backing recovery"
    );
}
