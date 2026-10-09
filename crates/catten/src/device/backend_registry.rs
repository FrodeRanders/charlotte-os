//! Shared admitted storage, while typed backends keep hardware completion proofs.
//! Preparation stays in the complete grant; installed nodes stay in its unit.
use core::mem::ManuallyDrop;

use super::{
    dma::Error,
    registry::Map,
};
use crate::{
    klib::collections::retirement_list::{
        PreparedEntry,
        RetiredEntry,
    },
    memory::physical::PAddr,
};

struct Storage<D, S> {
    domain: Option<PreparedEntry<(u64, Option<D>)>>,
    source: Option<PreparedEntry<(S, u64)>>,
    context: Option<PreparedEntry<(u16, PAddr)>>,
    source_needed: bool,
    context_needed: bool,
}
pub(super) struct Nodes<D, S>(ManuallyDrop<Storage<D, S>>);
impl<D, S: Ord + Copy> Nodes<D, S> {
    pub(super) fn new(source_needed: bool, context_needed: bool) -> Self {
        Self(ManuallyDrop::new(Storage {
            domain: None,
            source: None,
            context: None,
            source_needed,
            context_needed,
        }))
    }

    pub(super) fn allocate(&mut self) -> Result<(), Error> {
        assert!(self.0.domain.is_none() && self.0.source.is_none() && self.0.context.is_none());
        self.0.domain = Some(prepare(1)?);
        if self.0.source_needed {
            self.0.source = Some(prepare(2)?);
        }
        if self.0.context_needed {
            self.0.context = Some(prepare(3)?);
        }
        Ok(())
    }

    pub(super) fn publish_domain(&mut self, map: &mut Map<u64, Option<D>>, id: u64, domain: D) {
        map.insert(self.0.domain.take().unwrap(), id, Some(domain));
    }

    pub(super) fn publish_source(&mut self, map: &mut Map<S, u64>, source: S, id: u64) {
        if let Some(existing) = map.get_mut(&source) {
            assert_eq!(*existing, 0, "live requester fence replaced");
            assert!(!self.0.source_needed && self.0.source.is_none());
            *existing = id;
        } else {
            map.insert(self.0.source.take().unwrap(), source, id);
        }
    }

    #[cfg(target_arch = "x86_64")]
    pub(super) fn publish_context(&mut self, map: &mut Map<u16, PAddr>, bus: u16, frame: PAddr) {
        map.insert(self.0.context.take().unwrap(), bus, frame);
    }

    pub(super) fn finish(mut self) {
        tests::boundary(true);
        // Only unused empty nodes remain; published nodes belong to the unit.
        drop(self.0.context.take());
        drop(self.0.source.take());
        drop(self.0.domain.take());
    }
}

impl<D, S> Drop for Nodes<D, S> {
    fn drop(&mut self) {
        // Inert partial/unstarted preparation, including implicit enum fields.
    }
}

fn prepare<T>(stage: usize) -> Result<PreparedEntry<T>, Error> {
    tests::boundary(false);
    if tests::reject(stage) {
        return Err(Error::MapFailed);
    }
    PreparedEntry::try_new().map_err(|_| Error::MapFailed)
}

/// Typed preparation travels with DmaCreation through private/registered errors,
/// reset uncertainty and publication. ManuallyDrop node fallback is inert.
pub(super) enum Preparing {
    #[cfg(target_arch = "x86_64")]
    Vtd(Nodes<super::vt_d::Domain, u16>),
    #[cfg(target_arch = "x86_64")]
    AmdVi(Nodes<super::amd_vi::Domain, u16>),
    #[cfg(target_arch = "aarch64")]
    Smmu(Nodes<super::smmu::Domain, u32>),
}
impl Preparing {
    pub(super) fn allocate(&mut self) -> Result<(), Error> {
        match self {
            #[cfg(target_arch = "x86_64")]
            Self::Vtd(nodes) => nodes.allocate(),
            #[cfg(target_arch = "x86_64")]
            Self::AmdVi(nodes) => nodes.allocate(),
            #[cfg(target_arch = "aarch64")]
            Self::Smmu(nodes) => nodes.allocate(),
        }
    }

    pub(super) fn finish(self) {
        match self {
            #[cfg(target_arch = "x86_64")]
            Self::Vtd(nodes) => nodes.finish(),
            #[cfg(target_arch = "x86_64")]
            Self::AmdVi(nodes) => nodes.finish(),
            #[cfg(target_arch = "aarch64")]
            Self::Smmu(nodes) => nodes.finish(),
        }
    }
}

pub(super) fn release<T>(node: RetiredEntry<T>) {
    tests::boundary(true);
    node.release();
}
pub(super) mod tests;
