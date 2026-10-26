//! Production node admission, public rejection and exact lifetime at exhaustion.
use super::{
    super::*,
    *,
};
use crate::memory::{
    self,
    AddressSpaceHandle,
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn increment(before: ([u64; 2], [u64; 2]), bytes: usize, count: u64) -> ([u64; 2], [u64; 2]) {
    (
        [before.0[0] + bytes as u64, before.0[1] + count],
        [before.1[0] + bytes as u64, before.1[1] + count],
    )
}

pub(in crate::memory::object::cap_storage) fn run() {
    allocator_rejection::<(MemoryObjectCap, Record)>();
    allocator_rejection::<(AddressSpaceId, AddressSpaceCaps)>();
    allocator_rejection::<(AddressSpaceId, Borrower)>();
    captured_classification();
    detached_lifetime();
    actual_pressure();
    crate::logln!(
        "[memory capability node admission] 192 charged allocator rejections; captured \
         classification, actual ordinary/total exhaustion, trusted reserve, two-node partial \
         rejection, public allocate/move/copy/loan denial without source mutation, \
         published/detached charge lifetime and cleanup at exhaustion passed; no quota reset"
    );
}
fn allocator_rejection<T>() {
    let before = snapshot();
    for _ in 0..64 {
        let result = prepare_with::<T>(false, |_| {
            super::super::tests::available();
            assert_eq!(
                snapshot(),
                increment(before, PreparedEntry::<T, Charge>::allocation_bytes(), 1)
            );
            Err(AllocError)
        });
        assert!(result.is_err());
        assert_eq!(snapshot(), before);
    }
}
fn captured_classification() {
    let handle = root();
    let before = snapshot();
    let owner = Preparing::with(admit_capability(handle.id(), handle).unwrap(), handle, |stage| {
        if stage == 0 {
            memory::budget::mark_platform(handle);
        }
        Ok(())
    })
    .unwrap();
    let borrower = owner.prepare_borrower().unwrap();
    let bytes = Storage::allocation_bytes()
        + NamespaceStorage::allocation_bytes()
        + BorrowerStorage::allocation_bytes();
    assert_eq!(snapshot(), increment(before, bytes, 3));
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(snapshot(), increment(before, bytes, 3));
    let fresh = root();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    drop(borrower);
    owner.finish();
    assert_eq!(snapshot(), before);
    memory::close_user_address_space_handle(fresh).unwrap();
}
fn detached_lifetime() {
    let handle = root();
    let before = snapshot();
    let mut local = MemoryObjectRegistry::new();
    let mut owner = Preparing::new(admit_capability(handle.id(), handle).unwrap(), handle).unwrap();
    let bytes = Storage::allocation_bytes() + NamespaceStorage::allocation_bytes();
    assert_eq!(snapshot(), increment(before, bytes, 2));
    with_locked(|| {
        owner.publish().unwrap();
        owner.install(&mut local, super::super::tests::payload());
    });
    owner.finish();
    let namespace = {
        let _registry = MEMORY_OBJECTS.lock();
        let _heap = crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let mut retired = None;
        with_locked(|| retired = retire_namespace(&mut local, handle.id()));
        retired.unwrap()
    };
    assert_eq!(snapshot(), increment(before, bytes, 2));
    namespace.release();
    assert_eq!(snapshot(), before);
    memory::close_user_address_space_handle(handle).unwrap();
}
fn pressure(platform: bool) -> Vec<Storage> {
    let mut nodes = Vec::with_capacity(TOTAL_LIMIT[1] as usize);
    loop {
        match prepare_with(platform, |charge| {
            Ok(Storage::try_new_charged(charge)
                .expect("memory capability pressure allocator rejected"))
        }) {
            Ok(node) => {
                assert!(nodes.len() < TOTAL_LIMIT[1] as usize);
                nodes.push(node);
            }
            Err(_) => return nodes,
        }
    }
}
fn denied(handle: AddressSpaceHandle, cap: MemoryObjectCap, target: AddressSpaceHandle) {
    let before = snapshot();
    let authority = crate::capability::node_admission_used();
    let buffers = snapshot_storage::admitted();
    let used = memory::budget::used(handle);
    assert_eq!(allocate(handle.id(), 1), Err(MemoryObjectError::ResourceLimit));
    assert!(matches!(
        prepare_move(handle.id(), cap, target.id()),
        Err(MemoryObjectError::ResourceLimit)
    ));
    assert!(matches!(
        prepare_copy(handle.id(), cap, target.id()),
        Err(MemoryObjectError::ResourceLimit)
    ));
    for write in [false, true] {
        assert!(matches!(
            prepare_loan(handle.id(), cap, target.id(), write),
            Err(MemoryObjectError::ResourceLimit)
        ));
    }
    assert_eq!(snapshot(), before);
    assert_eq!(crate::capability::node_admission_used(), authority);
    assert_eq!(snapshot_storage::admitted(), buffers);
    assert_eq!(memory::budget::used(handle), used);
    let registry = MEMORY_OBJECTS.lock();
    let entry = registry.lookup(handle.id(), cap).unwrap();
    let object = &registry.objects[&entry.object];
    assert!(!entry.transfer_in_flight && !entry.delivery_pending);
    assert_eq!(object.copy_pins, 0);
    assert!(!object.lend_state.is_active());
    drop(registry);
    write_bytes(handle.id(), cap, &[0x5a]).unwrap();
}
fn full(used: [u64; 2], limit: [u64; 2]) {
    assert!(used[0] <= limit[0] && used[1] <= limit[1]);
    let smallest = Storage::allocation_bytes().min(NamespaceStorage::allocation_bytes()) as u64;
    assert!(used[0] + smallest > limit[0] || used[1] == limit[1]);
}
fn actual_pressure() {
    let before = snapshot();
    let handle = root();
    let target = root();
    let trusted = root();
    memory::budget::mark_platform(trusted);
    let cap = allocate(handle.id(), 1).unwrap();
    let trusted_cap = allocate(trusted.id(), 1).unwrap();
    let mut ordinary = pressure(false);
    full(snapshot().1, ORDINARY_LIMIT);
    for _ in 0..64 {
        assert!(
            prepare_with::<(MemoryObjectCap, Record)>(false, |_| panic!(
                "quota entered payload allocator"
            ))
            .is_err()
        );
        assert!(
            prepare_with::<(AddressSpaceId, AddressSpaceCaps)>(false, |_| panic!(
                "quota entered namespace allocator"
            ))
            .is_err()
        );
    }
    // Ordinary target classification must reject before changing the source.
    denied(handle, cap, target);
    // A single returned slot cannot publish a two-node preparation. Its partial
    // payload and hidden authority must be explicitly cancelled on rejection.
    drop(ordinary.pop().unwrap());
    let one_slot = snapshot();
    assert_eq!(allocate(handle.id(), 1), Err(MemoryObjectError::ResourceLimit));
    assert_eq!(snapshot(), one_slot);
    ordinary.push(prepare(false).unwrap());
    let ordinary_full = snapshot();
    let extra = allocate(trusted.id(), 1).unwrap();
    assert_eq!(snapshot().1, ordinary_full.1);
    close_cap(trusted.id(), extra).unwrap();
    assert_eq!(snapshot(), ordinary_full);
    let reserve = pressure(true);
    let total_full = snapshot();
    full(total_full.0, TOTAL_LIMIT);
    assert!(
        prepare_with::<(AddressSpaceId, AddressSpaceCaps)>(true, |_| panic!(
            "total quota entered allocator"
        ))
        .is_err()
    );
    memory::budget::mark_platform(target);
    denied(trusted, trusted_cap, target);
    memory::budget::mark_platform(handle);
    // A real published node is reclaimed at total exhaustion, with its original
    // ordinary charge despite promotion. Namespace storage stays until root close.
    close_cap(handle.id(), cap).unwrap();
    let bytes = Storage::allocation_bytes() as u64;
    assert_eq!(
        snapshot(),
        (
            [total_full.0[0] - bytes, total_full.0[1] - 1],
            [total_full.1[0] - bytes, total_full.1[1] - 1]
        )
    );
    memory::close_user_address_space_handle(handle).unwrap();
    let namespace_bytes = NamespaceStorage::allocation_bytes() as u64;
    assert_eq!(
        snapshot(),
        (
            [total_full.0[0] - bytes - namespace_bytes, total_full.0[1] - 2],
            [total_full.1[0] - bytes - namespace_bytes, total_full.1[1] - 2]
        )
    );
    let fresh = root();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    let fresh_cap = allocate(fresh.id(), 1).unwrap();
    assert_eq!(snapshot(), total_full);
    close_cap(fresh.id(), fresh_cap).unwrap();
    memory::close_user_address_space_handle(fresh).unwrap();
    let counts = (ordinary.len(), reserve.len());
    drop(reserve);
    drop(ordinary);
    close_cap(trusted.id(), trusted_cap).unwrap();
    memory::close_user_address_space_handle(trusted).unwrap();
    memory::close_user_address_space_handle(target).unwrap();
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[memory capability node pressure] {} ordinary + {} reserve unused payload nodes; \
         payload/namespace layouts {}/{} bytes, ordinary/total byte ceilings {}/{}, node ceilings \
         {}/{}; exact baseline after real cleanup and root reuse passed",
        counts.0,
        counts.1,
        bytes,
        namespace_bytes,
        ORDINARY_LIMIT[0],
        TOTAL_LIMIT[0],
        ORDINARY_LIMIT[1],
        TOTAL_LIMIT[1]
    );
}

