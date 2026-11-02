//! Real empty node backing and complete grant preparation; no hardware ACKs or
//! domain payloads are fabricated. The NVMe fixture supplies its own backend proof.
use super::*;
#[cfg(target_arch = "x86_64")]
use crate::memory::physical::PAddr;
use crate::{
    device::{
        self,
        PreparedDmaDomain,
        backend_registry::{
            Nodes,
            Preparing,
        },
    },
    memory::{
        self,
        AddressSpace,
        AddressSpaceHandle,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

#[cfg(target_arch = "x86_64")]
type Source = u16;
#[cfg(target_arch = "aarch64")]
type Source = u32;

fn metadata(platform: bool) -> Preparing {
    #[cfg(target_arch = "aarch64")]
    {
        Preparing::Smmu(Nodes::new(true, false, platform))
    }
    #[cfg(target_arch = "x86_64")]
    {
        if crate::environment::acpi::sdt::dmar::discover_vtd().is_some() {
            Preparing::Vtd(Nodes::new(true, true, platform))
        } else {
            Preparing::AmdVi(Nodes::new(true, false, platform))
        }
    }
}

fn grant(root: AddressSpaceHandle, reject: bool) {
    let before = snapshot();
    let mut grant =
        PreparedDmaDomain::new(root.id(), |_| panic!("empty preparation destroyed hardware"))
            .unwrap();
    let platform = grant.resources.creation.backing_platform;
    grant.resources.creation.metadata = Some(metadata(platform));
    let result = grant.resources.creation.metadata.as_mut().unwrap().allocate();
    assert_eq!(
        result,
        if reject {
            Err(device::dma::Error::MapFailed)
        } else {
            Ok(())
        }
    );
    assert!(grant.cancel_unpublished().is_ok());
    assert_eq!(snapshot(), before);
}

fn allocation<T>(name: &str) {
    let before = snapshot();
    for _ in 0..64 {
        assert!(prepare_with::<T>(false, |_| Err(AllocError)).is_err());
        assert_eq!(snapshot(), before);
    }
    crate::logln!(
        "[backend node layout] {}={} bytes; 64 allocator rejections passed",
        name,
        Prepared::<T>::allocation_bytes()
    );
}
fn deny(platform: bool) {
    assert!(prepare::<(Source, u64)>(platform).is_err());
    #[cfg(target_arch = "x86_64")]
    {
        assert!(prepare::<(u64, Option<device::vt_d::Domain>)>(platform).is_err());
        assert!(prepare::<(u64, Option<device::amd_vi::Domain>)>(platform).is_err());
        assert!(prepare::<(u16, PAddr)>(platform).is_err());
    }
    #[cfg(target_arch = "aarch64")]
    assert!(prepare::<(u64, Option<device::smmu::Domain>)>(platform).is_err());
}

fn count_pressure() {
    let before = snapshot();
    let ordinary_root =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    let trusted_root =
        memory::register_user_address_space(AddressSpace::try_new_platform_user().unwrap())
            .unwrap();
    let mut ordinary = alloc::vec::Vec::with_capacity(ORDINARY[1] as usize);
    while let Ok(node) = prepare::<(Source, u64)>(false) {
        ordinary.push(node);
        assert!(ordinary.len() <= ORDINARY[1] as usize);
    }
    assert_eq!(snapshot().1[1], ORDINARY[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            deny(false);
        }
    }
    grant(ordinary_root, true);
    grant(trusted_root, false);
    let mut trusted = alloc::vec::Vec::with_capacity((LIMIT[1] - ORDINARY[1]) as usize);
    while let Ok(node) = prepare::<(Source, u64)>(true) {
        trusted.push(node);
        assert!(trusted.len() <= (LIMIT[1] - ORDINARY[1]) as usize);
    }
    let full = snapshot();
    assert_eq!(full.0[1], LIMIT[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            deny(true);
        }
    }
    grant(ordinary_root, true);
    grant(trusted_root, true);
    for available in 1..=3 {
        drop(ordinary.pop().unwrap());
        let partial = snapshot();
        let mut nodes = metadata(false);
        let result = nodes.allocate();
        #[cfg(target_arch = "aarch64")]
        let needed = 2;
        #[cfg(target_arch = "x86_64")]
        let needed = if crate::environment::acpi::sdt::dmar::discover_vtd().is_some() {
            3
        } else {
            2
        };
        assert_eq!(result.is_ok(), available >= needed);
        nodes.finish();
        assert_eq!(snapshot(), partial);
    }
    for _ in 0..3 {
        ordinary.push(prepare::<(Source, u64)>(false).unwrap());
    }
    assert_eq!(snapshot(), full);
    let counts = (ordinary.len(), trusted.len());
    drop((ordinary, trusted));
    memory::close_user_address_space_handle(ordinary_root).unwrap();
    memory::close_user_address_space_handle(trusted_root).unwrap();
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[backend node count pressure] actual requester-layout candidates ordinary={} trusted={} \
         at {}/{}; heap-held all-layout denials, captured grant class, trusted reserve and \
         one/two/three-node rollback/recovery passed",
        counts.0,
        counts.1,
        ORDINARY[1],
        LIMIT[1]
    );
}

fn byte_pressure<D>() {
    let before = snapshot();
    let bytes = Prepared::<(u64, Option<D>)>::allocation_bytes() as u64;
    let mut ordinary = alloc::vec::Vec::with_capacity((ORDINARY[0] / bytes + 1) as usize);
    while let Ok(node) = prepare::<(u64, Option<D>)>(false) {
        ordinary.push(node);
    }
    let ordinary_full = snapshot();
    assert!(ordinary_full.1[1] < ORDINARY[1]);
    assert!(ORDINARY[0] - ordinary_full.1[0] < bytes);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(prepare::<(u64, Option<D>)>(false).is_err());
        }
    }
    let reserve = prepare::<(u64, Option<D>)>(true).unwrap();
    drop(reserve);
    assert_eq!(snapshot(), ordinary_full);
    let mut trusted =
        alloc::vec::Vec::with_capacity(((LIMIT[0] - ORDINARY[0]) / bytes + 1) as usize);
    while let Ok(node) = prepare::<(u64, Option<D>)>(true) {
        trusted.push(node);
    }
    let full = snapshot();
    assert!(full.0[1] < LIMIT[1]);
    assert!(LIMIT[0] - full.0[0] < bytes);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(prepare::<(u64, Option<D>)>(true).is_err());
        }
    }
    let counts = (ordinary.len(), trusted.len());
    drop((ordinary, trusted));
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[backend node byte pressure] actual domain-layout empty candidates ordinary={} \
         trusted={} at bytes={}; ordinary/total byte exhaustion below count ceilings, 128 \
         heap-held byte denials and exact recovery passed",
        counts.0,
        counts.1,
        bytes
    );
}

