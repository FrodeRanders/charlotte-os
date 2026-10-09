//! Admitted mapping nodes follow pins through publication, detach and quarantine.
//! Typed backends still own the hardware completion proof.
use super::{
    detached_domain::DetachedDomain,
    dma::Error,
    registry::Map,
};
use crate::{
    klib::collections::retirement_list::{
        PreparedEntry,
        RetiredEntry,
        RetirementList,
    },
    memory::object::{
        self,
        DmaPin,
    },
};

pub(super) struct Record {
    pin: Option<DmaPin>,
    pub(super) pages: usize,
}
impl Record {
    fn pin(&self) -> &DmaPin {
        self.pin.as_ref().expect("DMA record pin consumed twice")
    }
}
pub(super) type RetiredMapping = RetiredEntry<(u64, Record)>;

struct Pending {
    pin: Option<DmaPin>,
    preparing: Option<PreparedEntry<(u64, Record)>>,
    retired: Option<RetiredMapping>,
}
#[must_use]
pub(super) struct PendingPin(DetachedDomain<Pending>);
impl PendingPin {
    pub(super) fn new(pin: Option<DmaPin>) -> Self {
        Self(DetachedDomain::new(Pending {
            pin,
            preparing: None,
            retired: None,
        }))
    }

    pub(super) fn borrow(&mut self) -> &DmaPin {
        self.0.value_mut().pin.as_ref().expect("missing DMA pin")
    }

    pub(super) fn prepare(&mut self) -> Result<(), Error> {
        let pending = self.0.value_mut();
        assert!(pending.pin.is_some() && pending.preparing.is_none() && pending.retired.is_none());
        pending.preparing = Some(prepare(1)?);
        Ok(())
    }

    pub(super) fn retain_record(&mut self, record: RetiredMapping) {
        let pending = self.0.value_mut();
        assert!(pending.pin.is_none() && pending.preparing.is_none() && pending.retired.is_none());
        pending.retired = Some(record);
    }

    fn take_record(&mut self, iova: u64, pages: usize) -> RetiredMapping {
        let pending = self.0.value_mut();
        if let Some(retired) = pending.retired.take() {
            assert!(pending.pin.is_none() && pending.preparing.is_none());
            retired
        } else {
            pending.preparing.take().expect("DMA pin missing admitted storage").publish((
                iova,
                Record {
                    pin: pending.pin.take(),
                    pages,
                },
            ))
        }
    }

    pub(super) fn release(mut self) {
        if let Some(pin) = self.0.value_mut().pin.take() {
            object::unpin_dma(pin);
        }
        if let Some(record) = self.0.value_mut().retired.take() {
            release_record(record);
        }
        if self.0.value_mut().preparing.is_some() {
            tests::boundary(true);
            drop(self.0.value_mut().preparing.take());
        }
    }
}

pub(super) struct Records {
    live: Map<u64, Record>,
    quarantined: RetirementList<(u64, Record)>,
}
impl Records {
    pub(super) fn new() -> Self {
        Self {
            live: Map::new(),
            quarantined: RetirementList::new(),
        }
    }

    pub(super) fn prepare(&self, pending: &mut PendingPin) -> Result<(), Error> {
        let id = pending.borrow().object_id();
        if self.live.values().any(|record| record.pin().object_id() == id)
            || self.quarantined.iter().any(|(_, record)| record.pin().object_id() == id)
        {
            return Err(Error::AlreadyMapped);
        }
        pending.prepare()
    }

    pub(super) fn publish(&mut self, pending: &mut PendingPin, iova: u64, pages: usize) {
        let storage = pending.0.value_mut();
        self.live.insert(
            storage.preparing.take().expect("mapping publication without admission"),
            iova,
            Record {
                pin: storage.pin.take(),
                pages,
            },
        );
    }

    pub(super) fn take(&mut self, iova: u64) -> Option<RetiredMapping> {
        self.live.take(&iova)
    }

    pub(super) fn quarantine(&mut self, pending: &mut PendingPin) {
        // Prefix failure consumes its pre-leaf node; failed unmap relinks the
        // original detached node. No allocation or destruction on either path.
        self.quarantined.push(pending.take_record(0, 0));
    }

    pub(super) fn release(mut self) {
        while let Some((&iova, _)) = self.live.first_key_value() {
            release_record(self.live.take(&iova).unwrap());
        }
        while let Some(record) = self.quarantined.pop() {
            release_record(record);
        }
    }
}
fn release_record(mut record: RetiredMapping) {
    object::unpin_dma(record.value_mut().1.pin.take().unwrap());
    tests::boundary(true);
    record.release();
}
fn prepare<T>(stage: usize) -> Result<PreparedEntry<T>, Error> {
    tests::boundary(false);
    if tests::reject(stage) {
        return Err(Error::Memory);
    }
    PreparedEntry::try_new().map_err(|_| Error::Memory)
}

/// Cached SMMU leaf-table addresses own their metadata for the domain lifetime.
/// Partial walks retain unused admitted storage for a later cache miss.
#[cfg(target_arch = "aarch64")]
pub(super) struct WalkerCache {
    tables: Map<u64, crate::memory::physical::PAddr>,
    preparing:
        core::mem::ManuallyDrop<Option<PreparedEntry<(u64, crate::memory::physical::PAddr)>>>,
}
#[cfg(target_arch = "aarch64")]
impl WalkerCache {
    pub(super) fn new() -> Self {
        Self {
            tables: Map::new(),
            preparing: core::mem::ManuallyDrop::new(None),
        }
    }

    pub(super) fn get(&self, key: &u64) -> Option<&crate::memory::physical::PAddr> {
        self.tables.get(key)
    }

    pub(super) fn prepare(&mut self) -> Result<(), Error> {
        if self.preparing.is_none() {
            *self.preparing = Some(prepare(2)?);
        }
        Ok(())
    }

    pub(super) fn publish(&mut self, key: u64, frame: crate::memory::physical::PAddr) {
        self.tables.insert(self.preparing.take().unwrap(), key, frame);
    }

    pub(super) fn release(mut self) {
        if let Some(node) = self.preparing.take() {
            tests::boundary(true);
            drop(node);
        }
        while let Some((&key, _)) = self.tables.first_key_value() {
            let node = self.tables.take(&key).unwrap();
            tests::boundary(true);
            node.release();
        }
    }
}

pub(super) mod tests;
