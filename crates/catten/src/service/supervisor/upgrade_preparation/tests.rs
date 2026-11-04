//! Native handoff ownership controls.
use super::*;
use crate::{
    klib::admission::{
        self,
        Family,
        Reservation,
    },
    memory::{
        self,
        PHYSICAL_FRAME_ALLOCATOR,
        object,
    },
    service::{
        loader,
        supervisor::launch_transaction::tests::{
            dummy,
            guarded,
        },
    },
};
fn used() -> ([u64; 2], [u64; 2]) {
    admission::used::<IpcNodes, 2>()
}
pub(in crate::service::supervisor) fn run() {
    for platform in [false, true] {
        for prefix in [false, true] {
            let fixture_before = used();
            let source = loader::create_user_address_space_handle();
            let target = loader::create_user_address_space_handle();
            let cap = object::allocate(source.id(), 1).unwrap();
            let mut domain = DomainLaunchTransaction::new(dummy(target)).unwrap();
            domain.retain_source(source).unwrap();
            let mut owner = PreparingUpgrade::new(domain, platform);
            if prefix {
                owner.moves.try_reserve_exact(1).unwrap();
            }
            let limit = if platform {
                IpcNodes::TOTAL[0]
            } else {
                IpcNodes::ORDINARY[0]
            };
            let current = if platform {
                used().0[0]
            } else {
                used().1[0]
            };
            let pressure =
                Reservation::<IpcNodes, 2>::reserve(platform, [limit - current, 0]).unwrap();
            {
                let _heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
                assert!(owner.prepare_storage(1).is_err());
            }
            assert_eq!(owner.moves.capacity(), usize::from(prefix));
            assert_eq!(owner.targets.capacity(), 0);
            drop(pressure);
            owner.prepare_storage(2).unwrap();
            owner.moves.push(object::prepare_move(source.id(), cap, target.id()).unwrap());
            assert!(object::prepare_move(source.id(), u64::MAX, target.id()).is_err());
            assert!(owner.abort::<()>(UpgradeSpawnError::Allocation).is_err());
            assert!(memory::current_address_space_handle(target.id()).is_none());
            object::write_bytes(source.id(), cap, &[7]).unwrap();
            memory::close_user_address_space_handle(source).unwrap();
            assert_eq!(used(), fixture_before);
        }
    }
    for hold_guards in [false, true] {
        let source = loader::create_user_address_space_handle();
        let target = loader::create_user_address_space_handle();
        let cap = object::allocate(source.id(), 1).unwrap();
        let mut domain = DomainLaunchTransaction::new(dummy(target)).unwrap();
        domain.retain_source(source).unwrap();
        let mut owner = PreparingUpgrade::new(domain, false);
        owner.prepare_storage(1).unwrap();
        let transfer = object::prepare_move(source.id(), cap, target.id()).unwrap();
        owner.targets.push(transfer.target_cap());
        owner.moves.push(transfer);
        memory::budget::mark_platform(source);
        let before = used();
        let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        if hold_guards {
            guarded(|| drop(owner));
        } else {
            drop(owner);
        }
        assert_eq!(used(), before);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
        assert_eq!(
            object::write_bytes(source.id(), cap, &[8]),
            Err(object::MemoryObjectError::UnknownCapability)
        );
        assert_eq!(
            object::try_close_cap(source.id(), cap),
            Err(object::MemoryObjectError::LendingActive)
        );
        for root in [target, source] {
            assert_eq!(
                memory::close_user_address_space_handle(root),
                Err(memory::AddressSpaceCloseError::OperationsInFlight)
            );
        }
    }
    // Maximum arrays exercise actual layouts without 255 physical transfers.
    let target = loader::create_user_address_space_handle();
    let owner = DomainLaunchTransaction::new(dummy(target)).unwrap();
    let mut owner = PreparingUpgrade::new(owner, true);
    let baseline = used();
    let count = charlotte_launch::CAPABILITY_VECTOR_CAPACITY - 1;
    owner.prepare_storage(count).unwrap();
    let bytes = count * (core::mem::size_of::<PreparedTransfer>() + core::mem::size_of::<u64>());
    assert_eq!(used().0, [baseline.0[0] + bytes as u64, baseline.0[1]]);
    assert_eq!(used().1, baseline.1);
    assert!(owner.abort::<()>(UpgradeSpawnError::Allocation).is_err());
    assert_eq!(used(), baseline);
    crate::logln!(
        "[upgrade preparation] full arrays max={} entry={} bytes={}; original ordinary/trusted \
         class, pre-transfer quota rejection, prefix cancellation and complete guarded \
         abandonment passed",
        count,
        core::mem::size_of::<PreparedTransfer>(),
        bytes
    );
}
