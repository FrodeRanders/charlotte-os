//! Persistent-object integration test client.
#![no_std]
#![no_main]

catten_rt::entry!(main);

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    ns,
    objstore,
};
use catten_syscall::*;

const OBJECT_ID: u64 = 0x7fff_ffff_ffff_ff00;
const BYTES: usize = 2 * 1024 * 1024 + 4096;
const PAGES: usize = BYTES / 4096;
const DATA_VADDR: usize = 0x0000_0000_2000_0000;
const SIZE_VADDR: usize = 0x0000_0000_0070_0000;
const ELF_SIZE_KEY: u64 = charlotte_launch::manifest_key(b"elf_size");

fn fail(code: u32) -> ! {
    config::write::<u32>(0, code);
    unsafe { thread_exit() }
}

fn main(ctx: Context) -> ! {
    let ns_connection = ctx.bootstrap_cap().unwrap_or_else(|| fail(0xdea0));
    let lookup = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_LOOKUP,
        objstore::NAME,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if lookup == 0 {
        fail(0xdea1);
    }
    let (generation, connection) = catten_services::spin_reply(lookup);
    if generation < 1 || connection == 0 {
        fail(0xdea2);
    }

    let create = ipc_scalar_call(connection, objstore::OP_CREATE_AT, OBJECT_ID);
    if create == 0 {
        fail(0xdea3);
    }
    let created = catten_services::spin_reply(create).0;
    if created != objstore::ERR_OK && created != objstore::ERR_EXISTS {
        fail(0xdea4);
    }

    let size_cap = memory_alloc(1);
    if size_cap == 0 || memory_map(size_cap, SIZE_VADDR, true) != 0 {
        fail(0xdea5);
    }
    unsafe {
        (SIZE_VADDR as *mut u64).write_unaligned(BYTES as u64);
    }
    memory_unmap(size_cap);
    let set_size =
        ipc_scalar_call_borrow_read(connection, objstore::OP_SET_SIZE, OBJECT_ID, size_cap);
    if set_size == 0 || catten_services::spin_reply(set_size).0 != objstore::ERR_OK {
        fail(0xdea6);
    }
    memory_close(size_cap);

    let data = memory_alloc(PAGES);
    if data == 0 {
        fail(0xdea7);
    }
    let map_status = memory_map(data, DATA_VADDR, true);
    if map_status != 0 {
        config::write::<u32>(4, map_status as u32);
        fail(0xdea8);
    }
    for offset in 0..BYTES {
        unsafe {
            ((DATA_VADDR + offset) as *mut u8)
                .write_volatile((offset as u8).wrapping_mul(29) ^ 0x6d);
        }
    }
    memory_unmap(data);
    let write = ipc_scalar_call_move(connection, objstore::OP_WRITE, OBJECT_ID, data);
    if write == 0 {
        fail(0xdea9);
    }
    let write_result = catten_services::spin_reply(write).0;
    if write_result != objstore::ERR_OK {
        config::write::<u32>(4, write_result as u32);
        fail(0xdea9);
    }
    let flush = ipc_scalar_call(connection, objstore::OP_FLUSH, 0);
    if flush == 0 || catten_services::spin_reply(flush).0 != objstore::ERR_OK {
        fail(0xdeaa);
    }

    let read = ipc_scalar_call(connection, objstore::OP_READ, OBJECT_ID);
    if read == 0 {
        fail(0xdeab);
    }
    let (status, size, returned_connection, returned_memory) = ipc_reply_wait_with_memory(read);
    ipc_close(read);
    if returned_connection != 0 {
        ipc_close(returned_connection);
    }
    if status != 0 || size != BYTES as u64 || returned_memory == 0 {
        fail(0xdeac);
    }
    if memory_map(returned_memory, DATA_VADDR, false) != 0 {
        fail(0xdead);
    }
    for offset in 0..BYTES {
        let actual = unsafe { ((DATA_VADDR + offset) as *const u8).read_volatile() };
        let expected = (offset as u8).wrapping_mul(29) ^ 0x6d;
        if actual != expected {
            fail(0xdeae);
        }
    }
    memory_unmap(returned_memory);
    memory_close(returned_memory);

    let elf_cap = ctx.handoff_state_cap();
    if elf_cap != 0 {
        let Some(catten_rt::ManifestValue::Unsigned(elf_size)) = ctx.manifest_value(ELF_SIZE_KEY)
        else {
            fail(0xdeaf);
        };
        let create =
            ipc_scalar_call(connection, objstore::OP_CREATE_AT, objstore::EXECUTABLE_ECHO_ID);
        if create == 0 {
            fail(0xdeb0);
        }
        let created = catten_services::spin_reply(create).0;
        if created != objstore::ERR_OK && created != objstore::ERR_EXISTS {
            fail(0xdeb1);
        }
        let size_cap = memory_alloc(1);
        if size_cap == 0 || memory_map(size_cap, SIZE_VADDR, true) != 0 {
            fail(0xdeb2);
        }
        unsafe {
            (SIZE_VADDR as *mut u64).write_unaligned(elf_size);
        }
        memory_unmap(size_cap);
        let set_size = ipc_scalar_call_borrow_read(
            connection,
            objstore::OP_SET_SIZE,
            objstore::EXECUTABLE_ECHO_ID,
            size_cap,
        );
        if set_size == 0 || catten_services::spin_reply(set_size).0 != objstore::ERR_OK {
            fail(0xdeb3);
        }
        memory_close(size_cap);
        let write = ipc_scalar_call_move(
            connection,
            objstore::OP_WRITE,
            objstore::EXECUTABLE_ECHO_ID,
            elf_cap,
        );
        if write == 0 || catten_services::spin_reply(write).0 != objstore::ERR_OK {
            fail(0xdeb4);
        }
        let flush = ipc_scalar_call(connection, objstore::OP_FLUSH, 0);
        if flush == 0 || catten_services::spin_reply(flush).0 != objstore::ERR_OK {
            fail(0xdeb5);
        }
        config::write::<u32>(8, elf_size as u32);
    }
    config::write::<u32>(4, BYTES as u32);
    config::write::<u32>(0, 0x900d);
    unsafe { thread_exit() }
}
