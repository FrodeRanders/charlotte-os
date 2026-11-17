//! Generic consume--procedure--produce transactional Kafka step.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use catten_rt::{
    Context,
    ShutdownRequest,
    config,
    owned::{
        ConnectionRef,
        OwnedMemory,
    },
};
use catten_services::{
    kafka_client::{
        Client,
        DeliveryInfo,
        DeliveryToken,
        Error as KafkaError,
        Route,
        Transaction,
    },
    kafka_step::{
        self as protocol,
        AttemptTracker,
        FailureAction,
        OutputBatch,
    },
    sleep_ms,
    wait_for_registered_name_bytes_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::kafka_step_status as status;
use charlotte_protocol_kafka::{
    DeliveredRecord,
    RecordRequest,
};

catten_rt::entry!(main);

struct Profile {
    procedure_name: Vec<u8>,
    kafka_connector_name: Vec<u8>,
    allowed_routes: Vec<u16>,
    dlq_route: u16,
    max_outputs: usize,
    max_attempts: u16,
    procedure_timeout_ms: u32,
    retry_backoff_ms: u32,
    idle_poll_ms: u32,
}

impl Profile {
    fn from_context(ctx: &Context) -> Option<Self> {
        let memory = ctx.profile_memory()?;
        let mapping = memory.map_read_only().ok()?;
        let profile = protocol::Profile::decode(mapping.as_slice()).ok()?;
        Some(Self {
            procedure_name: profile.procedure_name.to_vec(),
            kafka_connector_name: profile.kafka_connector_name.to_vec(),
            allowed_routes: profile.allowed_routes,
            dlq_route: profile.dlq_route,
            max_outputs: usize::from(profile.max_outputs),
            max_attempts: profile.max_attempts,
            procedure_timeout_ms: profile.procedure_timeout_ms,
            retry_backoff_ms: profile.retry_backoff_ms,
            idle_poll_ms: profile.idle_poll_ms,
        })
    }

    fn allows(&self, route: u16) -> bool {
        self.allowed_routes.contains(&route)
    }
}

struct Counters {
    polled: u32,
    invoked: u32,
    produced: u32,
    commits: u32,
    retries: u32,
    dlq: u32,
    timeouts: u32,
    aborts: u32,
}

impl Counters {
    const fn new() -> Self {
        Self {
            polled: 0,
            invoked: 0,
            produced: 0,
            commits: 0,
            retries: 0,
            dlq: 0,
            timeouts: 0,
            aborts: 0,
        }
    }

    fn publish(&self) {
        for (offset, value) in [
            (status::POLLED, self.polled),
            (status::INVOKED, self.invoked),
            (status::PRODUCED, self.produced),
            (status::COMMITS, self.commits),
            (status::RETRIES, self.retries),
            (status::DLQ, self.dlq),
            (status::TIMEOUTS, self.timeouts),
            (status::ABORTS, self.aborts),
        ] {
            config::write::<u32>(offset, value);
        }
    }
}

struct StepOperation<'connection> {
    transaction: Option<Transaction<'connection>>,
    delivery: Option<DeliveryToken<'connection>>,
    input: Option<OwnedMemory>,
    info: DeliveryInfo,
}

impl<'connection> StepOperation<'connection> {
    fn begin(&mut self, kafka: &Client<'connection>) -> Result<(), KafkaError> {
        self.transaction = Some(kafka.begin_transaction()?);
        Ok(())
    }

    fn transaction(&mut self) -> Result<&mut Transaction<'connection>, KafkaError> {
        self.transaction.as_mut().ok_or(KafkaError::InvalidRequest)
    }

    fn include_and_commit(mut self) -> Result<(), KafkaError> {
        let delivery = self.delivery.take().ok_or(KafkaError::InvalidRequest)?;
        self.transaction()?.include(delivery)?;
        self.transaction.take().ok_or(KafkaError::InvalidRequest)?.commit()
    }
}

enum Invocation {
    Complete,
    Outputs {
        memory: OwnedMemory,
        len: usize,
    },
    Retry,
    Terminal,
    Timeout,
    Invalid,
}

