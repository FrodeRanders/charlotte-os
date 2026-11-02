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
    let root = new_root(false);
    let (captured, class) = memory::capture_address_space_storage(root.id());
    assert_eq!(captured, Some(root));
    assert!(!class);
    let storage = PreparedStorage::try_new(class).unwrap();
    let retained = snapshot();
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(snapshot(), retained);
    let successor = new_root(true);
    assert_eq!(successor.id(), root.id());
    assert_ne!(successor, root);
    let (captured, class) = memory::capture_address_space_storage(successor.id());
    assert_eq!(captured, Some(successor));
    assert!(class);
    let next = PreparedStorage::try_new(class).unwrap();
    let both = snapshot();
    let bytes = Prepared::<(usize, AsDeviceCaps)>::allocation_bytes()
        + Prepared::<(DeviceCap, DeviceObject)>::allocation_bytes();
    storage.release();
    assert_eq!(
        snapshot(),
        ([both.0[0] - bytes as u64, both.0[1] - 2], [both.1[0] - bytes as u64, both.1[1] - 2])
    );
    next.release();
    memory::close_user_address_space_handle(successor).unwrap();
    assert_eq!(snapshot(), baseline);
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
