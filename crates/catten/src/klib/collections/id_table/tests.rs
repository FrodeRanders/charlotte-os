use alloc::sync::Arc;

use super::*;

struct Tracked(Arc<AtomicUsize>);
impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
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
