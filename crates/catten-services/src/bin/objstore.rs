//! Persistent object store service — on-disk format v2.
//!
//! The directory is only an atomic locator. Every allocated object has a
//! versioned, extensible header containing its generation, exact data length,
//! extent descriptor, and FNV-1a content hash. Replacement is copy-on-write:
//! data, header, and allocation state are durable before the directory pointer
//! changes. Mount reconstructs the bitmap from reachable headers, reclaiming
//! abandoned pre-commit allocations.
#![no_std]
#![no_main]
extern crate alloc;

catten_rt::entry!(main);

use alloc::{
    collections::BTreeMap,
    vec,
    vec::Vec,
};

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

const BUFFER_VADDR: usize = 0x0000_0000_2000_0000;
const CHUNK_VADDR: usize = 0x0000_0000_3000_0000;
const PAGE_SIZE: usize = 4096;
const MAX_IO_BYTES: usize = 512 * 1024;

const SB_MAGIC: u64 = 0x3252_5453_424a_4f43; // "COBJSTR2"
const SB_VERSION: u32 = 2;
const SB_SLOTS: u32 = 2;
const SB_LEN: usize = 80;
const SB_CHECKSUM_OFFSET: usize = 76;

const DIR_ENTRIES: u32 = 512;
const DIR_ENTRY_SIZE: usize = 32;
const FLAG_ALLOCATED: u32 = 1;

const HEADER_MAGIC: u64 = 0x3244_484a_424f_4343; // "CCOBJHD2"
const HEADER_VERSION: u16 = 2;
const HEADER_LEN: usize = 128;
const HEADER_CHECKSUM_OFFSET: usize = 112;
const HASH_FNV1A64: u16 = 2;

#[derive(Clone, Copy)]
struct Layout {
    bitmap_lba: u64,
    bitmap_blocks: u32,
    directory_lba: u64,
    directory_blocks: u32,
    data_lba: u64,
}

impl Layout {
    fn for_device(block_size: u32, total_blocks: u32) -> Option<Self> {
        let bitmap_bytes = (total_blocks as usize).div_ceil(8);
        let bitmap_blocks = u32::try_from(bitmap_bytes.div_ceil(block_size as usize)).ok()?;
        let directory_bytes = DIR_ENTRIES as usize * DIR_ENTRY_SIZE;
        let directory_blocks = u32::try_from(directory_bytes.div_ceil(block_size as usize)).ok()?;
        let bitmap_lba = SB_SLOTS as u64;
        let directory_lba = bitmap_lba + bitmap_blocks as u64;
        let data_lba = directory_lba + directory_blocks as u64;
        (data_lba + 2 <= total_blocks as u64).then_some(Self {
            bitmap_lba,
            bitmap_blocks,
            directory_lba,
            directory_blocks,
            data_lba,
        })
    }
}

#[derive(Clone, Copy)]
struct Superblock {
    generation: u64,
    block_size: u32,
    total_blocks: u32,
    next_id: u64,
    layout: Layout,
}

#[derive(Clone, Copy, Default)]
struct DirectoryEntry {
    id: u64,
    flags: u32,
    generation: u32,
    header_lba: u64,
    header_blocks: u32,
}

#[derive(Clone, Copy)]
struct ObjectHeader {
    id: u64,
    generation: u64,
    data_len: u64,
    allocated_len: u64,
    data_offset: u64,
    header_blocks: u32,
    data_lba: u64,
    data_blocks: u32,
    data_hash: u64,
}

struct BlockDev {
    conn: u64,
    block_size: u32,
    total_blocks: u32,
}

impl BlockDev {
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
        let (generation, conn) = wait_scalar(lookup)?;
        if generation < 1 || conn == 0 {
            return None;
        }
        let info = ipc_scalar_call(conn, block::OP_INFO, 0);
        let (result, _) = wait_scalar(info)?;
        let (block_size, total_blocks) = charlotte_protocol_block::unpack_info(result);
        if !(512..=PAGE_SIZE as u32).contains(&block_size)
            || Layout::for_device(block_size, total_blocks).is_none()
        {
            return None;
        }
        Some(Self {
            conn,
            block_size,
            total_blocks,
        })
    }

    fn read_blocks_keep(&self, lba: u64, count: u32, memory: u64) -> bool {
        let call = ipc_scalar_call_borrow_write(
            self.conn,
            block::OP_READ,
            charlotte_protocol_block::pack_lba_count(lba, count),
            memory,
        );
        wait_scalar(call).is_some_and(|(result, _)| result == 0)
    }

    fn write_blocks(&self, lba: u64, count: u32, memory: u64) -> bool {
        let call = ipc_scalar_call_borrow_read(
            self.conn,
            block::OP_WRITE,
            charlotte_protocol_block::pack_lba_count(lba, count),
            memory,
        );
        wait_scalar(call).is_some_and(|(result, _)| result == 0)
    }

    fn flush(&self) -> bool {
        let call = ipc_scalar_call(self.conn, block::OP_FLUSH, 0);
        wait_scalar(call).is_some_and(|(result, _)| result == 0)
    }
}

