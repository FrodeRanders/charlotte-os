//! Verify that a connector fails closed when Kafka fences its transactional
//! producer identity.
#![no_std]
#![no_main]

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    kafka as protocol,
    kafka_client::{
        Client,
        Error,
    },
    wait_for_registered_name_bytes_owned,
};
use catten_syscall::thread_exit;
use charlotte_protocol_kafka::RecordRequest;

catten_rt::entry!(main);

const TRANSACTION_NAME: &[u8] = b"kafka/selftest/main/transactional";

fn fail(code: u32) -> ! {
    config::write::<u32>(charlotte_launch::kafka_fence_smoke_status::ERROR, code);
    unsafe { thread_exit() }
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(charlotte_launch::kafka_fence_smoke_status::STAGE, 1);
    let ns = ctx.bootstrap_connection().unwrap_or_else(|| fail(0x4b31));
    let (_, connection) =
        wait_for_registered_name_bytes_owned(ns, TRANSACTION_NAME).unwrap_or_else(|| fail(0x4b32));
    let client = Client::new(connection.as_ref());
    let mut transaction = client.begin_transaction().unwrap_or_else(|_| fail(0x4b33));
    match transaction.produce(RecordRequest::new(None, Some(b"must-be-fenced"))) {
        Err(Error::Service(protocol::ERR_FENCED)) => {}
        Err(_) => fail(0x4b34),
        Ok(_) => fail(0x4b35),
    }
    drop(transaction);
    config::write::<u32>(
        charlotte_launch::kafka_fence_smoke_status::STAGE,
        charlotte_launch::kafka_fence_smoke_status::SUCCESS,
    );
    catten_rt::logln!("[kafka-fence-smoke] stale transactional producer was fenced");
    unsafe { thread_exit() }
}
