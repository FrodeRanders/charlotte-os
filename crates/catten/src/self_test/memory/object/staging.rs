//! Deterministic pauses at the same preparation boundary used by IPC vectors.

use alloc::vec;

use super::create_memory_object_test_address_space as create;
use crate::{
    capability::{
        self,
        admission_tests,
    },
    logln,
    memory::{
        VAddr,
        budget,
        current_address_space_handle,
        object::{
            self,
            MemoryObjectError,
        },
    },
    self_test::close_test_address_space,
};

fn assert_hidden(target: usize, cap: u64) {
    assert_eq!(object::info(target, cap), Err(MemoryObjectError::UnknownCapability));
    assert_eq!(object::map_any(target, cap, true), Err(MemoryObjectError::UnknownCapability));
    assert_eq!(
        object::write_bytes(target, cap, &[0x5a]),
        Err(MemoryObjectError::UnknownCapability)
    );
    assert_eq!(object::close_cap(target, cap), Err(MemoryObjectError::UnknownCapability));
    assert_eq!(object::get_phys(target, cap), 0);
    assert!(matches!(object::pin_for_copy(target, cap), Err(MemoryObjectError::UnknownCapability)));
    assert!(matches!(
        object::pin_for_dma(target, cap, false, true, false),
        Err(MemoryObjectError::UnknownCapability)
    ));
}

pub(super) fn test_hidden_transfers() {
    for write in [false, true] {
        let source = create("hidden loan source");
        let target = create("hidden loan target");
        let cap = object::allocate(source, 1).unwrap();
        if !write {
            object::map(source, cap, VAddr::from(0x120000usize), false).unwrap();
        }
        let prepared = object::prepare_loan(source, cap, target, write).unwrap();
        assert_hidden(target, prepared.target_cap());
        assert_eq!(object::close_cap(source, cap), Err(MemoryObjectError::UnknownCapability));
        admission_tests::test_fill_remaining_namespace(source);
        drop(prepared);
        assert_eq!(
            admission_tests::test_namespace_used(source),
            admission_tests::TEST_NAMESPACE_LIMIT
        );
        assert_eq!(admission_tests::test_namespace_used(target), 0);
        assert!(!object::info(source, cap).unwrap().lent);
        let lent = object::prepare_loan(source, cap, target, write).unwrap().commit().unwrap();
        assert!(object::info(source, cap).unwrap().lent);
        assert_eq!(object::close_cap(source, cap), Err(MemoryObjectError::LendingActive));
        if write {
            object::map_any(target, lent, true).unwrap();
            object::write_bytes(target, lent, &[0x5a]).unwrap();
        } else {
            assert_eq!(object::map_any(target, lent, true), Err(MemoryObjectError::MissingRight));
            object::map_any(target, lent, false).unwrap();
        }
        // Ordinary revocation must also remove the committed borrower's mapping.
        object::revoke_lend(source, cap, target, lent).unwrap();
        assert_hidden(target, lent);
        assert!(!object::info(source, cap).unwrap().lent);
        close_test_address_space(source).unwrap();
        close_test_address_space(target).unwrap();
    }
    test_copy_staging();
    test_mixed_retirement();
    for mode in [0, 2, 3] {
        test_retired_source(mode);
        test_retired_target(mode);
    }
    logln!(
        "[memory staging] hidden copy/loans, mixed retirement, refunds and exact ASID/cap reuse \
         passed"
    );
}

fn test_copy_staging() {
    let source = create("private copy source");
    let target = create("private copy target");
    let handle = current_address_space_handle(source).unwrap();
    let cap = object::allocate(source, 1).unwrap();
    object::write_bytes(source, cap, &[0x5a]).unwrap();
    let before = budget::used(handle);
    let prepared = object::prepare_copy(source, cap, target).unwrap();
    assert_hidden(target, prepared.target_cap());
    assert_eq!(
        budget::used(handle),
        budget::Amount {
            pages: 2,
            objects: 2
        }
    );
    drop(prepared);
    assert_eq!(budget::used(handle), before);
    assert_eq!(admission_tests::test_namespace_used(target), 0);
    let copied = object::prepare_copy(source, cap, target).unwrap();
    object::write_bytes(source, cap, &[0xa5]).unwrap();
    let copied = copied.commit().unwrap();
    assert_eq!(object::snapshot_bytes(target, copied, 1).unwrap(), [0x5a]);
    object::close_cap(target, copied).unwrap();
    assert_eq!(budget::used(handle), before);
    close_test_address_space(source).unwrap();
    close_test_address_space(target).unwrap();
}