fn wait_scalar(call: u64) -> Option<(i64, u64)> {
    if call == 0 {
        return None;
    }
    let (status, result, cap) = ipc_reply_wait(call);
    ipc_close(call);
    (status == 0).then_some((result as i64, cap))
}

struct ObjStore {
    dev: BlockDev,
    layout: Layout,
    super_generation: u64,
    next_id: u64,
    bitmap: spin::Mutex<Vec<u8>>,
    directory: spin::Mutex<Vec<DirectoryEntry>>,
    index_cache: spin::Mutex<BTreeMap<u64, u32>>,
    pending_sizes: spin::Mutex<BTreeMap<u64, u32>>,
}

impl ObjStore {
    fn mount(dev: BlockDev) -> Option<Self> {
        let expected = Layout::for_device(dev.block_size, dev.total_blocks)?;
        let first = read_superblock(&dev, 0);
        let second = read_superblock(&dev, 1);
        let selected = match (first, second) {
            (Some(a), Some(b)) => Some(
                if a.generation >= b.generation {
                    a
                } else {
                    b
                },
            ),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let Some(sb) = selected else {
            return Self::format(dev);
        };
        if sb.block_size != dev.block_size
            || sb.total_blocks != dev.total_blocks
            || sb.layout.bitmap_lba != expected.bitmap_lba
            || sb.layout.bitmap_blocks != expected.bitmap_blocks
            || sb.layout.directory_lba != expected.directory_lba
            || sb.layout.directory_blocks != expected.directory_blocks
            || sb.layout.data_lba != expected.data_lba
        {
            return Self::format(dev);
        }

        let mut store = Self {
            dev,
            layout: sb.layout,
            super_generation: sb.generation,
            next_id: sb.next_id.max(1),
            bitmap: spin::Mutex::new(vec![0; bitmap_len_bytes(sb.total_blocks)]),
            directory: spin::Mutex::new(vec![DirectoryEntry::default(); DIR_ENTRIES as usize]),
            index_cache: spin::Mutex::new(BTreeMap::new()),
            pending_sizes: spin::Mutex::new(BTreeMap::new()),
        };
        store.rebuild_allocation_state()?;
        Some(store)
    }

    fn format(dev: BlockDev) -> Option<Self> {
        let layout = Layout::for_device(dev.block_size, dev.total_blocks)?;
        let mut bitmap = vec![0; bitmap_len_bytes(dev.total_blocks)];
        bitmap_set_range(&mut bitmap, 0, layout.data_lba as u32, true);
        let store = Self {
            dev,
            layout,
            super_generation: 1,
            next_id: 1,
            bitmap: spin::Mutex::new(bitmap),
            directory: spin::Mutex::new(vec![DirectoryEntry::default(); DIR_ENTRIES as usize]),
            index_cache: spin::Mutex::new(BTreeMap::new()),
            pending_sizes: spin::Mutex::new(BTreeMap::new()),
        };
        if !store.zero_region(layout.directory_lba, layout.directory_blocks)
            || !store.persist_bitmap()
            || !store.write_superblock_slot(0, 1)
            || !store.write_superblock_slot(1, 1)
            || !store.dev.flush()
        {
            return None;
        }
        Some(store)
    }

    fn rebuild_allocation_state(&mut self) -> Option<()> {
        let mut rebuilt = vec![0; bitmap_len_bytes(self.dev.total_blocks)];
        bitmap_set_range(&mut rebuilt, 0, self.layout.data_lba as u32, true);
        let mut cache = BTreeMap::new();
        let directory_bytes =
            self.read_blocks(self.layout.directory_lba, self.layout.directory_blocks)?;
        for index in 0..DIR_ENTRIES {
            let offset = index as usize * DIR_ENTRY_SIZE;
            let entry = decode_directory(&directory_bytes[offset..offset + DIR_ENTRY_SIZE])?;
            if entry.id == 0 {
                continue;
            }
            if entry.flags & FLAG_ALLOCATED == 0 {
                return None;
            }
            self.directory.lock()[index as usize] = entry;
            cache.insert(entry.id, index);
            if entry.header_lba == 0 {
                continue;
            }
            let header = self.read_header(entry)?;
            let end = header
                .data_lba
                .checked_add(header.data_blocks as u64)
                .filter(|end| *end <= self.dev.total_blocks as u64)?;
            if header.header_lba_end(entry.header_lba) > header.data_lba
                || bitmap_range_used(
                    &rebuilt,
                    entry.header_lba as u32,
                    u32::try_from(end - entry.header_lba).ok()?,
                )
            {
                return None;
            }
            bitmap_set_range(
                &mut rebuilt,
                entry.header_lba as u32,
                u32::try_from(end - entry.header_lba).ok()?,
                true,
            );
        }
        *self.bitmap.lock() = rebuilt;
        *self.index_cache.lock() = cache;
        self.persist_bitmap().then_some(())
    }

    fn create(&mut self, requested: Option<u64>) -> Result<u64, i64> {
        let id = match requested {
            Some(0) => return Err(objstore::ERR_INVALID_ID),
            Some(id) if self.find_index(id).is_some() => return Err(objstore::ERR_EXISTS),
            Some(id) => id,
            None => {
                let id = self.next_id;
                self.next_id = self.next_id.checked_add(1).ok_or(objstore::ERR_NO_SPACE)?;
                id
            }
        };
        let index = self.find_free_directory().ok_or(objstore::ERR_NO_SPACE)?;
        let generation = 1;
        let allocation = self.allocate_extent(1).ok_or(objstore::ERR_NO_SPACE)?;
        let mut header = ObjectHeader::empty(id, generation, self.dev.block_size);
        header.data_lba = allocation + 1;
        if !self.write_header(allocation, header)
            || !self.persist_bitmap()
            || !self.dev.flush()
            || !self.write_directory(
                index,
                DirectoryEntry {
                    id,
                    flags: FLAG_ALLOCATED,
                    generation: generation as u32,
                    header_lba: allocation,
                    header_blocks: 1,
                },
            )
            || !self.dev.flush()
        {
            self.free_extent(allocation, 1);
            return Err(objstore::ERR_IO_ERROR);
        }
        self.index_cache.lock().insert(id, index);
        if requested.is_none() && !self.persist_superblock() {
            return Err(objstore::ERR_IO_ERROR);
        }
        Ok(id)
    }

    fn delete(&self, id: u64) -> i64 {
        let Some(index) = self.find_index(id) else {
            return objstore::ERR_NOT_FOUND;
        };
        let Some(entry) = self.read_directory(index) else {
            return objstore::ERR_IO_ERROR;
        };
        let allocation =
            self.read_header(entry).map(|header| (entry.header_lba, header.total_blocks()));
        if !self.write_directory(index, DirectoryEntry::default()) || !self.dev.flush() {
            return objstore::ERR_IO_ERROR;
        }
        self.index_cache.lock().remove(&id);
        if let Some((lba, blocks)) = allocation {
            self.free_extent(lba, blocks);
            if !self.persist_bitmap() || !self.dev.flush() {
                return objstore::ERR_IO_ERROR;
            }
        }
        objstore::ERR_OK
    }

    fn set_size(&self, id: u64, size_cap: u64) -> i64 {
        if self.find_index(id).is_none() {
            return objstore::ERR_NOT_FOUND;
        }
        if size_cap == 0 || memory_map(size_cap, BUFFER_VADDR, false) != 0 {
            return objstore::ERR_IO_ERROR;
        }
        let size = unsafe { core::ptr::read_unaligned(BUFFER_VADDR as *const u64) };
        memory_unmap(size_cap);
        let Ok(size) = u32::try_from(size) else {
            return objstore::ERR_TOO_LARGE;
        };
        self.pending_sizes.lock().insert(id, size);
        objstore::ERR_OK
    }

    fn write(&self, id: u64, data_cap: u64) -> i64 {
        let Some(index) = self.find_index(id) else {
            return objstore::ERR_NOT_FOUND;
        };
        let Some(old_entry) = self.read_directory(index) else {
            return objstore::ERR_IO_ERROR;
        };
        let Some(old_header) = self.read_header(old_entry) else {
            return objstore::ERR_IO_ERROR;
        };
        let size = self.pending_sizes.lock().remove(&id).unwrap_or(PAGE_SIZE as u32);
        if size != 0 && memory_get_phys_page(data_cap, (size as usize).div_ceil(PAGE_SIZE) - 1) == 0
        {
            return objstore::ERR_IO_ERROR;
        }

        let data_blocks = size.div_ceil(self.dev.block_size);
        let total_blocks = 1u32.saturating_add(data_blocks);
        let Some(header_lba) = self.allocate_extent(total_blocks) else {
            return objstore::ERR_NO_SPACE;
        };
        let data_lba = header_lba + 1;
        let hash = if size == 0 {
            content_hash(&[])
        } else {
            let Some(hash) = hash_memory(data_cap, size as usize) else {
                self.free_extent(header_lba, total_blocks);
                return objstore::ERR_IO_ERROR;
            };
            if !self.transfer_object(data_cap, data_lba, size, true) {
                self.free_extent(header_lba, total_blocks);
                return objstore::ERR_IO_ERROR;
            }
            hash
        };
        let generation = old_header.generation.saturating_add(1);
        let header = ObjectHeader {
            id,
            generation,
            data_len: size as u64,
            allocated_len: data_blocks as u64 * self.dev.block_size as u64,
            data_offset: self.dev.block_size as u64,
            header_blocks: 1,
            data_lba,
            data_blocks,
            data_hash: hash,
        };
        if !self.write_header(header_lba, header)
            || !self.persist_bitmap()
            || !self.dev.flush()
            || !self.write_directory(
                index,
                DirectoryEntry {
                    id,
                    flags: FLAG_ALLOCATED,
                    generation: generation as u32,
                    header_lba,
                    header_blocks: 1,
                },
            )
            || !self.dev.flush()
        {
            self.free_extent(header_lba, total_blocks);
            return objstore::ERR_IO_ERROR;
        }
        self.free_extent(old_entry.header_lba, old_header.total_blocks());
        if !self.persist_bitmap() || !self.dev.flush() {
            return objstore::ERR_IO_ERROR;
        }
        objstore::ERR_OK
    }

    fn read_and_reply(&self, id: u64, reply: u64) {
        if reply == 0 {
            return;
        }
        let Some(index) = self.find_index(id) else {
            ipc_reply(reply, objstore::ERR_NOT_FOUND);
            return;
        };
        let Some(entry) = self.read_directory(index) else {
            ipc_reply(reply, objstore::ERR_IO_ERROR);
            return;
        };
        let Some(header) = self.read_header(entry) else {
            ipc_reply(reply, objstore::ERR_IO_ERROR);
            return;
        };
        let Ok(size) = usize::try_from(header.data_len) else {
            ipc_reply(reply, objstore::ERR_TOO_LARGE);
            return;
        };
        let memory = memory_alloc(size.max(1).div_ceil(PAGE_SIZE));
        if memory == 0 {
            ipc_reply(reply, objstore::ERR_IO_ERROR);
            return;
        }
        if size != 0
            && (!self.transfer_object(memory, header.data_lba, size as u32, false)
                || hash_memory(memory, size) != Some(header.data_hash))
        {
            memory_close(memory);
            ipc_reply(reply, objstore::ERR_IO_ERROR);
            return;
        }
        ipc_reply_move(reply, memory, size as i64);
    }

    fn object_size(&self, id: u64) -> Option<u64> {
        let entry = self.read_directory(self.find_index(id)?)?;
        Some(self.read_header(entry)?.data_len)
    }

    fn read_directory(&self, index: u32) -> Option<DirectoryEntry> {
        if index >= DIR_ENTRIES {
            return None;
        }
        Some(self.directory.lock()[index as usize])
    }

    fn write_directory(&self, index: u32, entry: DirectoryEntry) -> bool {
        if index >= DIR_ENTRIES {
            return false;
        }
        let byte_offset = index as usize * DIR_ENTRY_SIZE;
        let lba = self.layout.directory_lba + (byte_offset / self.dev.block_size as usize) as u64;
        let offset = byte_offset % self.dev.block_size as usize;
        let Some(mut block) = self.read_block(lba) else {
            return false;
        };
        encode_directory(entry, &mut block[offset..offset + DIR_ENTRY_SIZE]);
        if !self.write_block(lba, &block) {
            return false;
        }
        self.directory.lock()[index as usize] = entry;
        true
    }

    fn read_header(&self, entry: DirectoryEntry) -> Option<ObjectHeader> {
        if entry.header_lba < self.layout.data_lba || entry.header_blocks == 0 {
            return None;
        }
        let bytes = self.read_blocks(entry.header_lba, entry.header_blocks)?;
        let header = decode_header(&bytes)?;
        let allocation_bytes = header.data_blocks as u64 * self.dev.block_size as u64;
        (header.id == entry.id
            && header.generation as u32 == entry.generation
            && header.header_blocks == entry.header_blocks
            && header.data_offset == entry.header_blocks as u64 * self.dev.block_size as u64
            && header.data_lba == entry.header_lba + entry.header_blocks as u64)
            .then_some(())
            .filter(|_| {
                header.data_len <= header.allocated_len
                    && header.allocated_len == allocation_bytes
                    && header
                        .data_lba
                        .checked_add(header.data_blocks as u64)
                        .is_some_and(|end| end <= self.dev.total_blocks as u64)
            })
            .map(|()| header)
    }

    fn write_header(&self, lba: u64, header: ObjectHeader) -> bool {
        let mut bytes = vec![0; self.dev.block_size as usize * header.header_blocks as usize];
        encode_header(header, &mut bytes);
        self.write_blocks_from_slice(lba, header.header_blocks, &bytes)
    }

    fn find_index(&self, id: u64) -> Option<u32> {
        if let Some(index) = self.index_cache.lock().get(&id).copied() {
            return Some(index);
        }
        for index in 0..DIR_ENTRIES {
            let entry = self.read_directory(index)?;
            if entry.id == id {
                self.index_cache.lock().insert(id, index);
                return Some(index);
            }
        }
        None
    }

    fn find_free_directory(&self) -> Option<u32> {
        (0..DIR_ENTRIES)
            .find(|index| self.read_directory(*index).is_some_and(|entry| entry.id == 0))
    }

    fn allocate_extent(&self, blocks: u32) -> Option<u64> {
        if blocks == 0 {
            return None;
        }
        let mut bitmap = self.bitmap.lock();
        let limit = self.dev.total_blocks.checked_sub(blocks)?;
        for start in self.layout.data_lba as u32..=limit {
            if !bitmap_range_used(&bitmap, start, blocks) {
                bitmap_set_range(&mut bitmap, start, blocks, true);
                return Some(start as u64);
            }
        }
        None
    }

    fn free_extent(&self, lba: u64, blocks: u32) {
        if lba >= self.layout.data_lba {
            bitmap_set_range(&mut self.bitmap.lock(), lba as u32, blocks, false);
        }
    }

    fn persist_bitmap(&self) -> bool {
        let bitmap = self.bitmap.lock().clone();
        self.write_blocks_from_slice(self.layout.bitmap_lba, self.layout.bitmap_blocks, &bitmap)
    }

    fn persist_superblock(&mut self) -> bool {
        self.super_generation = self.super_generation.saturating_add(1);
        self.write_superblock_slot(self.super_generation & 1, self.super_generation)
            && self.dev.flush()
    }

    fn write_superblock_slot(&self, slot: u64, generation: u64) -> bool {
        let sb = Superblock {
            generation,
            block_size: self.dev.block_size,
            total_blocks: self.dev.total_blocks,
            next_id: self.next_id,
            layout: self.layout,
        };
        let mut block = vec![0; self.dev.block_size as usize];
        encode_superblock(sb, &mut block);
        self.write_block(slot, &block)
    }

    fn read_block(&self, lba: u64) -> Option<Vec<u8>> {
        self.read_blocks(lba, 1)
    }

    fn read_blocks(&self, lba: u64, blocks: u32) -> Option<Vec<u8>> {
        let bytes = blocks as usize * self.dev.block_size as usize;
        let memory = memory_alloc(bytes.div_ceil(PAGE_SIZE));
        if memory == 0 || !self.dev.read_blocks_keep(lba, blocks, memory) {
            if memory != 0 {
                memory_close(memory);
            }
            return None;
        }
        if memory_map(memory, CHUNK_VADDR, false) != 0 {
            memory_close(memory);
            return None;
        }
        let mut result = vec![0; bytes];
        unsafe {
            core::ptr::copy_nonoverlapping(
                CHUNK_VADDR as *const u8,
                result.as_mut_ptr(),
                result.len(),
            );
        }
        memory_unmap(memory);
        memory_close(memory);
        Some(result)
    }

    fn write_block(&self, lba: u64, bytes: &[u8]) -> bool {
        self.write_blocks_from_slice(lba, 1, bytes)
    }

    fn write_blocks_from_slice(&self, lba: u64, blocks: u32, bytes: &[u8]) -> bool {
        let total = blocks as usize * self.dev.block_size as usize;
        if bytes.len() > total {
            return false;
        }
        let memory = memory_alloc(total.div_ceil(PAGE_SIZE));
        if memory == 0 || memory_map(memory, CHUNK_VADDR, true) != 0 {
            if memory != 0 {
                memory_close(memory);
            }
            return false;
        }
        unsafe {
            core::ptr::write_bytes(CHUNK_VADDR as *mut u8, 0, total);
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), CHUNK_VADDR as *mut u8, bytes.len());
        }
        memory_unmap(memory);
        let result = self.dev.write_blocks(lba, blocks, memory);
        memory_close(memory);
        result
    }

    fn zero_region(&self, lba: u64, blocks: u32) -> bool {
        let zero = vec![0; blocks as usize * self.dev.block_size as usize];
        self.write_blocks_from_slice(lba, blocks, &zero)
    }

    fn transfer_object(&self, memory: u64, lba: u64, size: u32, write: bool) -> bool {
        if memory_map(memory, BUFFER_VADDR, !write) != 0 {
            return false;
        }
        let chunk_bytes = MAX_IO_BYTES.min((u16::MAX as usize) * self.dev.block_size as usize);
        let mut offset = 0usize;
        let mut ok = true;
        while offset < size as usize {
            let bytes = (size as usize - offset).min(chunk_bytes);
            let blocks = bytes.div_ceil(self.dev.block_size as usize) as u32;
            let chunk = memory_alloc(bytes.div_ceil(PAGE_SIZE));
            if chunk == 0 || memory_map(chunk, CHUNK_VADDR, true) != 0 {
                if chunk != 0 {
                    memory_close(chunk);
                }
                ok = false;
                break;
            }
            if write {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        (BUFFER_VADDR + offset) as *const u8,
                        CHUNK_VADDR as *mut u8,
                        bytes,
                    );
                }
            }
            memory_unmap(chunk);
            let chunk_lba = lba + (offset / self.dev.block_size as usize) as u64;
            let io_ok = if write {
                self.dev.write_blocks(chunk_lba, blocks, chunk)
            } else {
                self.dev.read_blocks_keep(chunk_lba, blocks, chunk)
            };
            if !io_ok {
                memory_close(chunk);
                ok = false;
                break;
            }
            if !write {
                if memory_map(chunk, CHUNK_VADDR, false) != 0 {
                    memory_close(chunk);
                    ok = false;
                    break;
                }
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        CHUNK_VADDR as *const u8,
                        (BUFFER_VADDR + offset) as *mut u8,
                        bytes,
                    );
                }
                memory_unmap(chunk);
            }
            memory_close(chunk);
            offset += bytes;
        }
        memory_unmap(memory);
        ok
    }
}

