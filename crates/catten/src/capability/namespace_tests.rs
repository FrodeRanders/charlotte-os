//! Serialized boot probes for namespace storage, not authority-record storage.
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use super::*;

static REJECT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static IRQ: AtomicBool = AtomicBool::new(false);
static REGISTRATION: AtomicBool = AtomicBool::new(false);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static DISPOSED: AtomicUsize = AtomicUsize::new(0);

pub(super) fn reject(stage: usize) -> bool {
    REJECT.compare_exchange(stage, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok()
}
pub(super) fn boundary(dispose: bool) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), IRQ.load(Ordering::Relaxed));
    assert!(CAPABILITIES.try_lock().is_some(), "namespace metadata below capability guard");
    assert!(crate::memory::allocators::global_allocator::PRIMARY_ALLOCATOR.try_lock().is_some());
    if REGISTRATION.load(Ordering::Relaxed) {
        assert!(crate::memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
        assert!(crate::memory::ADDRESS_SPACE_TABLE.try_lock().is_some());
        assert!(crate::memory::KERNEL_AS.try_lock().is_some());
        assert!(crate::memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
    }
    if dispose {
        DISPOSED.fetch_add(1, Ordering::Relaxed);
    } else {
        ALLOCATED.fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) fn run() {
    use crate::{
        memory,
        service::loader,
    };
    assert!(!ACTIVE.swap(true, Ordering::AcqRel));
    IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Relaxed);
    REGISTRATION.store(true, Ordering::Relaxed);
    let occupied = memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|e| e.is_some()).count();
    let namespaces = CAPABILITIES.lock().iter().count();
    let used = budget::node_used();
    let free = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for stage in [1, 2] {
        let root = memory::AddressSpace::try_new_user().unwrap();
        REJECT.store(stage, Ordering::Release);
        assert_eq!(
            memory::register_user_address_space(root),
            Err(memory::AddressSpaceRegistrationError::CapabilityNamespaceAllocationFailed)
        );
        assert_eq!(REJECT.load(Ordering::Acquire), 0);
        assert_eq!(
            memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|e| e.is_some()).count(),
            occupied
        );
        assert_eq!(CAPABILITIES.lock().iter().count(), namespaces);
        assert_eq!(budget::node_used(), used);
        assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    }
    prepare_namespace().unwrap().cancel_unpublished();
    REGISTRATION.store(false, Ordering::Relaxed);

    let root = loader::create_user_address_space_handle();
    let old_token = reserve(root.id(), ObjectKind::Memory).unwrap();
    let old = old_token.namespace.clone();
    // Namespace detachment cannot destroy remaining record nodes or counters,
    // even while the actual allocator is held. Preserve the complete node.
    let detached = {
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let mut tables = CAPABILITIES.lock();
        tables.get(&root.id()).unwrap().budget.retire();
        let detached = tables.take(&root.id()).unwrap();
        assert_eq!(detached.value().1.budget.used(), 1);
        drop(tables);
        drop(heap);
        detached
    };
    let prepared = prepare_namespace().unwrap();
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        prepared.publish(root);
        drop(heap);
        drop(lifecycle);
    }
    let new_token = reserve(root.id(), ObjectKind::Memory).unwrap();
    assert_eq!(new_token.identity(), old_token.identity());
    let new = new_token.namespace.clone();
    assert!(!Arc::ptr_eq(&old, &new));
    assert_eq!(old.used(), 1, "detachment must retain original charge");
    release_namespace(detached);
    assert_eq!(old.used(), 0);
    assert_eq!(old_token.publish(), Err(AllocationError::Retired));
    assert_eq!(new.used(), 1, "late token must not change replacement account");
    let cap = new_token.publish().unwrap();
    assert!(contains(root.id(), cap, ObjectKind::Memory));
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(new.used(), 0);

    // Preflight lost to another namespace publisher: the helper must leave
    // redundant storage with its preparation owner for post-guard disposal.
    let mut private = AdmittedMap::new();
    let winner = prepare_namespace().unwrap();
    winner.publish_into(&mut private, 0x5e49, None);
    let mut unused = Some(prepare_namespace().unwrap());
    let first = namespace(&mut private, 0x5e49, None, &mut unused).unwrap();
    assert_eq!(first.next_serial, 1);
    assert!(unused.is_some());
    unused.take().unwrap().cancel_unpublished();
    release_namespace(private.take(&0x5e49).unwrap());

    let abandoned = prepare_namespace().unwrap();
    let retained = abandoned.0.value.as_ref().unwrap().budget.clone();
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let registry = CAPABILITIES.lock();
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        drop(abandoned);
        drop(heap);
        drop(registry);
        drop(lifecycle);
    }
    assert_eq!(Arc::strong_count(&retained), 2);
    assert_eq!(retained.used(), 0);
    assert_eq!(budget::node_used(), used);
    ACTIVE.store(false, Ordering::Release);
    crate::logln!(
        "[capability namespace storage] first-node/account preparation rejection before ASID \
         publication; heap-held exact namespace publication/detachment; original-charge release \
         outside capability guard; late token cannot alter replacement; guarded abandonment \
         retains one empty node/account ({} allocation, {} disposal boundaries; entry IRQ \
         preserved)",
        ALLOCATED.load(Ordering::Acquire),
        DISPOSED.load(Ordering::Acquire)
    );
}
