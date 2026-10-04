//! Kernel fixtures for real namespace entries and owning admission tokens.
//! Fake payloads exist only in isolated fixture namespaces; no family registry
//! or live node-wide limit is filled/rewritten by the counter saturation test.

use alloc::vec::Vec;

use super::*;
use crate::logln;

const OWNER: AddressSpaceId = 0x5e40;

fn account(owner: AddressSpaceId) -> Arc<budget::DomainBudget> {
    CAPABILITIES.lock().get(&owner).unwrap().budget.clone()
}

pub(crate) fn test_admission() {
    budget::test_node_admission();
    let staged = reserve(OWNER, ObjectKind::Ipc).unwrap();
    let cancelled = staged.identity();
    let local = account(OWNER);
    assert_eq!(local.used(), 1);
    assert!(!contains(OWNER, cancelled, ObjectKind::Ipc));
    assert!(!remove(OWNER, cancelled, ObjectKind::Ipc));
    drop(staged);
    assert_eq!(local.used(), 0);
    let cap = try_allocate(OWNER, ObjectKind::Ipc).unwrap();
    assert!(cap > cancelled, "cancelled identities must never be reused");
    assert!(contains(OWNER, cap, ObjectKind::Ipc));
    assert!(!remove(OWNER, cap, ObjectKind::Memory));
    assert!(matches!(
        begin_move(OWNER, cap, ObjectKind::Memory),
        Err(AllocationError::UnknownCapability)
    ));
    assert!(remove(OWNER, cap, ObjectKind::Ipc));

    // All six kinds share one domain account. These are namespace records,
    // not subsystem payload objects; a full source retains its rollback slot.
    let kinds = [
        ObjectKind::Ipc,
        ObjectKind::Memory,
        ObjectKind::Completion,
        ObjectKind::Device,
        ObjectKind::Mailbox,
        ObjectKind::SystemObserver,
    ];
    let mut records = Vec::new();
    for n in 0..budget::DOMAIN_LIMIT {
        let kind = kinds[n % kinds.len()];
        records.push((try_allocate(OWNER, kind).unwrap(), kind));
    }
    assert_eq!(local.used(), budget::DOMAIN_LIMIT);
    let (total, ordinary) = budget::node_used();
    assert!(total >= local.used() && ordinary >= local.used());
    assert!(matches!(reserve(OWNER, ObjectKind::Memory), Err(AllocationError::ResourceLimit)));
    let (source, kind) = records.pop().unwrap();
    let escrow = begin_move(OWNER, source, kind).unwrap();
    assert_eq!(local.used(), budget::DOMAIN_LIMIT);
    assert!(!contains(OWNER, source, kind));
    assert!(!remove(OWNER, source, kind));
    assert!(matches!(reserve(OWNER, kind), Err(AllocationError::ResourceLimit)));
    assert_eq!(escrow.restore(), Ok(source));
    assert!(contains(OWNER, source, kind));
    drop(begin_move(OWNER, source, kind).unwrap());
    assert_eq!(local.used(), budget::DOMAIN_LIMIT - 1);
    let replacement = try_allocate(OWNER, kind).unwrap();
    assert!(replacement > source);
    assert!(remove(OWNER, replacement, kind));
    for (cap, kind) in records {
        assert!(remove(OWNER, cap, kind));
    }
    assert_eq!(local.used(), 0);

    // A rejected batch can cancel every staged entry without publishing any
    // authority. This is RAII staging, not an atomic vector admission API.
    let mut batch = Vec::new();
    for _ in 0..budget::DOMAIN_LIMIT {
        batch.push(reserve(OWNER, ObjectKind::Memory).unwrap());
    }
    assert!(matches!(reserve(OWNER, ObjectKind::Memory), Err(AllocationError::ResourceLimit)));
    assert!(batch.iter().all(|item| !contains(OWNER, item.identity(), ObjectKind::Memory)));
    drop(batch);
    assert_eq!(local.used(), 0);

    // Explicit legacy migration bridge: counted, not yet policy-limited.
    for _ in 0..=budget::DOMAIN_LIMIT {
        allocate_unmigrated(OWNER, ObjectKind::Memory);
    }
    assert_eq!(local.used(), budget::DOMAIN_LIMIT + 1);
    assert!(matches!(reserve(OWNER, ObjectKind::Mailbox), Err(AllocationError::ResourceLimit)));
    close_address_space(OWNER);
    assert_eq!(local.used(), 0);
    test_replacement_tokens();
    test_real_retirement();
    test_completion_admission();
    logln!(
        "[capability] shared counts, staged admission, move escrow and generation fencing passed"
    );
}

