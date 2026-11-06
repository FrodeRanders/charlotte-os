//! Serialized backing controls use empty real layouts and existing grant APIs.
//! They do not fabricate hardware completion or replace a live account.
use super::*;
use crate::{
    device::{
        self,
        AsDeviceCaps,
        DEVICES,
        DeviceCap,
        DeviceError,
        DeviceObject,
        GrantAdmission,
        registry::PreparedStorage,
    },
    memory::{
        self,
        AddressSpace,
        AddressSpaceHandle,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

enum Candidate {
    Namespace(Prepared<(usize, AsDeviceCaps)>),
    Capability(Prepared<(DeviceCap, DeviceObject)>),
}
impl Candidate {
    fn new(kind: usize, platform: bool) -> Result<Self, AllocError> {
        if kind.is_multiple_of(2) {
            prepare(platform).map(Self::Namespace)
        } else {
            prepare(platform).map(Self::Capability)
        }
    }

    fn touch(&self) {
        match self {
            Self::Namespace(node) => {
                core::hint::black_box(node);
            }
            Self::Capability(node) => {
                core::hint::black_box(node);
            }
        }
    }
}

fn reject_grants(root: AddressSpaceHandle) {
    let before = snapshot();
    let authority = crate::capability::node_admission_used();
    let caps = DEVICES.lock()[&root.id()].caps.iter().count();
    for _ in 0..32 {
        assert_eq!(device::grant_mmio(root.id(), 0x0900_0000, 1), Err(DeviceError::ResourceLimit));
        assert_eq!(device::grant_interrupt(root.id(), 225), Err(DeviceError::ResourceLimit));
        assert_eq!(
            device::grant_dma_domain_with_backend(
                root.id(),
                |_| panic!("node admission failure reached hardware"),
                |_| panic!("unstarted node failure destroyed hardware"),
            ),
            Err(DeviceError::ResourceLimit)
        );
    }
    assert_eq!(snapshot(), before);
    assert_eq!(crate::capability::node_admission_used(), authority);
    assert_eq!(DEVICES.lock()[&root.id()].caps.iter().count(), caps);
    assert_eq!(device::interrupt_route_owner(225), None);
}

fn new_root(trusted: bool) -> AddressSpaceHandle {
    let root = if trusted {
        AddressSpace::try_new_platform_user().unwrap()
    } else {
        AddressSpace::try_new_user().unwrap()
    };
    memory::register_user_address_space(root).unwrap()
}

fn pressure() {
    let baseline = snapshot();
    let ordinary_root = new_root(false);
    let trusted_root = new_root(true);
    for root in [ordinary_root, trusted_root] {
        let cap = device::grant_mmio(root.id(), 0x0900_0000, 1).unwrap();
        device::close_cap(root.id(), cap).unwrap();
    }
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
            assert!(PreparedStorage::try_new(false).is_err());
            for kind in 0..2 {
                assert!(Candidate::new(kind, false).is_err());
            }
        }
        reject_grants(ordinary_root);
    }
    // Trusted backing does not grant platform capability-resource admission.
    let resource = crate::capability::node_admission_used();
    let cap = device::grant_mmio(trusted_root.id(), 0x0900_0000, 1).unwrap();
    assert_eq!(crate::capability::node_admission_used(), (resource.0 + 1, resource.1 + 1));
    device::close_cap(trusted_root.id(), cap).unwrap();
    let mut admission = GrantAdmission::new(trusted_root.id()).unwrap();
    admission.finish().unwrap();
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
            assert!(PreparedStorage::try_new(true).is_err());
            for kind in 0..2 {
                assert!(Candidate::new(kind, true).is_err());
            }
        }
        reject_grants(ordinary_root);
        reject_grants(trusted_root);
    }
    // One available node permits the first allocation, but not a complete
    // two-node candidate. Its rollback must return that same original charge.
    drop(ordinary.pop().unwrap());
    let one = snapshot();
    reject_grants(ordinary_root);
    assert!(PreparedStorage::try_new(false).is_err());
    assert_eq!(snapshot(), one);
    drop(ordinary.pop().unwrap());
    let two = snapshot();
    let cap = device::grant_mmio(ordinary_root.id(), 0x0900_0000, 1).unwrap();
    memory::budget::mark_platform(ordinary_root);
    device::close_cap(ordinary_root.id(), cap).unwrap();
    assert_eq!(snapshot(), two, "promotion redirected original node refund");
    // Refill the exact two kinds removed above, independent of baseline.
    for _ in 0..2 {
        ordinary.push(Candidate::new(ordinary.len(), false).unwrap());
    }
    assert_eq!(snapshot(), full);
    let pressure_counts = (ordinary.len(), reserved.len());
    ordinary.iter().chain(reserved.iter()).for_each(Candidate::touch);
    drop((ordinary, reserved));
    for root in [ordinary_root, trusted_root] {
        memory::close_user_address_space_handle(root).unwrap();
    }
    assert_eq!(snapshot(), baseline);
    crate::logln!(
        "[device node pressure] actual mixed candidates ordinary={} trusted={} at {}/{}; 256 \
         heap-held layout and 128 complete-storage denials; MMIO/IRQ/DMA reject before \
         hardware/authority, one-node rollback, two-node recovery, trusted storage without \
         resource privilege and promoted original refund passed",
        pressure_counts.0,
        pressure_counts.1,
        ORDINARY[1],
        LIMIT[1]
    );
}

