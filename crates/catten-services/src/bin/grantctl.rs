//! Trusted capability-grant controller.
//!
//! An application receives only a connection to this endpoint plus its
//! immutable signed deployment descriptor. The controller asks the kernel to
//! attest that this exact descriptor was admitted for the caller's occupancy,
//! checks that it permits the requested service, then asks the private name
//! service to mint a
//! re-delegable connection. The reply attenuates it back to application
//! SEND/CALL rights; name-service authority and connector secrets never cross
//! this boundary.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use catten_rt::{
    Context,
    owned::{
        Endpoint,
        IncomingMessage,
        OwnedMemory,
        PendingCall,
    },
};
use catten_services::{
    grant,
    ns,
};
use catten_syscall::IpcRights;
use charlotte_authorization::{
    AuthorizationRights,
    wire,
};

catten_rt::entry!(main);

fn reply_error(message: &mut IncomingMessage, error: i64) {
    if let Some(reply) = message.reply.take() {
        let _ = reply.reply(error);
    }
}

fn authorized_request<'a>(
    message: &IncomingMessage,
    bytes: &'a [u8],
    publish: bool,
) -> Option<grant::AcquireRequest<'a>> {
    let request = grant::decode_request(bytes)?;
    if publish {
        if request.rights != charlotte_launch::deployment::RIGHT_PUBLISH {
            return None;
        }
    } else if request.rights & !charlotte_launch::deployment::CLIENT_RIGHTS != 0 {
        return None;
    }
    // Signature verification used the configured deployment key at launch.
    // A valid signature alone is insufficient: bind this exact policy to the
    // actual image/occupancy, including after controller restart or ASID reuse.
    if !catten_syscall::launch_descriptor_matches(
        message.sender,
        message.sender_generation,
        &charlotte_launch::sha256::digest(request.descriptor),
    ) {
        return None;
    }
    let descriptor = charlotte_launch::deployment::decode(request.descriptor)?;
    if charlotte_launch::artifact_principal_id(descriptor.artifact_name) != message.sender_principal
    {
        return None;
    }
    let allowed = descriptor.grants().any(|grant| {
        grant.service == request.service && grant.rights & request.rights == request.rights
    });
    if !allowed {
        return None;
    }
    Some(request)
}

fn authorization_memory(
    message: &IncomingMessage,
    request: grant::AcquireRequest<'_>,
) -> Result<(OwnedMemory, usize), ()> {
    let rights = AuthorizationRights::from_bits(u32::from(request.rights)).ok_or(())?;
    let memory = OwnedMemory::allocate(1).map_err(|_| ())?;
    let mut mapping = memory.map_writable().map_err(|_| ())?;
    let len = wire::encode_grant_lookup(
        request.service,
        rights,
        message.sender,
        message.sender_generation,
        message.sender_principal,
        mapping.as_mut_slice(),
    )
    .ok_or(())?;
    let memory = mapping.unmap().map_err(|_| ())?;
    Ok((memory, len))
}

fn publication_memory(request: grant::AcquireRequest<'_>) -> Result<(OwnedMemory, usize), ()> {
    let memory = OwnedMemory::allocate(1).map_err(|_| ())?;
    let mut mapping = memory.map_writable().map_err(|_| ())?;
    let len =
        wire::encode_publish(request.service, AuthorizationRights::CLIENT, mapping.as_mut_slice())
            .ok_or(())?;
    let memory = mapping.unmap().map_err(|_| ())?;
    Ok((memory, len))
}

