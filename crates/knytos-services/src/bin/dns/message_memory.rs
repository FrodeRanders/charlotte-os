//! Ownership-safe decoding and encoding for memory-backed DNS messages.

use alloc::vec::Vec;

use catten_services::dns;
use catten_syscall::{
    IpcMessage,
    ipc_reply,
    ipc_reply_move,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_size,
    memory_unmap,
};

/// Fully decoded client transaction; all reply authority transfers once.
pub(super) struct CallRequest {
    pub(super) name: Vec<u8>,
    pub(super) opcode: u32,
    pub(super) arg: i64,
    pub(super) reply: Option<catten_rt::owned::ReplyToken>,
}

pub(super) fn read_call_request(message: IpcMessage) -> Option<CallRequest> {
    // Raw receive ABI boundary. OP_CALL bypasses the scalar attachment guard
    // and this transaction consumes every unique attachment/reply exactly once.
    let Ok(Some(mut incoming)) = (unsafe { catten_rt::owned::IncomingMessage::from_raw(message) })
    else {
        return None;
    };
    let parsed = (|| {
        let length = usize::try_from(incoming.arg0).map_err(|_| dns::ERR_TOO_LARGE)?;
        if !(catten_services::dns_call::HEADER_LEN..=catten_services::dns_call::MAX_FRAME_LEN)
            .contains(&length)
        {
            return Err(dns::ERR_TOO_LARGE);
        }
        let memory = incoming.memory.take().ok_or(dns::ERR_BAD_OPCODE)?;
        if memory.len() < length {
            return Err(dns::ERR_TOO_LARGE);
        }
        let mapping = memory.map_read_only().map_err(|_| dns::ERR_RESOURCE_LIMIT)?;
        let request = catten_services::dns_call::decode(&mapping.as_slice()[..length])
            .ok_or(dns::ERR_BAD_OPCODE)?;
        let mut name = Vec::new();
        name.try_reserve_exact(request.name.len()).map_err(|_| dns::ERR_RESOURCE_LIMIT)?;
        name.extend_from_slice(request.name);
        let fields = (name, request.opcode, request.arg);
        drop(mapping.unmap().map_err(|_| dns::ERR_RESOURCE_LIMIT)?);
        Ok(fields)
    })();
    match parsed {
        Ok((name, opcode, arg)) => Some(CallRequest {
            name,
            opcode,
            arg,
            reply: incoming.reply.take(),
        }),
        Err(code) => {
            if let Some(reply) = incoming.reply.take() {
                let _ = reply.reply(code);
            }
            None
        }
    }
}

pub(super) fn read_generation(message: &IpcMessage) -> Option<u64> {
    if message.memory == 0 {
        return None;
    }
    let (map_status, vaddr) = memory_map_any(message.memory, false);
    if map_status != 0 {
        memory_close(message.memory);
        return None;
    }
    let generation = unsafe { core::ptr::read_volatile(vaddr as *const u64) };
    memory_unmap(message.memory);
    memory_close(message.memory);
    Some(generation)
}

/// Consume the exact signed descriptor supplied to `OP_DEPLOY_SUBMIT`.
pub(super) fn read_deployment_descriptor(message: &IpcMessage) -> Option<Vec<u8>> {
    let bytes = read_moved_bytes(message, charlotte_launch::deployment::MAX_DESCRIPTOR_LEN)?;
    charlotte_launch::deployment::decode(&bytes)?;
    Some(bytes)
}

/// Read a full-length name from moved memory. `arg0` is the byte length.
pub(super) fn read_named_bytes(message: &IpcMessage) -> Option<Vec<u8>> {
    if message.memory == 0 {
        return None;
    }
    let len = message.arg0 as usize;
    if len == 0 || len > 128 {
        memory_close(message.memory);
        return None;
    }
    let (map_status, vaddr) = memory_map_any(message.memory, false);
    if map_status != 0 {
        memory_close(message.memory);
        return None;
    }
    let mut name = Vec::with_capacity(len);
    unsafe {
        let src = vaddr as *const u8;
        for index in 0..len {
            name.push(core::ptr::read_volatile(src.add(index)));
        }
        memory_unmap(message.memory);
        memory_close(message.memory);
    }
    Some(name)
}

/// Read `[deployment_generation:u64][name]` from a moved agent registration.
pub(super) fn read_deployment_registration(message: &IpcMessage) -> Option<(Vec<u8>, u64)> {
    let bytes = read_moved_bytes(message, 8 + charlotte_launch::deployment::MAX_ARTIFACT_NAME_LEN)?;
    if bytes.len() < 9 || bytes.len() != message.arg0 as usize {
        return None;
    }
    let generation = u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?);
    let name = bytes.get(8..)?.to_vec();
    if generation == 0 || !charlotte_launch::deployment::valid_artifact_name(&name) {
        return None;
    }
    Some((name, generation))
}

