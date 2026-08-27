//! Test procedure for the generic transactional Kafka-step integration path.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
    owned::{
        Endpoint,
        OwnedMemory,
        ReplyToken,
    },
};
use catten_services::{
    kafka_step::{
        self as protocol,
        OutputBatch,
        OutputRecord,
    },
    ns,
    sleep_ms,
};
use catten_syscall::{
    IpcRights,
    thread_exit,
};
use charlotte_launch::kafka_step_procedure_status as status;
use charlotte_protocol_kafka::DeliveredRecord;

catten_rt::entry!(main);

const NAME: u64 = catten_services::name(b"kproc");
const INPUT: &[u8] = b"charlotte-step-input";
const RETRY_INPUT: &[u8] = b"charlotte-step-retry";
const TIMEOUT_INPUT: &[u8] = b"charlotte-step-timeout";
const DLQ_INPUT: &[u8] = b"charlotte-step-dlq";
const OUTPUT: &[u8] = b"charlotte-step-output";

enum Action {
    Retry,
    Timeout,
    Terminal,
    Output,
    Complete,
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    unsafe { thread_exit() }
}

fn reply_output(reply: ReplyToken, value: &[u8]) -> bool {
    let memory = match OwnedMemory::allocate(1) {
        Ok(memory) => memory,
        Err(_) => return false,
    };
    let mut mapping = match memory.map_writable() {
        Ok(mapping) => mapping,
        Err(_) => return false,
    };
    let batch = OutputBatch {
        records: alloc::vec![OutputRecord {
            route: 1,
            key: None,
            value: Some(value),
        }],
    };
    let len = match batch.encode(mapping.as_mut_slice()) {
        Ok(len) => len,
        Err(_) => return false,
    };
    let memory = match mapping.unmap() {
        Ok(memory) => memory,
        Err(_) => return false,
    };
    reply.reply_move(memory, len as i64).is_ok()
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns = ctx.bootstrap_connection().unwrap_or_else(|| fail(0x4d01));
    let endpoint = Endpoint::create(protocol::INTERFACE, protocol::VERSION, 8)
        .unwrap_or_else(|_| fail(0x4d02));
    let registration = ns
        .call_connection(
            ns::OP_REGISTER,
            NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .unwrap_or_else(|_| fail(0x4d03));
    if !registration.wait().is_ok_and(|reply| reply.result >= 1) {
        fail(0x4d03);
    }
    config::write::<u32>(status::STAGE, 2);
    let mut invocations = 0u32;
    loop {
        let mut message = match endpoint.receive() {
            Ok(message) => message,
            Err(_) => fail(0x4d04),
        };
        let Some(reply) = message.reply.take() else {
            continue;
        };
        if message.opcode != protocol::OP_INVOKE
            || message.arg0 == 0
            || message.arg0 > u16::MAX as u64
        {
            let _ = reply.reply(protocol::RESULT_INVALID);
            continue;
        }
        invocations = invocations.wrapping_add(1);
        config::write::<u32>(status::INVOCATIONS, invocations);
        let Some(memory) = message.memory.take() else {
            let _ = reply.reply(protocol::RESULT_INVALID);
            continue;
        };
        let mapping = match memory.map_read_only() {
            Ok(mapping) => mapping,
            Err(_) => {
                let _ = reply.reply(protocol::RESULT_INVALID);
                continue;
            }
        };
        let Some(record) = DeliveredRecord::decode(mapping.as_slice()) else {
            let _ = reply.reply(protocol::RESULT_INVALID);
            continue;
        };
        let value = record.value.unwrap_or_default();
        let attempt = message.arg0 as u16;
        let action = if value == RETRY_INPUT && attempt == 1 {
            Action::Retry
        } else if value == TIMEOUT_INPUT && attempt == 1 {
            Action::Timeout
        } else if value == DLQ_INPUT {
            Action::Terminal
        } else if value == INPUT || value == RETRY_INPUT || value == TIMEOUT_INPUT {
            Action::Output
        } else {
            Action::Complete
        };
        // End the input mapping before replying so loan termination never
        // races a live server-side mapping.
        drop(mapping);
        match action {
            Action::Retry => {
                let _ = reply.reply(protocol::RESULT_RETRY);
            }
            Action::Timeout => {
                sleep_ms(100);
                let _ = reply.reply(0);
            }
            Action::Terminal => {
                let _ = reply.reply(protocol::RESULT_TERMINAL);
            }
            Action::Output => {
                if !reply_output(reply, OUTPUT) {
                    config::write::<u32>(status::ERROR, 0x4d05);
                }
            }
            Action::Complete => {
                let _ = reply.reply(0);
            }
        }
    }
}
