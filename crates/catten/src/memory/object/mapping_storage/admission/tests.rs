//! Actual mapping nodes, fixed quotas and real public mapping/cleanup paths.
use super::*;
use crate::memory::{
    self,
    AddressSpaceHandle,
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn pressure(platform: bool) -> Vec<Storage> {
    let mut nodes = Vec::with_capacity(TOTAL_LIMIT[1] as usize);
    loop {
        match prepare_with(platform, |charge| {
            // This fixture distinguishes actual allocation failure from quota.
            Ok(Storage::try_new_charged(charge).expect("mapping pressure allocator rejected"))
        }) {
            Ok(node) => {
                assert!(nodes.len() < TOTAL_LIMIT[1] as usize);
                nodes.push(node);
            }
            Err(MemoryObjectError::ResourceLimit) => return nodes,
            Err(error) => panic!("unexpected mapping pressure error: {error:?}"),
        }
    }
}
fn increment(before: ([u64; 2], [u64; 2]), count: u64) -> ([u64; 2], [u64; 2]) {
    let bytes = Storage::allocation_bytes() as u64 * count;
    ([before.0[0] + bytes, before.0[1] + count], [before.1[0] + bytes, before.1[1] + count])
}

pub(in crate::memory::object::mapping_storage) fn run() {
    allocation_failure();
    captured_classification();
    published_lifetime();
    actual_pressure();
    crate::logln!(
        "[mapping node admission] 64 charged allocator rejections; captured classification, \
         published/detached lifetime, actual ordinary/total node exhaustion, trusted reserve, \
         denial before snapshots/pins/leaves and cleanup at exhaustion passed; no quota reset"
    );
}
fn allocation_failure() {
    let before = snapshot();
    for _ in 0..64 {
        assert_eq!(
            prepare_with(false, |_| {
                super::super::tests::available();
                assert_eq!(snapshot(), increment(before, 1));
                Err(MemoryObjectError::ResourceLimit)
            })
            .err(),
            Some(MemoryObjectError::ResourceLimit)
        );
        assert_eq!(snapshot(), before);
    }
}
fn captured_classification() {
    let handle = root();
    let before = snapshot();
    let owner = Preparing::with(handle, 1, |stage| {
        if stage == 0 {
            memory::budget::mark_platform(handle);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(snapshot(), increment(before, 1));
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(snapshot(), increment(before, 1));
    let successor = root();
    assert_eq!(successor.id(), handle.id());
    assert_ne!(successor, handle);
    owner.finish();
    assert_eq!(snapshot(), before);
    memory::close_user_address_space_handle(successor).unwrap();
}
fn published_lifetime() {
    let handle = root();
    let before = snapshot();
    let node = prepare(false).unwrap();
    let record = MemoryMappingRecord {
        state: MemoryMappingState {
            address_space: handle,
            base: VAddr::from(0x2c0000usize),
            installed_pages: 0,
            writable: false,
            scratch: false,
        },
        cleanup_lease: None,
    };
    // Isolated metadata only: no physical leaves or root lease are fabricated.
    let mut mappings = Mappings::new();
    with_locked(|| mappings.insert(node, handle.id(), record));
    assert_eq!(snapshot(), increment(before, 1));
    let node = with_locked(|| mappings.take(&handle.id()).unwrap());
    assert_eq!(snapshot(), increment(before, 1));
    node.release_with(|(_, record)| {
        super::super::tests::available();
        assert_eq!(record.state.address_space, handle);
        assert_eq!(snapshot(), increment(before, 1), "payload disposal cannot refund node storage");
    });
    assert_eq!(snapshot(), before);
    memory::close_user_address_space_handle(handle).unwrap();
}
fn denied(handle: AddressSpaceHandle, cap: MemoryObjectCap) {
    let before = snapshot();
    let buffers = snapshot_storage::admitted();
    assert_eq!(
        map(handle.id(), cap, VAddr::from(0x2e0000usize), false),
        Err(MemoryObjectError::ResourceLimit)
    );
    assert_eq!(map_any(handle.id(), cap, false), Err(MemoryObjectError::ResourceLimit));
    assert!(!info(handle.id(), cap).unwrap().mapped);
    let registry = MEMORY_OBJECTS.lock();
    let id = registry.lookup(handle.id(), cap).unwrap().object;
    assert_eq!(registry.objects[&id].retirement_pins, 0);
    drop(registry);
    assert_eq!(snapshot(), before);
    assert_eq!(snapshot_storage::admitted(), buffers);
}
fn full(used: [u64; 2], limit: [u64; 2]) {
    assert!(used[0] <= limit[0] && used[1] <= limit[1]);
    assert!(used[0] + Storage::allocation_bytes() as u64 > limit[0] || used[1] == limit[1]);
}
fn actual_pressure() {
    let before = snapshot();
    let handle = root();
    let trusted = root();
    memory::budget::mark_platform(trusted);
    let cap = allocate(handle.id(), 1).unwrap();
    let denied_cap = allocate(handle.id(), 1).unwrap();
    map(handle.id(), cap, VAddr::from(0x2c0000usize), false).unwrap();
    let ordinary = pressure(false);
    full(snapshot().1, ORDINARY_LIMIT);
    for _ in 0..64 {
        assert_eq!(
            prepare_with(false, |_| panic!("quota denial entered allocator")).err(),
            Some(MemoryObjectError::ResourceLimit)
        );
    }
    denied(handle, denied_cap);
    let ordinary_full = snapshot();
    let trusted_cap = allocate(trusted.id(), 1).unwrap();
    map(trusted.id(), trusted_cap, VAddr::from(0x2c0000usize), false).unwrap();
    assert_eq!(snapshot().1, ordinary_full.1);
    unmap(trusted.id(), trusted_cap).unwrap();
    assert_eq!(snapshot(), ordinary_full);
    let reserve = pressure(true);
    let total_full = snapshot();
    full(total_full.0, TOTAL_LIMIT);
    assert_eq!(
        prepare_with(true, |_| panic!("total denial entered allocator")).err(),
        Some(MemoryObjectError::ResourceLimit)
    );
    denied(trusted, trusted_cap);
    memory::budget::mark_platform(handle);
    assert_eq!(snapshot(), total_full);
    // Real leaves must unmap and invalidate without preparing a replacement
    // record, refunding this original ordinary node even after late promotion.
    unmap(handle.id(), cap).unwrap();
    let bytes = Storage::allocation_bytes() as u64;
    assert_eq!(
        snapshot(),
        (
            [total_full.0[0] - bytes, total_full.0[1] - 1],
            [total_full.1[0] - bytes, total_full.1[1] - 1]
        )
    );
    close_cap(handle.id(), cap).unwrap();
    close_cap(handle.id(), denied_cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
    let fresh = root();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    let fresh_cap = allocate(fresh.id(), 1).unwrap();
    map(fresh.id(), fresh_cap, VAddr::from(0x2c0000usize), false).unwrap();
    assert_eq!(snapshot(), total_full);
    unmap(fresh.id(), fresh_cap).unwrap();
    close_cap(fresh.id(), fresh_cap).unwrap();
    memory::close_user_address_space_handle(fresh).unwrap();
    let counts = (ordinary.len(), reserve.len());
    drop(reserve);
    drop(ordinary);
    close_cap(trusted.id(), trusted_cap).unwrap();
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[mapping node pressure] {} ordinary + {} reserve actual unused nodes plus one public \
         mapping, {} requested bytes/node; 768/1024 KiB and 6144/8192 ceilings; exact baseline \
         after deallocation, public unmap and generation reuse at total exhaustion passed",
        counts.0,
        counts.1,
        bytes
    );
}