fn test_replacement_tokens() {
    let old_staged = reserve(OWNER, ObjectKind::Memory).unwrap();
    let staged_id = old_staged.identity();
    let old_live = try_allocate(OWNER, ObjectKind::Ipc).unwrap();
    let old_escrow = begin_move(OWNER, old_live, ObjectKind::Ipc).unwrap();
    let old = account(OWNER);
    close_address_space(OWNER);
    assert_eq!(old.used(), 0);
    let new_staged = reserve(OWNER, ObjectKind::Memory).unwrap();
    assert_eq!(new_staged.identity(), staged_id);
    let new_live = try_allocate(OWNER, ObjectKind::Ipc).unwrap();
    assert_eq!(new_live, old_live);
    let new_escrow = begin_move(OWNER, new_live, ObjectKind::Ipc).unwrap();
    let new = account(OWNER);
    assert_eq!(old_staged.publish(), Err(AllocationError::Retired));
    assert_eq!(old_escrow.restore(), Err(AllocationError::Retired));
    assert_eq!(new.used(), 2, "old token Drop must leave same-state replacements alone");
    assert_eq!(new_staged.publish(), Ok(staged_id));
    assert_eq!(new_escrow.restore(), Ok(new_live));
    close_address_space(OWNER);
    assert_eq!(new.used(), 0);

    let staged = reserve(OWNER, ObjectKind::Memory).unwrap();
    let live = try_allocate(OWNER, ObjectKind::Ipc).unwrap();
    let escrow = begin_move(OWNER, live, ObjectKind::Ipc).unwrap();
    let local = account(OWNER);
    retire_address_space(OWNER);
    assert!(matches!(reserve(OWNER, ObjectKind::Memory), Err(AllocationError::Retired)));
    assert_eq!(staged.publish(), Err(AllocationError::Retired));
    assert_eq!(escrow.restore(), Err(AllocationError::Retired));
    assert_eq!(local.used(), 0);
    close_address_space(OWNER);
}

fn test_real_retirement() {
    use crate::{
        memory,
        service::loader,
    };

    let old = loader::create_user_address_space_handle();
    let staged = reserve(old.id(), ObjectKind::Device).unwrap();
    let staged_id = staged.identity();
    let local = account(old.id());
    assert_eq!(CAPABILITIES.lock().get(&old.id()).unwrap().address_space, Some(old));
    memory::close_user_address_space_handle(old).unwrap();
    assert_eq!(local.used(), 0);
    let new = loader::create_user_address_space_handle();
    assert_eq!(new.id(), old.id());
    assert_ne!(new.generation(), old.generation());
    assert!(matches!(
        reserve_captured(new.id(), ObjectKind::Device, Some(old)),
        Err(AllocationError::Retired)
    ));
    let replacement = reserve(new.id(), ObjectKind::Device).unwrap();
    assert_eq!(replacement.identity(), staged_id);
    assert_eq!(staged.publish(), Err(AllocationError::Retired));
    let cap = replacement.publish().unwrap();
    assert!(contains(new.id(), cap, ObjectKind::Device));
    memory::budget::retire(new);
    assert!(matches!(reserve(new.id(), ObjectKind::Device), Err(AllocationError::Retired)));
    memory::close_user_address_space_handle(new).unwrap();
}

/// Kernel-only records let mailbox fixtures fill the aggregate account
/// without also filling their narrower family pool.
pub(crate) fn test_fill_namespace(owner: AddressSpaceId) {
    for _ in 0..budget::DOMAIN_LIMIT {
        try_allocate(owner, ObjectKind::Memory).unwrap();
    }
}

fn test_completion_admission() {
    use crate::completion::{
        self,
        OpCode,
        SubmitError,
    };
    const CLIENT: AddressSpaceId = 0x5e42;
    completion::open_address_space(CLIENT, 8);
    test_fill_namespace(CLIENT);
    let local = account(CLIENT);
    let record = completion::record_admission(CLIENT).unwrap();
    assert_eq!(completion::submit(CLIENT, OpCode::Nop, None), Err(SubmitError::WouldBlock));
    assert_eq!(completion::submit_timer(CLIENT, 60_000), Err(SubmitError::WouldBlock));
    assert_eq!(completion::timer_events_used(CLIENT), 0);
    assert_eq!(record.used(), 0, "shared rejection must refund completion payload staging");
    assert_eq!(local.used(), budget::DOMAIN_LIMIT);
    assert!(remove(CLIENT, 1, ObjectKind::Memory));
    let cap = completion::submit(CLIENT, OpCode::Nop, None).unwrap();
    assert_eq!(cap, budget::DOMAIN_LIMIT as u64 + 1, "quota rejection must not consume serials");
    completion::complete(CLIENT, cap, completion::OpResult::Ok(0)).unwrap();
    completion::close(CLIENT, cap).unwrap();
    assert_eq!(local.used(), budget::DOMAIN_LIMIT - 1);
    let timer = completion::submit_timer(CLIENT, 60_000).unwrap();
    completion::abort_submission(CLIENT, timer).unwrap();
    assert_eq!(record.used(), 0);
    assert_eq!(local.used(), budget::DOMAIN_LIMIT - 1);
    completion::close_address_space(CLIENT);
    close_address_space(CLIENT);
    assert_eq!(local.used(), 0);
}
