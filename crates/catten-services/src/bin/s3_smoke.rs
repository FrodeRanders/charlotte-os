//! End-to-end S3 client smoke test used by the opt-in RustFS fixture.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use catten_rt::{
    Context,
    config,
    owned::OwnedMemory,
};
use catten_services::{
    s3,
    s3_client::{
        Client,
        Error,
        ObjectChunk,
        PutWriteError,
    },
    sleep_ms,
    wait_for_registered_name_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::{
    s3_smoke_status as status,
    sha256::Sha256,
};
use charlotte_protocol_s3::{
    ERR_TRANSPORT,
    ERR_UNSYNCHRONIZED,
    ObjectRequest,
};

catten_rt::entry!(main);

const BODY: &[u8] = b"CharlotteOS verified S3 over TLS\n";
const KEY: &[u8] = b"smoke/round-trip.txt";

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    unsafe { thread_exit() }
}

fn body_chunk(bytes: &[u8]) -> Result<ObjectChunk, ()> {
    let memory = OwnedMemory::allocate(1).map_err(|_| ())?;
    let mut mapping = memory.map_writable().map_err(|_| ())?;
    mapping.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
    let memory = mapping.unmap().map_err(|_| ())?;
    ObjectChunk::new(memory, bytes.len()).map_err(|_| ())
}

fn put_when_ready(client: &Client<'_>, digest: [u8; 32]) -> Result<(), ()> {
    for _ in 0..240 {
        let request = ObjectRequest::put(KEY, BODY.len() as u64, digest);
        match client.put(request) {
            Ok(mut put) => {
                match put.write(body_chunk(BODY)?) {
                    Ok(len) if len == BODY.len() => {}
                    Ok(_)
                    | Err(PutWriteError::Failed(_))
                    | Err(PutWriteError::NotSubmitted {
                        ..
                    }) => {
                        return Err(());
                    }
                }
                let info = put.finish().map_err(|_| ())?;
                return (info.status == 200 || info.status == 201).then_some(()).ok_or(());
            }
            Err(Error::Service(ERR_UNSYNCHRONIZED)) | Err(Error::Service(ERR_TRANSPORT)) => {
                sleep_ms(500)
            }
            Err(_) => return Err(()),
        }
    }
    Err(())
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns = ctx.bootstrap_connection().unwrap_or_else(|| fail(0x5301));
    let (_, connection) =
        wait_for_registered_name_owned(ns, s3::NAME).unwrap_or_else(|| fail(0x5302));
    let client = Client::new(connection.as_ref());

    let mut sha256 = Sha256::new();
    sha256.update(BODY);
    put_when_ready(&client, sha256.finalize()).unwrap_or_else(|_| fail(0x5303));
    config::write::<u32>(status::STAGE, 2);

    let info = client.head(ObjectRequest::get(KEY)).unwrap_or_else(|_| fail(0x5304));
    if info.content_length != BODY.len() as u64 {
        fail(0x5305);
    }

    let (mut get, info) = client.get(ObjectRequest::get(KEY)).unwrap_or_else(|_| fail(0x5306));
    if info.content_length != BODY.len() as u64 {
        fail(0x5307);
    }
    let mut received = Vec::new();
    while let Some(chunk) = get.read().unwrap_or_else(|_| fail(0x5308)) {
        let (memory, len) = chunk.into_parts();
        let mapping = memory.map_read_only().unwrap_or_else(|_| fail(0x5309));
        received.extend_from_slice(&mapping.as_slice()[..len]);
    }
    get.close().unwrap_or_else(|_| fail(0x530a));
    if received != BODY {
        fail(0x530b);
    }
    config::write::<u32>(status::BYTES, received.len() as u32);
    config::write::<u32>(status::STAGE, 3);

    client.delete(ObjectRequest::get(KEY)).unwrap_or_else(|_| fail(0x530c));
    config::write::<u32>(status::STAGE, status::SUCCESS);
    catten_rt::logln!("[s3-smoke] S3 TLS PUT/HEAD/GET/DELETE round trip succeeded");
    unsafe { thread_exit() }
}