/// The real NVMe creation control independently saturates count admission;
/// actual backing pressure is covered above. No heap work occurs under this pool.
pub(in crate::device) fn with_ordinary_pressure(work: impl FnOnce()) {
    let before = snapshot();
    let count = ORDINARY[1] - before.1[1];
    let pressure = admission::Reservation::<IommuNodes, 2>::reserve(false, [0, count]).unwrap();
    work();
    drop(pressure);
    assert_eq!(snapshot(), before);
}

pub(in crate::device) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    allocation::<(Source, u64)>("requester");
    #[cfg(target_arch = "x86_64")]
    {
        allocation::<(u64, Option<device::vt_d::Domain>)>("vtd-domain");
        allocation::<(u64, Option<device::amd_vi::Domain>)>("amd-domain");
        allocation::<(u16, PAddr)>("vtd-context");
    }
    #[cfg(target_arch = "aarch64")]
    allocation::<(u64, Option<device::smmu::Domain>)>("smmu-domain");
    count_pressure();
    #[cfg(target_arch = "aarch64")]
    byte_pressure::<device::smmu::Domain>();
    #[cfg(target_arch = "x86_64")]
    if crate::environment::acpi::sdt::dmar::discover_vtd().is_some() {
        byte_pressure::<device::vt_d::Domain>();
    } else {
        byte_pressure::<device::amd_vi::Domain>();
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[backend node admission] requested-layout bytes/count, original captured backing class, \
         empty real-layout pressure and complete preparation rollback passed; charge={} bytes",
        core::mem::size_of::<Charge>()
    );
}
