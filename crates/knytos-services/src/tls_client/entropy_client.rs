//! Bounded entropy requests with complete pending/reply/mapping ownership.

use catten_rt::owned::{
    CallResult,
    Connection,
    ConnectionRef,
    IpcError,
    MappedMemory,
    MemoryError,
    OwnedMemory,
    PendingCall,
    ReadOnly,
};
use zeroize::Zeroizing;

use super::runtime::{
    EntropyContext,
    Unavailable,
};
use crate::{
    deadline::Deadline,
    entropy,
};

/// Related entropy admission/observation settings, installed with the source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EntropyLimits {
    /// One total budget, captured before preparation; no retry restarts it.
    pub request_timeout_ms: u64,
}

impl EntropyLimits {
    /// Compile-time default; startup may supply another positive budget.
    pub const DEFAULT: Self = Self {
        request_timeout_ms: 5_000,
    };

    const fn valid(self) -> bool {
        self.request_timeout_ms != 0
    }
}

/// Startup consumes this complete configuration once. `None` explicitly
/// selects architectural-only entropy, without adopting launch-owned handles.
pub struct EntropyConfig {
    pub service: Option<Connection>,
    pub limits: EntropyLimits,
}

pub(super) enum EntropySource {
    System(EntropyConfig),
    #[cfg(test)]
    Host,
}

pub(super) static DOMAIN_ENTROPY: EntropyContext<EntropySource> = EntropyContext::new();