fn retained_candidates() {
    let baseline = snapshot();
    let (root, principal) =
        crate::device::registry::tests::principal_root(b"device-node-retained", false);
    let (captured, policy) = memory::capture_address_space_sponsor(root.id());
    assert_eq!(captured, Some(root));
    assert!(!policy.platform());
    let storage = PreparedStorage::try_new(policy).unwrap();
    let retained = snapshot();
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(snapshot(), retained);
    let retained_bytes = memory::kernel_buffers::test_metadata_used(principal);
    assert_eq!(
        retained_bytes,
        (Prepared::<(usize, AsDeviceCaps)>::allocation_bytes()
            + Prepared::<(DeviceCap, DeviceObject)>::allocation_bytes()) as u64
    );
    let (successor, other) =
        crate::device::registry::tests::principal_root(b"device-node-successor", true);
    let other_base = memory::kernel_buffers::test_metadata_used(other);
    assert_eq!(successor.id(), root.id());
    assert_ne!(successor, root);
    let (captured, policy) = memory::capture_address_space_sponsor(successor.id());
    assert_eq!(captured, Some(successor));
    assert!(policy.platform());
    let next = PreparedStorage::try_new(policy).unwrap();
    let both = snapshot();
    let bytes = Prepared::<(usize, AsDeviceCaps)>::allocation_bytes()
        + Prepared::<(DeviceCap, DeviceObject)>::allocation_bytes();
    let other_live = memory::kernel_buffers::test_metadata_used(other);
    storage.release();
    assert_eq!(memory::kernel_buffers::test_metadata_used(principal), 0);
    assert_eq!(memory::kernel_buffers::test_metadata_used(other), other_live);
    assert_eq!(
        snapshot(),
        ([both.0[0] - bytes as u64, both.0[1] - 2], [both.1[0] - bytes as u64, both.1[1] - 2])
    );
    next.release();
    assert_eq!(memory::kernel_buffers::test_metadata_used(other), other_base);
    memory::close_user_address_space_handle(successor).unwrap();
    assert_eq!(snapshot(), baseline);
    assert_eq!(memory::kernel_buffers::test_metadata_used(other), 0);
}

fn allocation_rejection<T>() {
    let before = snapshot();
    for _ in 0..64 {
        assert!(prepare_with::<T>(false, |_| Err(AllocError)).is_err());
        assert_eq!(snapshot(), before);
    }
}

pub(in crate::device) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    allocation_rejection::<(usize, AsDeviceCaps)>();
    allocation_rejection::<(DeviceCap, DeviceObject)>();
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
    retained_candidates();
    principal_funding();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[device node admission] layouts namespace={} capability={} charge={}; 128 allocator \
         rejections, independent byte denial, retained empty candidates across exact root \
         close/ASID reuse and original-class deallocation refunds passed",
        Prepared::<(usize, AsDeviceCaps)>::allocation_bytes(),
        Prepared::<(DeviceCap, DeviceObject)>::allocation_bytes(),
        core::mem::size_of::<Charge>()
    );
}

