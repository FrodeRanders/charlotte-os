//! CharlotteOS observability service.
//!
//! Exposes machine-wide scheduler statistics through endpoint IPC. It also
//! samples a bounded, in-memory resource history so operators can see trends
//! rather than only the current instant. History lives in this service's own
//! heap and is lost on restart; durable archival is a later phase of the
//! adaptive resource policy design.
#![no_std]
#![no_main]

extern crate alloc;

catten_rt::entry!(main);

use alloc::collections::VecDeque;

use catten_rt::{
    Context,
    owned::{
        Endpoint,
        OwnedMemory,
        ReceiveError,
        ReplyToken,
    },
};
use catten_services::{
    ns,
    observability,
};
use catten_syscall::{
    IpcRights,
    THREAD_STATISTICS_DOMAIN_RECORD_U64S,
    THREAD_STATISTICS_HEADER_U64S,
    THREAD_STATISTICS_MAGIC,
    THREAD_STATISTICS_RECORD_U64S,
    THREAD_STATISTICS_VERSION,
    cq_wait_timeout,
    monotonic_clock,
    thread_domain_record as domain_record,
    thread_exit,
    thread_statistics_header as statistics_header,
    thread_statistics_record as thread_record,
    thread_statistics_snapshot,
};

/// Aggregated sample retained in the history ring.
#[derive(Clone, Copy, Default)]
struct HistorySample {
    ticks: u64,
    threads: u64,
    domains: u64,
    owned_frames: u64,
    stack_pages: u64,
    stack_used_high_water: u64,
    threads_high_water: u64,
}

/// Upper bound on records read from one kernel snapshot. The kernel bounds its
/// own tables well below this; the check keeps a corrupt header from driving a
/// long loop in the service.
const MAX_SNAPSHOT_RECORDS: usize = 4096;

fn read_word(bytes: &[u8], index: usize) -> Option<u64> {
    let start = index.checked_mul(8)?;
    let end = start.checked_add(8)?;
    Some(u64::from_le_bytes(bytes.get(start..end)?.try_into().ok()?))
}

fn snapshot_sample(system_observer: u64) -> Option<HistorySample> {
    let (memory, length) = thread_statistics_snapshot(system_observer);
    if memory == 0 || length == 0 {
        return None;
    }
    // The syscall creates a fresh capability for this caller.
    let memory = unsafe { OwnedMemory::from_raw(memory) }.ok()?;
    let mapping = memory.map_read_only().ok()?;
    let bytes = mapping.as_slice();
    if read_word(bytes, statistics_header::MAGIC)? != THREAD_STATISTICS_MAGIC
        || read_word(bytes, statistics_header::VERSION)? != THREAD_STATISTICS_VERSION
        || read_word(bytes, statistics_header::RECORD_BYTES)?
            != (THREAD_STATISTICS_RECORD_U64S * core::mem::size_of::<u64>()) as u64
        || read_word(bytes, statistics_header::DOMAIN_RECORD_BYTES)?
            != (THREAD_STATISTICS_DOMAIN_RECORD_U64S * core::mem::size_of::<u64>()) as u64
    {
        return None;
    }
    let ticks = read_word(bytes, statistics_header::MONOTONIC_TICKS)?;
    let thread_count = read_word(bytes, statistics_header::RECORD_COUNT)? as usize;
    let domain_count = read_word(bytes, statistics_header::DOMAIN_RECORD_COUNT)? as usize;
    if thread_count > MAX_SNAPSHOT_RECORDS || domain_count > MAX_SNAPSHOT_RECORDS {
        return None;
    }

    let mut stack_pages = 0u64;
    let mut stack_used_high_water = 0u64;
    for index in 0..thread_count {
        let base = THREAD_STATISTICS_HEADER_U64S + index * THREAD_STATISTICS_RECORD_U64S;
        stack_pages = stack_pages
            .saturating_add(read_word(bytes, base + thread_record::STACK_RESERVED_PAGES)?);
        stack_used_high_water =
            stack_used_high_water.max(read_word(bytes, base + thread_record::STACK_USED_PAGES)?);
    }

    let domain_base = THREAD_STATISTICS_HEADER_U64S + thread_count * THREAD_STATISTICS_RECORD_U64S;
    let mut owned_frames = 0u64;
    let mut threads_high_water = 0u64;
    for index in 0..domain_count {
        let base = domain_base + index * THREAD_STATISTICS_DOMAIN_RECORD_U64S;
        owned_frames =
            owned_frames.saturating_add(read_word(bytes, base + domain_record::OWNED_FRAMES)?);
        threads_high_water =
            threads_high_water.max(read_word(bytes, base + domain_record::THREADS_HIGH_WATER)?);
    }

    Some(HistorySample {
        ticks,
        threads: thread_count as u64,
        domains: domain_count as u64,
        owned_frames,
        stack_pages,
        stack_used_high_water,
        threads_high_water,
    })
}

fn write_word(bytes: &mut [u8], index: usize, value: u64) {
    bytes[index * 8..(index + 1) * 8].copy_from_slice(&value.to_le_bytes());
}

