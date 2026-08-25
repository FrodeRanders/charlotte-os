//! Persistent-object integration test client.
#![no_std]
#![no_main]

catten_rt::entry!(main);

use catten_rt::{
    Context,
    config,
    owned::{
        Endpoint,
        OwnedMemory,
    },
};
use catten_services::{
    ns,
    objstore,
    wait_for_registered_name_owned,
};
use catten_syscall::{
    IpcRights,
    thread_exit,
};
use charlotte_launch::objstore_client_status as status;

const OBJECT_ID: u64 = 0x7fff_ffff_ffff_ff00;
const BYTES: usize = 2 * 1024 * 1024 + 4096;
const PAGES: usize = BYTES / 4096;
const ELF_SIZE_KEY: u64 = charlotte_launch::manifest_key(b"elf_size");

fn fail(code: u32) -> ! {
    config::write::<u32>(status::STAGE, code);
    unsafe { thread_exit() }
}

fn main(ctx: Context) -> ! {
    let ns_connection = ctx.bootstrap_connection().unwrap_or_else(|| fail(0xdea0));
    let (_, connection) = wait_for_registered_name_owned(ns_connection, objstore::NAME)
        .unwrap_or_else(|| fail(0xdea2));

    let create = connection.call(objstore::OP_CREATE_AT, OBJECT_ID);
    config::write::<u32>(status::STAGE, 1);
    let created = create.and_then(|call| call.wait()).unwrap_or_else(|_| fail(0xdea3)).result;
    if created != objstore::ERR_OK && created != objstore::ERR_EXISTS {
        fail(0xdea4);
    }

    let size_memory = OwnedMemory::allocate(1).unwrap_or_else(|_| fail(0xdea5));
    let mut size_mapping = size_memory.map_writable().unwrap_or_else(|_| fail(0xdea5));
    size_mapping.as_mut_slice()[..8].copy_from_slice(&(BYTES as u64).to_le_bytes());
    let size_memory = size_mapping.unmap().unwrap_or_else(|_| fail(0xdea5));
    let set_size = connection
        .call_borrow_read(objstore::OP_SET_SIZE, OBJECT_ID, &size_memory)
        .and_then(|call| call.wait())
        .map(|result| result.result);
    config::write::<u32>(status::STAGE, 2);
    if set_size != Ok(objstore::ERR_OK) {
        fail(0xdea6);
    }

    let data = OwnedMemory::allocate(PAGES).unwrap_or_else(|_| fail(0xdea7));
    let mut data_mapping = data.map_writable().unwrap_or_else(|_| fail(0xdea8));
    for offset in 0..BYTES {
        data_mapping.as_mut_slice()[offset] = (offset as u8).wrapping_mul(29) ^ 0x6d;
    }
    let data = data_mapping.unmap().unwrap_or_else(|_| fail(0xdea8));
    let write = connection.call_move(objstore::OP_WRITE, OBJECT_ID, data);
    config::write::<u32>(status::STAGE, 3);
    let write_result =
        write.unwrap_or_else(|_| fail(0xdea9)).wait().unwrap_or_else(|_| fail(0xdea9)).result;
    if write_result != objstore::ERR_OK {
        config::write::<u32>(status::ROUND_TRIP_BYTES, write_result as u32);
        fail(0xdea9);
    }
    let flush = connection.call(objstore::OP_FLUSH, 0);
    config::write::<u32>(status::STAGE, 4);
    if flush.and_then(|call| call.wait()).map(|result| result.result) != Ok(objstore::ERR_OK) {
        fail(0xdeaa);
    }

    let read = connection.call(objstore::OP_READ, OBJECT_ID);
    config::write::<u32>(status::STAGE, 5);
    let read = read.and_then(|call| call.wait()).unwrap_or_else(|_| fail(0xdeab));
    if read.result != BYTES as i64 {
        fail(0xdeac);
    }
    let returned_memory = read.memory.unwrap_or_else(|| fail(0xdeac));
    let data_mapping = returned_memory.map_read_only().unwrap_or_else(|_| fail(0xdead));
    config::write::<u32>(status::STAGE, 6);
    for offset in 0..BYTES {
        let actual = data_mapping.as_slice()[offset];
        let expected = (offset as u8).wrapping_mul(29) ^ 0x6d;
        if actual != expected {
            fail(0xdeae);
        }
    }
    let elf_cap = ctx.handoff_state_cap();
    if elf_cap != 0 {
        let Some(catten_rt::ManifestValue::Unsigned(elf_size)) = ctx.manifest_value(ELF_SIZE_KEY)
        else {
            fail(0xdeaf);
        };
        let created = connection
            .call(objstore::OP_CREATE_AT, objstore::EXECUTABLE_ECHO_ID)
            .and_then(|call| call.wait())
            .unwrap_or_else(|_| fail(0xdeb0))
            .result;
        if created != objstore::ERR_OK && created != objstore::ERR_EXISTS {
            fail(0xdeb1);
        }
        let size_memory = OwnedMemory::allocate(1).unwrap_or_else(|_| fail(0xdeb2));
        let mut size_mapping = size_memory.map_writable().unwrap_or_else(|_| fail(0xdeb2));
        size_mapping.as_mut_slice()[..8].copy_from_slice(&elf_size.to_le_bytes());
        let size_memory = size_mapping.unmap().unwrap_or_else(|_| fail(0xdeb2));
        let set_size = connection
            .call_borrow_read(objstore::OP_SET_SIZE, objstore::EXECUTABLE_ECHO_ID, &size_memory)
            .and_then(|call| call.wait())
            .map(|result| result.result);
        if set_size != Ok(objstore::ERR_OK) {
            fail(0xdeb3);
        }
        // The handoff-state memory is transferred exactly once to the object
        // store. This is the launch ABI boundary where raw adoption is needed.
        let elf_memory = unsafe { OwnedMemory::from_raw(elf_cap) }.unwrap_or_else(|_| fail(0xdeb4));
        let write = connection
            .call_move(objstore::OP_WRITE, objstore::EXECUTABLE_ECHO_ID, elf_memory)
            .map_err(|(_, error)| error)
            .and_then(|call| call.wait())
            .map(|result| result.result);
        if write != Ok(objstore::ERR_OK) {
            fail(0xdeb4);
        }
        let flush = connection
            .call(objstore::OP_FLUSH, 0)
            .and_then(|call| call.wait())
            .map(|result| result.result);
        if flush != Ok(objstore::ERR_OK) {
            fail(0xdeb5);
        }
        config::write::<u32>(status::ELF_SIZE, elf_size as u32);
    }
    config::write::<u32>(status::ROUND_TRIP_BYTES, BYTES as u32);
    config::write::<u32>(status::STAGE, 0x900d);
    let done_endpoint = Endpoint::create(objstore::INTERFACE, objstore::VERSION, 1)
        .unwrap_or_else(|_| fail(0xdeb6));
    let register = ns_connection
        .call_connection(
            ns::OP_REGISTER,
            objstore::TEST_DONE_NAME,
            &done_endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .and_then(|call| call.wait())
        .map(|result| result.result);
    if register.is_err() || register.is_ok_and(|generation| generation < 1) {
        fail(0xdeb7);
    }
    unsafe { thread_exit() }
}
