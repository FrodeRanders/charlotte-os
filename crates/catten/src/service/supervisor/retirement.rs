//! Supervisor-owned close policy, separate from memory's linear close authority.
//! Pending polls retain that authority; terminal errors never imply reclamation.

use super::{
    ServiceDomain,
    domain_exited,
    forget_retired_service_roles,
};
use crate::memory::{
    self,
    AddressSpaceCloseError,
    retirement::{
        CloseProgress,
        ClosingAddressSpace,
    },
};

const RECLAMATION_GRACE_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainTeardownError {
    ThreadsNotQuiescent,
    ThreadAbortRejected,
    AddressSpace(AddressSpaceCloseError),
}

/// One owner per retiring domain. No close authority is cloned with a handle.
/// Drop abandons any staged fence; it never waits or destroys uncertain backing.
#[must_use]
pub(crate) struct DomainTeardown {
    domain: ServiceDomain,
    deadline_ms: u64,
    closing: Option<ClosingAddressSpace>,
    complete: bool,
    failed: Option<DomainTeardownError>,
}

impl DomainTeardown {
    pub(crate) fn new(domain: ServiceDomain) -> Self {
        Self::with_deadline(
            domain,
            crate::cpu::scheduler::monotonic_millis().saturating_add(RECLAMATION_GRACE_MS),
        )
    }

    fn with_deadline(domain: ServiceDomain, deadline_ms: u64) -> Self {
        Self {
            domain,
            deadline_ms,
            closing: None,
            complete: false,
            failed: None,
        }
    }

    /// No scheduler wait. Caller must release registry/coordinator guards first.
    /// The deadline bounds thread/lease drain, not hardware rendezvous or cleanup.
    pub(crate) fn poll(&mut self) -> Result<bool, DomainTeardownError> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        if self.complete {
            return Ok(true);
        }
        let result = self.advance();
        if let Err(error) = result {
            // Drop retains a staged closing root, including when an older
            // operation subsequently completes. Never retry under a new owner.
            self.closing.take();
            self.failed = Some(error);
        }
        result
    }

    fn advance(&mut self) -> Result<bool, DomainTeardownError> {
        if self.closing.is_none() {
            match memory::current_address_space_handle(self.domain.asid) {
                Some(handle) if handle == self.domain.address_space => {}
                Some(_) => {
                    return Err(DomainTeardownError::AddressSpace(
                        AddressSpaceCloseError::StaleHandle,
                    ));
                }
                None => {
                    return Err(DomainTeardownError::AddressSpace(
                        AddressSpaceCloseError::AddressSpaceMissing,
                    ));
                }
            }
            // Includes staged/on-CPU thread reaping and the conservative
            // node-wide retirement epoch. A transient overlap is just pending.
            if !domain_exited(&self.domain) {
                if crate::cpu::scheduler::monotonic_millis() >= self.deadline_ms {
                    return Err(DomainTeardownError::ThreadsNotQuiescent);
                }
                return Ok(false);
            }
            self.closing = Some(
                ClosingAddressSpace::begin(self.domain.address_space)
                    .map_err(DomainTeardownError::AddressSpace)?,
            );
        }
        let closing = self.closing.take().expect("staged supervisor close missing");
        match closing.poll().map_err(DomainTeardownError::AddressSpace)? {
            CloseProgress::Complete => {
                forget_retired_service_roles(self.domain);
                self.complete = true;
                Ok(true)
            }
            CloseProgress::Pending(closing) => {
                self.closing = Some(closing);
                if crate::cpu::scheduler::monotonic_millis() >= self.deadline_ms {
                    return Err(DomainTeardownError::AddressSpace(
                        AddressSpaceCloseError::OperationDrainTimedOut,
                    ));
                }
                Ok(false)
            }
        }
    }
}

/// A registry entry stays admitted while a poll temporarily borrows its owner.
/// Polling prevents another caller from beginning/stealing the same close.
pub(crate) enum DeploymentTeardown {
    NotStarted,
    Pending(DomainTeardown),
    Polling,
    Failed(DomainTeardownError),
}

pub(crate) mod tests;