impl ObjectHeader {
    fn empty(id: u64, generation: u64, block_size: u32) -> Self {
        Self {
            id,
            generation,
            data_len: 0,
            allocated_len: 0,
            data_offset: block_size as u64,
            header_blocks: 1,
            data_lba: 0, // fixed by write_header caller below
            data_blocks: 0,
            data_hash: content_hash(&[]),
        }
    }

    fn total_blocks(self) -> u32 {
        self.header_blocks.saturating_add(self.data_blocks)
    }

    fn header_lba_end(self, header_lba: u64) -> u64 {
        header_lba + self.header_blocks as u64
    }
}

fn read_superblock(dev: &BlockDev, slot: u64) -> Option<Superblock> {
    let memory = memory_alloc(1);
    if memory == 0 || !dev.read_blocks_keep(slot, 1, memory) {
        if memory != 0 {
            memory_close(memory);
        }
        return None;
    }
    if memory_map(memory, CHUNK_VADDR, false) != 0 {
        memory_close(memory);
        return None;
    }
    let bytes =
        unsafe { core::slice::from_raw_parts(CHUNK_VADDR as *const u8, dev.block_size as usize) };
    let result = decode_superblock(bytes);
    memory_unmap(memory);
    memory_close(memory);
    result
}

fn encode_superblock(sb: Superblock, bytes: &mut [u8]) {
    bytes.fill(0);
    put_u64(bytes, 0, SB_MAGIC);
    put_u32(bytes, 8, SB_VERSION);
    put_u32(bytes, 12, SB_LEN as u32);
    put_u64(bytes, 16, sb.generation);
    put_u32(bytes, 24, sb.block_size);
    put_u32(bytes, 28, sb.total_blocks);
    put_u64(bytes, 32, sb.next_id);
    put_u64(bytes, 40, sb.layout.bitmap_lba);
    put_u32(bytes, 48, sb.layout.bitmap_blocks);
    put_u32(bytes, 52, sb.layout.directory_blocks);
    put_u64(bytes, 56, sb.layout.directory_lba);
    put_u64(bytes, 64, sb.layout.data_lba);
    put_u32(bytes, 72, 0);
    put_u32(bytes, SB_CHECKSUM_OFFSET, crc32(&bytes[..SB_CHECKSUM_OFFSET]));
}

