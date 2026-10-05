//! Loan authority, mapped backing and namespace retention through revocation.
//! Direct callers, borrowed-memory replies and explicit call/reply cancellation
//! lease both roots. Endpoint/domain bulk cleanup retains IPC serialization.

use super::*;

pub(crate) mod tests;

#[must_use]
struct LeasedRevocation {
    namespaces: [Option<AddressSpaceOperation>; 2],
    loan: Option<LoanRevocation>,
}

impl LeasedRevocation {
    fn prepare(
        owner: AddressSpaceId,
        cap: MemoryObjectCap,
        borrower: AddressSpaceId,
        borrower_cap: MemoryObjectCap,
    ) -> Result<Self, MemoryObjectError> {
        let mut operation = Self {
            namespaces: [None, None],
            loan: None,
        };
        let result = (|| {
            for (index, asid) in [owner, borrower].into_iter().enumerate() {
                let handle = super::super::current_address_space_handle(asid)
                    .ok_or(MemoryObjectError::AddressSpaceMissing)?;
                operation.namespaces[index] =
                    Some(AddressSpaceOperation::acquire(handle).map_err(operation_error)?);
            }
            operation.loan = Some(LoanRevocation::prepare(owner, cap, borrower, borrower_cap)?);
            Ok(())
        })();
        if let Err(error) = result {
            operation.release_namespaces()?;
            return Err(error);
        }
        Ok(operation)
    }

    fn finish_with(
        mut self,
        finish: impl FnOnce(LoanRevocation) -> Result<(), MemoryObjectError>,
    ) -> Result<(), MemoryObjectError> {
        let result = finish(self.loan.take().expect("prepared loan revocation missing"));
        self.release_namespaces()?;
        result
    }

    fn release_namespaces(self) -> Result<(), MemoryObjectError> {
        assert!(self.loan.is_none(), "releasing roots before loan completion");
        let mut error = None;
        for namespace in self.namespaces.into_iter().flatten() {
            if let Err(release_error) = namespace.release() {
                error.get_or_insert(operation_error(release_error));
            }
        }
        error.map_or(Ok(()), Err)
    }
}

fn operation_error(error: OperationError) -> MemoryObjectError {
    match error {
        OperationError::Closing => MemoryObjectError::AddressSpaceClosing,
        OperationError::Limit => MemoryObjectError::ResourceLimit,
        _ => MemoryObjectError::AddressSpaceMissing,
    }
}

/// Owns the existing borrower list and backing fence after publication of
/// Revoking. Drop leaves that state and its pin in the registry. It never
/// restores authority, releases scratch or invalidates under unknown guards.
#[must_use]
pub(crate) struct LoanRevocation {
    owner: AddressSpaceId,
    owner_cap: MemoryObjectCap,
    borrower: AddressSpaceId,
    borrower_cap: MemoryObjectCap,
    pin: MappingRetirementPin,
    prior: LendState,
    mapping: Option<MemoryMappingState>,
    pages: usize,
}

impl LoanRevocation {
    /// Caller holds leases for both namespaces or the complete IPC write
    /// serialization (the kernel namespace is permanent). No table guard or
    /// lifecycle acquisition occurs here.
    pub(crate) fn prepare(
        owner: AddressSpaceId,
        cap: MemoryObjectCap,
        borrower: AddressSpaceId,
        borrower_cap: MemoryObjectCap,
    ) -> Result<Self, MemoryObjectError> {
        let mut registry = MEMORY_OBJECTS.lock();
        let cap_entry = registry.lookup(owner, cap)?;
        let object =
            registry.objects.get(&cap_entry.object).ok_or(MemoryObjectError::UnknownCapability)?;
        if object.owner != owner {
            return Err(MemoryObjectError::WrongOwner);
        }
        if object.destroy_when_unpinned || object.retirement_pins != 0 {
            return Err(MemoryObjectError::LendingActive);
        }
        match &object.lend_state {
            LendState::None => return Err(MemoryObjectError::NotLent),
            LendState::Revoking => return Err(MemoryObjectError::LendingActive),
            LendState::Read {
                borrowers,
            } if borrowers.get(&borrower) == Some(&borrower_cap) => {}
            LendState::Write {
                borrower: target,
                cap: lent,
            } if *target == borrower && *lent == borrower_cap => {}
            _ => return Err(MemoryObjectError::UnknownCapability),
        }
        if registry.lookup(borrower, borrower_cap)?.object != cap_entry.object {
            return Err(MemoryObjectError::UnknownCapability);
        }
        let pin = MappingRetirementPin::acquire(&mut registry, cap_entry.object);
        let object = registry.objects.get_mut(&cap_entry.object).unwrap();
        Ok(Self {
            owner,
            owner_cap: cap,
            borrower,
            borrower_cap,
            pin,
            prior: core::mem::replace(&mut object.lend_state, LendState::Revoking),
            mapping: object.mappings.get(&borrower).copied(),
            pages: object.frames.len(),
        })
    }

