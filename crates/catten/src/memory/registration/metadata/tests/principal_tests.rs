//! Actual root construction rejects each funded container prefix atomically.
use super::*;
use crate::memory::kernel_buffers::{
    self,
    test_metadata_limit as limit,
    test_metadata_used as used,
};
fn root(principal: u64) -> AddressSpaceHandle {
    let value = AddressSpace::try_new_principal_user(
        false,
        prepare_principal_backing(principal, false).unwrap(),
    )
    .unwrap();
    register_user_address_space(value).unwrap()
}
pub(super) fn run() {
    let principal = charlotte_launch::artifact_principal_id(b"root-container-principal");
    let healthy = charlotte_launch::artifact_principal_id(b"root-container-healthy");
    let roots_before = admission::snapshot();
    let ns_before = crate::klib::namespace_node_budget::snapshot();
    let first = root(principal);
    let mandatory = used(principal);
    assert!(mandatory > 0);
    let second = root(principal);
    assert_eq!(used(principal), 2 * mandatory);
    let storage = kernel_buffers::capture_storage(first).unwrap();
    let sizes = [
        crate::capability::namespace_node_bytes_for_test(),
        budget::tests::account_node_bytes(),
        Storage::<LimitsRecord>::allocation_bytes() as u64,
        Storage::<UsageRecord>::allocation_bytes() as u64,
        Storage::<AuthorityRecord>::allocation_bytes() as u64,
        crate::ipc::namespace::node_bytes_for_test(),
    ];
    assert_eq!(sizes.iter().sum::<u64>(), mandatory);
    let base = used(principal);
    let pools = admission::snapshot();
    let namespaces = crate::klib::namespace_node_budget::snapshot();
    let accounts = crate::klib::namespace_account_budget::snapshot();
    let reactors = (
        crate::klib::observer::account_budget::snapshot(),
        crate::timers::account_budget::snapshot(),
    );
    let mut headroom = 0;
    for (stage, bytes) in sizes.into_iter().enumerate() {
        for prefix in [headroom, headroom + bytes - 1] {
            limit(principal, base + prefix);
            let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
            let occupied = ADDRESS_SPACE_TABLE.lock().iter().filter(|r| r.is_some()).count();
            let space = AddressSpace::try_new_principal_user(
                false,
                prepare_principal_backing(principal, false).unwrap(),
            )
            .unwrap();
            assert_eq!(
                register_user_address_space(space),
                Err(match stage {
                    0 => AddressSpaceRegistrationError::CapabilityNamespaceAllocationFailed,
                    1 => AddressSpaceRegistrationError::MemorySponsorAllocationFailed,
                    2..=4 => AddressSpaceRegistrationError::DomainMetadataAllocationFailed,
                    _ => AddressSpaceRegistrationError::IpcNamespaceAllocationFailed,
                })
            );
            assert_eq!(used(principal), base);
            assert_eq!(admission::snapshot(), pools);
            assert_eq!(crate::klib::namespace_node_budget::snapshot(), namespaces);
            assert_eq!(crate::klib::namespace_account_budget::snapshot(), accounts);
            assert_eq!(
                (
                    crate::klib::observer::account_budget::snapshot(),
                    crate::timers::account_budget::snapshot()
                ),
                reactors
            );
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
            assert_eq!(ADDRESS_SPACE_TABLE.lock().iter().filter(|r| r.is_some()).count(), occupied);
        }
        headroom += bytes;
    }
    limit(principal, base);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(admission::prepare::<LimitsRecord>(storage).is_err());
            assert!(crate::klib::namespace_node_budget::prepare::<u64>(storage).is_err());
        }
    }
    assert_eq!(used(principal), base);
    let independent = root(healthy);
    assert_eq!(used(healthy), mandatory);
    limit(principal, crate::resource_policy::POLICY.memory.principal_metadata_bytes);
    crate::capability::test_funded_namespace_fixture(first, principal);
    for value in [first, second, independent] {
        close_user_address_space_handle(value).unwrap();
    }
    assert_eq!(used(principal), 0);
    assert_eq!(used(healthy), 0);
    assert_eq!(admission::snapshot(), roots_before);
    assert_eq!(crate::klib::namespace_node_budget::snapshot(), ns_before);
    crate::logln!(
        "[root container principal] root construction layouts={:?}, mandatory={} bytes; sibling \
         sharing, twelve exact quota prefixes, 128 heap-held denials, healthy principal and full \
         five-namespace accounting/release passed",
        sizes,
        mandatory
    );
}
