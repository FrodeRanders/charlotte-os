//! NVMe block device client — deferred-lookup validation.
//!
//! Makes a single OP_LOOKUP call. The name service defers the reply until
//! "blk0" registers. No retry loop.
#![no_std]
#![no_main]
extern crate alloc;

catten_rt::entry!(main);

use catten_rt::Context;
use catten_rt::config;
use catten_services::block;
use catten_services::ns;
use catten_syscall::*;

fn main(ctx: Context) -> ! {
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 1);

    let lookup = ipc_scalar_call_connection(ns_connection, ns::OP_LOOKUP, block::NAME, 0, IpcRights::SEND | IpcRights::CALL);
    if lookup == 0 { unsafe { thread_exit() }; }
    let (generation, blk_conn) = catten_services::spin_reply(lookup);
    if generation < 1 || blk_conn == 0 { unsafe { thread_exit() }; }
    config::write::<u32>(0, 2);

    let info_call = ipc_scalar_call_connection(blk_conn, block::OP_INFO, 0, 0, IpcRights::SEND | IpcRights::CALL);
    if info_call == 0 { unsafe { thread_exit() }; }
    let (info_result, _) = catten_services::spin_reply(info_call);
    let (block_size, total_blocks) = charlotte_protocol_block::unpack_info(info_result);

    if block_size >= 512 && total_blocks > 0 {
        config::write::<u32>(0, 0x900d);
        config::write::<u32>(4, block_size);
        config::write::<u32>(8, total_blocks);
    }

    unsafe { thread_exit() };
}
