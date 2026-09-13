//! CharlotteOS observability service.
//!
//! Exposes machine-wide scheduler statistics through endpoint IPC. It also
//! samples a bounded, in-memory resource history so operators can see trends
//! rather than only the current instant. History lives in this service's own
//! heap and is also archived asynchronously to a bounded object-store ring.
#![no_std]
#![no_main]

extern crate alloc;

catten_rt::entry!(main);

use alloc::collections::VecDeque;

use catten_rt::{
    Context,
    owned::{
        Connection,
        Endpoint,
        OwnedMemory,
        PendingCall,
        ReceiveError,
        ReplyToken,
    },
};
use catten_services::{
    ns,
    objstore,
    observability,
    try_registered_name_owned,
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
    free_frames: u64,
    usable_frames: u64,
    logical_processors: u64,
    cpu_busy_ticks: u64,
    threads: u64,
    domains: u64,
    owned_frames: u64,
    heap_allocated_bytes: u64,
    heap_peak_bytes: u64,
    stack_pages: u64,
    stack_used_high_water: u64,
    threads_high_water: u64,
}

/// Durable archive: a bounded ring of fixed-size object-store chunks. Each
/// flush rewrites the active chunk, so appends never require an append opcode
/// or more than one directory slot per chunk. Sequence numbers let an offline
/// reader detect overwritten history after the ring wraps.
enum FlushStage {
    Creating {
        call: PendingCall<'static>,
        data: OwnedMemory,
        size: OwnedMemory,
    },
    Sizing {
        call: PendingCall<'static>,
        data: OwnedMemory,
    },
    Writing {
        call: PendingCall<'static>,
    },
    Flushing {
        call: PendingCall<'static>,
    },
}

struct ActiveFlush {
    stage: FlushStage,
    started_ticks: u64,
    record_count: usize,
    full: bool,
}

enum FlushProgress {
    Idle,
    Pending,
    Complete {
        record_count: usize,
        full: bool,
    },
    Failed,
}

struct Archive {
    connection: Connection,
    chunk: alloc::vec::Vec<(u64, HistorySample)>,
    chunk_index: u64,
    session_ticks: u64,
    next_sequence: u64,
    last_flush_ticks: u64,
    active_flush: Option<ActiveFlush>,
}

fn archive_records_per_chunk() -> usize {
    use observability::{
        archive_header as header,
        archive_record as record,
    };
    (observability::ARCHIVE_CHUNK_BYTES / core::mem::size_of::<u64>() - header::WORDS)
        / record::WORDS
}

impl Archive {
    fn new(connection: Connection, ticks: u64) -> Self {
        Self {
            connection,
            chunk: alloc::vec::Vec::new(),
            chunk_index: 0,
            session_ticks: ticks,
            next_sequence: 1,
            last_flush_ticks: ticks,
            active_flush: None,
        }
    }

    fn record(&mut self, sample: HistorySample) {
        self.chunk.push((self.next_sequence, sample));
        self.next_sequence = self.next_sequence.saturating_add(1);
    }

    fn seed_recent(&mut self, history: &VecDeque<HistorySample>) {
        let retain = archive_records_per_chunk();
        for sample in history.iter().skip(history.len().saturating_sub(retain)).copied() {
            self.record(sample);
        }
    }

    fn due(&self, ticks: u64, flush_ticks: u64) -> bool {
        self.active_flush.is_none()
            && !self.chunk.is_empty()
            && (self.chunk.len() >= archive_records_per_chunk()
                || ticks.saturating_sub(self.last_flush_ticks) >= flush_ticks)
    }