fn test_mixed_retirement() {
    let source = create("mixed retirement source");
    let target = create("mixed retirement target");
    let handle = current_address_space_handle(source).unwrap();
    let caps = [
        object::allocate(source, 1).unwrap(),
        object::allocate(source, 1).unwrap(),
        object::allocate(source, 1).unwrap(),
        object::allocate(source, 1).unwrap(),
    ];
    let before = budget::used(handle);
    let mut batch = vec![
        object::prepare_move(source, caps[0], target).unwrap(),
        object::prepare_copy(source, caps[1], target).unwrap(),
        object::prepare_loan(source, caps[2], target, false).unwrap(),
        object::prepare_loan(source, caps[3], target, true).unwrap(),
    ];
    for transfer in &batch {
        assert_hidden(target, transfer.target_cap());
    }
    capability::retire_address_space(target);
    assert_eq!(object::commit_transfers(&mut batch), Err(MemoryObjectError::AddressSpaceMissing));
    for transfer in &batch {
        assert_hidden(target, transfer.target_cap());
    }
    admission_tests::test_fill_remaining_namespace(source);
    drop(batch);
    assert_eq!(budget::used(handle), before);
    assert_eq!(admission_tests::test_namespace_used(target), 0);
    assert_eq!(admission_tests::test_namespace_used(source), admission_tests::TEST_NAMESPACE_LIMIT);
    for cap in caps {
        assert!(!object::info(source, cap).unwrap().lent);
        object::write_bytes(source, cap, &[0x5a]).unwrap();
    }
    close_test_address_space(source).unwrap();
    close_test_address_space(target).unwrap();
}

fn prepare(mode: u32, source: usize, cap: u64, target: usize) -> object::PreparedTransfer {
    match mode {
        0 => object::prepare_copy(source, cap, target),
        2 => object::prepare_loan(source, cap, target, false),
        3 => object::prepare_loan(source, cap, target, true),
        _ => unreachable!(),
    }
    .unwrap()
}

fn test_retired_source(mode: u32) {
    let source = create("staged alias retiring source");
    let target = create("staged alias stable target");
    let handle = current_address_space_handle(source).unwrap();
    let cap = object::allocate(source, 1).unwrap();
    let prepared = prepare(mode, source, cap, target);
    close_test_address_space(source).unwrap();
    assert_eq!(
        budget::used(handle),
        budget::Amount {
            pages: 1,
            objects: 1
        }
    );
    let replacement = create("staged alias source successor");
    assert_eq!(replacement, source);
    let next = current_address_space_handle(replacement).unwrap();
    assert_ne!(next, handle);
    assert!(matches!(
        budget::reserve_captured(
            handle,
            budget::Amount {
                pages: 1,
                objects: 1
            }
        ),
        Err(budget::Error::StaleDomain)
    ));
    assert_eq!(budget::used(next), budget::Amount::default());
    let next_cap = object::allocate(replacement, 1).unwrap();
    assert_eq!(next_cap, cap);
    assert_eq!(prepared.commit(), Err(MemoryObjectError::AddressSpaceMissing));
    assert_eq!(budget::used(handle), budget::Amount::default());
    assert_eq!(
        budget::used(next),
        budget::Amount {
            pages: 1,
            objects: 1
        }
    );
    assert_eq!(admission_tests::test_namespace_used(replacement), 1);
    assert_eq!(admission_tests::test_namespace_used(target), 0);
    object::write_bytes(replacement, next_cap, &[0x5a]).unwrap();
    close_test_address_space(replacement).unwrap();
    close_test_address_space(target).unwrap();
}

fn test_retired_target(mode: u32) {
    let source = create("staged alias stable source");
    let target = create("staged alias retiring target");
    let source_handle = current_address_space_handle(source).unwrap();
    let handle = current_address_space_handle(target).unwrap();
    let cap = object::allocate(source, 1).unwrap();
    let before = budget::used(source_handle);
    let prepared = prepare(mode, source, cap, target);
    let destination = prepared.target_cap();
    close_test_address_space(target).unwrap();
    let replacement = create("staged alias target successor");
    assert_eq!(replacement, target);
    let next = current_address_space_handle(replacement).unwrap();
    assert_ne!(next, handle);
    let next_cap = object::allocate(replacement, 1).unwrap();
    assert_eq!(next_cap, destination);
    assert_eq!(prepared.commit(), Err(MemoryObjectError::AddressSpaceMissing));
    assert_eq!(budget::used(source_handle), before);
    assert_eq!(
        budget::used(next),
        budget::Amount {
            pages: 1,
            objects: 1
        }
    );
    assert_eq!(admission_tests::test_namespace_used(replacement), 1);
    assert!(!object::info(source, cap).unwrap().lent);
    object::write_bytes(source, cap, &[0x5a]).unwrap();
    object::write_bytes(replacement, next_cap, &[0xa5]).unwrap();
    close_test_address_space(replacement).unwrap();
    close_test_address_space(source).unwrap();
}