    /// Roll back admission only: this receipt has not started detachment.
    /// `finish` consumes it, so uncertain cleanup cannot use this restoration.
    pub(crate) fn cancel_prepared(mut self) {
        let mut registry = MEMORY_OBJECTS.lock();
        let object = registry.objects.get_mut(&self.pin.object).expect("prepared loan missing");
        assert!(matches!(object.lend_state, LendState::Revoking));
        assert_eq!(object.mappings.get(&self.borrower).copied(), self.mapping);
        object.lend_state = core::mem::replace(&mut self.prior, LendState::None);
        drop(registry);
        self.pin.release(None);
    }

    pub(crate) fn finish(self) -> Result<(), MemoryObjectError> {
        self.finish_observed(|| {})
    }

    /// Boot fixtures inspect each unlocked physical-cleanup boundary; the
    /// ordinary path supplies no callback work.
    pub(crate) fn finish_observed(self, checkpoint: impl FnMut()) -> Result<(), MemoryObjectError> {
        let checkpoint = core::cell::RefCell::new(checkpoint);
        self.finish_with(
            |asid, base, frames| {
                checkpoint.borrow_mut()();
                unmap_pages(asid, base, frames)
            },
            |asid, base, pages| {
                checkpoint.borrow_mut()();
                crate::cpu::isa::memory::tlb::inval_range_user(asid, base, pages);
                true
            },
            |asid, base, pages| {
                checkpoint.borrow_mut()();
                release_scratch(asid, base, pages)
            },
        )
    }

    fn finish_with(
        mut self,
        unmap: impl FnMut(AddressSpaceId, VAddr, &[PAddr]) -> Result<(), MemoryObjectError>,
        mut invalidate: impl FnMut(AddressSpaceId, VAddr, usize) -> bool,
        mut release: impl FnMut(AddressSpaceId, VAddr, usize) -> Result<(), MemoryObjectError>,
    ) -> Result<(), MemoryObjectError> {
        if let Some(mapping) = self.mapping {
            let detached =
                self.pin.unmap_with(self.borrower, mapping.base, mapping.installed_pages, unmap);
            // Even partial detachment needs invalidation. Uncertain cleanup
            // retains the pin, borrower capability and Revoking authority fence.
            let quiescent = invalidate(self.borrower, mapping.base, self.pages);
            detached?;
            if !quiescent {
                return Err(MemoryObjectError::UnmapFailed);
            }
            if mapping.scratch {
                release(self.borrower, mapping.base, self.pages)?;
            }
        }

        let mut registry = MEMORY_OBJECTS.lock();
        // The leases (or IPC guard), Revoking state and pin exclude namespace
        // cleanup, capability removal and mapping mutation. No fallible work
        // may follow successful scratch release: these are ownership invariants,
        // not a new lookup through a reusable namespace.
        assert_eq!(
            registry.lookup(self.owner, self.owner_cap).expect("revoking owner cap missing").object,
            self.pin.object,
        );
        assert_eq!(
            registry
                .lookup(self.borrower, self.borrower_cap)
                .expect("revoking borrower cap missing")
                .object,
            self.pin.object,
        );
        let object = registry.objects.get_mut(&self.pin.object).expect("revoking object missing");
        assert!(matches!(object.lend_state, LendState::Revoking), "loan revocation fence lost");
        assert_eq!(
            object.mappings.get(&self.borrower).copied(),
            self.mapping,
            "revoking mapping changed"
        );
        object.mappings.remove(&self.borrower);
        object.lend_state = match &mut self.prior {
            LendState::Read {
                borrowers,
            } => {
                borrowers.remove(&self.borrower);
                if borrowers.is_empty() {
                    LendState::None
                } else {
                    self.prior
                }
            }
            LendState::Write {
                ..
            } => LendState::None,
            LendState::None | LendState::Revoking => unreachable!(),
        };
        registry.caps.get_mut(&self.borrower).unwrap().caps.remove(&self.borrower_cap);
        assert!(
            crate::capability::remove(
                self.borrower,
                self.borrower_cap,
                crate::capability::ObjectKind::Memory,
            ),
            "borrower capability was absent from unified table"
        );
        drop(registry);
        self.pin.release(None);
        Ok(())
    }
}

pub(super) fn revoke(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
    borrower_cap: MemoryObjectCap,
) -> Result<(), MemoryObjectError> {
    LeasedRevocation::prepare(owner, cap, borrower, borrower_cap)?
        .finish_with(LoanRevocation::finish)
}

pub(super) fn revoke_serialized(
    owner: AddressSpaceId,
    cap: MemoryObjectCap,
    borrower: AddressSpaceId,
    borrower_cap: MemoryObjectCap,
) -> Result<(), MemoryObjectError> {
    LoanRevocation::prepare(owner, cap, borrower, borrower_cap)?.finish()
}
