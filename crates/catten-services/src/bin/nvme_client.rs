//! NVMe block device client — deferred-lookup validation.
//!
//! Makes a single OP_LOOKUP call. The name service defers the reply until
//! "blk0" registers. No retry loop.
#![no_std]
#![no_main]
extern crate alloc;

catten_rt::entry!(main);

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    block,
    ns,
};
use catten_syscall::*;


fn main(ctx: Context) -> ! {
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 1);

    let lookup = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_LOOKUP,
        block::NAME,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if lookup == 0 {
        unsafe { thread_exit() };
    }
    let (generation, blk_conn) = catten_services::spin_reply(lookup);
    if generation < 1 || blk_conn == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 2);

    let info_call = ipc_scalar_call_connection(
        blk_conn,
        block::OP_INFO,
        0,
        0,
        IpcRights::SEND | IpcRights::CALL,
    );
    if info_call == 0 {
        unsafe { thread_exit() };
    }
    let (info_result, _) = catten_services::spin_reply(info_call);
    let (block_size, total_blocks) = charlotte_protocol_block::unpack_info(info_result);

    if !(512..=4096).contains(&block_size) || total_blocks == 0 {
        config::write::<u32>(0, 0xdead);
        unsafe { thread_exit() };
    }
    config::write::<u32>(4, block_size);
    config::write::<u32>(8, total_blocks);

    // Exercise PRP-list DMA, not merely PRP1: three pages require PRP1 plus a
    // PRP2 list containing the second and third physical frames. Use the final
    // namespace blocks so object-store formatting cannot overwrite the pattern.
    const TEST_PAGES: usize = 3;
    let transfer_bytes = TEST_PAGES * 4096;
    let block_count = (transfer_bytes / block_size as usize) as u32;
    if block_count == 0 || block_count > total_blocks {
        config::write::<u32>(0, 0xdea0);
        unsafe { thread_exit() };
    }
    let test_lba = (total_blocks - block_count) as u64;
    let write_mem = memory_alloc(TEST_PAGES);
    let read_mem = memory_alloc(TEST_PAGES);
    if write_mem == 0 || read_mem == 0 {
        config::write::<u32>(0, 0xdea1);
        unsafe { thread_exit() };
    }

        let (write_vaddr_map_status, write_vaddr_vaddr) = memory_map_any(write_mem, true);
    if write_vaddr_map_status != 0 {
        config::write::<u32>(0, 0xdea2);
        unsafe { thread_exit() };
    }
    for i in 0..transfer_bytes {
        unsafe {
            ((write_vaddr_vaddr + i) as *mut u8).write_volatile((i as u8).wrapping_mul(37) ^ 0xa5);
        }
    }
    memory_unmap(write_mem);

    let write_call = ipc_scalar_call_borrow_read(
        blk_conn,
        block::OP_WRITE,
        charlotte_protocol_block::pack_lba_count(test_lba, block_count),
        write_mem,
    );
    if write_call == 0 || catten_services::spin_reply(write_call).0 != block::ERR_OK {
        config::write::<u32>(0, 0xdea3);
        unsafe { thread_exit() };
    }

    let flush_call = ipc_scalar_call(blk_conn, block::OP_FLUSH, 0);
    if flush_call == 0 || catten_services::spin_reply(flush_call).0 != block::ERR_OK {
        config::write::<u32>(0, 0xdea4);
        unsafe { thread_exit() };
    }

    let read_call = ipc_scalar_call_borrow_write(
        blk_conn,
        block::OP_READ,
        charlotte_protocol_block::pack_lba_count(test_lba, block_count),
        read_mem,
    );
    if read_call == 0 || catten_services::spin_reply(read_call).0 != block::ERR_OK {
        config::write::<u32>(0, 0xdea5);
        unsafe { thread_exit() };
    }

        let (read_vaddr_map_status, read_vaddr_vaddr) = memory_map_any(read_mem, false);
    if read_vaddr_map_status != 0 {
        config::write::<u32>(0, 0xdea6);
        unsafe { thread_exit() };
    }
    for i in 0..transfer_bytes {
        let actual = unsafe { ((read_vaddr_vaddr + i) as *const u8).read_volatile() };
        let expected = (i as u8).wrapping_mul(37) ^ 0xa5;
        if actual != expected {
            config::write::<u32>(0, 0xdea7);
            config::write::<u32>(12, i as u32);
            unsafe { thread_exit() };
        }
    }

    config::write::<u32>(0, 0x900d);

    unsafe { thread_exit() };
}
