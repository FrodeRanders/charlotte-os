use alloc::sync::Arc;

use super::*;

struct Tracked(Arc<AtomicUsize>);
impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn staged_close_fences_new_leases_but_allows_existing_completions() {
    let mut table = IdTable::new();
    let id = table.add_element(1);
    let generation = table.generation(id).unwrap();
    let lease = table.lease(id, generation).unwrap();
    let close = table.begin_close(id, generation).unwrap();
    assert_eq!(table.is_closing(id), Ok(true));
    assert!(matches!(table.lease(id, generation), Err(Error::Closing)));
    assert!(matches!(table.begin_close(id, generation), Err(Error::Closing)));
    assert_eq!(table.prepare_retirement(id), Err(Error::Closing));
    assert_eq!(table.take_element(id), Err(Error::Closing));
    assert_eq!(table.prepare_closing_retirement(&close), Err(Error::Leased));
    table.finish_lease(lease).unwrap();
    table.prepare_closing_retirement(&close).unwrap();
    let capacity = table.available_ids.capacity();
    let retired = table.retire_closing(close).unwrap();
    table.finish_retirement(retired.release_value()).unwrap();
    assert_eq!(table.available_ids.capacity(), capacity);
    assert_eq!(table.add_element(2), id);
    assert_eq!(table.is_closing(id), Ok(false));
    assert_ne!(table.generation(id).unwrap(), generation);
}

#[test]
fn staged_close_preparation_failure_does_not_publish_the_fence() {
    let mut table = IdTable::new();
    let id = table.add_element(1);
    let generation = table.generation(id).unwrap();
    let lease = table.lease(id, generation).unwrap();
    assert!(matches!(
        table.begin_close_with(id, generation, |_, _| Err(())),
        Err(Error::AllocationFailed)
    ));
    assert_eq!(table.is_closing(id), Ok(false));
    assert_eq!(table.slots[id].leases, 1);
    let second = table.lease(id, generation).unwrap();
    table.finish_lease(lease).unwrap();
    table.finish_lease(second).unwrap();
    assert_eq!(table.take_element(id), Ok(1));
}

#[test]
fn staged_close_identity_cannot_retire_another_table_or_generation() {
    let mut first = IdTable::new();
    let mut second = IdTable::new();
    let a = first.add_element(1);
    let b = second.add_element(2);
    let close = first.begin_close(a, first.generation(a).unwrap()).unwrap();
    let own = second.begin_close(b, second.generation(b).unwrap()).unwrap();
    assert_eq!(second.prepare_closing_retirement(&close), Err(Error::WrongRetirement));
    assert!(matches!(second.retire_closing(close), Err(Error::WrongRetirement)));
    assert_eq!(first.is_closing(a), Ok(true));
    let mut own = own;
    own.generation += 1; // Private corruption fixture, never a recovery API.
    assert_eq!(second.prepare_closing_retirement(&own), Err(Error::WrongRetirement));
    assert!(matches!(second.retire_closing(own), Err(Error::WrongRetirement)));
    assert_eq!(second.get(b), Ok(&2));
    assert_eq!(second.is_closing(b), Ok(true));
}

#[test]
fn dropped_staged_close_retains_payload_even_without_remaining_leases() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut table = IdTable::new();
    let id = table.add_element(Tracked(drops.clone()));
    drop(table.begin_close(id, table.generation(id).unwrap()).unwrap());
    assert_eq!(table.is_closing(id), Ok(true));
    assert!(matches!(table.take_element(id), Err(Error::Closing)));
    drop(table);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
}

#[test]
fn staged_close_refreshes_completion_capacity_after_growth() {
    let mut table = IdTable::new();
    let id = table.add_element(1);
    let close = table.begin_close(id, table.generation(id).unwrap()).unwrap();
    for value in 2..128 {
        table.add_element(value);
    }
    assert!(table.available_ids.capacity() < table.list.len());
    table.prepare_closing_retirement(&close).unwrap();
    let capacity = table.available_ids.capacity();
    let retired = table.retire_closing(close).unwrap();
    table.finish_retirement(retired.release_value()).unwrap();
    assert_eq!(table.available_ids.capacity(), capacity);
}

#[test]
fn staged_detachment_never_allocates_on_unprepared_capacity() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut table = IdTable::new();
    let id = table.add_element(Tracked(drops.clone()));
    let close = table.begin_close(id, table.generation(id).unwrap()).unwrap();
    table.available_ids = Vec::new(); // Completion-invariant corruption fixture.
    assert!(matches!(table.retire_closing(close), Err(Error::AllocationFailed)));
    assert_eq!(table.is_closing(id), Ok(true));
    assert_eq!(drops.load(Ordering::Relaxed), 0);
}

