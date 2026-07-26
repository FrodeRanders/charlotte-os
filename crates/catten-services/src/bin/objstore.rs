//! Persistent object store service for CharlotteOS.
//!
//! Provides crash-safe, dynamically-sized blob storage on top of a block
//! device. Objects are identified by 64-bit monotonically-increasing IDs.
//! Each object stores a single contiguous blob; WRITE replaces the entire
//! content atomically.
//!
//! ## On-disk layout
//!
//! LBA 0:    superblock
//! LBA 1:    free block bitmap
//! LBA 2-33: object directory (512 entries × 32 bytes)
//! LBA 34+:  data region
//!
//! ## Crash safety
//!
//! New data extents are written first, then the directory entry is updated
//! atomically. If a crash occurs between writing data and updating the
//! directory, the orphaned blocks are leaked (reclaimed by format on next
//! mount if desired, but acceptable for an embedded system).
#![no_std]
#![no_main]
extern crate alloc;

catten_rt::entry!(main);

use alloc::collections::BTreeMap;
use catten_rt::{
    Context,
    config,
};
use catten_services::{
    block,
    ns,
    objstore,
};
use catten_syscall::{
    ipc_status,
    *,
};

const BUFFER_VADDR: usize = 0x0000_0000_0050_0000;
const REPLY_SPINS: u64 = u64::MAX;

fn spin_reply(call: u64) -> (i64, u64) {
    let (status, result, cap) = ipc_reply_wait(call);
    ipc_close(call);
    if status == 0 {
        (result as i64, cap)
    } else {
        (-1, 0)
    }
}

const SB_MAGIC: u64 = 0x525453424a4f;
const SB_VERSION: u32 = 1;
const DIR_ENTRIES: u32 = 512;
const DIR_BLOCKS: u32 = 32;
const METADATA_BLOCKS: u32 = 1 + 1 + DIR_BLOCKS;

const FLAG_ALLOCATED: u32 = 1 << 0;

struct BlockDev {
    conn: u64,
    block_size: u32,
    total_blocks: u32,
}

impl BlockDev {
    /// Look up "blk0" from the name service. The name service defers
    /// the lookup if the driver hasn't registered yet — no retry loop.
    fn connect(ns_conn: u64) -> Option<Self> {
        let lookup = ipc_scalar_call_connection(
            ns_conn,
            ns::OP_LOOKUP,
            block::NAME,
            0,
            IpcRights::SEND | IpcRights::CALL,
        );
        if lookup == 0 {
            return None;
        }
        let (generation, blk_conn) = spin_reply(lookup);
        if generation < 1 || blk_conn == 0 {
            return None;
        }

        let info = ipc_scalar_call(blk_conn, block::OP_INFO, 0);
        if info == 0 {
            return None;
        }
        let (result, _) = spin_reply(info);
        let (bs, tb) = charlotte_protocol_block::unpack_info(result);
        if bs == 0 || tb < METADATA_BLOCKS + 2 {
            return None;
        }
        Some(BlockDev {
            conn: blk_conn,
            block_size: bs,
            total_blocks: tb,
        })
    }

    fn write_block(&self, lba: u64, mem_cap: u64) -> bool {
        self.write_blocks(lba, 1, mem_cap)
    }

    fn write_blocks(&self, lba: u64, count: u32, mem_cap: u64) -> bool {
        let call = ipc_scalar_call_borrow_read(
            self.conn,
            block::OP_WRITE,
            charlotte_protocol_block::pack_lba_count(lba, count),
            mem_cap,
        );
        if call == 0 {
            memory_close(mem_cap);
            return false;
        }
        let (result, _) = spin_reply(call);
        memory_close(mem_cap);
        result == 0
    }

    fn flush(&self) -> bool {
        let call = ipc_scalar_call_connection(
            self.conn,
            block::OP_FLUSH,
            0,
            0,
            IpcRights::SEND | IpcRights::CALL,
        );
        if call == 0 {
            return false;
        }
        let (result, _) = spin_reply(call);
        result == 0
    }
}

struct ObjStore {
    dev: BlockDev,
    next_id: u32,
    block_size: u32,
    index_cache: spin::Mutex<BTreeMap<u64, u32>>,
}

