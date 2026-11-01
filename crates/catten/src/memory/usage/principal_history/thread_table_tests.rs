//! Exact signed sponsorship and real replacement buffers, outside global table
//! publication. Physical threads are never scheduled by these controls.
use super::*;
use crate::{
    cpu::scheduler::threads::{
        MASTER_THREAD_TABLE,
        Thread,
        ThreadTable,
    },
    memory::{
        self,
        AddressSpaceHandle,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
        budget,
    },
};

fn domain(principal: u64) -> AddressSpaceHandle {
    let root =
        memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap()).unwrap();
    memory::register_domain_authority(root, principal, 0).unwrap();
    root
}
fn used(principal: u64) -> [u64; 2] {
    HISTORY.lock().get(&principal).unwrap().thread_table.used()
}
fn limit(principal: u64, amount: [u64; 2]) {
    let mut history = HISTORY.lock();
    let account = &mut history.get_mut(&principal).unwrap().thread_table;
    assert_eq!(account.used(), [0, 0]);
    *account = VectorBudget::new(amount);
}
extern "C" fn unused() {}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let principal = charlotte_launch::artifact_principal_id(b"master-table-sponsor");
    let other = charlotte_launch::artifact_principal_id(b"master-table-other");
    let first = domain(principal);
    let sibling = domain(principal);
    let healthy = domain(other);
    let baseline = budget::thread_table::snapshot();
    let mut table = ThreadTable::new();
    let request = table.growth_request().unwrap().unwrap();
    let amount = request.amount().unwrap();
    limit(principal, amount);
    let charge = budget::reserve_thread_table(first, amount).unwrap();
    let mut growth = request.prepare(charge).unwrap();
    assert_eq!(used(principal), amount);
    for _ in 0..64 {
        assert!(matches!(budget::reserve_thread_table(sibling, amount), Err(budget::Error::Limit)));
    }
    // Promotion never reclassifies the candidate or bypasses principal bounds.
    let charged = budget::thread_table::snapshot();
    budget::mark_platform(sibling);
    assert_eq!(budget::thread_table::snapshot(), charged);
    assert!(budget::reserve_thread_table(sibling, amount).is_err());
    let thread = Thread::try_new(first.id(), unused).unwrap();
    let generation = thread.generation;
    let tid = {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let _master = MASTER_THREAD_TABLE.write();
        let _history = HISTORY.lock();
        table.try_insert_prepared(thread, Some(&mut growth)).unwrap()
    };
    growth.finish();
    let thread = table.take_element(tid).unwrap();
    assert_eq!(thread.generation, generation);
    thread.release_unstarted().unwrap();
    // Retained capacity outlives all payloads and the creating generation.
    assert_eq!(used(principal), amount);
    memory::close_user_address_space_handle(first).unwrap();
    let replacement = domain(other);
    assert_eq!(replacement.id(), first.id());
    assert_ne!(replacement, first);
    assert!(matches!(budget::reserve_thread_table(first, amount), Err(budget::Error::StaleDomain)));
    let other_request = ThreadTable::new().growth_request().unwrap().unwrap();
    let other_growth =
        other_request.prepare(budget::reserve_thread_table(replacement, amount).unwrap()).unwrap();
    let other_used = used(other);
    drop(table); // backing deallocation refunds the original principal only
    assert_eq!(used(principal), [0, 0]);
    assert_eq!(used(other), other_used);
    other_growth.finish();
    assert_eq!(used(other), [0, 0]);
    // Independently reject bytes and slots before any backing allocation.
    for ceiling in [[amount[0] - 1, amount[1]], [amount[0], amount[1] - 1]] {
        limit(principal, ceiling);
        assert!(budget::reserve_thread_table(sibling, amount).is_err());
        assert_eq!(used(principal), [0, 0]);
    }
    assert_eq!(budget::thread_table::snapshot(), baseline);
    memory::close_user_address_space_handle(sibling).unwrap();
    memory::close_user_address_space_handle(healthy).unwrap();
    memory::close_user_address_space_handle(replacement).unwrap();
    budget::thread_table::test_node_pressure();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[thread table admission] real backing and guarded publication, shared principal \
         rejection, sticky promotion, independent bytes/slots, retired root and ASID reuse passed"
    );
}
