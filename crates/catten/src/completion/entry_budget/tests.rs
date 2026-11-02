//! Serialized real backing pressure; physical CQ mappings and sustained EL0
//! fairness are separate qualification. No live account is replaced or invented.
use super::*;
use crate::{
    completion::{
        self,
        COMPLETIONS,
        CapabilityRecord,
        CompletionCap,
        CqId,
        CqOpenError,
        CqState,
        DEFAULT_CQ,
        DetachedOp,
        OpCode,
        OpResult,
        OperationId,
        RetiredCq,
        SubmitError,
    },
    memory::{
        self,
        AddressSpace,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

enum Candidate {
    Capability(Prepared<(CompletionCap, CapabilityRecord)>),
    Detached(Prepared<(OperationId, DetachedOp)>),
    Queue(Prepared<(CqId, CqState)>),
}
impl Candidate {
    fn new(kind: usize, platform: bool) -> Result<Self, AllocError> {
        match kind % 3 {
            0 => prepare(platform).map(Self::Capability),
            1 => prepare(platform).map(Self::Detached),
            _ => prepare(platform).map(Self::Queue),
        }
    }

    fn bytes(&self) -> u64 {
        // Borrow the actual candidate as well as checking its concrete layout.
        match self {
            Self::Capability(node) => {
                core::hint::black_box(node);
                Prepared::<(CompletionCap, CapabilityRecord)>::allocation_bytes() as u64
            }
            Self::Detached(node) => {
                core::hint::black_box(node);
                Prepared::<(OperationId, DetachedOp)>::allocation_bytes() as u64
            }
            Self::Queue(node) => {
                core::hint::black_box(node);
                Prepared::<(CqId, CqState)>::allocation_bytes() as u64
            }
        }
    }
}

fn allocation_rejection<T>() {
    let before = snapshot();
    for _ in 0..64 {
        assert!(prepare_with::<T>(false, |_| Err(AllocError)).is_err());
        assert_eq!(snapshot(), before);
    }
}

fn reject_work(asid: usize) {
    let before = snapshot();
    let records = completion::record_admission(asid).unwrap();
    let queues = completion::cq_admission(asid).unwrap();
    let record_used = records.used();
    let queue_used = queues.used();
    let authority = crate::capability::node_admission_used();
    let source = completion::capture_cq_interest(asid, DEFAULT_CQ).unwrap();
    for _ in 0..64 {
        assert!(matches!(
            completion::submit(asid, OpCode::Nop, None),
            Err(SubmitError::WouldBlock)
        ));
        assert!(matches!(
            completion::submit_detached(asid, DEFAULT_CQ, OpCode::Nop, 0),
            Err(SubmitError::WouldBlock)
        ));
        assert_eq!(completion::open_cq(asid, DEFAULT_CQ, 8), Err(CqOpenError::AllocationFailed));
    }
    let current = completion::capture_cq_interest(asid, DEFAULT_CQ).unwrap();
    assert!(core::ptr::eq(&**source.waiter_source(), &**current.waiter_source()));
    assert_eq!(snapshot(), before);
    assert_eq!(records.used(), record_used);
    assert_eq!(queues.used(), queue_used);
    assert_eq!(crate::capability::node_admission_used(), authority);
}

fn pressure() {
    let baseline = snapshot();
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    let trusted =
        memory::register_user_address_space(AddressSpace::try_new_platform_user().unwrap())
            .unwrap();
    memory::budget::mark_platform(trusted);
    for owner in [root, trusted] {
        completion::open_address_space_with_cq(owner.id(), 8, 8).unwrap();
    }
    let cap = completion::submit(root.id(), OpCode::Read, Some(alloc::vec![7])).unwrap();
    completion::complete(root.id(), cap, OpResult::Ok(1)).unwrap();
    let completion = completion::completion_of(root.id(), cap).unwrap();
    let weak = alloc::sync::Arc::downgrade(&completion);
    drop(completion);
    let mut ordinary = alloc::vec::Vec::with_capacity(ORDINARY[1] as usize);
    while let Ok(candidate) = Candidate::new(ordinary.len(), false) {
        ordinary.push(candidate);
        assert!(ordinary.len() <= ORDINARY[1] as usize);
    }
    let full_ordinary = snapshot();
    assert_eq!(full_ordinary.1[1], ORDINARY[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            for kind in 0..3 {
                assert!(Candidate::new(kind, false).is_err());
            }
        }
    }
    reject_work(root.id());
    // Real trusted adapters still progress while ordinary backing is full.
    let pending = completion::submit(trusted.id(), OpCode::Nop, None).unwrap();
    completion::complete(trusted.id(), pending, OpResult::Ok(2)).unwrap();
    completion::close(trusted.id(), pending).unwrap();
    let detached = completion::submit_detached(trusted.id(), DEFAULT_CQ, OpCode::Nop, 0).unwrap();
    completion::complete_detached(trusted.id(), detached, OpResult::Ok(3)).unwrap();
    completion::open_cq(trusted.id(), DEFAULT_CQ, 8).unwrap();
    assert_eq!(snapshot(), full_ordinary);
    let mut reserved = alloc::vec::Vec::with_capacity((LIMIT[1] - ORDINARY[1]) as usize);
    while let Ok(candidate) = Candidate::new(reserved.len(), true) {
        reserved.push(candidate);
        assert!(reserved.len() <= (LIMIT[1] - ORDINARY[1]) as usize);
    }
    let full = snapshot();
    assert_eq!(full.0[1], LIMIT[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            for kind in 0..3 {
                assert!(Candidate::new(kind, true).is_err());
            }
        }
    }
    reject_work(root.id());
    reject_work(trusted.id());
    // Promotion cannot reclassify an existing ordinary node's eventual refund.
    memory::budget::mark_platform(root);
    let node_bytes = Prepared::<(CompletionCap, CapabilityRecord)>::allocation_bytes() as u64;
    completion::close(root.id(), cap).unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(
        snapshot(),
        ([full.0[0] - node_bytes, full.0[1] - 1], [full.1[0] - node_bytes, full.1[1] - 1])
    );
    // The record's weak-only backing remains charged independently of its node.
    assert_eq!(completion::record_admission(root.id()).unwrap().used(), 1);
    let next = completion::submit(trusted.id(), OpCode::Nop, None).unwrap();
    assert_eq!(snapshot().0, full.0);
    completion::complete(trusted.id(), next, OpResult::Ok(5)).unwrap();
    completion::close(trusted.id(), next).unwrap();
    drop(weak);
    let pressure_counts = (ordinary.len(), reserved.len());
    let candidate_bytes: u64 = ordinary.iter().chain(reserved.iter()).map(Candidate::bytes).sum();
    assert!(candidate_bytes < LIMIT[0]);
    drop((ordinary, reserved));
    for owner in [root, trusted] {
        memory::close_user_address_space_handle(owner).unwrap();
    }
    assert_eq!(snapshot(), baseline);
    crate::logln!(
        "[completion node pressure] actual mixed candidates ordinary={} trusted={} at {}/{}; 384 \
         heap-held three-layout denials; all three production adapters reject/recover, trusted \
         reserve progresses, promoted original refund and weak-record independence passed",
        pressure_counts.0,
        pressure_counts.1,
        ORDINARY[1],
        LIMIT[1]
    );
}

fn lifetime() {
    let baseline = snapshot();
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    completion::open_address_space_with_cq(root.id(), 4, 8).unwrap();
    let cap = completion::submit(root.id(), OpCode::Read, Some(alloc::vec![8])).unwrap();
    let completion = completion::completion_of(root.id(), cap).unwrap();
    completion::complete(root.id(), cap, OpResult::Ok(4)).unwrap();
    let record = completion::retire_completed(root.id(), cap, &completion).unwrap();
    let queue = {
        let mut registry = COMPLETIONS.write();
        RetiredCq::from_node(registry.get_mut(&root.id()).unwrap().cqs.take(&DEFAULT_CQ).unwrap())
    };
    let retained = snapshot();
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(snapshot(), retained);
    let successor =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(successor.id(), root.id());
    assert_ne!(successor, root);
    completion::open_address_space_with_cq(successor.id(), 4, 8).unwrap();
    let next = completion::submit(successor.id(), OpCode::Nop, None).unwrap();
    let successor_charge = snapshot();
    record.finish();
    queue.finish();
    let bytes = Prepared::<(CompletionCap, CapabilityRecord)>::allocation_bytes()
        + Prepared::<(CqId, CqState)>::allocation_bytes();
    assert_eq!(
        snapshot(),
        (
            [successor_charge.0[0] - bytes as u64, successor_charge.0[1] - 2],
            [successor_charge.1[0] - bytes as u64, successor_charge.1[1] - 2]
        )
    );
    assert!(completion.holds_buffer());
    assert!(completion::poll(successor.id(), next).unwrap().is_none());
    drop(completion);
    memory::close_user_address_space_handle(successor).unwrap();
    assert_eq!(snapshot(), baseline);
}

pub(in crate::completion) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    allocation_rejection::<(CompletionCap, CapabilityRecord)>();
    allocation_rejection::<(OperationId, DetachedOp)>();
    allocation_rejection::<(CqId, CqState)>();
    let before = snapshot();
    assert!(
        prepare_with::<[u8; ORDINARY[0] as usize]>(false, |_| panic!(
            "ordinary bytes reached heap"
        ))
        .is_err()
    );
    assert!(
        prepare_with::<[u8; LIMIT[0] as usize]>(true, |_| panic!("total bytes reached heap"))
            .is_err()
    );
    assert_eq!(snapshot(), before);
    pressure();
    lifetime();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[completion node admission] layouts capability={} detached={} queue={}; 192 injected \
         allocation rejections, independent byte denial, root close/exact ASID reuse, retained \
         original nodes and exact post-deallocation refund passed",
        Prepared::<(CompletionCap, CapabilityRecord)>::allocation_bytes(),
        Prepared::<(OperationId, DetachedOp)>::allocation_bytes(),
        Prepared::<(CqId, CqState)>::allocation_bytes()
    );
}
