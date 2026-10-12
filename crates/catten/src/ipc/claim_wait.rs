//! Readiness of one exact IPC reply/cancellation or endpoint-close claim.
//! A wake permits revalidation, never replay of uncertain loan cleanup.

use core::mem::ManuallyDrop;

use super::*;
use crate::{
    cpu::scheduler::{
        self,
        deferred,
        threads::MigrationConstraint,
    },
    klib::collections::retirement_list::PreparedEntry,
    memory::{
        AddressSpaceHandle,
        KERNEL_ASID,
        operation::AddressSpaceOperation,
    },
};

#[derive(Debug)]
struct SourcePayload {
    list: WaiterList,
    disposal: PreparedEntry<WaiterList>,
}

/// Retain both admitted allocations on fallback, including beneath IPC.
#[derive(Debug)]
pub(super) struct Source(ManuallyDrop<SourcePayload>);

impl Source {
    fn prepare(platform: bool) -> Result<Self, IpcError> {
        let disposal = PreparedEntry::try_new().map_err(|_| IpcError::ResourceLimit)?;
        let list = ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
            .map_err(|_| IpcError::ResourceLimit)?;
        Ok(Self(ManuallyDrop::new(SourcePayload {
            list,
            disposal,
        })))
    }

    fn interest(&self) -> Interest {
        Interest(self.0.list.clone())
    }

    fn cancel(self) {
        drop(ManuallyDrop::into_inner(self.0));
    }

    /// The caller clears/removes the exact claim under IPC first. Deferred
    /// transport owns notifications and final list disposal until guard exit.
    pub(super) fn finish(self) {
        let payload = ManuallyDrop::into_inner(self.0);
        deferred::publish(payload.list.close(), Some(payload.disposal.publish(payload.list)));
    }
}

pub(super) fn finish(source: Option<Source>) {
    if let Some(source) = source {
        source.finish();
    }
}

pub(super) struct Interest(WaiterList);

impl Interest {
    fn completed(&self) -> bool {
        self.0.is_closed()
    }

    pub(super) fn wait(&self) -> Result<(), IpcError> {
        scheduler::try_park_until(self, MigrationConstraint::GeneralWait, || self.completed())
            .map_err(|_| IpcError::ResourceLimit)
    }
}