fn invoke(
    procedure: ConnectionRef<'_>,
    input: &OwnedMemory,
    attempt: u16,
    timeout_ms: u32,
) -> Invocation {
    let Ok(mut call) = procedure.call_borrow_read(protocol::OP_INVOKE, u64::from(attempt), input)
    else {
        return Invocation::Retry;
    };
    let mut elapsed = 0u32;
    loop {
        match call.poll() {
            Ok(Some(reply)) => {
                return match (reply.result, reply.memory) {
                    (0, None) => Invocation::Complete,
                    (result, Some(memory)) if result > 0 => match usize::try_from(result) {
                        Ok(len) => Invocation::Outputs {
                            memory,
                            len,
                        },
                        Err(_) => Invocation::Invalid,
                    },
                    (protocol::RESULT_RETRY, None) => Invocation::Retry,
                    (protocol::RESULT_TERMINAL, None) => Invocation::Terminal,
                    _ => Invocation::Invalid,
                };
            }
            Ok(None) => {}
            Err(_) => return Invocation::Retry,
        }
        if elapsed >= timeout_ms {
            // Dropping the pending call cancels it and revokes the input loan
            // before this function returns the delivery owner to the runner.
            return Invocation::Timeout;
        }
        let wait = (timeout_ms - elapsed).min(10);
        sleep_ms(u64::from(wait));
        elapsed += wait;
    }
}

fn transact_outputs<'connection>(
    kafka: &Client<'connection>,
    operation: &mut StepOperation<'connection>,
    profile: &Profile,
    memory: OwnedMemory,
    len: usize,
    counters: &mut Counters,
) -> Result<(), OutputFailure> {
    let mapping = memory.map_read_only().map_err(|_| OutputFailure::Invalid)?;
    let bytes = mapping.as_slice().get(..len).ok_or(OutputFailure::Invalid)?;
    let batch = OutputBatch::decode(bytes).map_err(|_| OutputFailure::Invalid)?;
    if batch.records.len() > profile.max_outputs
        || batch.records.iter().any(|record| !profile.allows(record.route))
    {
        return Err(OutputFailure::Invalid);
    }
    operation.begin(kafka).map_err(|_| OutputFailure::Kafka)?;
    for output in batch.records {
        operation
            .transaction()
            .map_err(|_| OutputFailure::Kafka)?
            .produce_to(
                Route::provisioned(output.route),
                RecordRequest::new(output.key, output.value),
            )
            .map_err(|_| OutputFailure::Kafka)?;
        counters.produced = counters.produced.wrapping_add(1);
    }
    Ok(())
}

enum OutputFailure {
    Invalid,
    Kafka,
}

fn dead_letter<'connection>(
    kafka: &Client<'connection>,
    operation: &mut StepOperation<'connection>,
    route: u16,
    counters: &mut Counters,
) -> Result<(), ()> {
    let input = operation.input.take().ok_or(())?;
    let mapping = input.map_read_only().map_err(|_| ())?;
    let record = DeliveredRecord::decode(mapping.as_slice()).ok_or(())?;
    operation.begin(kafka).map_err(|_| ())?;
    operation
        .transaction()
        .map_err(|_| ())?
        .produce_to(Route::provisioned(route), RecordRequest::new(record.key, record.value))
        .map_err(|_| ())?;
    counters.produced = counters.produced.wrapping_add(1);
    counters.dlq = counters.dlq.wrapping_add(1);
    Ok(())
}

fn finish_transaction(
    operation: StepOperation<'_>,
    tracker: &mut AttemptTracker,
    counters: &mut Counters,
) -> bool {
    let info = operation.info;
    match operation.include_and_commit() {
        Ok(()) => {
            tracker.complete(info.partition, info.offset);
            counters.commits = counters.commits.wrapping_add(1);
            true
        }
        Err(_) => {
            counters.aborts = counters.aborts.wrapping_add(1);
            false
        }
    }
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    unsafe { thread_exit() }
}

