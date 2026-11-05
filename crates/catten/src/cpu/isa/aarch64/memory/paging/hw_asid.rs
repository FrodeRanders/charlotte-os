//! Hardware-bounded tag storage prepared before any user tag is published.
use alloc::{
    collections::TryReserveError,
    vec::Vec,
};

use super::HwAsid;

pub(super) struct HwAsidAllocator {
    next: u32,
    limit: u32,
    reserved_kernel: HwAsid,
    free: Vec<HwAsid>,
}

impl HwAsidAllocator {
    pub(super) fn try_new(limit: u32, reserved_kernel: HwAsid) -> Result<Self, TryReserveError> {
        assert!(limit > 1 && limit <= u32::from(u16::MAX) + 1);
        assert!(u32::from(reserved_kernel) < limit);
        let mut free = Vec::new();
        free.try_reserve_exact(limit as usize)?;
        Ok(Self {
            next: 1,
            limit,
            reserved_kernel,
            free,
        })
    }

    pub(super) fn capacity(&self) -> usize {
        self.free.capacity()
    }

    pub(super) fn allocate(&mut self) -> Option<HwAsid> {
        if let Some(asid) = self.free.pop() {
            return Some(asid);
        }
        if self.next == u32::from(self.reserved_kernel) {
            self.next += 1;
        }
        if self.next >= self.limit {
            return None;
        }
        let asid = self.next as HwAsid;
        self.next += 1;
        Some(asid)
    }

    // The enclosing exact root owns each tag linearly. Its existing confirmed
    // invalidation boundary must still precede this return; storage is no proof.
    pub(super) fn release(&mut self, asid: HwAsid) {
        assert!(asid != 0 && asid != self.reserved_kernel && u32::from(asid) < self.next);
        assert!(
            self.free.len() < self.free.capacity(),
            "hardware ASID return storage not prepared"
        );
        self.free.push(asid);
    }
}