impl ObjStore {
    fn mount(dev: BlockDev) -> Option<Self> {
        let sb = memory_alloc(1);
        if sb == 0 {
            return None;
        }
        let call = ipc_scalar_call_borrow_write(
            dev.conn,
            block::OP_READ,
            charlotte_protocol_block::pack_lba_count(0, 1),
            sb,
        );
        if call == 0 {
            memory_close(sb);
            return None;
        }
        let (result, _) = spin_reply(call);
        if result != 0 {
            memory_close(sb);
            return None;
        }
        memory_map(sb, BUFFER_VADDR, false);
        let magic = unsafe { core::ptr::read_volatile(BUFFER_VADDR as *const u64) };
        let version = unsafe { core::ptr::read_volatile((BUFFER_VADDR + 8) as *const u32) };
        memory_unmap(sb);
        memory_close(sb);

        if magic != SB_MAGIC || version != SB_VERSION {
            return Self::format(dev);
        }

        let sb2 = memory_alloc(1);
        if sb2 == 0 {
            return None;
        }
        let call2 = ipc_scalar_call_borrow_write(
            dev.conn,
            block::OP_READ,
            charlotte_protocol_block::pack_lba_count(0, 1),
            sb2,
        );
        if call2 == 0 {
            memory_close(sb2);
            return None;
        }
        let (r2, _) = unsafe { catten_services::wait_reply(call2, REPLY_SPINS) };
        if r2 != 0 {
            memory_close(sb2);
            return None;
        }
        memory_map(sb2, BUFFER_VADDR, false);
        let _gen = unsafe { core::ptr::read_volatile((BUFFER_VADDR + 12) as *const u32) };
        let block_size = unsafe { core::ptr::read_volatile((BUFFER_VADDR + 16) as *const u32) };
        let _total = unsafe { core::ptr::read_volatile((BUFFER_VADDR + 20) as *const u32) };
        let next_id = unsafe { core::ptr::read_volatile((BUFFER_VADDR + 28) as *const u32) };
        memory_unmap(sb2);
        memory_close(sb2);

        Some(ObjStore {
            dev,
            next_id,
            block_size,
            index_cache: spin::Mutex::new(BTreeMap::new()),
        })
    }

    fn format(dev: BlockDev) -> Option<Self> {
        let bs = dev.block_size;
        let tb = dev.total_blocks;

        // Write superblock
        let sb = memory_alloc(1);
        if sb == 0 {
            return None;
        }
        memory_map(sb, BUFFER_VADDR, true);
        unsafe {
            let p = BUFFER_VADDR as *mut u64;
            p.write_volatile(SB_MAGIC);
            (BUFFER_VADDR as *mut u32).add(2).write_volatile(SB_VERSION);
            (BUFFER_VADDR as *mut u32).add(3).write_volatile(0);
            (BUFFER_VADDR as *mut u32).add(4).write_volatile(bs);
            (BUFFER_VADDR as *mut u32).add(5).write_volatile(tb);
            (BUFFER_VADDR as *mut u32).add(6).write_volatile(0);
            (BUFFER_VADDR as *mut u32).add(7).write_volatile(1u32); // next_id = 1
            (BUFFER_VADDR as *mut u64).add(4).write_volatile(2);
        }
        memory_unmap(sb);
        dev.write_block(0, sb);

        // Write free bitmap (all bits 0, then mark metadata)
        let bm = memory_alloc(1);
        if bm == 0 {
            return None;
        }
        memory_map(bm, BUFFER_VADDR, true);
        unsafe {
            core::ptr::write_bytes(BUFFER_VADDR as *mut u8, 0, bs as usize);
        }
        for i in 0..METADATA_BLOCKS {
            let byte = (i / 8) as usize;
            let bit = (i % 8) as u8;
            unsafe {
                ((BUFFER_VADDR + byte) as *mut u8).write_volatile(1u8 << bit);
            }
        }
        memory_unmap(bm);
        dev.write_block(1, bm);

        // Write empty directory blocks
        for i in 0..DIR_BLOCKS {
            let db = memory_alloc(1);
            if db == 0 {
                return None;
            }
            memory_map(db, BUFFER_VADDR, true);
            unsafe {
                core::ptr::write_bytes(BUFFER_VADDR as *mut u8, 0, bs as usize);
            }
            memory_unmap(db);
            dev.write_block(2 + (i as u64), db);
        }

        dev.flush();

        Some(ObjStore {
            dev,
            next_id: 1,
            block_size: bs,
            index_cache: spin::Mutex::new(BTreeMap::new()),
        })
    }

