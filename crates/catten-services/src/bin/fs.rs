//! Native filesystem service for CharlotteOS.
//!
//! Built on top of the persistent object store. Directories and files are
//! stored as objects. The root directory is object ID 100.
//!
//! ## Directory encoding
//!
//! A directory object stores entries as:
//!   [name_len: u32][name: [u8; name_len]][file_id: u64][flags: u32][size: u64]
//! repeated until end of data. name_len=0 marks end.
//!
//! ## Crash model
//!
//! The object store handles crash safety at the per-object level. The
//! filesystem creates new objects for data before updating parent directory
//! entries, so interrupted writes leave orphaned objects (recoverable) and
//! never partial directory state.
#![no_std]
#![no_main]
extern crate alloc;

catten_rt::entry!(main);

use alloc::vec::Vec;
use catten_rt::Context;
use catten_services::fs;
use catten_services::ns;
use catten_services::objstore;
use catten_syscall::ipc_status;
use catten_syscall::*;

const REPLY_SPINS: u64 = u64::MAX;
const BUFFER_VADDR: usize = 0x0000_0000_0050_0000;
const ROOT_ID: u64 = 100;
const MAX_DIR_SIZE: usize = 4096;

fn objstore_connect(ns_conn: u64) -> Option<u64> {
    let lookup = ipc_scalar_call_connection(ns_conn, ns::OP_LOOKUP, objstore::NAME, 0, IpcRights::SEND | IpcRights::CALL);
    if lookup == 0 { return None; }
    let (generation, conn) = unsafe { catten_services::wait_reply(lookup, REPLY_SPINS) };
    if generation < 1 || conn == 0 { return None; }
    Some(conn)
}

fn obj_write(obj_conn: u64, object_id: u64, data: &[u8]) -> bool {
    let len = data.len().min(4096);
    let mem = memory_alloc(1);
    if mem == 0 { return false; }
    if memory_map(mem, BUFFER_VADDR, true) != 0 { memory_close(mem); return false; }
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), BUFFER_VADDR as *mut u8, len); }
    memory_unmap(mem);
    let call = ipc_scalar_call_move(obj_conn, objstore::OP_WRITE, object_id, mem);
    if call == 0 { return false; }
    let (result, _) = unsafe { catten_services::wait_reply(call, REPLY_SPINS) };
    result == 0
}

fn obj_read(obj_conn: u64, object_id: u64) -> Option<Vec<u8>> {
    let mem = memory_alloc(1);
    if mem == 0 { return None; }
    let call = ipc_scalar_call_borrow_write(obj_conn, objstore::OP_READ, object_id, mem);
    if call == 0 { memory_close(mem); return None; }
    let (result, _) = unsafe { catten_services::wait_reply(call, REPLY_SPINS) };
    if result != 0 { memory_close(mem); return None; }
    if memory_map(mem, BUFFER_VADDR, false) != 0 { memory_close(mem); return None; }
    let mut buf = alloc::vec![0u8; 4096];
    unsafe { core::ptr::copy_nonoverlapping(BUFFER_VADDR as *const u8, buf.as_mut_ptr(), 4096); }
    memory_unmap(mem);
    memory_close(mem);
    Some(buf)
}

fn obj_create(obj_conn: u64) -> u64 {
    let call = ipc_scalar_call_connection(obj_conn, objstore::OP_CREATE, 0, 0, IpcRights::SEND | IpcRights::CALL);
    if call == 0 { return 0; }
    let (id, _) = unsafe { catten_services::wait_reply(call, REPLY_SPINS) };
    if id <= 0 { 0 } else { id as u64 }
}

fn obj_flush(obj_conn: u64) -> bool {
    let call = ipc_scalar_call_connection(obj_conn, objstore::OP_FLUSH, 0, 0, IpcRights::SEND | IpcRights::CALL);
    if call == 0 { return false; }
    let (result, _) = unsafe { catten_services::wait_reply(call, REPLY_SPINS) };
    result == 0
}

