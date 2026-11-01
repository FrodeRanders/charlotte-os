//! Real unpublished roots supply signed sponsorship before ASID publication.
use super::*;
use crate::memory::{
    self,
    AddressSpace,
    AddressSpaceHandle,
    AddressSpaceTable,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
    registration::table_backing,
};

fn root(principal: u64) -> AddressSpace {
    AddressSpace::try_new_principal_user(
        false,
        memory::prepare_principal_backing(principal, false).unwrap(),
    )
    .unwrap()
}
fn used(principal: u64) -> [u64; 2] {
    HISTORY.lock().get(&principal).unwrap().root_table.used()
}
fn limit(principal: u64, amount: [u64; 2]) {
    let mut history = HISTORY.lock();
    let budget = &mut history.get_mut(&principal).unwrap().root_table;
    assert_eq!(budget.used(), [0, 0]);
    *budget = VectorBudget::new(amount);
}
fn warm_slot() {
    let warm = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    memory::close_user_address_space_handle(warm).unwrap();
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let principal = charlotte_launch::artifact_principal_id(b"root-table-sponsor");
    let other = charlotte_launch::artifact_principal_id(b"root-table-other");
    let mut first = root(principal);
    let sibling = root(principal);
    let healthy = root(other);
    let mut table = AddressSpaceTable::new();
    let request = table.growth_request().unwrap().unwrap();
    let amount = request.amount().unwrap();
    limit(principal, amount);
    let baseline = table_backing::snapshot();
    let mut growth = table_backing::prepare_request(&first, request).unwrap();
    for _ in 0..64 {
        assert!(
            table_backing::prepare_request(&sibling, table.growth_request().unwrap().unwrap())
                .is_err()
        );
    }
    assert_eq!(used(principal), amount);
    first.table_account.mark_platform();
    assert_eq!(used(principal), amount);
    let charged = table_backing::snapshot();
    let mut owner = Some(first);
    let id = {
        let _lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let _roots = memory::ADDRESS_SPACE_TABLE.lock();
        let _history = HISTORY.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        table.try_insert_from(&mut owner, Some(&mut growth)).unwrap()
    };
    assert!(owner.is_none());
    growth.finish();
    let lease = table.lease(id, 1).unwrap();
    assert!(matches!(
        table.take_element(id),
        Err(crate::klib::collections::id_table::Error::Leased)
    ));
    table.finish_lease(lease).unwrap();
    drop(table.take_element(id).unwrap());
    assert_eq!(table_backing::snapshot(), charged);
    drop(table);
    assert_eq!(used(principal), [0, 0]);
    for ceiling in [[amount[0] - 1, amount[1]], [amount[0], amount[1] - 1]] {
        limit(principal, ceiling);
        let empty = AddressSpaceTable::new();
        assert!(
            table_backing::prepare_request(&sibling, empty.growth_request().unwrap().unwrap())
                .is_err()
        );
        assert_eq!(used(principal), [0, 0]);
    }
    assert_eq!(table_backing::snapshot(), baseline);
    drop((sibling, healthy));
    late_refund();
    abandonment();
    table_backing::test_node_pressure();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[root table admission] real signed backing, 64 pre-allocation denials, guarded scalar \
         publication, exact lease, sticky promotion, independent bytes/slots and late refund \
         after ASID reuse passed"
    );
}

fn late_refund() {
    warm_slot(); // Both publications below reuse an existing slot without growth.
    let principal = charlotte_launch::artifact_principal_id(b"root-table-late");
    let other = charlotte_launch::artifact_principal_id(b"root-table-successor");
    let first = root(principal);
    let table = AddressSpaceTable::new();
    let request = table.growth_request().unwrap().unwrap();
    let amount = request.amount().unwrap();
    let growth = table_backing::prepare_request(&first, request).unwrap();
    let old: AddressSpaceHandle = memory::register_user_address_space(first).unwrap();
    assert_eq!(used(principal), amount);
    memory::close_user_address_space_handle(old).unwrap();
    assert_eq!(used(principal), amount);
    let successor = memory::register_user_address_space(root(other)).unwrap();
    assert_eq!(old.id(), successor.id());
    assert_ne!(old, successor);
    growth.finish();
    assert_eq!(used(principal), [0, 0]);
    assert_eq!(used(other), [0, 0]);
    memory::close_user_address_space_handle(successor).unwrap();
}

fn abandonment() {
    let principal = charlotte_launch::artifact_principal_id(b"root-table-abandoned");
    let root = root(principal);
    let table = AddressSpaceTable::new();
    let request = table.growth_request().unwrap().unwrap();
    let amount = request.amount().unwrap();
    let growth = table_backing::prepare_request(&root, request).unwrap();
    let charged = table_backing::snapshot();
    {
        let _lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let _roots = memory::ADDRESS_SPACE_TABLE.lock();
        let _history = HISTORY.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        table_backing::with_locked(|| drop(growth));
    }
    drop(root);
    assert_eq!(used(principal), amount);
    assert_eq!(table_backing::snapshot(), charged);
    crate::logln!(
        "[root table retention] four-slot candidate and original signed charge retained under \
         lifecycle/root/history/heap/pool guards after original root disposal; no cleanup/retry \
         authority"
    );
}
