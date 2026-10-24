//! Root-retaining call/endpoint factory controls before AP schedulers start.
use super::*;
use crate::memory::{
    self,
    AddressSpaceHandle,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn used(root: AddressSpaceHandle) -> usize {
    crate::capability::admission_tests::test_namespace_used(root.id())
}
fn available(root: AddressSpaceHandle) {
    assert!(IPC.try_write().is_some());
    assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
    assert!(memory::ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
}
fn prepare(
    root: AddressSpaceHandle,
    before: impl FnMut(u8) -> Result<(), IpcError>,
) -> Result<PreparedCall, IpcError> {
    PreparedCall::prepare_checked_with(
        root.id(),
        None,
        || PendingStorage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit),
        || ReplyStorage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit),
        before,
    )
}

pub(super) fn run() {
    let handle = root();
    let records = IPC.read().caps[&handle.id()].record_budget.clone();
    let baseline = record_budget::node_used();
    for reject in 1..=5 {
        for _ in 0..64 {
            assert!(matches!(
                prepare(handle, |stage| {
                    available(handle);
                    assert_eq!(records.used(), [0, 1, 1]);
                    assert_eq!(used(handle), usize::from(stage >= 3));
                    if stage == reject {
                        Err(IpcError::ResourceLimit)
                    } else {
                        Ok(())
                    }
                }),
                Err(IpcError::ResourceLimit)
            ));
            assert_eq!(records.used(), [0, 0, 0]);
            assert_eq!(record_budget::node_used(), baseline);
            assert_eq!(used(handle), 0);
        }
    }
    for _ in 0..64 {
        crate::capability::record_tests::reject_next();
        assert!(matches!(prepare(handle, |_| Ok(())), Err(IpcError::ResourceLimit)));
        assert_eq!(records.used(), [0, 0, 0]);
        assert_eq!(record_budget::node_used(), baseline);
        assert_eq!(used(handle), 0);
    }
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let mut owner = prepare(handle, |stage| {
            available(handle);
            assert!((1..=5).contains(&stage));
            Ok(())
        })
        .unwrap();
        {
            let mut ipc = IPC.write();
            let _heap = PRIMARY_ALLOCATOR.lock();
            ipc.stage_call(handle.id(), &mut owner).unwrap();
        }
        owner.finish();
        assert_eq!(used(handle), 0);
        assert_eq!(records.used(), [0, 0, 0]);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    crate::capability::admission_tests::test_fill_remaining_namespace(handle.id());
    assert!(matches!(
        prepare(handle, |stage| {
            assert!(stage <= 2, "full shared namespace reached payload/list allocation");
            available(handle);
            Ok(())
        }),
        Err(IpcError::ResourceLimit)
    ));
    assert_eq!(records.used(), [0, 0, 0]);
    memory::close_user_address_space_handle(handle).unwrap();
    let handle = root();
    assert!(matches!(
        prepare(handle, |stage| {
            available(handle);
            if stage == 2 {
                crate::capability::retire_address_space(handle.id());
            }
            Ok(())
        }),
        Err(IpcError::PermissionDenied)
    ));
    assert_eq!(used(handle), 0);
    memory::close_user_address_space_handle(handle).unwrap();
    crate::logln!(
        "[IPC unlocked preparation] 320 call-stage and 64 authority-factory rejections; 64 \
         heap-held call revalidations, quota-before-payload/list rejection, retirement race and \
         exact-root/IRQ retention passed; publication vectors and other capability callers remain \
         separate"
    );
}