fn reply_history(reply: ReplyToken, frequency_hz: u64, history: &VecDeque<HistorySample>) {
    use observability::{
        history_header as header,
        history_record as record,
    };

    let exact_len = (header::WORDS + history.len() * record::WORDS) * core::mem::size_of::<u64>();
    let pages = exact_len.div_ceil(4096);
    let Ok(memory) = OwnedMemory::allocate(pages) else {
        let _ = reply.reply(observability::ERR_UNAVAILABLE);
        return;
    };
    let Ok(mut mapping) = memory.map_writable() else {
        let _ = reply.reply(observability::ERR_UNAVAILABLE);
        return;
    };
    {
        let bytes = mapping.as_mut_slice();
        write_word(bytes, header::MAGIC, observability::HISTORY_MAGIC);
        write_word(bytes, header::VERSION, observability::HISTORY_VERSION);
        write_word(
            bytes,
            header::RECORD_BYTES,
            (record::WORDS * core::mem::size_of::<u64>()) as u64,
        );
        write_word(bytes, header::RECORD_COUNT, history.len() as u64);
        write_word(bytes, header::COUNTER_FREQUENCY_HZ, frequency_hz);
        write_word(bytes, header::SAMPLE_INTERVAL_MS, observability::HISTORY_SAMPLE_INTERVAL_MS);
        for (index, sample) in history.iter().enumerate() {
            let base = header::WORDS + index * record::WORDS;
            write_word(bytes, base + record::MONOTONIC_TICKS, sample.ticks);
            write_word(bytes, base + record::THREADS, sample.threads);
            write_word(bytes, base + record::DOMAINS, sample.domains);
            write_word(bytes, base + record::OWNED_FRAMES, sample.owned_frames);
            write_word(bytes, base + record::STACK_PAGES, sample.stack_pages);
            write_word(bytes, base + record::STACK_USED_HIGH_WATER, sample.stack_used_high_water);
            write_word(bytes, base + record::THREADS_HIGH_WATER, sample.threads_high_water);
        }
    }
    match mapping.unmap() {
        Ok(memory) => {
            let _ = reply.reply_move(memory, exact_len as i64);
        }
        Err(_) => {
            let _ = reply.reply(observability::ERR_UNAVAILABLE);
        }
    }
}

fn main(ctx: Context) -> ! {
    let ns_connection = ctx.bootstrap_connection().unwrap_or_else(|| unsafe { thread_exit() });
    let system_observer = ctx.system_observer_cap().unwrap_or_else(|| unsafe { thread_exit() });
    let endpoint = Endpoint::create(observability::INTERFACE, observability::VERSION, 16)
        .unwrap_or_else(|_| unsafe { thread_exit() });
    let generation = ns_connection
        .call_connection(
            ns::OP_REGISTER,
            observability::NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .and_then(|call| call.wait())
        .unwrap_or_else(|_| unsafe { thread_exit() })
        .result;
    if generation < 1 || endpoint.bind_completion_queue(0).is_err() {
        unsafe { thread_exit() };
    }

    let mut history: VecDeque<HistorySample> =
        VecDeque::with_capacity(observability::HISTORY_CAPACITY);
    let (mut ticks, frequency_hz) = monotonic_clock();
    let interval_ticks =
        frequency_hz.saturating_mul(observability::HISTORY_SAMPLE_INTERVAL_MS) / 1000;
    let mut next_sample_ticks = ticks.saturating_add(interval_ticks.max(1));
    if let Some(sample) = snapshot_sample(system_observer) {
        history.push_back(sample);
    }

    loop {
        if ticks >= next_sample_ticks {
            if let Some(sample) = snapshot_sample(system_observer) {
                if history.len() == observability::HISTORY_CAPACITY {
                    history.pop_front();
                }
                history.push_back(sample);
            }
            next_sample_ticks = ticks.saturating_add(interval_ticks.max(1));
        }

        let remaining_ms = if ticks >= next_sample_ticks {
            1
        } else {
            ((next_sample_ticks - ticks) * 1000 / frequency_hz.max(1)).max(1)
        };
        cq_wait_timeout(1, remaining_ms, 0);
        ticks = monotonic_clock().0;

        loop {
            let message = match endpoint.try_receive() {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(ReceiveError::EndpointClosed) => unsafe { thread_exit() },
                Err(_) => continue,
            };
            let Some(reply) = message.reply else {
                continue;
            };
            match message.opcode {
                observability::OP_THREAD_SNAPSHOT => {
                    let (memory, length) = thread_statistics_snapshot(system_observer);
                    if memory == 0 || length == 0 {
                        let _ = reply.reply(observability::ERR_UNAVAILABLE);
                    } else {
                        // thread_statistics_snapshot creates a new owned
                        // memory capability for this caller.
                        match unsafe { OwnedMemory::from_raw(memory) } {
                            Ok(memory) => {
                                let _ = reply.reply_move(memory, length as i64);
                            }
                            Err(_) => {
                                let _ = reply.reply(observability::ERR_UNAVAILABLE);
                            }
                        }
                    }
                }
                observability::OP_HISTORY => {
                    reply_history(reply, frequency_hz, &history);
                }
                _ => {
                    let _ = reply.reply(observability::ERR_BAD_OPCODE);
                }
            }
        }
    }
}
