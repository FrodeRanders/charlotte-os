//! NVMe block device client — self-test helper that validates the NVMe
//! driver initialization and block protocol connectivity. Phase 1: OP_INFO
//! only (I/O queues not yet created).
#![no_std]
#![no_main]
extern crate alloc;

catten_rt::entry!(main);

use catten_rt::Context;
use catten_rt::config;
use catten_services::block;
use catten_services::ns;
use catten_syscall::*;

const REPLY_SPINS: u64 = u64::MAX;

fn main(ctx: Context) -> ! {
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };

    config::write::<u32>(0, 1); // started

    // Retry lookup — the driver may still be initialising.
    let mut blk_conn: u64 = 0;
    for _ in 0..200 {
        let lookup = ipc_scalar_call_connection(ns_connection, ns::OP_LOOKUP, block::NAME, 0, IpcRights::SEND | IpcRights::CALL);
        if lookup != 0 {
            let mut s: u64 = 0;
            loop {
                let (status, result, cap) = ipc_reply_poll(lookup);
                if status == 0 {
                    ipc_close(lookup);
                    if (result as i64) >= 1 && cap != 0 {
                        blk_conn = cap;
                    }
                    break;
                }
                s += 1;
                if s > 5000 { ipc_close(lookup); break; }
                catten_services::sleep_ms(1);
            }
        }
        if blk_conn != 0 { break; }
        catten_services::sleep_ms(20);
    }
    if blk_conn == 0 { unsafe { thread_exit() }; }
    config::write::<u32>(0, 2); // lookup succeeded

    let info_call = ipc_scalar_call_connection(blk_conn, block::OP_INFO, 0, 0, IpcRights::SEND | IpcRights::CALL);
    if info_call == 0 { unsafe { thread_exit() }; }
    let (info_result, _) = unsafe { catten_services::wait_reply(info_call, REPLY_SPINS) };
    let (block_size, total_blocks) = charlotte_protocol_block::unpack_info(info_result);

    if block_size >= 512 && total_blocks > 0 {
        config::write::<u32>(0, 0x900d);
        config::write::<u32>(4, block_size);
        config::write::<u32>(8, total_blocks);
    }

    unsafe { thread_exit() };
}