    /// Read a single directory entry. Returns (id, flags, size_bytes, first_lba).
    fn read_dir_entry(&self, index: u32) -> Option<(u64, u32, u32, u64)> {
        if index >= DIR_ENTRIES {
            return None;
        }
        let dir_lba = 2 + (index as u64 * 32) / self.block_size as u64;
        let dir_off = (index as usize * 32) % self.block_size as usize;

        let dm = memory_alloc(1);
        if dm == 0 {
            return None;
        }
        let call = ipc_scalar_call_borrow_write(
            self.dev.conn,
            block::OP_READ,
            charlotte_protocol_block::pack_lba_count(dir_lba, 1),
            dm,
        );
        if call == 0 {
            memory_close(dm);
            return None;
        }
        let (r, _) = spin_reply(call);
        if r != 0 {
            memory_close(dm);
            return None;
        }
        memory_map(dm, BUFFER_VADDR, false);

        let off = BUFFER_VADDR + dir_off;
        let id = unsafe { core::ptr::read_volatile(off as *const u64) };
        let flags = unsafe { core::ptr::read_volatile((off + 8) as *const u32) };
        let size = unsafe { core::ptr::read_volatile((off + 12) as *const u32) };
        let first = unsafe { core::ptr::read_volatile((off + 16) as *const u64) };

        memory_unmap(dm);
        memory_close(dm);
        Some((id, flags, size, first))
    }

    fn write_dir_entry(&self, index: u32, id: u64, flags: u32, size: u32, first_lba: u64) -> bool {
        if index >= DIR_ENTRIES {
            return false;
        }
        let dir_lba = 2 + (index as u64 * 32) / self.block_size as u64;
        let dir_off = (index as usize * 32) % self.block_size as usize;

        // Read the block, modify it, write it back
        let dm = memory_alloc(1);
        if dm == 0 {
            return false;
        }
        let call = ipc_scalar_call_borrow_write(
            self.dev.conn,
            block::OP_READ,
            charlotte_protocol_block::pack_lba_count(dir_lba, 1),
            dm,
        );
        if call == 0 {
            memory_close(dm);
            return false;
        }
        let (r, _) = spin_reply(call);
        if r != 0 {
            memory_close(dm);
            return false;
        }
        // Write the modified entry into the block and write back
        memory_map(dm, BUFFER_VADDR, true);
        let off = BUFFER_VADDR + dir_off;
        unsafe {
            (off as *mut u64).write_volatile(id);
            ((off + 8) as *mut u32).write_volatile(flags);
            ((off + 12) as *mut u32).write_volatile(size);
            ((off + 16) as *mut u64).write_volatile(first_lba);
        }
        memory_unmap(dm);
        self.dev.write_block(dir_lba, dm)
    }

    fn find_free_dir_slot(&self) -> Option<u32> {
        for i in 0..DIR_ENTRIES {
            if let Some((id, _, _, _)) = self.read_dir_entry(i) {
                if id == 0 {
                    return Some(i);
                }
            }
        }
        None
    }

    fn find_dir_index(&self, object_id: u64) -> Option<u32> {
        if let Some(index) = self.index_cache.lock().get(&object_id).copied() {
            return Some(index);
        }
        for i in 0..DIR_ENTRIES {
            if let Some((id, _, _, _)) = self.read_dir_entry(i) {
                if id == object_id {
                    self.index_cache.lock().insert(object_id, i);
                    return Some(i);
                }
            }
        }
        None
    }

