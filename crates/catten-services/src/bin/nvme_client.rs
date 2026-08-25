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
    owned::OwnedMemory,
};
use catten_services::{
    block,
    wait_for_registered_name_owned,
};
use catten_syscall::thread_exit;
use charlotte_launch::nvme_client_status as status;

fn main(ctx: Context) -> ! {
    let ns_connection = match ctx.bootstrap_connection() {
        Some(connection) => connection,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 1);

    let (_, blk_conn) = wait_for_registered_name_owned(ns_connection, block::NAME)
        .unwrap_or_else(|| unsafe { thread_exit() });
    config::write::<u32>(status::STAGE, 2);

    let info_result = blk_conn
        .call(block::OP_INFO, 0)
        .and_then(|call| call.wait())
        .unwrap_or_else(|_| unsafe { thread_exit() })
        .result;
    let (block_size, total_blocks) = charlotte_protocol_block::unpack_info(info_result);

    if !(512..=4096).contains(&block_size) || total_blocks == 0 {
        config::write::<u32>(status::STAGE, 0xdead);
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::BLOCK_SIZE, block_size);
    config::write::<u32>(status::TOTAL_BLOCKS, total_blocks);

    // Exercise PRP-list DMA, not merely PRP1: three pages require PRP1 plus a
    // PRP2 list containing the second and third physical frames. Use the final
    // namespace blocks so object-store formatting cannot overwrite the pattern.
    const TEST_PAGES: usize = 3;
    let transfer_bytes = TEST_PAGES * 4096;
    let block_count = (transfer_bytes / block_size as usize) as u32;
    if block_count == 0 || block_count > total_blocks {
        config::write::<u32>(status::STAGE, 0xdea0);
        unsafe { thread_exit() };
    }
    let test_lba = (total_blocks - block_count) as u64;
    let write_mem = OwnedMemory::allocate(TEST_PAGES).unwrap_or_else(|_| {
        config::write::<u32>(status::STAGE, 0xdea1);
        unsafe { thread_exit() }
    });
    let mut read_mem = OwnedMemory::allocate(TEST_PAGES).unwrap_or_else(|_| {
        config::write::<u32>(status::STAGE, 0xdea1);
        unsafe { thread_exit() }
    });

    let mut write_mapping = write_mem.map_writable().unwrap_or_else(|_| {
        config::write::<u32>(status::STAGE, 0xdea2);
        unsafe { thread_exit() }
    });
    for i in 0..transfer_bytes {
        write_mapping.as_mut_slice()[i] = (i as u8).wrapping_mul(37) ^ 0xa5;
    }
    let write_mem = write_mapping.unmap().unwrap_or_else(|_| unsafe { thread_exit() });

    let write_result = blk_conn
        .call_borrow_read(
            block::OP_WRITE,
            charlotte_protocol_block::pack_lba_count(test_lba, block_count),
            &write_mem,
        )
        .and_then(|call| call.wait())
        .map(|result| result.result);
    if write_result != Ok(block::ERR_OK) {
        config::write::<u32>(status::STAGE, 0xdea3);
        unsafe { thread_exit() };
    }

    let flush_result =
        blk_conn.call(block::OP_FLUSH, 0).and_then(|call| call.wait()).map(|result| result.result);
    if flush_result != Ok(block::ERR_OK) {
        config::write::<u32>(status::STAGE, 0xdea4);
        unsafe { thread_exit() };
    }

    let read_result = blk_conn
        .call_borrow_write(
            block::OP_READ,
            charlotte_protocol_block::pack_lba_count(test_lba, block_count),
            &mut read_mem,
        )
        .and_then(|call| call.wait())
        .map(|result| result.result);
    if read_result != Ok(block::ERR_OK) {
        config::write::<u32>(status::STAGE, 0xdea5);
        unsafe { thread_exit() };
    }

    let read_mapping = read_mem.map_read_only().unwrap_or_else(|_| {
        config::write::<u32>(status::STAGE, 0xdea6);
        unsafe { thread_exit() }
    });
    for i in 0..transfer_bytes {
        let actual = read_mapping.as_slice()[i];
        let expected = (i as u8).wrapping_mul(37) ^ 0xa5;
        if actual != expected {
            config::write::<u32>(status::STAGE, 0xdea7);
            config::write::<u32>(status::MISMATCH_INDEX, i as u32);
            unsafe { thread_exit() };
        }
    }

    config::write::<u32>(status::STAGE, 0x900d);

    unsafe { thread_exit() };
}
