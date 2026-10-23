//! Real root/context/retirement-node admission. Private fixture ceilings leave
//! normal policy unchanged and distinguish byte pressure from bundle counts.
use super::*;
use crate::{
    cpu::{
        isa::lp::thread_context::ThreadContext,
        scheduler::threads::{
            Thread,
            retirement_tests,
        },
    },
    memory::{
        self,
        AddressSpace,
        AddressSpaceHandle,
        budget,
    },
};

type Entry = PreparedEntry<Thread, budget::thread_metadata::Charge>;
extern "C" fn unused() {}

fn domain(principal: u64) -> AddressSpaceHandle {
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    memory::register_domain_authority(root, principal, 0).unwrap();
    root
}
fn used(principal: u64) -> [u64; 2] {
    HISTORY.lock().get(&principal).unwrap().thread_metadata.used()
}
fn set_limit(principal: u64, limit: [u64; 2]) {
    let mut history = HISTORY.lock();
    let budget = &mut history.get_mut(&principal).unwrap().thread_metadata;
    assert_eq!(budget.used(), [0; 2]);
    *budget = VectorBudget::new(limit);
}
fn prepare(root: AddressSpaceHandle, bytes: usize) -> Entry {
    Entry::try_new_charged(budget::reserve_thread_metadata(root, bytes).unwrap()).unwrap()
}

pub(super) fn run() {
    let principal = charlotte_launch::artifact_principal_id(b"thread-metadata-pressure");
    let other = charlotte_launch::artifact_principal_id(b"thread-metadata-healthy");
    let first = domain(principal);
    let second = domain(principal);
    let healthy = domain(other);
    let bytes = Entry::allocation_bytes() + core::mem::size_of::<ThreadContext>();
    let b = bytes as u64;
    set_limit(principal, [2 * b, 2]);
    let node = prepare(first, bytes);
    let thread = Thread::try_new(second.id(), unused).unwrap();
    assert_eq!(used(principal), [2 * b, 2]);
    let baseline = budget::thread_metadata::snapshot();
    for _ in 0..128 {
        retirement_tests::assert_metadata_rejection(first.id());
        retirement_tests::assert_metadata_rejection(second.id());
    }
    assert_eq!(budget::thread_metadata::snapshot(), baseline);
    let healthy_thread = Thread::try_new(healthy.id(), unused).unwrap();
    healthy_thread.release_unstarted().unwrap();
    assert_eq!(used(other), [0; 2]);
    // Classification changes cannot erase an already-admitted ordinary charge
    // or bypass the signed principal's aggregate ceiling.
    budget::mark_platform(second);
    assert_eq!(budget::thread_metadata::snapshot(), baseline);
    retirement_tests::assert_metadata_rejection(second.id());
    thread.release_unstarted().unwrap();
    assert_eq!(used(principal), [b, 1]);
    memory::close_user_address_space_handle(first).unwrap();
    let replacement = domain(other);
    assert_eq!(replacement.id(), first.id());
    assert_ne!(replacement, first);
    assert!(matches!(
        budget::reserve_thread_metadata(first, bytes),
        Err(budget::Error::StaleDomain)
    ));
    let other_thread = Thread::try_new(replacement.id(), unused).unwrap();
    let other_used = used(other);
    assert_eq!(other_used, [b, 1]);
    let second_node = prepare(second, bytes);
    retirement_tests::assert_metadata_rejection(second.id());
    drop(node);
    assert_eq!(used(principal), [b, 1]);
    assert_eq!(used(other), other_used);
    drop(second_node);
    assert_eq!(used(principal), [0; 2]);
    // Independent dimensions: byte denial below one allocation even with 128
    // record slots, then record denial with abundant byte allowance.
    set_limit(principal, [b - 1, 128]);
    retirement_tests::assert_metadata_rejection(second.id());
    set_limit(principal, [1024 * 1024, 1]);
    let node = prepare(second, bytes);
    retirement_tests::assert_metadata_rejection(second.id());
    drop(node);
    let recovered = Thread::try_new(second.id(), unused).unwrap();
    recovered.release_unstarted().unwrap();
    other_thread.release_unstarted().unwrap();
    assert_eq!(used(principal), [0; 2]);
    assert_eq!(used(other), [0; 2]);
    memory::close_user_address_space_handle(second).unwrap();
    memory::close_user_address_space_handle(healthy).unwrap();
    memory::close_user_address_space_handle(replacement).unwrap();
    node_pressure(bytes);
    abandoned_signed_thread(bytes);
    crate::logln!(
        "[thread metadata admission] bytes={} per bundle; shared-root pressure, 256 \
         pre-allocation denials, independent byte/count limits, promotion, retired sponsor/ASID \
         reuse, late release and healthy progress passed",
        bytes
    );
}

fn node_pressure(bytes: usize) {
    use alloc::vec::Vec;
    let before = budget::thread_metadata::snapshot();
    let roots: Vec<_> = (0..32)
        .map(|n| {
            let name = alloc::format!("thread-metadata-node-pressure-{n}");
            let principal = charlotte_launch::artifact_principal_id(name.as_bytes());
            (domain(principal), principal)
        })
        .collect();
    let mut nodes = Vec::new();
    let mut denied = None;
    'fill: for (root, principal) in &roots {
        for _ in 0..128 {
            match budget::reserve_thread_metadata(*root, bytes) {
                Ok(charge) => nodes.push(Entry::try_new_charged(charge).unwrap()),
                Err(budget::Error::Limit) => {
                    // This principal has capacity. Rejection comes from the
                    // shared ordinary/node pool, with principal admission undone.
                    assert!(used(*principal)[1] < 128);
                    denied = Some(*root);
                    break 'fill;
                }
                Err(error) => panic!("metadata pressure root lost: {error:?}"),
            }
        }
    }
    let root = denied.expect("ordinary metadata pool did not reject within 4096 bundles");
    retirement_tests::assert_metadata_rejection(root.id());
    let full = budget::thread_metadata::snapshot();
    let kernel = Thread::try_new(memory::KERNEL_ASID, unused).unwrap();
    kernel.release_unstarted().unwrap();
    assert_eq!(budget::thread_metadata::snapshot(), full);
    let count = nodes.len();
    drop(nodes);
    assert_eq!(budget::thread_metadata::snapshot(), before);
    let recovered = Thread::try_new(root.id(), unused).unwrap();
    recovered.release_unstarted().unwrap();
    for (root, principal) in roots {
        assert_eq!(used(principal), [0; 2]);
        memory::close_user_address_space_handle(root).unwrap();
    }
    crate::logln!(
        "[thread metadata node pressure] {} real prepared nodes exhausted ordinary admission; \
         kernel thread reserve and post-deallocation recovery passed",
        count
    );
}

fn abandoned_signed_thread(bytes: usize) {
    let principal = charlotte_launch::artifact_principal_id(b"thread-metadata-abandoned");
    let root = domain(principal);
    let thread = Thread::try_new(root.id(), unused).unwrap();
    let before = budget::thread_metadata::snapshot();
    {
        let _history = HISTORY.lock();
        let _heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let _root = memory::ADDRESS_SPACE_TABLE.lock();
        let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock();
        let free = frames.free_frames();
        budget::thread_metadata::with_locked(|| drop(thread));
        assert_eq!(frames.free_frames(), free);
    }
    assert_eq!(budget::thread_metadata::snapshot(), before);
    assert_eq!(used(principal), [bytes as u64, 1]);
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
}