    fn start_flush(&mut self, ticks: u64, frequency_hz: u64) -> bool {
        use observability::{
            archive_header as header,
            archive_record as record,
        };
        let record_count = self.chunk.len().min(archive_records_per_chunk());
        let exact_len =
            (header::WORDS + record_count * record::WORDS) * core::mem::size_of::<u64>();
        let pages = exact_len.div_ceil(4096).max(1);
        let Ok(memory) = OwnedMemory::allocate(pages) else {
            return false;
        };
        let Ok(mut mapping) = memory.map_writable() else {
            return false;
        };
        {
            let bytes = mapping.as_mut_slice();
            write_word(bytes, header::MAGIC, observability::ARCHIVE_MAGIC);
            write_word(bytes, header::VERSION, observability::ARCHIVE_VERSION);
            write_word(bytes, header::CHUNK_INDEX, self.chunk_index);
            write_word(bytes, header::SESSION_TICKS, self.session_ticks);
            write_word(
                bytes,
                header::FIRST_SEQUENCE,
                self.chunk.first().map_or(0, |(sequence, _)| *sequence),
            );
            write_word(bytes, header::RECORD_COUNT, record_count as u64);
            write_word(bytes, header::COUNTER_FREQUENCY_HZ, frequency_hz);
            for (index, (sequence, sample)) in self.chunk.iter().take(record_count).enumerate() {
                let base = header::WORDS + index * record::WORDS;
                write_word(bytes, base + record::SEQUENCE, *sequence);
                write_word(bytes, base + record::MONOTONIC_TICKS, sample.ticks);
                write_word(bytes, base + record::THREADS, sample.threads);
                write_word(bytes, base + record::DOMAINS, sample.domains);
                write_word(bytes, base + record::OWNED_FRAMES, sample.owned_frames);
                write_word(bytes, base + record::STACK_PAGES, sample.stack_pages);
                write_word(
                    bytes,
                    base + record::STACK_USED_HIGH_WATER,
                    sample.stack_used_high_water,
                );
                write_word(bytes, base + record::THREADS_HIGH_WATER, sample.threads_high_water);
                write_word(bytes, base + record::FREE_FRAMES, sample.free_frames);
                write_word(bytes, base + record::USABLE_FRAMES, sample.usable_frames);
                write_word(bytes, base + record::LOGICAL_PROCESSORS, sample.logical_processors);
                write_word(bytes, base + record::CPU_BUSY_TICKS, sample.cpu_busy_ticks);
                write_word(bytes, base + record::HEAP_ALLOCATED_BYTES, sample.heap_allocated_bytes);
                write_word(bytes, base + record::HEAP_PEAK_BYTES, sample.heap_peak_bytes);
            }
        }
        let Ok(memory) = mapping.unmap() else {
            return false;
        };

        let Ok(size_memory) = OwnedMemory::allocate(1) else {
            return false;
        };
        let Ok(mut size_mapping) = size_memory.map_writable() else {
            return false;
        };
        size_mapping.as_mut_slice()[..8].copy_from_slice(&(exact_len as u64).to_le_bytes());
        let Ok(size_memory) = size_mapping.unmap() else {
            return false;
        };
        let object_id = observability::ARCHIVE_BASE_ID + self.chunk_index;
        let Ok(call) = self.connection.as_ref().call(objstore::OP_CREATE_AT, object_id) else {
            return false;
        };
        self.active_flush = Some(ActiveFlush {
            stage: FlushStage::Creating {
                call,
                data: memory,
                size: size_memory,
            },
            started_ticks: ticks,
            record_count,
            full: record_count >= archive_records_per_chunk(),
        });
        true
    }

    fn poll_flush(&mut self, ticks: u64, frequency_hz: u64) -> FlushProgress {
        const TIMEOUT_MS: u64 = 2_000;
        let Some(active) = self.active_flush.take() else {
            return FlushProgress::Idle;
        };
        let timeout_ticks = frequency_hz.saturating_mul(TIMEOUT_MS) / 1_000;
        if ticks.saturating_sub(active.started_ticks) >= timeout_ticks.max(1) {
            return FlushProgress::Failed;
        }
        let ActiveFlush {
            stage,
            started_ticks,
            record_count,
            full,
        } = active;
        let pending = |stage| ActiveFlush {
            stage,
            started_ticks,
            record_count,
            full,
        };
        let object_id = observability::ARCHIVE_BASE_ID + self.chunk_index;
        let next = match stage {
            FlushStage::Creating {
                mut call,
                data,
                size,
            } => match call.poll() {
                Ok(None) => pending(FlushStage::Creating {
                    call,
                    data,
                    size,
                }),
                Ok(Some(result))
                    if result.result == objstore::ERR_OK
                        || result.result == objstore::ERR_EXISTS =>
                {
                    let Ok(call) =
                        self.connection.as_ref().call_copy(objstore::OP_SET_SIZE, object_id, &size)
                    else {
                        return FlushProgress::Failed;
                    };
                    pending(FlushStage::Sizing {
                        call,
                        data,
                    })
                }
                _ => return FlushProgress::Failed,
            },
            FlushStage::Sizing {
                mut call,
                data,
            } => match call.poll() {
                Ok(None) => pending(FlushStage::Sizing {
                    call,
                    data,
                }),
                Ok(Some(result)) if result.result == 0 => {
                    let Ok(call) =
                        self.connection.as_ref().call_move(objstore::OP_WRITE, object_id, data)
                    else {
                        return FlushProgress::Failed;
                    };
                    pending(FlushStage::Writing {
                        call,
                    })
                }
                _ => return FlushProgress::Failed,
            },
            FlushStage::Writing {
                mut call,
            } => match call.poll() {
                Ok(None) => pending(FlushStage::Writing {
                    call,
                }),
                Ok(Some(result)) if result.result == 0 => {
                    let Ok(call) = self.connection.as_ref().call(objstore::OP_FLUSH, 0) else {
                        return FlushProgress::Failed;
                    };
                    pending(FlushStage::Flushing {
                        call,
                    })
                }
                _ => return FlushProgress::Failed,
            },
            FlushStage::Flushing {
                mut call,
            } => match call.poll() {
                Ok(None) => pending(FlushStage::Flushing {
                    call,
                }),
                Ok(Some(result)) if result.result == 0 => {
                    if full {
                        self.chunk.drain(..record_count.min(self.chunk.len()));
                        self.chunk_index = (self.chunk_index + 1) % observability::ARCHIVE_CHUNKS;
                    }
                    return FlushProgress::Complete {
                        record_count,
                        full,
                    };
                }
                _ => return FlushProgress::Failed,
            },
        };
        self.active_flush = Some(next);
        FlushProgress::Pending
    }
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
    let free_frames = read_word(bytes, statistics_header::FREE_FRAMES)?;
    let usable_frames = read_word(bytes, statistics_header::USABLE_FRAMES)?;
    let logical_processors = read_word(bytes, statistics_header::LOGICAL_PROCESSORS)?;
    let cpu_busy_ticks = read_word(bytes, statistics_header::CPU_BUSY_TICKS)?;
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
    let mut heap_allocated_bytes = 0u64;
    let mut heap_peak_bytes = 0u64;
    let mut threads_high_water = 0u64;
    for index in 0..domain_count {
        let base = domain_base + index * THREAD_STATISTICS_DOMAIN_RECORD_U64S;
        owned_frames =
            owned_frames.saturating_add(read_word(bytes, base + domain_record::OWNED_FRAMES)?);
        if read_word(bytes, base + domain_record::HEAP_STATUS_VALID)? != 0 {
            heap_allocated_bytes = heap_allocated_bytes
                .saturating_add(read_word(bytes, base + domain_record::HEAP_ALLOCATED_BYTES)?);
            heap_peak_bytes = heap_peak_bytes
                .saturating_add(read_word(bytes, base + domain_record::HEAP_PEAK_BYTES)?);
        }
        stack_used_high_water = stack_used_high_water
            .max(read_word(bytes, base + domain_record::STACK_PAGES_USED_HIGH_WATER)?);
        threads_high_water =
            threads_high_water.max(read_word(bytes, base + domain_record::THREADS_HIGH_WATER)?);
    }