// ---------------------------------------------------------------------------
// Directory encoding
// ---------------------------------------------------------------------------
// Entry: (u32 name_len, [u8] name, u64 file_id, u32 flags, u64 size)

fn decode_dir(data: &[u8]) -> Vec<(alloc::string::String, u64, u32, u64)> {
    let mut entries = Vec::new();
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let name_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap_or([0; 4])) as usize;
        if name_len == 0 { break; }
        if pos + 4 + name_len + 20 > data.len() { break; }
        let name = core::str::from_utf8(&data[pos + 4..pos + 4 + name_len])
            .ok()
            .map(|s| alloc::string::String::from(s))
            .unwrap_or_default();
        let file_id = u64::from_le_bytes(data[pos + 4 + name_len..pos + 4 + name_len + 8].try_into().unwrap_or([0; 8]));
        let flags = u32::from_le_bytes(data[pos + 4 + name_len + 8..pos + 4 + name_len + 12].try_into().unwrap_or([0; 4]));
        let size = u64::from_le_bytes(data[pos + 4 + name_len + 12..pos + 4 + name_len + 20].try_into().unwrap_or([0; 8]));
        if !name.is_empty() {
            entries.push((name, file_id, flags, size));
        }
        pos += 4 + name_len + 20;
    }
    entries
}

fn encode_dir(entries: &[(alloc::string::String, u64, u32, u64)]) -> Vec<u8> {
    let mut buf = Vec::new();
    for (name, file_id, flags, size) in entries {
        let name_bytes = name.as_bytes();
        let name_len = (name_bytes.len() as u32).min(128);
        let mut entry = alloc::vec![0u8; 4 + name_len as usize + 20];
        entry[0..4].copy_from_slice(&name_len.to_le_bytes());
        entry[4..4 + name_len as usize].copy_from_slice(&name_bytes[..name_len as usize]);
        entry[4 + name_len as usize..4 + name_len as usize + 8].copy_from_slice(&file_id.to_le_bytes());
        entry[4 + name_len as usize + 8..4 + name_len as usize + 12].copy_from_slice(&flags.to_le_bytes());
        entry[4 + name_len as usize + 12..4 + name_len as usize + 20].copy_from_slice(&size.to_le_bytes());
        buf.extend_from_slice(&entry);
    }
    buf
}

// ---------------------------------------------------------------------------
// Filesystem operations
// ---------------------------------------------------------------------------

struct FileSystem {
    obj_conn: u64,
}

impl FileSystem {
    fn mount(obj_conn: u64) -> Self {
        // Ensure root directory exists
        if obj_read(obj_conn, ROOT_ID).is_none() {
            let empty_dir = [0u8; 4]; // name_len=0 terminator
            obj_write(obj_conn, ROOT_ID, &empty_dir);
        }
        FileSystem { obj_conn }
    }

    fn lookup(&self, parent_id: u64, name: &str) -> Option<(u64, u32, u64)> {
        let dir_data = obj_read(self.obj_conn, parent_id)?;
        for (entry_name, file_id, flags, size) in decode_dir(&dir_data) {
            if entry_name == name {
                return Some((file_id, flags, size));
            }
        }
        None
    }

    fn op_lookup(&self, parent_id: u64, name: &str) -> i64 {
        if let Some((file_id, flags, _size)) = self.lookup(parent_id, name) {
            ((file_id as i64) << 32) | ((flags as i64) & 0xFFFF_FFFF)
        } else {
            fs::ERR_NOT_FOUND
        }
    }