fn submit(
    message: &mut IncomingMessage,
    name_service: catten_rt::owned::ConnectionRef<'_>,
) -> Result<(PendingCall<'static>, bool, IpcRights), i64> {
    let publish = message.opcode == grant::OP_PUBLISH;
    if (message.opcode != grant::OP_ACQUIRE && !publish)
        || message.reply.is_none()
        || (publish != message.connection.is_some())
    {
        return Err(grant::ERR_INVALID);
    }
    let Some(memory) = message.memory.take() else {
        return Err(grant::ERR_INVALID);
    };
    let Ok(len) = usize::try_from(message.arg0) else {
        return Err(grant::ERR_INVALID);
    };
    let Ok(mapping) = memory.map_read_only() else {
        return Err(grant::ERR_INVALID);
    };
    let Some(bytes) = mapping.as_slice().get(..len) else {
        return Err(grant::ERR_INVALID);
    };
    let Some(request) = authorized_request(message, bytes, publish) else {
        return Err(grant::ERR_UNAUTHORIZED);
    };
    let rights = IpcRights::from_bits(u32::from(request.rights));
    if publish {
        let Some(endpoint_connection) = message.connection.as_ref() else {
            return Err(grant::ERR_INVALID);
        };
        let Ok((authorization, authorization_len)) = publication_memory(request) else {
            return Err(grant::ERR_INVALID);
        };
        let pending = match name_service.call_delegated_connection_copy(
            ns::OP_REGISTER_AUTHORIZED,
            authorization_len as u64,
            endpoint_connection.as_ref(),
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
            &authorization,
        ) {
            Ok(pending) => pending,
            Err(_) => return Err(grant::ERR_UNAVAILABLE),
        };
        return Ok((pending, true, rights));
    }

    let Ok((authorization, authorization_len)) = authorization_memory(message, request) else {
        return Err(grant::ERR_INVALID);
    };
    let pending = match name_service.call_move(
        ns::OP_TRY_LOOKUP_FOR_GRANT,
        authorization_len as u64,
        authorization,
    ) {
        Ok(pending) => pending,
        Err((_authorization, _error)) => return Err(grant::ERR_UNAVAILABLE),
    };
    Ok((pending, false, rights))
}

/// Dropping one operation cancels its lookup and releases all local resources.
struct PendingGrant {
    message: IncomingMessage,
    call: PendingCall<'static>,
    publish: bool,
    rights: IpcRights,
    deadline: catten_services::deadline::Deadline,
}

impl PendingGrant {
    fn poll(&mut self) -> bool {
        if self.deadline.expired() {
            reply_error(&mut self.message, grant::ERR_UNAVAILABLE);
            return true;
        }
        match self.call.poll() {
            Ok(None) => false,
            Ok(Some(result)) if result.result >= 1 && result.memory.is_none() => {
                if self.publish && result.connection.is_none() {
                    if let Some(reply) = self.message.reply.take() {
                        let _ = reply.reply(result.result);
                    }
                } else if !self.publish && result.connection.is_some() {
                    if let Some(reply) = self.message.reply.take() {
                        let _ = reply.reply_connection_ref(
                            result.connection.as_ref().unwrap().as_ref(),
                            self.rights,
                            result.result,
                        );
                    }
                } else {
                    reply_error(&mut self.message, grant::ERR_UNAVAILABLE);
                }
                true
            }
            _ => {
                reply_error(&mut self.message, grant::ERR_UNAVAILABLE);
                true
            }
        }
    }
}

fn main(ctx: Context) -> ! {
    let endpoint_cap = ctx.bootstrap_cap().unwrap_or_else(|| catten_rt::domain_abort());
    // Ownership transfers exactly once from the typed launch bootstrap slot.
    let endpoint =
        unsafe { Endpoint::from_raw(endpoint_cap) }.unwrap_or_else(|_| catten_rt::domain_abort());
    let name_service = ctx.name_service_connection().unwrap_or_else(|| catten_rt::domain_abort());
    let mut pending: Vec<PendingGrant> = Vec::new();
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            drop(pending);
            drop(endpoint);
            request.complete();
        }
        pending.retain_mut(|operation| !operation.poll());
        // Bound intake per cycle as well as outstanding work, so a busy client
        // cannot prevent polling existing operations and lifecycle requests.
        for _ in 0..16 {
            let mut message = match endpoint.try_receive_authenticated() {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(_) => catten_rt::domain_abort(),
            };
            let owner_pending = pending
                .iter()
                .filter(|operation| {
                    operation.message.sender == message.sender
                        && operation.message.sender_generation == message.sender_generation
                })
                .count();
            if pending.len() >= 32 || owner_pending >= 4 {
                reply_error(&mut message, grant::ERR_UNAVAILABLE);
                continue;
            }
            match submit(&mut message, name_service) {
                Ok((call, publish, rights)) => pending.push(PendingGrant {
                    message,
                    call,
                    publish,
                    rights,
                    deadline: catten_services::deadline::Deadline::after(5_000),
                }),
                Err(error) => reply_error(&mut message, error),
            }
        }
        catten_services::sleep_ms(5);
    }
}
