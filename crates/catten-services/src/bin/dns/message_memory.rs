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

/// Read the `[opcode:u32 LE][arg:i64 LE]` request from an `OP_CALL` memory
/// object, consuming the moved object.
pub(super) fn read_call_request(message: &IpcMessage) -> (u32, i64) {
    if message.memory == 0 {
        return (0, 0);
    }
    let (map_status, vaddr) = memory_map_any(message.memory, false);
    if map_status != 0 {
        memory_close(message.memory);
        return (0, 0);
    }
    let opcode = unsafe { core::ptr::read_volatile(vaddr as *const u32) };
    let arg = unsafe { core::ptr::read_volatile((vaddr + 4) as *const i64) };
    memory_unmap(message.memory);
    memory_close(message.memory);
    (opcode, arg)
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

/// Read an `OP_DEPLOY` request:
/// `[object_id:u64][node_key:u64][artifact_sha256:32]
/// [descriptor_magic:u32][descriptor_len:u32][signed_descriptor]`.
///
/// The descriptor suffix is optional for compatibility with legacy callers.
pub(super) fn read_deploy_request(message: &IpcMessage) -> Option<(u64, u64, [u8; 32], Vec<u8>)> {
    if message.memory == 0 {
        return None;
    }
    if memory_size(message.memory) < 48 {
        memory_close(message.memory);
        return None;
    }
    let (map_status, vaddr) = memory_map_any(message.memory, false);
    if map_status != 0 {
        memory_close(message.memory);
        return None;
    }
    let object_id = unsafe { core::ptr::read_volatile(vaddr as *const u64) };
    let node_key = unsafe { core::ptr::read_volatile((vaddr + 8) as *const u64) };
    let mut digest = [0u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((vaddr + 16 + index) as *const u8) };
    }
    let capacity = memory_size(message.memory);
    let descriptor_magic = if capacity >= 56 {
        unsafe { core::ptr::read_volatile((vaddr + 48) as *const u32) }
    } else {
        0
    };
    let descriptor = if descriptor_magic != dns::DEPLOY_DESCRIPTOR_MAGIC {
        Vec::new()
    } else {
        let len = unsafe { core::ptr::read_volatile((vaddr + 52) as *const u32) } as usize;
        if len > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
            || 56usize.saturating_add(len) > capacity
        {
            memory_unmap(message.memory);
            memory_close(message.memory);
            return None;
        }
        let mut descriptor = Vec::with_capacity(len);
        for index in 0..len {
            descriptor.push(unsafe { core::ptr::read_volatile((vaddr + 56 + index) as *const u8) });
        }
        descriptor
    };
    memory_unmap(message.memory);
    memory_close(message.memory);
    Some((object_id, node_key, digest, descriptor))
}

/// Read an `OP_DEPLOY_NAMED` request, consuming its moved memory object.
pub(super) struct NamedDeployRequest {
    pub name: Vec<u8>,
    pub object_id: u64,
    pub node_key: u64,
    pub digest: [u8; 32],
    pub descriptor: Vec<u8>,
}

pub(super) fn read_named_deploy_request(message: &IpcMessage) -> Option<NamedDeployRequest> {
    let bytes = read_moved_bytes(message, 4096)?;
    let name_len = usize::from(u16::from_le_bytes(bytes.get(0..2)?.try_into().ok()?));
    if name_len == 0 || name_len > charlotte_launch::deployment::MAX_ARTIFACT_NAME_LEN {
        return None;
    }
    let after_name = 2usize.checked_add(name_len)?;
    let name = bytes.get(2..after_name)?.to_vec();
    let object_id = u64::from_le_bytes(bytes.get(after_name..after_name + 8)?.try_into().ok()?);
    let node_key = u64::from_le_bytes(bytes.get(after_name + 8..after_name + 16)?.try_into().ok()?);
    let digest: [u8; 32] = bytes.get(after_name + 16..after_name + 48)?.try_into().ok()?;
    let descriptor_len = usize::try_from(u32::from_le_bytes(
        bytes.get(after_name + 48..after_name + 52)?.try_into().ok()?,
    ))
    .ok()?;
    if descriptor_len > charlotte_launch::deployment::MAX_DESCRIPTOR_LEN
        || bytes.len() != after_name + 52 + descriptor_len
    {
        return None;
    }
    Some(NamedDeployRequest {
        name,
        object_id,
        node_key,
        digest,
        descriptor: bytes[after_name + 52..].to_vec(),
    })
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
        ipc_reply_move(reply, cap, bytes.len() as i64);
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