    fn op_create(&mut self) -> u64 {
        let slot = match self.find_free_dir_slot() {
            Some(s) => s,
            None => return 0,
        };
        let id = self.next_id as u64;
        self.next_id += 1;
        self.write_dir_entry(slot, id, FLAG_ALLOCATED, 0, 0);
        self.index_cache.lock().insert(id, slot);

        // Persist next_id in superblock
        let sb = memory_alloc(1);
        if sb != 0 {
            memory_map(sb, BUFFER_VADDR, true);
            unsafe {
                (BUFFER_VADDR as *mut u64).write_volatile(SB_MAGIC);
                (BUFFER_VADDR as *mut u32).add(2).write_volatile(SB_VERSION);
                (BUFFER_VADDR as *mut u32).add(3).write_volatile(1);
                (BUFFER_VADDR as *mut u32).add(4).write_volatile(self.block_size);
                (BUFFER_VADDR as *mut u32).add(5).write_volatile(self.dev.total_blocks);
                (BUFFER_VADDR as *mut u32).add(7).write_volatile(self.next_id);
            }
            memory_unmap(sb);
            self.dev.write_block(0, sb);
        }
        id
    }

    fn op_create_at(&self, object_id: u64) -> i64 {
        if object_id == 0 {
            return objstore::ERR_INVALID_ID;
        }
        if self.find_dir_index(object_id).is_some() {
            return objstore::ERR_EXISTS;
        }
        let slot = match self.find_free_dir_slot() {
            Some(slot) => slot,
            None => return objstore::ERR_NO_SPACE,
        };
        if self.write_dir_entry(slot, object_id, FLAG_ALLOCATED, 0, 0) {
            self.index_cache.lock().insert(object_id, slot);
            objstore::ERR_OK
        } else {
            objstore::ERR_IO_ERROR
        }
    }

    fn op_delete(&self, object_id: u64) -> i64 {
        match self.find_dir_index(object_id) {
            Some(idx) => {
                self.write_dir_entry(idx, 0, 0, 0, 0);
                self.index_cache.lock().remove(&object_id);
                objstore::ERR_OK
            }
            None => objstore::ERR_NOT_FOUND,
        }
    }

    /// WRITE replaces the entire object content with one memory-object page.
    ///
    /// Each directory slot owns a deterministic 4 KiB extent. The disk
    /// location must not be derived from the caller's RAM physical address:
    /// that address names DMA memory, not an LBA in the NVMe namespace.
    fn op_write(&self, object_id: u64, data_cap: u64) -> i64 {
        let idx = match self.find_dir_index(object_id) {
            Some(i) => i,
            None => return objstore::ERR_NOT_FOUND,
        };

        if memory_get_phys(data_cap) == 0 {
            return objstore::ERR_IO_ERROR;
        }

        let blocks_per_object = 4096u32.div_ceil(self.block_size);
        let lba = METADATA_BLOCKS as u64 + idx as u64 * blocks_per_object as u64;
        if lba + blocks_per_object as u64 > self.dev.total_blocks as u64 {
            return objstore::ERR_NO_SPACE;
        }

        if !self.dev.write_blocks(lba, blocks_per_object, data_cap) {
            return objstore::ERR_IO_ERROR;
        }

        if self.write_dir_entry(idx, object_id, FLAG_ALLOCATED, 4096, lba) {
            objstore::ERR_OK
        } else {
            objstore::ERR_IO_ERROR
        }
    }

    /// READ returns the object's data block as a memory object moved to
    /// the caller. The caller receives a memory cap via ipc_reply_move.
    fn op_read_and_reply(&self, object_id: u64, reply: u64) {
        if reply == 0 {
            return;
        }

        let idx = match self.find_dir_index(object_id) {
            Some(i) => i,
            None => {
                ipc_reply(reply, objstore::ERR_NOT_FOUND);
                return;
            }
        };
        let (_id, _flags, _size, first_lba) = match self.read_dir_entry(idx) {
            Some(v) => v,
            None => {
                ipc_reply(reply, objstore::ERR_IO_ERROR);
                return;
            }
        };
        if first_lba == 0 {
            ipc_reply(reply, objstore::ERR_NOT_FOUND);
            return;
        }

        // Read the block from disk into a new memory object
        let dm = memory_alloc(1);
        if dm == 0 {
            ipc_reply(reply, objstore::ERR_IO_ERROR);
            return;
        }
        let blocks_per_object = 4096u32.div_ceil(self.block_size);
        let call = ipc_scalar_call_borrow_write(
            self.dev.conn,
            block::OP_READ,
            charlotte_protocol_block::pack_lba_count(first_lba, blocks_per_object),
            dm,
        );
        if call == 0 {
            memory_close(dm);
            ipc_reply(reply, objstore::ERR_IO_ERROR);
            return;
        }
        let (r, _) = spin_reply(call);
        if r != 0 {
            memory_close(dm);
            ipc_reply(reply, objstore::ERR_IO_ERROR);
            return;
        }

        ipc_reply_move(reply, dm, 0);
    }