    fn op_create(&self, parent_id: u64, name: &str, is_dir: bool) -> i64 {
        if self.lookup(parent_id, name).is_some() {
            return fs::ERR_EXISTS;
        }
        let new_id = obj_create(self.obj_conn);
        if new_id == 0 { return fs::ERR_NO_SPACE; }

        // If directory, write an empty directory entry
        if is_dir {
            let empty = [0u8; 4];
            obj_write(self.obj_conn, new_id, &empty);
        }

        // Add entry to parent directory
        let dir_data = obj_read(self.obj_conn, parent_id).unwrap_or_default();
        let mut entries = decode_dir(&dir_data);
        let flags = if is_dir { fs::FLAG_DIR } else { 0 };
        entries.push((alloc::string::String::from(name), new_id, flags, 0));
        let new_dir = encode_dir(&entries);
        if new_dir.len() > MAX_DIR_SIZE {
            return fs::ERR_NO_SPACE;
        }
        obj_write(self.obj_conn, parent_id, &new_dir);
        new_id as i64
    }

    fn op_read(&self, file_id: u64) -> Option<Vec<u8>> {
        obj_read(self.obj_conn, file_id)
    }

    fn op_write(&self, file_id: u64, data: &[u8]) -> bool {
        // Update the object content
        if !obj_write(self.obj_conn, file_id, data) { return false; }
        // Update size in parent directory
        true
    }

    fn op_delete(&self, parent_id: u64, name: &str) -> i64 {
        let (target_id, flags, _size) = match self.lookup(parent_id, name) {
            Some(v) => v,
            None => return fs::ERR_NOT_FOUND,
        };
        // If directory, check it's empty
        if flags & fs::FLAG_DIR != 0 {
            let dir_data = obj_read(self.obj_conn, target_id);
            if let Some(data) = dir_data {
                let entries = decode_dir(&data);
                if !entries.is_empty() {
                    return fs::ERR_DIR_NOT_EMPTY;
                }
            }
        }
        // Remove entry from parent
        let dir_data = obj_read(self.obj_conn, parent_id).unwrap_or_default();
        let mut entries = decode_dir(&dir_data);
        entries.retain(|(n, _, _, _)| n != name);
        obj_write(self.obj_conn, parent_id, &encode_dir(&entries));
        fs::ERR_OK
    }

    fn op_list(&self, parent_id: u64) -> Vec<u8> {
        let dir_data = obj_read(self.obj_conn, parent_id).unwrap_or_default();
        let entries = decode_dir(&dir_data);
        encode_dir(&entries)
    }
}