#[test]
fn live_leases_block_all_extraction_until_the_last_completion() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut table = IdTable::new();
    let id = table.add_element(Tracked(drops.clone()));
    let generation = table.generation(id).unwrap();
    let capacity = table.slots.capacity();
    let first = table.lease(id, generation).unwrap();
    let second = table.lease(id, generation).unwrap();
    assert_eq!(table.slots.capacity(), capacity);
    assert_eq!(
        table.prepare_retirement_with(id, |_, _| panic!("leased preflight allocated")),
        Err(Error::Leased)
    );
    assert!(matches!(table.take_element(id), Err(Error::Leased)));
    assert!(matches!(table.retire_element(id), Err(Error::Leased)));
    assert_eq!(table.generation(id), Ok(generation));
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    table.finish_lease(first).unwrap();
    assert_eq!(table.prepare_retirement(id), Err(Error::Leased));
    table.finish_lease(second).unwrap();
    let retired = table.retire_element(id).unwrap();
    table.finish_retirement(retired.release_value()).unwrap();
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(table.add_element(Tracked(drops.clone())), id);
    assert_ne!(table.generation(id).unwrap(), generation);
    assert!(matches!(table.lease(id, generation), Err(Error::WrongLease)));
}

#[test]
fn wrong_table_or_generation_completion_does_not_release_another_lease() {
    let mut first = IdTable::new();
    let mut second = IdTable::new();
    let a = first.add_element(1);
    let b = second.add_element(2);
    let lease = first.lease(a, first.generation(a).unwrap()).unwrap();
    let valid = second.lease(b, second.generation(b).unwrap()).unwrap();
    assert_eq!(second.finish_lease(lease), Err(Error::WrongLease));
    assert_eq!(first.slots[a].leases, 1);
    assert_eq!(second.slots[b].leases, 1);
    let mut corrupt = second.lease(b, second.generation(b).unwrap()).unwrap();
    // Kernel-boundary corruption, not a production token constructor.
    corrupt.generation += 1;
    assert_eq!(second.finish_lease(corrupt), Err(Error::WrongLease));
    assert_eq!(second.slots[b].leases, 2);
    second.finish_lease(valid).unwrap();
    assert_eq!(second.slots[b].leases, 1);
    assert_eq!(second.prepare_retirement(b), Err(Error::Leased));
}

#[test]
fn abandoned_lease_and_table_destruction_retain_the_leased_payload() {
    let retained_drops = Arc::new(AtomicUsize::new(0));
    let normal_drops = Arc::new(AtomicUsize::new(0));
    let mut table = IdTable::new();
    let id = table.add_element(Tracked(retained_drops.clone()));
    table.add_element(Tracked(normal_drops.clone()));
    drop(table.lease(id, table.generation(id).unwrap()).unwrap());
    assert_eq!(table.slots[id].leases, 1);
    assert_eq!(table.prepare_retirement(id), Err(Error::Leased));
    assert_eq!(retained_drops.load(Ordering::Relaxed), 0);
    drop(table);
    assert_eq!(retained_drops.load(Ordering::Relaxed), 0);
    assert_eq!(normal_drops.load(Ordering::Relaxed), 1);
}

#[test]
fn lease_overflow_and_completion_underflow_fail_without_mutation() {
    let mut table = IdTable::new();
    let id = table.add_element(1);
    let generation = table.generation(id).unwrap();
    // Private counter corruption fixture; no unbounded token minting loop.
    table.slots[id].leases = usize::MAX;
    assert!(matches!(table.lease(id, generation), Err(Error::LeaseLimit)));
    assert_eq!(table.slots[id].leases, usize::MAX);
    let mut empty = IdTable::new();
    let id = empty.add_element(2);
    let forged = SlotLease {
        table: empty.identity,
        id,
        generation: empty.generation(id).unwrap(),
    };
    assert_eq!(empty.finish_lease(forged), Err(Error::WrongLease));
    assert_eq!(empty.slots[id].leases, 0);
    assert!(matches!(empty.lease(usize::MAX, 1), Err(Error::IdNotActive)));
}

#[test]
fn lease_identity_survives_table_vector_growth() {
    let mut table = IdTable::new();
    let id = table.add_element(1);
    let generation = table.generation(id).unwrap();
    let lease = table.lease(id, generation).unwrap();
    for value in 2..2048 {
        table.add_element(value);
    }
    assert_eq!(table.generation(id), Ok(generation));
    assert_eq!(table.prepare_retirement(id), Err(Error::Leased));
    table.finish_lease(lease).unwrap();
    assert_eq!(table.take_element(id), Ok(1));
    assert_eq!(table.add_element(3), id);
    assert_ne!(table.generation(id).unwrap(), generation);
}

