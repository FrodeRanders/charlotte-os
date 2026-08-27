//! Trusted capability-grant controller.
//!
//! An application receives only a connection to this endpoint plus its
//! immutable signed deployment descriptor. The controller verifies that the
//! descriptor names the kernel-authenticated caller principal and permits the
//! requested service, then asks the private name service to mint a
//! re-delegable connection. The reply attenuates it back to application
//! SEND/CALL rights; name-service authority and connector secrets never cross
//! this boundary.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;

use catten_rt::{
    Context,
    owned::{
        Endpoint,
        IncomingMessage,
        IpcError,
        OwnedMemory,
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

#[derive(Clone, Copy)]
struct AcceptedRevision {
    sequence: u64,
    digest: [u8; 32],
}

fn reply_error(message: &mut IncomingMessage, error: i64) {
    if let Some(reply) = message.reply.take() {
        let _ = reply.reply(error);
    }
}

fn authorized_request<'a>(
    message: &IncomingMessage,
    bytes: &'a [u8],
    revisions: &mut BTreeMap<u64, AcceptedRevision>,
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
    if charlotte_launch::deployment::verify(
        request.descriptor,
        &charlotte_launch::CLUSTER_PUBLIC_KEY,
    ) != charlotte_launch::deployment::VerifyOutcome::Valid
    {
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
    let digest = charlotte_launch::sha256::digest(request.descriptor);
    match revisions.get(&message.sender_principal) {
        Some(previous) if descriptor.sequence < previous.sequence => return None,
        Some(previous) if descriptor.sequence == previous.sequence && digest != previous.digest => {
            return None;
        }
        Some(_) => {}
        None => {}
    }
    revisions.insert(
        message.sender_principal,
        AcceptedRevision {
            sequence: descriptor.sequence,
            digest,
        },
    );
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

fn handle(
    mut message: IncomingMessage,
    name_service: catten_rt::owned::ConnectionRef<'_>,
    revisions: &mut BTreeMap<u64, AcceptedRevision>,
) {
    let publish = message.opcode == grant::OP_PUBLISH;
    if (message.opcode != grant::OP_ACQUIRE && !publish)
        || message.reply.is_none()
        || (publish != message.connection.is_some())
    {
        reply_error(&mut message, grant::ERR_INVALID);
        return;
    }
    let Some(memory) = message.memory.take() else {
        reply_error(&mut message, grant::ERR_INVALID);
        return;
    };
    let Ok(len) = usize::try_from(message.arg0) else {
        reply_error(&mut message, grant::ERR_INVALID);
        return;
    };
    let Ok(mapping) = memory.map_read_only() else {
        reply_error(&mut message, grant::ERR_INVALID);
        return;
    };
    let Some(bytes) = mapping.as_slice().get(..len) else {
        reply_error(&mut message, grant::ERR_INVALID);
        return;
    };
    let Some(request) = authorized_request(&message, bytes, revisions, publish) else {
        reply_error(&mut message, grant::ERR_UNAUTHORIZED);
        return;
    };
    if publish {
        let Some(endpoint_connection) = message.connection.take() else {
            reply_error(&mut message, grant::ERR_INVALID);
            return;
        };
        let Ok((authorization, authorization_len)) = publication_memory(request) else {
            reply_error(&mut message, grant::ERR_INVALID);
            return;
        };
        let pending = match name_service.call_delegated_connection_copy(
            ns::OP_REGISTER_AUTHORIZED,
            authorization_len as u64,
            endpoint_connection.as_ref(),
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
            &authorization,
        ) {
            Ok(pending) => pending,
            Err(_) => {
                reply_error(&mut message, grant::ERR_UNAVAILABLE);
                return;
            }
        };
        let generation = match pending.wait() {
            Ok(result)
                if result.result >= 1 && result.connection.is_none() && result.memory.is_none() =>
            {
                result.result
            }
            Ok(_) | Err(_) => {
                reply_error(&mut message, grant::ERR_UNAVAILABLE);
                return;
            }
        };
        if let Some(reply) = message.reply.take() {
            let _ = reply.reply(generation);
        }
        return;
    }

    let Ok((authorization, authorization_len)) = authorization_memory(&message, request) else {
        reply_error(&mut message, grant::ERR_INVALID);
        return;
    };
    let pending = match name_service.call_move(
        ns::OP_LOOKUP_FOR_GRANT,
        authorization_len as u64,
        authorization,
    ) {
        Ok(pending) => pending,
        Err((_authorization, _error)) => {
            reply_error(&mut message, grant::ERR_UNAVAILABLE);
            return;
        }
    };
    let result = match pending.wait() {
        Ok(result) if result.result >= 1 => result,
        Ok(_) | Err(IpcError::CreationFailed | IpcError::Status(_)) => {
            reply_error(&mut message, grant::ERR_UNAVAILABLE);
            return;
        }
        Err(_) => {
            reply_error(&mut message, grant::ERR_UNAVAILABLE);
            return;
        }
    };
    let Some(connection) = result.connection else {
        reply_error(&mut message, grant::ERR_UNAVAILABLE);
        return;
    };
    let Some(reply) = message.reply.take() else {
        return;
    };
    let rights = IpcRights::from_bits(u32::from(request.rights));
    let _ = reply.reply_connection_ref(connection.as_ref(), rights, result.result);
}

fn main(ctx: Context) -> ! {
    let endpoint_cap = ctx.bootstrap_cap().unwrap_or_else(|| catten_rt::domain_abort());
    // Ownership transfers exactly once from the typed launch bootstrap slot.
    let endpoint =
        unsafe { Endpoint::from_raw(endpoint_cap) }.unwrap_or_else(|_| catten_rt::domain_abort());
    let name_service = ctx.name_service_connection().unwrap_or_else(|| catten_rt::domain_abort());
    let mut revisions = BTreeMap::new();
    loop {
        match endpoint.receive() {
            Ok(message) => handle(message, name_service, &mut revisions),
            Err(_) => catten_rt::domain_abort(),
        }
    }
}