impl Observable for Interest {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        if self.completed() {
            return Ok(WaitRegistration::ready());
        }
        match sponsor.register(&self.0, observer) {
            Err(RegistrationError::Closed) => Ok(WaitRegistration::ready()),
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Claim {
    Reply(ReplyTokenId, u64),
    Endpoint(EndpointId, u64),
}

impl Claim {
    fn source(self, ipc: &mut IpcRegistry) -> Option<(&mut Option<Source>, bool)> {
        match self {
            Self::Reply(id, epoch) => ipc
                .reply_tokens
                .get_mut(&id)
                .filter(|token| token.completing && token.claim_epoch == epoch)
                .map(|token| (&mut token.claim_interest, token._charge.platform())),
            Self::Endpoint(id, epoch) => ipc
                .endpoints
                .get_mut(&id)
                .filter(|endpoint| endpoint.closing && endpoint.claim_epoch == epoch)
                .map(|endpoint| {
                    (&mut endpoint.claim_interest, endpoint.metadata_charge.platform())
                }),
        }
    }
}

/// Choose one active claim; after it ends the close loop revalidates all others.
/// Token/endpoint IDs are monotonic, and each claim episode owns a fresh source.
fn busy(
    ipc: &IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
    payload: Capability,
) -> Option<Claim> {
    let source = || {
        ipc.reply_tokens.iter().find_map(|(&id, token)| {
            (token.completing && token.server == asid && token.connection_source == Some(cap))
                .then_some(Claim::Reply(id, token.claim_epoch))
        })
    };
    match payload {
        Capability::PendingCall {
            call,
        } => ipc.reply_tokens.iter().find_map(|(&id, token)| {
            (token.call == call && token.completing).then_some(Claim::Reply(id, token.claim_epoch))
        }),
        Capability::ReplyToken {
            token,
        } => ipc
            .reply_tokens
            .get(&token)
            .filter(|token| token.completing)
            .map(|record| Claim::Reply(token, record.claim_epoch)),
        Capability::Connection {
            ..
        } => source(),
        Capability::Endpoint {
            endpoint,
            ..
        } => {
            let endpoint_record = ipc.endpoints.get(&endpoint)?;
            if endpoint_record.closing {
                return Some(Claim::Endpoint(endpoint, endpoint_record.claim_epoch));
            }
            source().or_else(|| {
                endpoint_record.queue.iter().find_map(|message| {
                    let id = message.reply?;
                    ipc.reply_tokens
                        .get(&id)
                        .filter(|token| token.completing)
                        .map(|token| Claim::Reply(id, token.claim_epoch))
                })
            })
        }
    }
}

pub(super) fn is_busy(
    ipc: &IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
    payload: Capability,
) -> bool {
    busy(ipc, asid, cap, payload).is_some()
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    root: Option<AddressSpaceHandle>,
    payload: Capability,
}

fn identity(
    ipc: &IpcRegistry,
    asid: AddressSpaceId,
    cap: CapabilityId,
) -> Result<Identity, IpcError> {
    Ok(Identity {
        root: ipc.caps.get(&asid).and_then(|caps| caps.address_space),
        payload: ipc.cap(asid, cap)?, // Hidden grants never qualify a close wait.
    })
}

fn bind(
    asid: AddressSpaceId,
    cap: CapabilityId,
    captured: Identity,
    claim: Claim,
    mut prepare: impl FnMut(bool) -> Result<Source, IpcError>,
) -> Result<Option<Interest>, IpcError> {
    let mut ipc = IPC.write();
    if identity(&ipc, asid, cap)? != captured {
        return Err(IpcError::UnknownCapability);
    }
    let Some((source, platform)) = claim.source(&mut ipc) else {
        return Ok(None);
    };
    if let Some(source) = source {
        return Ok(Some(source.interest()));
    }
    drop(ipc);
    let mut prepared = Some(prepare(platform)?);
    let result = (|| {
        let mut ipc = IPC.write();
        if identity(&ipc, asid, cap)? != captured {
            Err(IpcError::UnknownCapability)
        } else {
            Ok(claim.source(&mut ipc).map(|(source, _)| {
                if source.is_none() {
                    *source = prepared.take();
                }
                source.as_ref().unwrap().interest()
            }))
        }
    })();
    if let Some(unused) = prepared {
        unused.cancel(); // Never dispose losing storage beneath IPC.
    }
    result
}

struct WaitPayload {
    identity: Option<Identity>,
    root: Option<AddressSpaceOperation>,
    interest: Option<Interest>,
}

/// One contended close retains its captured root and transient interest through
/// the whole close loop. Fallback retains every field and the operation count.
pub(super) struct Wait(ManuallyDrop<WaitPayload>);

impl Wait {
    pub(super) fn new() -> Self {
        Self(ManuallyDrop::new(WaitPayload {
            identity: None,
            root: None,
            interest: None,
        }))
    }

    pub(super) fn wait(&mut self, asid: AddressSpaceId, cap: CapabilityId) -> Result<(), IpcError> {
        self.wait_with(asid, cap, Interest::wait, Source::prepare)
    }

    fn wait_with(
        &mut self,
        asid: AddressSpaceId,
        cap: CapabilityId,
        wait: impl FnOnce(&Interest) -> Result<(), IpcError>,
        prepare: impl FnMut(bool) -> Result<Source, IpcError>,
    ) -> Result<(), IpcError> {
        let (captured, claim) = {
            let ipc = IPC.read();
            let captured = identity(&ipc, asid, cap)?;
            if self.0.identity.is_some_and(|original| original != captured) {
                return Err(IpcError::UnknownCapability);
            }
            let Some(claim) = busy(&ipc, asid, cap, captured.payload) else {
                return Ok(());
            };
            (captured, claim)
        };
        if self.0.identity.is_none() {
            if asid != KERNEL_ASID {
                let handle = captured.root.ok_or(IpcError::UnknownCapability)?;
                self.0.root = Some(
                    AddressSpaceOperation::acquire(handle).map_err(|_| IpcError::ResourceLimit)?,
                );
            }
            self.0.identity = Some(captured);
        }
        self.0.interest = bind(asid, cap, captured, claim, prepare)?;
        if let Some(interest) = &self.0.interest {
            wait(interest)?;
        }
        Ok(())
    }

    pub(super) fn finish(self, result: Result<(), IpcError>) -> Result<(), IpcError> {
        let payload = ManuallyDrop::into_inner(self.0);
        drop(payload.interest);
        if let Some(root) = payload.root {
            root.release().map_err(|_| IpcError::UnknownCapability)?;
        }
        result
    }
}

/// Borrow the endpoint close's existing server lease; never acquire a new one
/// after its close claim. Only the claimed queue front can block this phase.
pub(super) fn front_interest(
    owner: &endpoint_close::PreparedEndpointClose,
) -> Result<Option<Interest>, IpcError> {
    let (captured, claim) = {
        let ipc = IPC.read();
        let captured = identity(&ipc, owner.asid(), owner.cap())?;
        if captured.root != Some(owner.handle()) {
            return Err(IpcError::UnknownCapability);
        }
        let Capability::Endpoint {
            endpoint,
            ..
        } = captured.payload
        else {
            return Err(IpcError::WrongType);
        };
        let record = &ipc.endpoints[&endpoint];
        assert!(record.closing && !record.closed);
        let Some(token) = record.queue.front().and_then(|message| message.reply) else {
            return Ok(None);
        };
        if !ipc.reply_tokens.get(&token).is_some_and(|token| token.completing) {
            return Ok(None);
        }
        (captured, Claim::Reply(token, ipc.reply_tokens[&token].claim_epoch))
    };
    bind(owner.asid(), owner.cap(), captured, claim, Source::prepare)
}

pub(crate) mod tests;