/// Read an arbitrary bounded payload from moved memory, consuming the object.
pub(super) fn read_moved_bytes(message: &IpcMessage, max_len: usize) -> Option<Vec<u8>> {
    if message.memory == 0 {
        return None;
    }
    let len = usize::try_from(message.arg0).ok()?;
    if len == 0 || len > max_len || memory_size(message.memory) < len {
        memory_close(message.memory);
        return None;
    }
    let (map_status, vaddr) = memory_map_any(message.memory, false);
    if map_status != 0 {
        memory_close(message.memory);
        return None;
    }
    let mut bytes = Vec::with_capacity(len);
    unsafe {
        let src = vaddr as *const u8;
        for index in 0..len {
            bytes.push(core::ptr::read_volatile(src.add(index)));
        }
    }
    memory_unmap(message.memory);
    memory_close(message.memory);
    Some(bytes)
}

/// Read the 32 key bytes attached to an `OP_SET_KEY` request.
pub(super) fn read_key(message: &IpcMessage) -> Option<[u8; 32]> {
    if message.memory == 0 {
        return None;
    }
    if memory_size(message.memory) < 32 {
        memory_close(message.memory);
        return None;
    }
    let (map_status, vaddr) = memory_map_any(message.memory, false);
    if map_status != 0 {
        memory_close(message.memory);
        return None;
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((vaddr + index) as *const u8) };
    }
    memory_unmap(message.memory);
    memory_close(message.memory);
    Some(key)
}

/// Reply by moving a page containing `bytes`.
pub(super) fn reply_move_bytes(reply: u64, bytes: &[u8]) {
    if reply == 0 || bytes.len() > 64 * 4096 {
        if reply != 0 {
            ipc_reply(reply, dns::ERR_TOO_LARGE);
        }
        return;
    }
    let cap = memory_alloc(bytes.len().div_ceil(4096).max(1));
    let (map_status, vaddr) = memory_map_any(cap, true);
    if cap != 0 && map_status == 0 {
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), vaddr as *mut u8, bytes.len());
        }
        memory_unmap(cap);
        if ipc_reply_move(reply, cap, bytes.len() as i64) != 0 {
            memory_close(cap);
        }
    } else {
        if cap != 0 {
            memory_close(cap);
        }
        ipc_reply(reply, dns::ERR_TOO_LARGE);
    }
}

/// Unpack a short (at most eight byte) service name from scalar form.
pub(super) fn packed_name(packed: u64) -> Vec<u8> {
    let bytes = packed.to_le_bytes();
    let len = bytes.iter().rposition(|byte| *byte != 0).map_or(0, |index| index + 1);
    bytes[..len].to_vec()
}

/// Adopt the release transaction once and prepare its terminal output before admission.
pub(super) fn read_release_request(
    message: IpcMessage,
) -> Option<(Vec<u8>, catten_services::release_admission::PreparedReply)> {
    // Raw receive boundary: no scalar attachment guard or caller may reuse this message.
    let Ok(Some(mut incoming)) = (unsafe { catten_rt::owned::IncomingMessage::from_raw(message) })
    else {
        return None;
    };
    let token = incoming.reply.take()?;
    let parsed = (|| {
        if incoming.connection.is_some() {
            return Err(dns::ERR_BAD_OPCODE);
        }
        let len = usize::try_from(incoming.arg0).map_err(|_| dns::ERR_TOO_LARGE)?;
        if len == 0 || len > charlotte_launch::release::MAX_RELEASE_LEN {
            return Err(dns::ERR_TOO_LARGE);
        }
        let memory = incoming.memory.take().ok_or(dns::ERR_BAD_OPCODE)?;
        if len > memory.len() {
            return Err(dns::ERR_TOO_LARGE);
        }
        let mapping = memory.map_read_only().map_err(|_| dns::ERR_RESOURCE_LIMIT)?;
        let mut envelope = Vec::new();
        envelope.try_reserve_exact(len).map_err(|_| dns::ERR_RESOURCE_LIMIT)?;
        envelope.extend_from_slice(&mapping.as_slice()[..len]);
        drop(mapping.unmap().map_err(|_| dns::ERR_RESOURCE_LIMIT)?);
        Ok(envelope)
    })();
    let envelope = match parsed {
        Ok(envelope) => envelope,
        Err(code) => {
            let _ = token.reply(code);
            return None;
        }
    };
    match catten_services::release_admission::PreparedReply::prepare(token, &envelope) {
        Ok(reply) => Some((envelope, reply)),
        Err((token, code)) => {
            let _ = token.reply(code);
            None
        }
    }
}