    Some(HistorySample {
        ticks,
        free_frames,
        usable_frames,
        logical_processors,
        cpu_busy_ticks,
        threads: thread_count as u64,
        domains: domain_count as u64,
        owned_frames,
        heap_allocated_bytes,
        heap_peak_bytes,
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
            write_word(bytes, base + record::FREE_FRAMES, sample.free_frames);
            write_word(bytes, base + record::USABLE_FRAMES, sample.usable_frames);
            write_word(bytes, base + record::LOGICAL_PROCESSORS, sample.logical_processors);
            write_word(bytes, base + record::CPU_BUSY_TICKS, sample.cpu_busy_ticks);
            write_word(bytes, base + record::HEAP_ALLOCATED_BYTES, sample.heap_allocated_bytes);
            write_word(bytes, base + record::HEAP_PEAK_BYTES, sample.heap_peak_bytes);
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
    let flush_ticks = frequency_hz.saturating_mul(observability::ARCHIVE_FLUSH_INTERVAL_MS) / 1000;
    let mut next_sample_ticks = ticks.saturating_add(interval_ticks.max(1));
    let mut next_archive_retry_ticks = ticks;
    let mut archive: Option<Archive> = None;
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
                if let Some(archive) = archive.as_mut() {
                    archive.record(sample);
                }
            }
            next_sample_ticks = ticks.saturating_add(interval_ticks.max(1));
        }

        // Storage may not exist yet (the object store starts after observe) or
        // may be restarting. Look up infrequently and fail soft: a missing
        // archive delays durability, never sampling.
        if archive.is_none() && ticks >= next_archive_retry_ticks {
            if let Some((_generation, connection)) =
                try_registered_name_owned(ns_connection, objstore::NAME)
            {
                let mut connected = Archive::new(connection, ticks);
                connected.seed_recent(&history);
                archive = Some(connected);
            }
            next_archive_retry_ticks = ticks.saturating_add(interval_ticks.max(1));
        }

        let mut drop_archive = false;
        if let Some(current) = archive.as_mut() {
            if current.due(ticks, flush_ticks) && !current.start_flush(ticks, frequency_hz) {
                drop_archive = true;
            }
            match current.poll_flush(ticks, frequency_hz) {
                FlushProgress::Complete {
                    record_count,
                    full,
                } => {
                    current.last_flush_ticks = ticks;
                    catten_rt::logln!(
                        "[observe] archived {} resource sample(s){}",
                        record_count,
                        if full {
                            " and rotated"
                        } else {
                            ""
                        }
                    );
                }
                FlushProgress::Failed => drop_archive = true,
                FlushProgress::Idle | FlushProgress::Pending => {}
            }
        }
        if drop_archive {
            catten_rt::logln!("[observe] telemetry archive flush failed; retrying later");
            archive = None;
        }

        let remaining_ms = if ticks >= next_sample_ticks {
            1
        } else {
            ((next_sample_ticks - ticks) * 1000 / frequency_hz.max(1)).max(1)
        };
        let remaining_ms = if archive.as_ref().is_some_and(|archive| archive.active_flush.is_some())
        {
            remaining_ms.min(10)
        } else {
            remaining_ms
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
