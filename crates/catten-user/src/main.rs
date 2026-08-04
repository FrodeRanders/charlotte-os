//! CharlotteOS sitas smoke image — runs `basic_kv`, the mailbox-index demo,
//! and a thread-join probe behind the crt0 contract.
//!
//! Results are written to the status page: `basic_kv` writes its verified key
//! count at offset 0, the mailbox-index demo writes its verified record count
//! at offset 1, and the join probe writes the sum of the joined shard values
//! at offset 2 (a `u32` each). The kernel self-test verifier checks all three.
#![no_std]
#![no_main]

extern crate alloc;
use catten_rt::{
    Context,
    config,
};
use catten_syscall::thread_exit;
use sitas_charlotte::CharlotteReactor;

fn main(_ctx: Context) -> ! {
    let reactor = CharlotteReactor::new(0);
    let output = config::output_ptr::<u32>();

    unsafe {
        sitas_core::basic_kv::basic_kv_test(&reactor, output);
        sitas_core::mailbox_index::mailbox_index_test(&reactor, output.add(1));
        run_join_probe(&reactor, output.add(2));
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

catten_rt::entry!(main);