#[test]
fn detachment_hides_but_does_not_recycle_or_destroy() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut table = IdTable::new();
    let id = table.add_element(Tracked(drops.clone()));
    let generation = table.generation(id).unwrap();
    let retired = table.retire_element(id).unwrap();
    assert_eq!(retired.value().0.load(Ordering::Relaxed), 0);
    assert!(table.get(id).is_err());
    assert!(table.get_mut(id).is_err());
    assert!(table.take_element(id).is_err());
    assert!(table.generation(id).is_err());
    let another = table.add_element(Tracked(drops.clone()));
    assert_ne!(another, id);
    let slot = retired.release_value();
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert!(!table.available_ids.contains(&id));
    let capacity = table.available_ids.capacity();
    table.finish_retirement(slot).unwrap();
    assert_eq!(table.available_ids.capacity(), capacity);
    assert_eq!(table.add_element(Tracked(drops.clone())), id);
    assert_ne!(table.generation(id).unwrap(), generation);
    drop(table);
    assert_eq!(drops.load(Ordering::Relaxed), 3);
}

#[test]
fn abandoned_receipt_retains_resource_and_slot() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut table = IdTable::new();
    let id = table.add_element(Tracked(drops.clone()));
    drop(table.retire_element(id).unwrap());
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    assert!(table.slots[id].retiring);
    assert_ne!(table.add_element(Tracked(drops.clone())), id);
    drop(table);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn table_identity_rejects_equal_id_and_generation() {
    let mut first = IdTable::new();
    let mut second = IdTable::new();
    let first_id = first.add_element(1);
    let second_id = second.add_element(2);
    assert_eq!(first_id, second_id);
    assert_eq!(first.generation(first_id), second.generation(second_id));
    let first_retired = first.retire_element(first_id).unwrap();
    let second_retired = second.retire_element(second_id).unwrap();
    assert_eq!(
        second.finish_retirement(first_retired.release_value()),
        Err(Error::WrongRetirement)
    );
    assert!(first.slots[first_id].retiring);
    assert!(second.slots[second_id].retiring);
    second.finish_retirement(second_retired.release_value()).unwrap();
    assert_eq!(second.add_element(3), second_id);
    assert_ne!(first.add_element(4), first_id);
}

#[test]
fn wrong_generation_completion_cannot_release_slot() {
    let mut table = IdTable::new();
    let id = table.add_element(1);
    let mut slot = table.retire_element(id).unwrap().release_value();
    // Kernel-boundary corruption fixture, not a production token constructor.
    slot.generation += 1;
    assert_eq!(table.finish_retirement(slot), Err(Error::WrongRetirement));
    assert!(table.slots[id].retiring);
    assert_ne!(table.add_element(2), id);
}

#[test]
fn failed_preflight_keeps_active_value_and_generation() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut table = IdTable::new();
    let id = table.add_element(Tracked(drops.clone()));
    let generation = table.generation(id).unwrap();
    assert_eq!(table.prepare_retirement_with(id, |_, _| Err(())), Err(Error::AllocationFailed));
    assert!(table.get(id).is_ok());
    assert_eq!(table.generation(id), Ok(generation));
    assert!(!table.slots[id].retiring);
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    drop(table.take_element(id).unwrap());
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn interleaved_detached_slots_complete_without_allocating() {
    let mut table = IdTable::new();
    for value in 0..16 {
        table.add_element(value);
    }
    let mut receipts = Vec::new();
    for id in 0..16 {
        receipts.push(table.retire_element(id).unwrap());
    }
    let added = table.add_element(16);
    let added_receipt = table.retire_element(added).unwrap();
    let capacity = table.available_ids.capacity();
    for receipt in receipts {
        table.finish_retirement(receipt.release_value()).unwrap();
        assert_eq!(table.available_ids.capacity(), capacity);
    }
    table.finish_retirement(added_receipt.release_value()).unwrap();
    assert_eq!(table.available_ids.capacity(), capacity);
    assert_eq!(table.available_ids.len(), 17);
}

#[test]
fn completion_never_allocates_when_preflight_invariant_is_corrupted() {
    let mut table = IdTable::new();
    let id = table.add_element(1);
    let slot = table.retire_element(id).unwrap().release_value();
    // Private-state corruption fixture: production cannot discard the
    // preflighted completion storage while an owner is detached.
    table.available_ids = Vec::new();
    assert_eq!(table.finish_retirement(slot), Err(Error::AllocationFailed));
    assert!(table.slots[id].retiring);
    assert_ne!(table.add_element(2), id);
}
