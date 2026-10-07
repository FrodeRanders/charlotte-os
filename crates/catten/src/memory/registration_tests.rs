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
    let limits = DOMAIN_LIMITS.lock().len();
    let occupied = ADDRESS_SPACE_TABLE.lock().iter().filter(|entry| entry.is_some()).count();

    for _ in 0..64 {
        let space = AddressSpace::try_new_user().unwrap();
        assert_eq!(
            super::register_user_address_space_with(
                space,
                |_, space| Err((space, Error::AllocationFailed)),
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
        assert_eq!(DOMAIN_LIMITS.lock().len(), limits);
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