fn principal_funding() {
    use memory::kernel_buffers::{
        capture_storage,
        test_metadata_limit as limit,
        test_metadata_used as used,
    };
    let before = snapshot();
    let (root, principal) =
        crate::device::registry::tests::principal_root(b"device-node-principal", false);
    let (sibling, same) =
        crate::device::registry::tests::principal_root(b"device-node-principal", false);
    assert_eq!(same, principal);
    let (healthy, other) =
        crate::device::registry::tests::principal_root(b"device-node-healthy", false);
    let policy = capture_storage(root).unwrap();
    let namespace = Prepared::<(usize, AsDeviceCaps)>::allocation_bytes() as u64;
    let capability = Prepared::<(DeviceCap, DeviceObject)>::allocation_bytes() as u64;
    let base = used(principal);
    let healthy_base = used(other);
    for _ in 0..64 {
        assert!(prepare_with::<(usize, AsDeviceCaps)>(policy, |_| Err(AllocError)).is_err());
        assert!(prepare_with::<(DeviceCap, DeviceObject)>(policy, |_| Err(AllocError)).is_err());
        assert_eq!(used(principal), base);
        assert_eq!(snapshot(), before);
    }
    let first = device::grant_mmio(root.id(), 0x0900_0000, 1).unwrap();
    let live = used(principal);
    let authority = live - base - namespace - capability;
    assert!(authority > 0);
    memory::budget::mark_platform(root);
    limit(principal, live);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        reject_grants(root);
    }
    assert_eq!(used(principal), live);
    let family = snapshot();
    let counts = crate::capability::node_admission_used();
    for headroom in [
        0,
        namespace - 1,
        namespace,
        namespace + capability - 1,
        namespace + capability,
        namespace + capability + authority - 1,
    ] {
        limit(principal, live + headroom);
        assert_eq!(device::grant_mmio(root.id(), 0x0900_0000, 1), Err(DeviceError::ResourceLimit));
        assert_eq!(device::grant_interrupt(root.id(), 225), Err(DeviceError::ResourceLimit));
        assert_eq!(
            device::grant_dma_domain_with_backend(
                root.id(),
                |_| panic!("principal denial reached hardware"),
                |_| panic!("unstarted denial destroyed hardware")
            ),
            Err(DeviceError::ResourceLimit)
        );
        assert_eq!(used(principal), live);
        assert_eq!(snapshot(), family);
        assert_eq!(crate::capability::node_admission_used(), counts);
        assert_eq!(device::interrupt_route_owner(225), None);
    }
    let independent = device::grant_mmio(healthy.id(), 0x0900_0000, 1).unwrap();
    assert_eq!(used(other), healthy_base + namespace + capability + authority);
    device::close_cap(healthy.id(), independent).unwrap();
    assert_eq!(used(other), healthy_base + namespace);
    limit(principal, crate::resource_policy::POLICY.memory.principal_metadata_bytes);
    device::close_cap(root.id(), first).unwrap();
    assert_eq!(used(principal), base + namespace);
    for value in [root, sibling, healthy] {
        memory::close_user_address_space_handle(value).unwrap();
    }
    assert_eq!(used(principal), 0);
    assert_eq!(used(other), 0);
    assert_eq!(snapshot(), before);
    let (root, pressure_principal) =
        crate::device::registry::tests::principal_root(b"device-node-byte-pressure", false);
    let policy = capture_storage(root).unwrap();
    let baseline = used(pressure_principal);
    let ceiling = crate::resource_policy::POLICY.memory.principal_metadata_bytes;
    let mut held = alloc::vec::Vec::with_capacity((ceiling / capability) as usize + 1);
    while let Ok(node) = prepare::<(DeviceCap, DeviceObject)>(policy) {
        held.push(node);
        assert!(held.len() <= (ceiling / capability) as usize);
    }
    assert!(ceiling - used(pressure_principal) < capability);
    assert!(snapshot().1[1] < ORDINARY[1]);
    let count = held.len();
    drop(held);
    assert_eq!(used(pressure_principal), baseline);
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(used(pressure_principal), 0);
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[device node principal] namespace/payload/authority={}/{}/{} bytes; siblings, 128 funded \
         allocator failures, 96 heap-held adapter denials, six MMIO/IRQ/DMA prefix controls, \
         healthy grant, immutable promotion, retained candidates across funded ASID reuse and {} \
         real candidates at {}-byte ceiling passed",
        namespace,
        capability,
        authority,
        count,
        ceiling
    );
}