fn decode_superblock(bytes: &[u8]) -> Option<Superblock> {
    if bytes.len() < SB_LEN
        || get_u64(bytes, 0)? != SB_MAGIC
        || get_u32(bytes, 8)? != SB_VERSION
        || get_u32(bytes, 12)? as usize != SB_LEN
        || get_u32(bytes, SB_CHECKSUM_OFFSET)? != crc32(&bytes[..SB_CHECKSUM_OFFSET])
    {
        return None;
    }
    Some(Superblock {
        generation: get_u64(bytes, 16)?,
        block_size: get_u32(bytes, 24)?,
        total_blocks: get_u32(bytes, 28)?,
        next_id: get_u64(bytes, 32)?,
        layout: Layout {
            bitmap_lba: get_u64(bytes, 40)?,
            bitmap_blocks: get_u32(bytes, 48)?,
            directory_blocks: get_u32(bytes, 52)?,
            directory_lba: get_u64(bytes, 56)?,
            data_lba: get_u64(bytes, 64)?,
        },
    })
}

fn encode_directory(entry: DirectoryEntry, bytes: &mut [u8]) {
    bytes.fill(0);
    put_u64(bytes, 0, entry.id);
    put_u32(bytes, 8, entry.flags);
    put_u32(bytes, 12, entry.generation);
    put_u64(bytes, 16, entry.header_lba);
    put_u32(bytes, 24, entry.header_blocks);
    put_u32(bytes, 28, crc32(&bytes[..28]));
}

