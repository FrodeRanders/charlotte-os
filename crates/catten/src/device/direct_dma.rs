//! x86_64 direct (identity) DMA domains.
//!
//! A full Intel VT-d / AMD-Vi IOMMU driver is not yet implemented, so devices
//! DMA directly to physical addresses. This module mirrors the AArch64 SMMU
//! domain API (`stream_id`, `create_domain`, `map`, `unmap`,
//! `destroy_domain`) but returns the pinned physical address as the IOVA.
//!
//! This is the storage bring-up path: it is correct *provided* a memory object
//! mapped for DMA is physically contiguous (which the sequential frame
//! allocator satisfies for the boot-time object sizes). Real DMA protection and
//! arbitrary IOVA allocation are a follow-up VT-d implementation.

use alloc::collections::BTreeMap;
use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use spin::LazyLock;

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    memory::object::{
        self,
        DmaPin,
    },
};

#[derive(Debug)]
pub enum Error {
    Unsupported,
    InvalidStream,
    StreamInUse,
    InvalidDirection,
    Memory,
    OutOfIova,
    MapFailed,
    UnknownDomain,
    UnknownMapping,
    HardwareTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Direction(u32);

impl Direction {
    pub const DEVICE_READ: Self = Self(1);
    pub const DEVICE_WRITE: Self = Self(2);

    pub fn from_bits(bits: u32) -> Result<Self, Error> {
        if bits != 0 && bits & !3 == 0 {
            Ok(Self(bits))
        } else {
            Err(Error::InvalidDirection)
        }
    }

    fn device_writes(self) -> bool {
        self.0 & Self::DEVICE_WRITE.0 != 0
    }
}

struct Domain {
    sid: u32,
    mappings: BTreeMap<u64, DmaPin>,
}

static NEXT_DOMAIN: AtomicU32 = AtomicU32::new(0);
static DOMAINS: LazyLock<Mutex<BTreeMap<u64, Domain>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static STREAMS: LazyLock<Mutex<BTreeMap<u32, u64>>> = LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub fn initialize_early() -> Result<(), Error> {
    Ok(())
}

/// The PCI requester id doubles as the stream id in the direct (no-remap) model.
pub fn stream_id(requester_id: u32) -> Result<u32, Error> {
    Ok(requester_id)
}

pub fn create_domain(sid: u32, _msi_address: Option<u64>) -> Result<u64, Error> {
    let mut streams = STREAMS.lock();
    if streams.contains_key(&sid) {
        return Err(Error::StreamInUse);
    }
    let id = NEXT_DOMAIN.fetch_add(1, Ordering::Relaxed) as u64;
    let mut domains = DOMAINS.lock();
    domains.insert(
        id,
        Domain {
            sid,
            mappings: BTreeMap::new(),
        },
    );
    streams.insert(sid, id);
    Ok(id)
}

pub fn map(
    domain_id: u64,
    caller: crate::memory::AddressSpaceId,
    memory_cap: u64,
    direction: Direction,
    exclusive: bool,
) -> Result<u64, Error> {
    let pin = object::pin_for_dma(
        caller,
        memory_cap,
        direction.0 & Direction::DEVICE_READ.0 != 0,
        direction.device_writes(),
        exclusive,
    )
    .map_err(|_| Error::Memory)?;

    // The IOVA is the object's first physical frame. The device addresses the
    // remaining pages at `iova + n * PAGE_SIZE`, so the frames must be
    // physically contiguous.
    let frames = pin.frames();
    if frames.is_empty() {
        object::unpin_dma(pin);
        return Err(Error::OutOfIova);
    }
    let iova: u64 = frames[0].into();
    for (index, frame) in frames.iter().enumerate() {
        let expected =
            frames[0] + index as isize * crate::cpu::isa::memory::paging::PAGE_SIZE as isize;
        if *frame != expected {
            object::unpin_dma(pin);
            return Err(Error::OutOfIova);
        }
    }

    let mut domains = DOMAINS.lock();
    let domain = domains.get_mut(&domain_id).ok_or(Error::UnknownDomain)?;
    domain.mappings.insert(iova, pin);
    Ok(iova)
}

pub fn unmap(domain_id: u64, iova: u64) -> Result<(), Error> {
    let pin = {
        let mut domains = DOMAINS.lock();
        let domain = domains.get_mut(&domain_id).ok_or(Error::UnknownDomain)?;
        domain.mappings.remove(&iova).ok_or(Error::UnknownMapping)?
    };
    object::unpin_dma(pin);
    Ok(())
}

pub fn destroy_domain(domain_id: u64) -> Result<(), Error> {
    let domain = {
        let mut domains = DOMAINS.lock();
        domains.remove(&domain_id).ok_or(Error::UnknownDomain)?
    };
    for (_, pin) in domain.mappings {
        object::unpin_dma(pin);
    }
    STREAMS.lock().remove(&domain.sid);
    Ok(())
}

/// Direct DMA has no fault-reporting interrupt source.
pub fn handle_interrupt(_intid: u32) -> bool {
    false
}

pub fn fault_count() -> u64 {
    0
}

pub fn pending_fault_events() -> u32 {
    0
}
