//! Input producer for the generic transactional Kafka-step integration test.
#![no_std]
#![no_main]

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    kafka,
    kafka_client::{
        Client,
        Error,
    },
    sleep_ms,
    wait_for_registered_name_bytes_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::kafka_step_input_status as status;
use charlotte_protocol_kafka::RecordRequest;

catten_rt::entry!(main);

const INPUTS: [&[u8]; 4] = [
    b"charlotte-step-input",
    b"charlotte-step-retry",
    b"charlotte-step-timeout",
    b"charlotte-step-dlq",
];
const CONNECTOR_NAME: &[u8] = b"kafka/selftest/main/transactional";

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    unsafe { thread_exit() }
}

fn produce(client: &Client<'_>, value: &[u8]) -> Result<(), Error> {
    for _ in 0..120 {
        match client.produce(RecordRequest::new(None, Some(value))) {
            Ok(_) => return Ok(()),
            Err(Error::Service(kafka::ERR_TIMEOUT | kafka::ERR_TRANSPORT)) => sleep_ms(100),
            Err(error) => return Err(error),
        }
    }
    Err(Error::Service(kafka::ERR_TIMEOUT))
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns = ctx.bootstrap_connection().unwrap_or_else(|| fail(0x4e01));
    let (_, connection) =
        wait_for_registered_name_bytes_owned(ns, CONNECTOR_NAME).unwrap_or_else(|| fail(0x4e02));
    let client = Client::new(connection.as_ref());
    for input in INPUTS {
        produce(&client, input).unwrap_or_else(|_| fail(0x4e03));
    }
    config::write::<u32>(status::STAGE, status::SUCCESS);
    unsafe { thread_exit() }
}