fn decode_directory(bytes: &[u8]) -> Option<DirectoryEntry> {
    if bytes.len() < DIR_ENTRY_SIZE {
        return None;
    }
    let id = get_u64(bytes, 0)?;
    if id == 0 && bytes.iter().all(|byte| *byte == 0) {
        return Some(DirectoryEntry::default());
    }
    if get_u32(bytes, 28)? != crc32(&bytes[..28]) {
        return None;
    }
    Some(DirectoryEntry {
        id,
        flags: get_u32(bytes, 8)?,
        generation: get_u32(bytes, 12)?,
        header_lba: get_u64(bytes, 16)?,
        header_blocks: get_u32(bytes, 24)?,
    })
}

fn encode_header(header: ObjectHeader, bytes: &mut [u8]) {
    bytes.fill(0);
    put_u64(bytes, 0, HEADER_MAGIC);
    put_u16(bytes, 8, HEADER_VERSION);
    put_u16(bytes, 10, HEADER_LEN as u16);
    put_u32(bytes, 12, FLAG_ALLOCATED);
    put_u64(bytes, 16, header.id);
    put_u64(bytes, 24, header.generation);
    put_u64(bytes, 32, header.data_len);
    put_u64(bytes, 40, header.allocated_len);
    put_u64(bytes, 48, header.data_offset);
    put_u16(bytes, 56, 1);
    put_u16(bytes, 58, HASH_FNV1A64);
    put_u32(bytes, 60, header.header_blocks);
    put_u64(bytes, 64, header.data_lba);
    put_u32(bytes, 72, header.data_blocks);
    put_u64(bytes, 80, header.data_hash);
    put_u32(bytes, HEADER_CHECKSUM_OFFSET, 0);
    put_u32(bytes, HEADER_CHECKSUM_OFFSET, crc32(&bytes[..HEADER_LEN]));
}