fn run(ctx: &Context) -> ShutdownRequest {
    config::write::<u32>(status::STAGE, 1);
    let profile = Profile::from_context(ctx).unwrap_or_else(|| fail(0x4c01));
    let ns = ctx.bootstrap_connection().unwrap_or_else(|| fail(0x4c02));
    let (_, kafka_connection) =
        wait_for_registered_name_bytes_owned(ns, &profile.kafka_connector_name)
            .unwrap_or_else(|| fail(0x4c03));
    let (_, procedure_connection) =
        wait_for_registered_name_bytes_owned(ns, &profile.procedure_name)
            .unwrap_or_else(|| fail(0x4c04));
    let kafka = Client::new(kafka_connection.as_ref());
    let mut consumer = kafka.consumer().unwrap_or_else(|_| fail(0x4c05));
    let mut tracker = AttemptTracker::new();
    let mut counters = Counters::new();
    config::write::<u32>(status::STAGE, 2);

    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            drop(consumer);
            return request;
        }
        let delivery = match consumer.poll() {
            Ok(Some(delivery)) => delivery,
            Ok(None) => {
                sleep_ms(u64::from(profile.idle_poll_ms));
                continue;
            }
            Err(_) => {
                sleep_ms(u64::from(profile.retry_backoff_ms));
                continue;
            }
        };
        counters.polled = counters.polled.wrapping_add(1);
        let (token, input, info) = delivery.into_parts();
        let mut operation = StepOperation {
            transaction: None,
            delivery: Some(token),
            input: Some(input),
            info,
        };
        let Some(attempt) = tracker.attempt(info.partition, info.offset) else {
            config::write::<u32>(status::ERROR, 0x4c06);
            drop(operation);
            sleep_ms(u64::from(profile.retry_backoff_ms));
            continue;
        };
        counters.invoked = counters.invoked.wrapping_add(1);
        let invocation = invoke(
            procedure_connection.as_ref(),
            operation.input.as_ref().expect("step input missing"),
            attempt,
            profile.procedure_timeout_ms,
        );
        if matches!(invocation, Invocation::Timeout) {
            counters.timeouts = counters.timeouts.wrapping_add(1);
        }
        let outcome = match invocation {
            Invocation::Complete => match operation.begin(&kafka) {
                Ok(()) => AttemptOutcome::Ready,
                Err(_) => AttemptOutcome::KafkaFailure,
            },
            Invocation::Outputs {
                memory,
                len,
            } => {
                match transact_outputs(&kafka, &mut operation, &profile, memory, len, &mut counters)
                {
                    Ok(()) => AttemptOutcome::Ready,
                    Err(OutputFailure::Invalid) => AttemptOutcome::ProcedureFailure {
                        terminal: true,
                    },
                    Err(OutputFailure::Kafka) => AttemptOutcome::KafkaFailure,
                }
            }
            Invocation::Retry | Invocation::Timeout => AttemptOutcome::ProcedureFailure {
                terminal: false,
            },
            Invocation::Terminal | Invocation::Invalid => AttemptOutcome::ProcedureFailure {
                terminal: true,
            },
        };
        match outcome {
            AttemptOutcome::Ready => {
                let committed = finish_transaction(operation, &mut tracker, &mut counters);
                if !committed {
                    counters.retries = counters.retries.wrapping_add(1);
                }
                counters.publish();
                if !committed {
                    sleep_ms(u64::from(profile.retry_backoff_ms));
                }
                continue;
            }
            AttemptOutcome::KafkaFailure => {
                counters.aborts = counters.aborts.wrapping_add(1);
                counters.retries = counters.retries.wrapping_add(1);
                counters.publish();
                drop(operation);
                sleep_ms(u64::from(profile.retry_backoff_ms));
                continue;
            }
            AttemptOutcome::ProcedureFailure {
                terminal,
            } => {
                let action = if terminal {
                    FailureAction::DeadLetter
                } else {
                    tracker.fail(info.partition, info.offset, profile.max_attempts)
                };
                if action == FailureAction::DeadLetter
                    && dead_letter(&kafka, &mut operation, profile.dlq_route, &mut counters).is_ok()
                {
                    let committed = finish_transaction(operation, &mut tracker, &mut counters);
                    if !committed {
                        counters.retries = counters.retries.wrapping_add(1);
                    }
                    counters.publish();
                    if !committed {
                        sleep_ms(u64::from(profile.retry_backoff_ms));
                    }
                    continue;
                }
            }
        }
        counters.retries = counters.retries.wrapping_add(1);
        counters.publish();
        drop(operation);
        sleep_ms(u64::from(profile.retry_backoff_ms));
    }
}

fn main(ctx: Context) -> ! {
    run(&ctx).complete()
}

enum AttemptOutcome {
    Ready,
    KafkaFailure,
    ProcedureFailure {
        terminal: bool,
    },
}