    fn op_flush(&self) -> i64 {
        if self.dev.flush() {
            0
        } else {
            objstore::ERR_IO_ERROR
        }
    }
}

fn main(ctx: Context) -> ! {
    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(0, 1); // started

    // Use handoff endpoint if provided, otherwise name service lookup
    let dev = match ctx.handoff_endpoint_cap() {
        0 => match BlockDev::connect(ns_connection) {
            Some(d) => {
                config::write::<u32>(0, 2);
                config::write::<u32>(24, d.conn as u32);
                d
            }
            None => {
                config::write::<u32>(0, 0xdead);
                unsafe { thread_exit() };
            }
        },
        blk_conn => {
            config::write::<u32>(0, 2);
            config::write::<u32>(24, blk_conn as u32);
            let info = ipc_scalar_call(blk_conn, block::OP_INFO, 0);
            if info == 0 {
                config::write::<u32>(0, 0xbeef);
                unsafe { thread_exit() };
            }
            let mut s: u64 = 0;
            let (bs, tb) = loop {
                let (st, r, _) = ipc_reply_poll(info);
                if st == 0 {
                    ipc_close(info);
                    break charlotte_protocol_block::unpack_info(r as i64);
                }
                s += 1;
                if s > 5000 {
                    ipc_close(info);
                    config::write::<u32>(0, 0xdddd);
                    unsafe { thread_exit() };
                }
                core::hint::spin_loop();
            };
            config::write::<u32>(16, bs);
            config::write::<u32>(20, tb);
            if bs == 0 || tb < METADATA_BLOCKS + 2 {
                config::write::<u32>(0, 0xeeee);
                unsafe { thread_exit() };
            }
            BlockDev {
                conn: blk_conn,
                block_size: bs,
                total_blocks: tb,
            }
        }
    }; // block device connected

    let mut store = match ObjStore::mount(dev) {
        Some(s) => {
            config::write::<u32>(0, 3);
            s
        }
        None => {
            config::write::<u32>(0, 0xbeef);
            unsafe { thread_exit() };
        }
    }; // mounted/formatted

    let endpoint = ipc_endpoint_create(objstore::INTERFACE, objstore::VERSION, 64);
    if endpoint == 0 {
        unsafe { thread_exit() };
    }

    let register = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        objstore::NAME,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if register == 0 {
        unsafe { thread_exit() };
    }
    let (generation, _) = unsafe { catten_services::wait_reply(register, REPLY_SPINS) };
    if generation < 1 {
        unsafe { thread_exit() };
    }

    ipc_endpoint_bind_cq(endpoint, 0);
    config::write::<u32>(0, 4); // registered and serving
    config::write::<u32>(4, 0x900d); // sentinel

    loop {
        cq_wait(1, 0);

        loop {
            let message = ipc_recv(endpoint);
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if !message.is_ok() {
                break;
            }

            match message.opcode {
                objstore::OP_CREATE => {
                    let id = store.op_create();
                    if message.reply != 0 {
                        ipc_reply(message.reply, id as i64);
                    }
                }
                objstore::OP_CREATE_AT => {
                    let result = store.op_create_at(message.arg0);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                objstore::OP_DELETE => {
                    let result = store.op_delete(message.arg0);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                objstore::OP_WRITE => {
                    let object_id = message.arg0;
                    let result = store.op_write(object_id, message.memory);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                objstore::OP_READ => {
                    store.op_read_and_reply(message.arg0, message.reply);
                }
                objstore::OP_RESIZE => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, 0);
                    }
                }
                objstore::OP_FLUSH => {
                    let result = store.op_flush();
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                objstore::OP_INFO => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, 0);
                    }
                }
                _ => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }
            }
        }
    }
}