fn decode_header(bytes: &[u8]) -> Option<ObjectHeader> {
    if bytes.len() < HEADER_LEN
        || get_u64(bytes, 0)? != HEADER_MAGIC
        || get_u16(bytes, 8)? != HEADER_VERSION
        || (get_u16(bytes, 10)? as usize) > bytes.len()
        || (get_u16(bytes, 10)? as usize) < HEADER_LEN
        || get_u16(bytes, 56)? != 1
        || get_u16(bytes, 58)? != HASH_FNV1A64
    {
        return None;
    }
    let stored = get_u32(bytes, HEADER_CHECKSUM_OFFSET)?;
    let mut checksum_bytes = bytes[..get_u16(bytes, 10)? as usize].to_vec();
    put_u32(&mut checksum_bytes, HEADER_CHECKSUM_OFFSET, 0);
    if stored != crc32(&checksum_bytes) {
        return None;
    }
    Some(ObjectHeader {
        id: get_u64(bytes, 16)?,
        generation: get_u64(bytes, 24)?,
        data_len: get_u64(bytes, 32)?,
        allocated_len: get_u64(bytes, 40)?,
        data_offset: get_u64(bytes, 48)?,
        header_blocks: get_u32(bytes, 60)?,
        data_lba: get_u64(bytes, 64)?,
        data_blocks: get_u32(bytes, 72)?,
        data_hash: get_u64(bytes, 80)?,
    })
}

