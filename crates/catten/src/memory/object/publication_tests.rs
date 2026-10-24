//! Serialized ABI fixtures hold the heap through complete mixed publication,
//! including namespace, payload, object and borrower-node relinking.
use super::*;
use crate::{
    capability::{
        self,
        ObjectKind,
    },
    memory::{
        self,
        AddressSpaceHandle,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn prepare_mode(
    source: AddressSpaceId,
    cap: MemoryObjectCap,
    target: AddressSpaceId,
    mode: usize,
) -> PreparedTransfer {
    match mode % 3 {
        0 => prepare_move(source, cap, target).unwrap(),
        1 => prepare_copy(source, cap, target).unwrap(),
        _ => prepare_loan(source, cap, target, false).unwrap(),
    }
}

pub(super) fn run() {
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        commit_transfers_with_authority(&mut [], &mut []).unwrap();
    }
    maximum();
    for extra in [false, true] {
        rejected_tail(extra);
    }
    crate::logln!(
        "[IPC borrowed publication] empty and 257-member mixed authority batches with heap held; \
         late transfer/extra namespace rejection preserved every destination/source, cancellation \
         restored baselines; complete mixed commit including payloads needs no heap"
    );
}

fn maximum() {
    let source = root();
    let target = root();
    let baseline = capability::node_admission_used();
    let count = catten_syscall::CAP_VECTOR_MAX;
    let mut sources = Vec::new();
    let mut transfers = Vec::new();
    for index in 0..count {
        let cap = allocate(source.id(), 1).unwrap();
        sources.push(cap);
        transfers.push(prepare_mode(source.id(), cap, target.id(), index));
    }
    let outputs: Vec<_> = transfers.iter().map(PreparedTransfer::target_cap).collect();
    // Raw authority fixtures have no IPC payload and are explicitly removed.
    let mut extras = [0, 1].map(|_| capability::reserve(target.id(), ObjectKind::Ipc).unwrap());
    let ids = extras.each_ref().map(capability::Reservation::identity);
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let [a, b] = &mut extras;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        commit_transfers_with_delivery(&mut transfers, &mut [a, b], true).unwrap();
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    for &id in &ids {
        assert!(capability::contains(target.id(), id, ObjectKind::Ipc));
    }
    for (index, &cap) in outputs.iter().enumerate() {
        assert!(capability::contains(target.id(), cap, ObjectKind::Memory));
        assert!(transfers[index].committed);
        if index % 3 != 2 {
            assert!(info(target.id(), cap).is_err(), "undelivered owning memory became visible");
        }
    }
    drop(transfers);
    deliver_memory(target.id(), &outputs);
    for (index, (&cap, &output)) in sources.iter().zip(&outputs).enumerate() {
        assert!(info(target.id(), output).is_ok());
        match index % 3 {
            0 => {
                assert!(info(source.id(), cap).is_err());
                close_cap(target.id(), output).unwrap();
            }
            1 => {
                close_cap(target.id(), output).unwrap();
                close_cap(source.id(), cap).unwrap();
            }
            _ => {
                assert!(info(source.id(), cap).unwrap().lent);
                revoke_lend(source.id(), cap, target.id(), output).unwrap();
                close_cap(source.id(), cap).unwrap();
            }
        }
    }
    for id in ids {
        assert!(capability::remove(target.id(), id, ObjectKind::Ipc));
    }
    drop(extras);
    assert_eq!(capability::node_admission_used(), baseline);
    assert_eq!(memory::budget::used(source), memory::budget::Amount::default());
    assert_eq!(memory::budget::used(target), memory::budget::Amount::default());
    memory::close_user_address_space_handle(target).unwrap();
    memory::close_user_address_space_handle(source).unwrap();
}

fn rejected_tail(retire_extra: bool) {
    let source = root();
    let target = root();
    let late = root();
    let baseline = capability::node_admission_used();
    let sources: [_; 3] = core::array::from_fn(|_| allocate(source.id(), 1).unwrap());
    let mut transfers: [_; 3] = core::array::from_fn(|index| {
        prepare_mode(
            source.id(),
            sources[index],
            if !retire_extra && index == 2 {
                late.id()
            } else {
                target.id()
            },
            index,
        )
    });
    let outputs = transfers.each_ref().map(PreparedTransfer::target_cap);
    let mut extras = [
        target.id(),
        if retire_extra {
            late.id()
        } else {
            target.id()
        },
    ]
    .map(|id| capability::reserve(id, ObjectKind::Ipc).unwrap());
    let ids = extras.each_ref().map(capability::Reservation::identity);
    let admitted = capability::node_admission_used();
    capability::retire_address_space(late.id());
    let [a, b] = &mut extras;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(
            commit_transfers_with_delivery(&mut transfers, &mut [a, b], true),
            Err(MemoryObjectError::AddressSpaceMissing)
        );
    }
    assert_eq!(capability::node_admission_used(), admitted);
    for (index, transfer) in transfers.iter().enumerate() {
        assert!(!transfer.committed);
        assert!(!capability::contains(transfer.target(), outputs[index], ObjectKind::Memory));
    }
    for (&id, owner) in ids.iter().zip([
        target.id(),
        if retire_extra {
            late.id()
        } else {
            target.id()
        },
    ]) {
        assert!(!capability::contains(owner, id, ObjectKind::Ipc));
    }
    drop(transfers);
    drop(extras);
    for cap in sources {
        assert!(info(source.id(), cap).is_ok());
        assert!(!info(source.id(), cap).unwrap().lent);
        close_cap(source.id(), cap).unwrap();
    }
    assert_eq!(capability::node_admission_used(), baseline);
    assert_eq!(memory::budget::used(source), memory::budget::Amount::default());
    memory::close_user_address_space_handle(late).unwrap();
    memory::close_user_address_space_handle(target).unwrap();
    memory::close_user_address_space_handle(source).unwrap();
}