/// A rejected installation returns its complete new configuration. Invalid
/// limits and duplicate installation never mutate the installed source/policy.
pub fn install_domain_entropy(config: EntropyConfig) -> Result<(), EntropyConfig> {
    if !config.limits.valid() {
        return Err(config);
    }
    DOMAIN_ENTROPY.install(EntropySource::System(config)).map_err(|source| match source {
        EntropySource::System(config) => config,
        #[cfg(test)]
        EntropySource::Host => unreachable!("installation returns its new input only"),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntropyError {
    Uninstalled,
    Busy,
    InvalidLimits,
    InvalidLength,
    NoService,
    /// Budget/clock rejected before this request was submitted.
    ExpiredBeforeAdmission,
    Preparation(MemoryError),
    Submission(IpcError),
    /// Includes exact wait-admission rejection; never interpreted as timeout.
    Observation(IpcError),
    /// The request was accepted. Timeout grants no completion/replay proof.
    ObservationTimeout,
    Rejected(i64),
    InvalidReply,
    Mapping(MemoryError),
}

/// Source checkout retains the installed owner/policy through the entire read.
/// Its metadata lock is released before any syscall, wait, mapping or copy.
pub fn fill_domain_entropy(destination: &mut [u8]) -> Result<(), EntropyError> {
    let lease = DOMAIN_ENTROPY.checkout().map_err(|error| match error {
        Unavailable::Uninstalled => EntropyError::Uninstalled,
        Unavailable::Busy => EntropyError::Busy,
    })?;
    match lease.source() {
        #[cfg(test)]
        EntropySource::Host => getrandom::fill(destination).map_err(|_| EntropyError::NoService),
        EntropySource::System(config) => fill_entropy(
            config.service.as_ref().map(Connection::as_ref),
            destination,
            config.limits,
        ),
    }
}

/// Fill from architectural entropy, then one explicit service request for any
/// remainder. The same deadline includes preparation, admission and observation.
/// TCP/IP uses this independent borrowed-source path without installing TLS.
pub fn fill_entropy(
    service: Option<ConnectionRef<'_>>,
    destination: &mut [u8],
    limits: EntropyLimits,
) -> Result<(), EntropyError> {
    if !limits.valid() {
        return Err(EntropyError::InvalidLimits);
    }
    if destination.is_empty() {
        return Ok(());
    }
    let deadline = Deadline::after(limits.request_timeout_ms);
    if deadline.expired() {
        return Err(EntropyError::ExpiredBeforeAdmission);
    }
    let mut offset = 0;
    while offset < destination.len() {
        let Some(word) = catten_syscall::random_u64() else {
            break;
        };
        let bytes = Zeroizing::new(word.to_ne_bytes());
        let length = (destination.len() - offset).min(bytes.len());
        destination[offset..offset + length].copy_from_slice(&bytes[..length]);
        offset += length;
    }
    if deadline.expired() {
        return Err(EntropyError::ExpiredBeforeAdmission);
    }
    if offset == destination.len() {
        return Ok(());
    }
    fill_service_entropy(
        service.ok_or(EntropyError::NoService)?,
        &mut destination[offset..],
        deadline,
    )
}

/// Service-only client with a caller's original deadline. No architecture
/// shortcut, queue retry or accepted-request replay is performed.
pub fn fill_service_entropy(
    service: ConnectionRef<'_>,
    destination: &mut [u8],
    deadline: Deadline,
) -> Result<(), EntropyError> {
    ServiceEntropyRead::start(service, destination, deadline)?.finish()
}

/// One admitted scalar request owns its pending call, returned resources and
/// mapping while borrowing the original source and output. Fields stay private.
/// Default field Drop cancels pending work before ending those Rust borrows;
/// the runtime aborts the domain if cancellation cannot safely complete.
#[must_use = "dropping a service entropy read cancels its exact pending request"]
pub struct ServiceEntropyRead<'service, 'output> {
    pending: Option<PendingCall<'static>>,
    buffer: Option<OwnedMemory>,
    destination: &'output mut [u8],
    _service: ConnectionRef<'service>,
    deadline: Deadline,
}

impl<'service, 'output> ServiceEntropyRead<'service, 'output> {
    pub fn start(
        service: ConnectionRef<'service>,
        destination: &'output mut [u8],
        deadline: Deadline,
    ) -> Result<Self, EntropyError> {
        if destination.is_empty() || destination.len() > entropy::MAX_REQUEST {
            return Err(EntropyError::InvalidLength);
        }
        if deadline.expired() {
            return Err(EntropyError::ExpiredBeforeAdmission);
        }
        let mut read = Self {
            pending: None,
            buffer: None,
            destination,
            _service: service,
            deadline,
        };
        read.buffer =
            Some(OwnedMemory::allocate(entropy::BUFFER_PAGES).map_err(EntropyError::Preparation)?);
        if read.deadline.expired() {
            return Err(EntropyError::ExpiredBeforeAdmission);
        }
        read.pending = Some(
            service
                .call_move(
                    entropy::OP_FILL,
                    read.destination.len() as u64,
                    read.buffer.take().expect("prepared caller buffer"),
                )
                .map_err(|(memory, error)| {
                    read.buffer = Some(memory);
                    EntropyError::Submission(error)
                })?,
        );
        Ok(read)
    }

    /// Observe once within the remaining budget. On ordinary failure this
    /// consuming owner cancels/releases through its existing runtime owners.
    /// A ready but late result is also retained for disposal, never published.
    pub fn finish(self) -> Result<(), EntropyError> {
        CompletingEntropyRead {
            read: self,
            reply: None,
            mapping: None,
        }
        .finish()
    }

    /// Explicit cancellation before observation. Rejection returns the complete
    /// operation and both borrows; it supplies no cleanup or replay authority.
    pub fn cancel(mut self) -> Result<(), (Self, IpcError)> {
        let pending = self.pending.take().expect("unobserved entropy request");
        match pending.close() {
            Ok(()) => Ok(()),
            Err((pending, error)) => {
                self.pending = Some(pending);
                Err((self, error))
            }
        }
    }
}

// Completion owns the original operation and all newly returned resources.
// The cancellable public phase needs no reply/mapping storage or heap allocation.
struct CompletingEntropyRead<'service, 'output> {
    read: ServiceEntropyRead<'service, 'output>,
    reply: Option<CallResult>,
    mapping: Option<MappedMemory<ReadOnly>>,
}

impl CompletingEntropyRead<'_, '_> {
    fn finish(mut self) -> Result<(), EntropyError> {
        let remaining = self.read.deadline.remaining_ms();
        if remaining == 0 {
            return Err(EntropyError::ObservationTimeout);
        }
        self.reply = self
            .read
            .pending
            .as_mut()
            .expect("one pending request")
            .wait_timeout(remaining)
            .map_err(EntropyError::Observation)?;
        let reply = self.reply.as_mut().ok_or(EntropyError::ObservationTimeout)?;
        if self.read.deadline.expired() {
            return Err(EntropyError::ObservationTimeout);
        }
        if reply.connection.is_some() {
            return Err(EntropyError::InvalidReply);
        }
        if reply.result < 0 {
            return Err(EntropyError::Rejected(reply.result));
        }
        if reply.result != self.read.destination.len() as i64 {
            return Err(EntropyError::InvalidReply);
        }
        let memory = reply.memory.take().ok_or(EntropyError::InvalidReply)?;
        if memory.len() != entropy::BUFFER_BYTES {
            reply.memory = Some(memory);
            return Err(EntropyError::InvalidReply);
        }
        self.mapping = Some(match memory.map_read_only() {
            Ok(mapping) => mapping,
            Err((memory, error)) => {
                // Keep the returned owner in this transaction through failure.
                reply.memory = Some(memory);
                return Err(EntropyError::Mapping(error));
            }
        });
        if self.read.deadline.expired() {
            return Err(EntropyError::ObservationTimeout);
        }
        let input = self
            .mapping
            .as_ref()
            .expect("mapped entropy reply")
            .as_slice()
            .get(..self.read.destination.len())
            .ok_or(EntropyError::InvalidReply)?;
        self.read.destination.copy_from_slice(input);
        if self.read.deadline.expired() {
            use zeroize::Zeroize;
            self.read.destination.zeroize();
            return Err(EntropyError::ObservationTimeout);
        }
        Ok(())
    }
}

pub(super) struct SystemRng;
impl embassy_crypto::driver::Rng for SystemRng {
    fn fill_bytes(destination: &mut [u8]) {
        // The callback cannot return failure. No accepted request is replayed,
        // and a rejected wait is not converted to a timeout or fabricated bytes.
        fill_domain_entropy(destination).expect("system entropy unavailable during TLS crypto");
    }
}
embassy_crypto::rng_impl!(SystemRng);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_configuration_returns_the_complete_input_without_installation() {
        let invalid = EntropyConfig {
            service: None,
            limits: EntropyLimits {
                request_timeout_ms: 0,
            },
        };
        let returned = install_domain_entropy(invalid).err().expect("reject zero budget");
        assert!(returned.service.is_none());
        assert_eq!(returned.limits.request_timeout_ms, 0);
        assert_eq!(
            fill_entropy(None, &mut [0; 1], returned.limits),
            Err(EntropyError::InvalidLimits)
        );
    }

    #[test]
    fn empty_architectural_fill_requires_no_clock_or_service() {
        assert_eq!(fill_entropy(None, &mut [], EntropyLimits::DEFAULT), Ok(()));
    }
}