fn bitmap_len_bytes(total_blocks: u32) -> usize {
    (total_blocks as usize).div_ceil(8)
}

fn bitmap_set_range(bitmap: &mut [u8], start: u32, blocks: u32, used: bool) {
    for block in start..start.saturating_add(blocks) {
        let byte = block as usize / 8;
        let mask = 1u8 << (block % 8);
        if used {
            bitmap[byte] |= mask;
        } else {
            bitmap[byte] &= !mask;
        }
    }
}

fn bitmap_range_used(bitmap: &[u8], start: u32, blocks: u32) -> bool {
    (start..start.saturating_add(blocks))
        .any(|block| bitmap[block as usize / 8] & (1u8 << (block % 8)) != 0)
}

fn hash_memory(memory: u64, size: usize) -> Option<u64> {
    if memory_map(memory, BUFFER_VADDR, false) != 0 {
        return None;
    }
    let bytes = unsafe { core::slice::from_raw_parts(BUFFER_VADDR as *const u8, size) };
    let result = content_hash(bytes);
    memory_unmap(memory);
    Some(result)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320u32 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[allow(dead_code)]
fn sha256(bytes: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let total = (bytes.len() + 9).div_ceil(64) * 64;
    let mut padded = vec![0; total];
    padded[..bytes.len()].copy_from_slice(bytes);
    padded[bytes.len()] = 0x80;
    padded[total - 8..].copy_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.as_chunks::<64>().0 {
        let mut words = [0u32; 64];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            *word =
                u32::from_be_bytes(chunk[index * 4..index * 4 + 4].try_into().unwrap_or([0; 4]));
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] =
                words[index - 16].wrapping_add(s0).wrapping_add(words[index - 7]).wrapping_add(s1);
        }
        let mut work = state;
        for index in 0..64 {
            let sum1 =
                work[4].rotate_right(6) ^ work[4].rotate_right(11) ^ work[4].rotate_right(25);
            let choice = (work[4] & work[5]) ^ (!work[4] & work[6]);
            let temp1 = work[7]
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let sum0 =
                work[0].rotate_right(2) ^ work[0].rotate_right(13) ^ work[0].rotate_right(22);
            let majority = (work[0] & work[1]) ^ (work[0] & work[2]) ^ (work[1] & work[2]);
            let temp2 = sum0.wrapping_add(majority);
            work = [
                temp1.wrapping_add(temp2),
                work[0],
                work[1],
                work[2],
                work[3].wrapping_add(temp1),
                work[4],
                work[5],
                work[6],
            ];
        }
        for index in 0..8 {
            state[index] = state[index].wrapping_add(work[index]);
        }
    }
    let mut output = [0; 32];
    for (index, word) in state.iter().enumerate() {
        output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}

