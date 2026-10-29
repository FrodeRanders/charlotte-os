//! CharlotteOS sitas smoke image — runs `basic_kv`, the mailbox-index demo,
//! a thread-join probe and a shared-CQ probe behind the crt0 contract.
//!
//! Results are written to the status page: `basic_kv` writes its verified key
//! count at offset 0, the mailbox-index demo writes its verified record count
//! at offset 1, and the join probe writes the sum of the joined shard values
//! at offset 2, and the CQ probe writes its verified count at offset 3 (a `u32`
//! each). The kernel self-test verifier checks all four.
#![no_std]
#![no_main]

extern crate alloc;
use catten_rt::{
    Context,
    config,
};
use catten_syscall::thread_exit;
use sitas_charlotte::CharlotteReactor;

fn main(ctx: Context) -> ! {
    let reactor = CharlotteReactor::new(0);
    let output = config::output_ptr::<u32>();

    unsafe {
        sitas_core::basic_kv::basic_kv_test(&reactor, output);
        sitas_core::mailbox_index::mailbox_index_test(&reactor, output.add(1));
        run_join_probe(&reactor, output.add(2));
        // SAFETY: output is the live writable status page supplied by crt0.
        output.add(3).write_volatile(
            if run_cq_probe(ctx, &reactor) {
                64
            } else {
                0xdead
            },
        );
    }

    unsafe {
        thread_exit();
    }
}

/// Spawn two shard threads whose closures return values, join both through
/// the kernel-backed `CharlotteJoinHandle`, and write the sum to `out`.
///
/// The closures return `40` and `41`, so a correct join probe writes `81`.
/// The main thread blocks in the kernel's completion wait while joining — the
/// same wait the shard executors use — and the exit observer fires only after
/// the kernel has reaped each thread.
fn run_join_probe(reactor: &CharlotteReactor, out: *mut u32) {
    use alloc::{
        boxed::Box,
        vec::Vec,
    };

    use sitas_core::{
        placement::ShardPlacement,
        shard::ShardId,
        shard_runtime::ShardRuntime,
    };

    let mut handles = Vec::new();
    for i in 0..2u64 {
        let handle = reactor.spawn_shard(
            ShardId(i as usize),
            ShardPlacement::Sequential,
            Box::new(move || 40u64 + i),
        );
        handles.push(handle);
    }

    let mut sum = 0u64;
    for handle in handles {
        match handle.join() {
            Ok(value) => sum += value,
            Err(_) => {
                unsafe { core::ptr::write_volatile(out, 0xdead) };
                return;
            }
        }
    }

    unsafe { core::ptr::write_volatile(out, sum as u32) };
}

/// Sequentially alternate the SDK and Sitas readers on the same kernel CQ.
/// All shard threads have joined; main is the only CQ0 consumer. A shared
/// monotonic deadline bounds readiness waits, including unrelated wakeups.
fn run_cq_probe(ctx: Context, reactor: &CharlotteReactor) -> bool {
    let layout = ctx.completion_queue_layout();
    let (started, frequency) = catten_syscall::monotonic_clock();
    if frequency == 0 {
        return false;
    }
    for index in 0..64 {
        let Ok(mut timer) = catten_rt::owned::Completion::timer(0) else {
            return false;
        };
        loop {
            let (now, current_frequency) = catten_syscall::monotonic_clock();
            if current_frequency != frequency || now < started {
                return false;
            }
            let elapsed = u128::from(now - started) * 1000;
            let budget = u128::from(frequency) * 2000;
            if elapsed >= budget {
                return false;
            }
            let entry = if index % 2 == 0 {
                // SAFETY: crt0 retains this mapped CQ page; all workers have
                // joined. SDK and Sitas readers alternate on this one thread,
                // never concurrently accessing tail or entries.
                unsafe { catten_syscall::cq_read(layout.base, layout.entries) }
            } else {
                reactor.cq().read_one().map(|entry| catten_syscall::CqEntry {
                    operation: entry.operation,
                    cookie: entry.cookie,
                    status: entry.status,
                    flags: entry.flags,
                    result: entry.result,
                })
            };
            if let Some(entry) = entry {
                // Old notifications are hints, never authority to adopt a cap.
                if timer.matches_cq(&entry) {
                    if entry.status != 0
                        || entry.flags != 0
                        || timer.poll() != Ok(Some(entry.result))
                    {
                        return false;
                    }
                    break;
                }
            } else {
                let remaining_ms = (budget - elapsed).div_ceil(u128::from(frequency)) as u64;
                if !matches!(
                    catten_rt::cq::wait(Some(remaining_ms), 0),
                    Ok(catten_rt::cq::WaitOutcome::Ready)
                ) {
                    return false;
                }
            }
        }
    }
    true
}

catten_rt::entry!(main);