fn main(ctx: Context) -> ! {
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };

    let obj_conn = match objstore_connect(ns_connection) {
        Some(c) => c,
        None => unsafe { thread_exit() },
    };

    let ffs = FileSystem::mount(obj_conn);

    let endpoint = ipc_endpoint_create(fs::INTERFACE, fs::VERSION, 64);
    if endpoint == 0 { unsafe { thread_exit() }; }

    let register = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        fs::NAME,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if register == 0 { unsafe { thread_exit() }; }
    let (generation, _) = unsafe { catten_services::wait_reply(register, REPLY_SPINS) };
    if generation < 1 { unsafe { thread_exit() }; }

    ipc_endpoint_bind_cq(endpoint, 0);

    loop {
        cq_wait(1, 0);

        loop {
            let message = ipc_recv(endpoint);
            if message.status == ipc_status::NO_MESSAGE { break; }
            if message.status == ipc_status::ENDPOINT_CLOSED { unsafe { thread_exit() }; }
            if !message.is_ok() { break; }

            match message.opcode {
                fs::OP_LOOKUP => {
                    let parent_id = message.arg0 >> 32;
                    let name_len = (message.arg0 & 0xFF) as usize;
                    if message.reply != 0 {
                        let name = if message.memory != 0 && name_len > 0 {
                            if memory_map(message.memory, BUFFER_VADDR, false) == 0 {
                                let nm = unsafe {
                                    let bytes = core::slice::from_raw_parts(BUFFER_VADDR as *const u8, name_len);
                                    core::str::from_utf8(bytes).ok().unwrap_or("")
                                };
                                memory_unmap(message.memory);
                                nm
                            } else { "" }
                        } else { "" };
                        if name.is_empty() {
                            ipc_reply(message.reply, fs::ERR_NOT_FOUND);
                        } else {
                            let result = ffs.op_lookup(parent_id, name);
                            ipc_reply(message.reply, result);
                        }
                    }
                }
                fs::OP_CREATE => {
                    let parent_id = message.arg0 >> 32;
                    let is_dir = (message.arg0 & 0x1) != 0;
                    let name_len = (message.arg0 >> 8) as u8 as usize;
                    if message.reply != 0 {
                        let name = read_name_from_msg(message.memory, name_len);
                        let result = ffs.op_create(parent_id, &name, is_dir);
                        ipc_reply(message.reply, result);
                    }
                }
                fs::OP_READ => {
                    let file_id = message.arg0;
                    if message.reply != 0 {
                        if let Some(data) = ffs.op_read(file_id) {
                            let mem = memory_alloc(1);
                            if mem != 0 {
                                memory_map(mem, BUFFER_VADDR, true);
                                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), BUFFER_VADDR as *mut u8, data.len().min(4096)); }
                                memory_unmap(mem);
                                ipc_reply_move(message.reply, mem, data.len() as i64);
                            } else {
                                ipc_reply(message.reply, fs::ERR_IO_ERROR);
                            }
                        } else {
                            ipc_reply(message.reply, fs::ERR_NOT_FOUND);
                        }
                    }
                }
                fs::OP_WRITE => {
                    let file_id = message.arg0;
                    if message.reply != 0 {
                        let result = if message.memory != 0 {
                            if memory_map(message.memory, BUFFER_VADDR, false) == 0 {
                                let data = unsafe { core::slice::from_raw_parts(BUFFER_VADDR as *const u8, 4096) };
                                let ok = ffs.op_write(file_id, data);
                                memory_unmap(message.memory);
                                if ok { 0 } else { fs::ERR_IO_ERROR }
                            } else { fs::ERR_IO_ERROR }
                        } else { fs::ERR_IO_ERROR };
                        ipc_reply(message.reply, result);
                    }
                }
                fs::OP_DELETE => {
                    let parent_id = message.arg0 >> 32;
                    let name_len = (message.arg0 & 0xFF) as usize;
                    if message.reply != 0 {
                        let name = read_name_from_msg(message.memory, name_len);
                        let result = ffs.op_delete(parent_id, &name);
                        ipc_reply(message.reply, result);
                    }
                }
                fs::OP_LIST => {
                    let parent_id = message.arg0;
                    if message.reply != 0 {
                        let data = ffs.op_list(parent_id);
                        let mem = memory_alloc(1);
                        if mem != 0 {
                            memory_map(mem, BUFFER_VADDR, true);
                            unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), BUFFER_VADDR as *mut u8, data.len().min(4096)); }
                            memory_unmap(mem);
                            ipc_reply_move(message.reply, mem, data.len() as i64);
                        } else {
                            ipc_reply(message.reply, fs::ERR_IO_ERROR);
                        }
                    }
                }
                fs::OP_FLUSH => {
                    obj_flush(ffs.obj_conn);
                    if message.reply != 0 { ipc_reply(message.reply, 0); }
                }
                _ => {
                    if message.reply != 0 { ipc_reply(message.reply, -1); }
                }
            }
        }
    }
}

fn read_name_from_msg(memory: u64, name_len: usize) -> alloc::string::String {
    if memory == 0 || name_len == 0 { return alloc::string::String::new(); }
    if memory_map(memory, BUFFER_VADDR, false) != 0 { return alloc::string::String::new(); }
    let s = unsafe {
        let bytes = core::slice::from_raw_parts(BUFFER_VADDR as *const u8, name_len.min(128));
        core::str::from_utf8(bytes).ok().unwrap_or("")
    };
    memory_unmap(memory);
    alloc::string::String::from(s)
}
