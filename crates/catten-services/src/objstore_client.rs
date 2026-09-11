//! Shared persistent-object-store IPC helpers.
//!
//! Persistent service state (`disk_raft`, `node_identity`) uses one scratch
//! page and the same object-store opcodes. Keeping one implementation here
//! avoids the copies drifting apart, and every length returned by the object
//! store is validated against the mapped memory object before it can drive a
//! copy.

use alloc::vec::Vec;

use catten_syscall::*;

const BUFFER_VADDR: usize = 0x0000_0000_2000_0000;

/// Looks up the object-store connection. `wait_for_service` selects a
/// deferred (blocking) name lookup for stores that must survive first boot.
pub(crate) fn connect(ns_conn: u64, wait_for_service: bool) -> Option<u64> {
    let opcode = if wait_for_service {
        crate::ns::OP_LOOKUP
    } else {
        crate::ns::OP_TRY_LOOKUP
    };
    let lookup = ipc_scalar_call_connection(
        ns_conn,
        opcode,
        crate::objstore::NAME,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if lookup == 0 {
        return None;
    }
    let (generation, conn) = unsafe { crate::wait_reply(lookup) };
    if generation < 1 || conn == 0 {
        return None;
    }
    Some(conn)
}

pub(crate) fn create_at(obj_conn: u64, object_id: u64) -> bool {
    let call = ipc_scalar_call(obj_conn, charlotte_protocol_objstore::OP_CREATE_AT, object_id);
    if call == 0 {
        return false;
    }
    let (result, _) = unsafe { crate::wait_reply(call) };
    result == charlotte_protocol_objstore::ERR_OK
        || result == charlotte_protocol_objstore::ERR_EXISTS
}

pub(crate) fn write(obj_conn: u64, object_id: u64, data: &[u8]) -> bool {
    let size_mem = memory_alloc(1);
    if size_mem == 0 || memory_map(size_mem, BUFFER_VADDR, true) != 0 {
        if size_mem != 0 {
            memory_close(size_mem);
        }
        return false;
    }
    unsafe {
        (BUFFER_VADDR as *mut u64).write_unaligned(data.len() as u64);
    }
    memory_unmap(size_mem);
    let size_call =
        ipc_scalar_call_borrow_read(obj_conn, crate::objstore::OP_SET_SIZE, object_id, size_mem);
    if size_call == 0 {
        memory_close(size_mem);
        return false;
    }
    let (size_result, _) = unsafe { crate::wait_reply(size_call) };
    memory_close(size_mem);
    if size_result != 0 {
        return false;
    }

    let pages = data.len().max(1).div_ceil(4096);
    let mem = memory_alloc(pages);
    if mem == 0 {
        return false;
    }
    if memory_map(mem, BUFFER_VADDR, true) != 0 {
        memory_close(mem);
        return false;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), BUFFER_VADDR as *mut u8, data.len());
    }
    memory_unmap(mem);
    let call = ipc_scalar_call_move(obj_conn, crate::objstore::OP_WRITE, object_id, mem);
    if call == 0 {
        return false;
    }
    let (result, _) = unsafe { crate::wait_reply(call) };
    result == 0
}

/// Reads one object. `Ok(None)` means the object does not exist yet;
/// `Err(())` means the transport, device, or a length validation failed.
pub(crate) fn read(obj_conn: u64, object_id: u64) -> Result<Option<Vec<u8>>, ()> {
    let call = ipc_scalar_call(obj_conn, crate::objstore::OP_READ, object_id);
    if call == 0 {
        return Err(());
    }
    let (status, result, returned_connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    if returned_connection != 0 {
        ipc_close(returned_connection);
    }
    if status != 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return if status == charlotte_protocol_objstore::ERR_NOT_FOUND as u64 {
            Ok(None)
        } else {
            Err(())
        };
    }
    if memory == 0 {
        return Err(());
    }
    let capacity = memory_size(memory);
    let Ok(size) = usize::try_from(result) else {
        memory_close(memory);
        return Err(());
    };
    if size > capacity {
        memory_close(memory);
        return Err(());
    }
    if memory_map(memory, BUFFER_VADDR, false) != 0 {
        memory_close(memory);
        return Err(());
    }
    let mut buf = alloc::vec![0u8; size];
    unsafe {
        core::ptr::copy_nonoverlapping(BUFFER_VADDR as *const u8, buf.as_mut_ptr(), size);
    }
    memory_unmap(memory);
    memory_close(memory);
    Ok(Some(buf))
}

pub(crate) fn flush(obj_conn: u64) -> bool {
    let call = ipc_scalar_call_connection(
        obj_conn,
        crate::objstore::OP_FLUSH,
        0,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if call == 0 {
        return false;
    }
    let (result, _) = unsafe { crate::wait_reply(call) };
    result == 0
}