/// Run before other borrower-charge assertions, so restoring the old uncharged
/// factory fails here without entering a deliberately held-lock test.
pub(in crate::memory::object) fn borrower_pressure() {
    let before = snapshot();
    let source = root();
    let target = root();
    let trusted = root();
    memory::budget::mark_platform(trusted);
    let cap = allocate(source.id(), 1).unwrap();
    let retained_cap = allocate(source.id(), 1).unwrap();
    let target_cap = allocate(target.id(), 1).unwrap();
    let trusted_cap = allocate(trusted.id(), 1).unwrap();
    let retained_loan = lend_read(source.id(), retained_cap, target.id()).unwrap();
    map_any(target.id(), retained_loan, false).unwrap();
    let mut ordinary = pressure(false);
    full(snapshot().1, ORDINARY_LIMIT);
    drop(ordinary.pop().unwrap());
    drop(ordinary.pop().unwrap());
    let two_slots = snapshot();
    let authority = crate::capability::node_admission_used();
    let backing = memory::budget::used(source);
    assert!(
        matches!(
            prepare_loan(source.id(), cap, target.id(), false),
            Err(MemoryObjectError::ResourceLimit)
        ),
        "read-loan bypassed third-node metadata admission"
    );
    assert_eq!(snapshot(), two_slots);
    assert_eq!(crate::capability::node_admission_used(), authority);
    assert_eq!(memory::budget::used(source), backing);
    {
        let registry = MEMORY_OBJECTS.lock();
        let entry = registry.lookup(source.id(), cap).unwrap();
        let object = &registry.objects[&entry.object];
        assert!(!entry.transfer_in_flight && !entry.delivery_pending);
        assert_eq!(object.copy_pins, 0);
        assert!(!object.lend_state.is_active());
    }
    write_bytes(source.id(), cap, &[0x5a]).unwrap();
    // Write borrowing is inline: it must remain admissible with two slots.
    drop(prepare_loan(source.id(), cap, target.id(), true).unwrap());
    assert_eq!(snapshot(), two_slots);
    assert_eq!(crate::capability::node_admission_used(), authority);
    write_bytes(source.id(), cap, &[0xa5]).unwrap();
    let trusted_loan = lend_read(source.id(), cap, trusted.id()).unwrap();
    assert_eq!(snapshot().1, two_slots.1);
    map_any(trusted.id(), trusted_loan, false).unwrap();
    let reserve = pressure(true);
    let total_full = snapshot();
    full(total_full.0, TOTAL_LIMIT);
    memory::budget::mark_platform(target);
    let bytes = (Storage::allocation_bytes() + BorrowerStorage::allocation_bytes()) as u64;
    // Mapped revocation needs no new node even at total exhaustion. Promotion
    // cannot redirect either detached node's original ordinary reservation.
    revoke_lend(source.id(), retained_cap, target.id(), retained_loan).unwrap();
    let after_ordinary = (
        [total_full.0[0] - bytes, total_full.0[1] - 2],
        [total_full.1[0] - bytes, total_full.1[1] - 2],
    );
    assert_eq!(snapshot(), after_ordinary);
    let authority = crate::capability::node_admission_used();
    assert!(matches!(
        prepare_loan(trusted.id(), trusted_cap, target.id(), false),
        Err(MemoryObjectError::ResourceLimit)
    ));
    assert_eq!(snapshot(), after_ordinary);
    assert_eq!(crate::capability::node_admission_used(), authority);
    write_bytes(trusted.id(), trusted_cap, &[0x3c]).unwrap();
    revoke_lend(source.id(), cap, trusted.id(), trusted_loan).unwrap();
    assert_eq!(
        snapshot(),
        ([after_ordinary.0[0] - bytes, after_ordinary.0[1] - 2], after_ordinary.1,)
    );
    write_bytes(source.id(), cap, &[0xc3]).unwrap();
    let counts = (ordinary.len(), reserve.len());
    drop(reserve);
    drop(ordinary);
    close_cap(source.id(), cap).unwrap();
    close_cap(source.id(), retained_cap).unwrap();
    close_cap(target.id(), target_cap).unwrap();
    close_cap(trusted.id(), trusted_cap).unwrap();
    memory::close_user_address_space_handle(source).unwrap();
    memory::close_user_address_space_handle(target).unwrap();
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[memory borrower admission] {} ordinary + {} reserve pressure nodes; \
         payload/namespace/borrower layouts {}/{}/{} bytes; ordinary/total third-node rejection, \
         two-node write cancellation, trusted read reserve and mapped revocation at exhaustion \
         with original charges passed; exact baseline restored",
        counts.0,
        counts.1,
        Storage::allocation_bytes(),
        NamespaceStorage::allocation_bytes(),
        BorrowerStorage::allocation_bytes()
    );
}
