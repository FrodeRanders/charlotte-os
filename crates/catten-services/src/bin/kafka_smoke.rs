//! End-to-end Kafka producer, read-committed consumer, and transactional
//! offset smoke application.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

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
    sleep_ms,
    wait_for_registered_name_owned,
};
use catten_syscall::thread_exit;
use charlotte_protocol_kafka::RecordRequest;

catten_rt::entry!(main);

const INPUT: &[u8] = b"charlotte-input";
const OUTPUT: &[u8] = b"charlotte-output";
const ABORTED: &[u8] = b"charlotte-aborted";

mod status {
    pub const STAGE: usize = 0;
    pub const ERROR: usize = 4;
    pub const OFFSET: usize = 8;
    pub const SUCCESS: u32 = 0x4b46_4f4b; // "KFOK"
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    unsafe { thread_exit() }
}

fn produce_when_ready(client: &Client<'_>, value: &[u8]) -> Result<i64, Error> {
    for _ in 0..120 {
        match client.produce(RecordRequest::new(None, Some(value))) {
            Ok(offset) => return Ok(offset),
            Err(Error::Service(protocol::ERR_TIMEOUT))
            | Err(Error::Service(protocol::ERR_TRANSPORT)) => sleep_ms(500),
            Err(error) => return Err(error),
        }
    }
    Err(Error::Service(protocol::ERR_TIMEOUT))
}

fn next_value<'connection>(
    consumer: &mut catten_services::kafka_client::Consumer<'connection>,
) -> Result<Option<(catten_services::kafka_client::DeliveryToken<'connection>, Vec<u8>, i64)>, Error>
{
    let Some(delivery) = consumer.poll()? else {
        return Ok(None);
    };
    let (token, memory, info) = delivery.into_parts();
    let mapping = memory.map_read_only().map_err(|(_, error)| Error::Memory(error))?;
    let value =
        info.value_range().map(|range| mapping.as_slice()[range].to_vec()).unwrap_or_default();
    Ok(Some((token, value, info.offset)))
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns = ctx.bootstrap_connection().unwrap_or_else(|| fail(0x4b11));
    let (_, connection) =
        wait_for_registered_name_owned(ns, protocol::NAME).unwrap_or_else(|| fail(0x4b12));
    let client = Client::new(connection.as_ref());

    let input_offset = produce_when_ready(&client, INPUT).unwrap_or_else(|_| fail(0x4b13));
    config::write::<u32>(status::STAGE, 2);

    let mut consumer = client.consumer().unwrap_or_else(|_| fail(0x4b14));
    let (input, input_value, observed_offset) =
        next_value(&mut consumer).unwrap_or_else(|_| fail(0x4b15)).unwrap_or_else(|| fail(0x4b16));
    if input_value != INPUT || observed_offset != input_offset {
        fail(0x4b17);
    }

    let mut transaction = client.begin_transaction().unwrap_or_else(|_| fail(0x4b18));
    let output_offset = transaction
        .produce(RecordRequest::new(None, Some(OUTPUT)))
        .unwrap_or_else(|_| fail(0x4b19));
    transaction.include(input).unwrap_or_else(|_| fail(0x4b1a));
    transaction.commit().unwrap_or_else(|_| fail(0x4b1b));
    config::write::<u32>(status::STAGE, 3);

    let (output, output_value, observed_output_offset) =
        next_value(&mut consumer).unwrap_or_else(|_| fail(0x4b1c)).unwrap_or_else(|| fail(0x4b1d));
    if output_value != OUTPUT || observed_output_offset != output_offset {
        fail(0x4b1e);
    }
    output.commit().unwrap_or_else(|_| fail(0x4b1f));

    let mut aborted = client.begin_transaction().unwrap_or_else(|_| fail(0x4b20));
    aborted.produce(RecordRequest::new(None, Some(ABORTED))).unwrap_or_else(|_| fail(0x4b21));
    aborted.abort().unwrap_or_else(|_| fail(0x4b22));
    if let Some((delivery, value, _)) = next_value(&mut consumer).unwrap_or_else(|_| fail(0x4b23)) {
        if value == ABORTED {
            fail(0x4b24);
        }
        drop(delivery);
        fail(0x4b25);
    }
    consumer.close().unwrap_or_else(|_| fail(0x4b26));

    config::write::<u32>(status::OFFSET, output_offset as u32);
    config::write::<u32>(status::STAGE, status::SUCCESS);
    catten_rt::logln!(
        "[kafka-smoke] idempotent produce, read_committed consume, transactional offset commit, \
         and abort filtering succeeded"
    );
    unsafe { thread_exit() }
}
