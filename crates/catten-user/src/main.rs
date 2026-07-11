//! CharlotteOS sitas shard test — exercises ShardedExecutor with
//! CharlotteReactor via SVC #7 (SPAWN_THREAD).
//!
//! Creates a `ShardedExecutor` with one shard using the `CharlotteReactor`
//! runtime. The shard runs a simple task that writes a sentinel to the result
//! page. The main thread polls for the sentinel.

#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

use core::panic::PanicInfo;
use sitas_charlotte::CharlotteReactor;
use sitas_core::sharded_executor::{ShardedExecutor, ShardedExecutorConfig};

const RESULT_PAGE: *mut u32 = 0x0000_0000_0001_2000usize as *mut u32;

/// The function each shard runs. On the spawned thread, it writes 0xCAFE.
fn shard_main() {
    unsafe { core::ptr::write_volatile(RESULT_PAGE, 0xCAFEu32) };
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let runtime = CharlotteReactor::new(1, 0);

    // Create a single-shard executor that spawns one thread via SVC #7.
    let config = ShardedExecutorConfig::new().with_shard_count(1);
    let _executor = ShardedExecutor::start_with_runtime(config, &runtime)
        .expect("failed to start shard executor");

    // The spawned thread should have called shard_main() and written 0xCAFE.
    for _ in 0..10_000_000 {
        let sentinel = unsafe { core::ptr::read_volatile(RESULT_PAGE) };
        if sentinel == 0xCAFE {
            // Overwrite with the shard count as confirmation.
            unsafe { core::ptr::write_volatile(RESULT_PAGE, 1u32) };
            break;
        }
        core::hint::spin_loop();
    }

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}
