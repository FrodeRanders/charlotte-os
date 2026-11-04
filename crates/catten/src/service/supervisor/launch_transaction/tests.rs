//! Native complete-parent controls.
use super::*;
use crate::{
    ipc,
    klib::admission::{
        self,
        IpcNodes,
        MemoryCapabilityNodes,
    },
    memory::{
        PHYSICAL_FRAME_ALLOCATOR,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
        object,
    },
};

pub(in crate::service::supervisor) fn dummy(handle: AddressSpaceHandle) -> loader::LoadedDomain {
    // This fixture never reads/writes bootstrap pages or starts a thread.
    loader::LoadedDomain {
        asid: handle.id(),
        address_space: handle,
        entry_vaddr: 0,
        config_frame: 0u64.into(),
        status_frame: 0u64.into(),
    }
}
pub(in crate::service::supervisor) fn guarded(work: impl FnOnce()) {
    let _lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
    let _roots = memory::ADDRESS_SPACE_TABLE.lock();
    ipc::with_registry_locked(|| {
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _physical = PHYSICAL_FRAME_ALLOCATOR.lock();
        object::retirement_tests::with_registry_locked(|| {
            crate::capability::admission_tests::test_with_registry_locked(|| {
                admission::with_locked::<IpcNodes, 2, _>(|| {
                    admission::with_locked::<MemoryCapabilityNodes, 2, _>(work)
                });
            });
        })
    });
}
pub(in crate::service::supervisor) fn run() {
    let stale = loader::create_user_address_space_handle();
    memory::close_user_address_space_handle(stale).unwrap();
    let fresh = loader::create_user_address_space_handle();
    assert_eq!(fresh.id(), stale.id());
    let target = loader::create_user_address_space_handle();
    let mut owner = DomainLaunchTransaction::new(dummy(target)).unwrap();
    assert!(owner.retain_source(stale).is_err());
    owner.cancel().unwrap();
    assert_eq!(memory::current_address_space_handle(fresh.id()), Some(fresh));
    memory::close_user_address_space_handle(fresh).unwrap();
    for _ in 0..16 {
        let target = loader::create_user_address_space_handle();
        let source = loader::create_user_address_space_handle();
        let mut owner = DomainLaunchTransaction::new(dummy(target)).unwrap();
        owner.retain_source(source).unwrap();
        owner.retain_source(source).unwrap();
        object::allocate(target.id(), 1).unwrap();
        owner.kernel_profile = Some(object::allocate(KERNEL_ASID, 1).unwrap());
        owner.cancel().unwrap();
        assert!(memory::current_address_space_handle(target.id()).is_none());
        memory::close_user_address_space_handle(source).unwrap();
    }
    for hold_guards in [false, true] {
        let target = loader::create_user_address_space_handle();
        let source = loader::create_user_address_space_handle();
        let mut owner = DomainLaunchTransaction::new(dummy(target)).unwrap();
        owner.retain_source(source).unwrap();
        object::allocate(target.id(), 1).unwrap();
        let profile = object::allocate(KERNEL_ASID, 1).unwrap();
        owner.kernel_profile = Some(profile);
        let before = admission::used::<MemoryCapabilityNodes, 2>();
        let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let irq = crate::cpu::isa::lp::ops::get_int_state();
        if hold_guards {
            guarded(|| drop(owner));
        } else {
            drop(owner);
        }
        assert_eq!(admission::used::<MemoryCapabilityNodes, 2>(), before);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
        assert!(object::info(KERNEL_ASID, profile).is_ok());
        for root in [target, source] {
            assert_eq!(
                memory::close_user_address_space_handle(root),
                Err(memory::AddressSpaceCloseError::OperationsInFlight)
            );
        }
        // Terminal owner abandonment: never reconstruct/cancel it by IDs.
    }
    let target = loader::create_user_address_space_handle();
    let mut owner = DomainLaunchTransaction::new(dummy(target)).unwrap();
    let profile = object::allocate(KERNEL_ASID, 1).unwrap();
    owner.kernel_profile = Some(profile);
    let pin = object::pin_for_copy(KERNEL_ASID, profile).unwrap();
    assert!(matches!(
        owner.cancel(),
        Err(ProfileLaunchError::RollbackMemory(object::MemoryObjectError::LendingActive))
    ));
    assert_eq!(
        memory::close_user_address_space_handle(target),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    object::unpin_copy(pin);
    assert!(object::info(KERNEL_ASID, profile).is_ok(), "failure retried profile cleanup");
    let target = loader::create_user_address_space_handle();
    let source = loader::create_user_address_space_handle();
    let mut owner = DomainLaunchTransaction::new(dummy(target)).unwrap();
    owner.retain_source(source).unwrap();
    let outstanding = AddressSpaceOperation::acquire(target).unwrap();
    assert!(matches!(
        owner.cancel(),
        Err(ProfileLaunchError::RollbackDomain(memory::AddressSpaceCloseError::OperationsInFlight))
    ));
    outstanding.release().unwrap();
    assert_eq!(memory::current_address_space_handle(target.id()), Some(target));
    assert_eq!(
        memory::close_user_address_space_handle(source),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    profile_rollback();
    crate::logln!(
        "[launch transaction] 16 explicit profile/domain/source rollbacks; complete \
         unguarded/guarded abandonment retains roots, profile, backing and charges; busy-profile \
         failure stops without domain cleanup or retry"
    );
}

fn profile_rollback() {
    use crate::service::supervisor::{
        NameServiceHandle,
        ServiceDomain,
        ServiceLimits,
        try_spawn_with_read_only_profile_and_limits,
    };
    let source = loader::create_user_address_space_handle();
    let endpoint = ipc::endpoint_create(source.id(), 1, 1, 4).unwrap();
    let naming = NameServiceHandle {
        domain: ServiceDomain {
            asid: source.id(),
            address_space: source,
            tid: 0,
            generation: 0,
            config_frame: 0u64.into(),
            status_frame: 0u64.into(),
        },
        endpoint_cap: endpoint,
    };
    let image = crate::service::store::service_elf(b"ns").unwrap();
    for valid_endpoint in [false, true] {
        let mut naming = naming;
        if !valid_endpoint {
            naming.endpoint_cap = u64::MAX;
        }
        let before = admission::used::<MemoryCapabilityNodes, 2>();
        let ipc_before = admission::used::<IpcNodes, 2>();
        let result = try_spawn_with_read_only_profile_and_limits(
            image,
            &naming,
            ipc::ConnectionRights::CALL,
            b"fixture-profile",
            ServiceLimits::default().with_max_threads(0),
        );
        if valid_endpoint {
            assert!(matches!(result, Err(ProfileLaunchError::Limits(_))));
        } else {
            assert!(matches!(result, Err(ProfileLaunchError::BootstrapConnection(_))));
        }
        assert_eq!(admission::used::<MemoryCapabilityNodes, 2>(), before);
        assert_eq!(admission::used::<IpcNodes, 2>(), ipc_before);
    }
    memory::close_user_address_space_handle(source).unwrap();
    crate::logln!(
        "[launch transaction profile] real signed ELF, bootstrap rejection and \
         post-profile-transfer limits rejection recover exact memory/IPC metadata; stale source \
         generation and one-shot busy-root termination passed"
    );
}
