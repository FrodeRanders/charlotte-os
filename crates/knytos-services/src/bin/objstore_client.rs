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
/// Kernel-owned ordinary object staged by the NVMe verifier before this
/// client starts. Mirrors the kernel fixture constant.
const FOREIGN_OBJECT_ID: u64 = 0x7fff_ffff_ffff_ee00;
/// Kernel-owned ordinary object granted to this client read-only through the
/// operator scope table. Mirrors the kernel fixture constant.
const GRANTED_OBJECT_ID: u64 = 0x7fff_ffff_ffff_ee01;
const GRANTED_PROBE: &[u8] = b"scoped probe";
const BYTES: usize = 2 * 1024 * 1024 + 4096;
const PAGES: usize = BYTES / 4096;

fn fail(code: u32) -> ! {
    config::write::<u32>(status::STAGE, code);
    unsafe { thread_exit() }
}

fn main(ctx: Context) -> ! {
    let ns_connection = ctx.bootstrap_connection().unwrap_or_else(|| fail(0xdea0));
    let (_, connection) = wait_for_registered_name_owned(ns_connection, objstore::NAME)
        .unwrap_or_else(|| fail(0xdea2));

    let automatic = connection
        .call(objstore::OP_CREATE, 0)
        .and_then(|call| call.wait())
        .unwrap_or_else(|_| fail(0xdec0))
        .result;
    if automatic <= 1 {
        fail(0xdec1);
    }
    for opcode in [objstore::OP_READ, objstore::OP_INFO, objstore::OP_DELETE] {
        let result =
            connection.call(opcode, 1).and_then(|call| call.wait()).map(|reply| reply.result);
        if result != Ok(objstore::ERR_DENIED) {
            fail(0xdec2);
        }
    }
    if connection
        .call(objstore::OP_DELETE, automatic as u64)
        .and_then(|call| call.wait())
        .map(|reply| reply.result)
        != Ok(objstore::ERR_OK)
    {
        fail(0xdec3);
    }
    // An ordinary principal cannot replace the operator table, even with a
    // well-formed empty table. The pre-existing read grant must remain live.
    let policy = OwnedMemory::allocate(1).unwrap_or_else(|_| fail(0xdec4));
    let mut mapping = policy.map_writable().unwrap_or_else(|_| fail(0xdec4));
    let empty = objstore::ScopeTable::default().encode();
    mapping.as_mut_slice()[..empty.len()].copy_from_slice(&empty);
    let policy = mapping.unmap().unwrap_or_else(|_| fail(0xdec4));
    let apply = connection
        .call_move(objstore::OP_APPLY_SCOPE, empty.len() as u64, policy)
        .unwrap_or_else(|_| fail(0xdec5));
    if apply.wait().map(|reply| reply.result) != Ok(objstore::ERR_DENIED) {
        fail(0xdec5);
    }

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
    // Reserved artifact/executable mutation is denied to this ordinary client
    // and performed by the kernel before this client starts. Verify the
    // denial here so the stored executable cannot be tampered with by an
    // application holding a store connection.
    let denied = connection
        .call(objstore::OP_DELETE, objstore::EXECUTABLE_ECHO_ID)
        .and_then(|call| call.wait())
        .map(|result| result.result);
    if denied != Ok(objstore::ERR_DENIED) {
        fail(0xdeaf);
    }
    // A foreign principal's ordinary object is neither readable nor mutable,
    // while this client's own object above stayed readable and writable.
    let foreign_read = connection
        .call(objstore::OP_READ, FOREIGN_OBJECT_ID)
        .and_then(|call| call.wait())
        .map(|result| result.result);
    if foreign_read != Ok(objstore::ERR_DENIED) {
        fail(0xdeb8);
    }
    let foreign_delete = connection
        .call(objstore::OP_DELETE, FOREIGN_OBJECT_ID)
        .and_then(|call| call.wait())
        .map(|result| result.result);
    if foreign_delete != Ok(objstore::ERR_DENIED) {
        fail(0xdeb9);
    }
    // Operator scope grants read-only access to the second kernel probe:
    // reading succeeds with the exact bytes while delete stays denied.
    let granted = connection
        .call(objstore::OP_READ, GRANTED_OBJECT_ID)
        .and_then(|call| call.wait())
        .unwrap_or_else(|_| fail(0xdeba));
    if granted.result != GRANTED_PROBE.len() as i64 {
        fail(0xdebb);
    }
    let granted_memory = granted.memory.unwrap_or_else(|| fail(0xdebe));
    let granted_mapping = granted_memory.map_read_only().unwrap_or_else(|_| fail(0xdebe));
    if granted_mapping.as_slice().get(..GRANTED_PROBE.len()) != Some(GRANTED_PROBE) {
        fail(0xdebe);
    }
    drop(granted_mapping);
    let granted_delete = connection
        .call(objstore::OP_DELETE, GRANTED_OBJECT_ID)
        .and_then(|call| call.wait())
        .map(|result| result.result);
    if granted_delete != Ok(objstore::ERR_DENIED) {
        fail(0xdebf);
    }
    catten_rt::logln!(concat!(
        "[objstore-client] automatic-ID collision preserved foreign ownership; ",
        "unauthorized scope replacement denied and original read grant preserved"
    ));
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
    // The kernel verifier replaces the scope table after the initial grant
    // checks and then asks this still-live principal to observe revocation.
    loop {
        let mut request = done_endpoint.receive().unwrap_or_else(|_| fail(0xdec6));
        if request.sender_principal != 1 || request.opcode != 1 {
            if let Some(reply) = request.reply.take() {
                let _ = reply.reply(objstore::ERR_DENIED);
            }
            continue;
        }
        for opcode in [objstore::OP_READ, objstore::OP_INFO] {
            let result = connection
                .call(opcode, GRANTED_OBJECT_ID)
                .and_then(|call| call.wait())
                .unwrap_or_else(|_| fail(0xdec7));
            if result.result != objstore::ERR_DENIED || result.memory.is_some() {
                fail(0xdec8);
            }
        }
        catten_rt::logln!("[objstore-client] live scope revocation denied read and info");
        let reply = request.reply.take().unwrap_or_else(|| fail(0xdec9));
        reply.reply(objstore::ERR_OK).unwrap_or_else(|_| fail(0xdec9));
        break;
    }
    unsafe { thread_exit() }
}