fn get_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?))
}
fn get_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}
fn get_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}
fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn main(ctx: Context) -> ! {
    let ns_connection = ctx.bootstrap_cap().unwrap_or_else(|| unsafe { thread_exit() });
    config::write::<u32>(0, 1);
    let dev = BlockDev::connect(ns_connection).unwrap_or_else(|| unsafe { thread_exit() });
    config::write::<u32>(0, 2);
    config::write::<u32>(16, dev.block_size);
    config::write::<u32>(20, dev.total_blocks);
    let mut store = ObjStore::mount(dev).unwrap_or_else(|| unsafe { thread_exit() });
    config::write::<u32>(0, 3);

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
    if wait_scalar(register).is_none_or(|(generation, _)| generation < 1)
        || ipc_endpoint_bind_cq(endpoint, 0) != 0
    {
        unsafe { thread_exit() };
    }
    config::write::<u32>(0, 4);
    config::write::<u32>(4, 0x900d);

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
                    let result = store.create(None).map(|id| id as i64).unwrap_or(0);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                objstore::OP_CREATE_AT => {
                    let result = store.create(Some(message.arg0)).map(|_| 0).unwrap_or_else(|e| e);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                objstore::OP_DELETE => {
                    let result = store.delete(message.arg0);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                objstore::OP_SET_SIZE => {
                    let result = store.set_size(message.arg0, message.memory);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                objstore::OP_WRITE => {
                    let result = store.write(message.arg0, message.memory);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                objstore::OP_READ => store.read_and_reply(message.arg0, message.reply),
                objstore::OP_RESIZE => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, objstore::ERR_IO_ERROR);
                    }
                }
                objstore::OP_FLUSH => {
                    if message.reply != 0 {
                        ipc_reply(
                            message.reply,
                            if store.dev.flush() {
                                0
                            } else {
                                objstore::ERR_IO_ERROR
                            },
                        );
                    }
                }
                objstore::OP_INFO => {
                    let result = store
                        .object_size(message.arg0)
                        .and_then(|size| i64::try_from(size).ok())
                        .unwrap_or(objstore::ERR_NOT_FOUND);
                    if message.reply != 0 {
                        ipc_reply(message.reply, result);
                    }
                }
                _ if message.reply != 0 => {
                    ipc_reply(message.reply, -1);
                }
                _ => {}
            }
        }
    }
}
