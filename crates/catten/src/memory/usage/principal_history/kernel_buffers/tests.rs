//! Serialized boot controls use real buffers and the real captured-principal
//! allocator. Counter-only node exhaustion is not a heap-pressure campaign.
use super::*;
use crate::{
    klib::{
        admission::{
            self,
            Reservation,
        },
        collections::snapshot::Budget,
    },
    memory::{
        self,
        AddressSpace,
        AddressSpaceHandle,
    },
};

fn domain(principal: u64) -> AddressSpaceHandle {
    let root =
        AddressSpace::try_new_principal_user(false, prepare_backing(principal, false).unwrap())
            .unwrap();
    let handle = memory::register_user_address_space(root).unwrap();
    memory::register_domain_authority(handle, principal, 0).unwrap();
    handle
}
fn used(principal: u64) -> u64 {
    HISTORY.lock().get(&principal).unwrap().kernel_buffers.used()[0]
}
fn budget(root: AddressSpaceHandle) -> Budget<SnapshotAllocator> {
    Budget::with_allocator(1024, telemetry_allocator(root.id()).unwrap())
}

pub(crate) fn run() {
    let principal = charlotte_launch::artifact_principal_id(b"diagnostic-buffer-principal");
    let other = charlotte_launch::artifact_principal_id(b"diagnostic-buffer-other");
    let first = domain(principal);
    let second = domain(principal);
    let healthy = domain(other);
    let maximum = crate::klib::collections::snapshot::MAX_WORKING_BYTES;
    let maximum_buffer =
        Budget::with_allocator(maximum, telemetry_allocator(healthy.id()).unwrap())
            .prepare::<u8>(maximum)
            .unwrap();
    assert_eq!(used(other), maximum as u64);
    drop(maximum_buffer);
    assert_eq!(used(other), 0);
    HISTORY.lock().get_mut(&principal).unwrap().kernel_buffers = VectorBudget::new([192]);
    let baseline = admission::used::<SnapshotBuffers, 2>();
    let a = budget(first).prepare::<u8>(96).unwrap().into_vec();
    let b = budget(second).prepare::<u8>(96).unwrap().into_vec();
    assert_eq!(used(principal), 192);
    for _ in 0..128 {
        assert!(budget(first).prepare::<u8>(1).is_err());
    }
    let healthy_buffer = budget(healthy).prepare::<u8>(128).unwrap();
    assert_eq!(used(other), 128);
    let charged = admission::used::<SnapshotBuffers, 2>();
    assert_eq!(charged.1[0], baseline.1[0] + 320);
    memory::budget::mark_platform(second);
    assert_eq!(admission::used::<SnapshotBuffers, 2>(), charged);
    assert!(budget(second).prepare::<u8>(1).is_err());
    memory::close_user_address_space_handle(first).unwrap();
    assert_eq!(used(principal), 192);
    let replacement = domain(other);
    assert_eq!(replacement.id(), first.id());
    assert_ne!(replacement, first);
    drop(a);
    assert_eq!(used(principal), 96);
    assert_eq!(used(other), 128);
    drop(b);
    assert_eq!(used(principal), 0);
    drop(healthy_buffer);
    assert_eq!(used(other), 0);
    assert_eq!(admission::used::<SnapshotBuffers, 2>(), baseline);

    // Family rejection must roll back the earlier principal reservation.
    let limit = crate::resource_policy::POLICY.snapshot_buffers.ordinary[0];
    let pressure =
        Reservation::<SnapshotBuffers, 2>::reserve(false, [limit - baseline.1[0], 0]).unwrap();
    assert!(budget(replacement).prepare::<u8>(1).is_err());
    assert_eq!(used(other), 0);
    let platform_buffer = budget(second).prepare::<u8>(96).unwrap();
    assert_eq!(used(principal), 96);
    assert_eq!(admission::used::<SnapshotBuffers, 2>().1[0], limit);
    drop(platform_buffer);
    drop(pressure);
    assert_eq!(admission::used::<SnapshotBuffers, 2>(), baseline);
    let recovered = budget(replacement).prepare::<u8>(96).unwrap();
    assert_eq!(used(other), 96);
    drop(recovered);
    for root in [second, healthy, replacement] {
        memory::close_user_address_space_handle(root).unwrap();
    }
    assert_eq!(used(principal), 0);
    assert_eq!(used(other), 0);
    crate::logln!(
        "[diagnostic buffer admission] real shared-principal buffers, 128 denials, healthy \
         sponsor, promotion, retired sponsor/ASID reuse, family rejection rollback and recovery \
         passed"
    );
}
