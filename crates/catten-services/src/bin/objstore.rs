//! Persistent object store service — on-disk format v3.
//!
//! The mirrored directory banks contain only atomic locators. Every allocated
//! object has a versioned, extensible header containing its generation, exact
//! data length, extent descriptor, and FNV-1a corruption-detection hash.
//! Replacement is copy-on-write: data, header, and allocation state are durable
//! before a locator in the alternate directory bank changes. Mount selects the
//! newest valid copy of each locator and reconstructs the bitmap from reachable
//! headers, reclaiming abandoned pre-commit allocations.
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
const METADATA_IO_BYTES: usize = 32 * 1024;

const SB_MAGIC: u64 = 0x3352_5453_424a_4f43; // "COBJSTR3"
const SB_VERSION: u32 = 3;
const SB_SLOTS: u32 = 2;
const SB_LEN: usize = 80;
const SB_CHECKSUM_OFFSET: usize = 76;

const DIR_ENTRY_SIZE: usize = 32;
const DIR_CRC_SALT: u32 = 0x3344_4952; // "3DIR"
const MAX_DIRECTORY_BLOCKS: u32 = 4096;
const FLAG_ALLOCATED: u32 = 1;

const HEADER_MAGIC: u64 = 0x3244_484a_424f_4343; // "CCOBJHD2"
const HEADER_VERSION: u16 = 3;
const HEADER_LEN: usize = 384;
const HEADER_CHECKSUM_OFFSET: usize = 112;
const EXTENTS_OFFSET: usize = 128;
const EXTENT_SIZE: usize = 16;
const MAX_EXTENTS: usize = 16;
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
        let bitmap_lba = SB_SLOTS as u64;
        let directory_lba = bitmap_lba + bitmap_blocks as u64;
        // Use roughly 1/2048 of the device for each directory bank. This keeps
        // metadata proportional to capacity without allowing it to dominate a
        // very large device.
        let directory_blocks = (total_blocks / 2048).clamp(1, MAX_DIRECTORY_BLOCKS);
        let data_lba = directory_lba.checked_add(u64::from(directory_blocks).checked_mul(2)?)?;
        (data_lba + 2 <= total_blocks as u64).then_some(Self {
            bitmap_lba,
            bitmap_blocks,
            directory_lba,
            directory_blocks,
            data_lba,
        })
    }

    fn directory_entries(self, block_size: u32) -> Option<u32> {
        let bytes = u64::from(self.directory_blocks).checked_mul(u64::from(block_size))?;
        u32::try_from(bytes / DIR_ENTRY_SIZE as u64).ok()
    }

    fn directory_bank_lba(self, bank: u32) -> Option<u64> {
        (bank < 2).then(|| self.directory_lba + u64::from(bank * self.directory_blocks))
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
struct Extent {
    lba: u64,
    blocks: u32,
}

const EMPTY_EXTENT: Extent = Extent {
    lba: 0,
    blocks: 0,
};

#[derive(Clone, Copy)]
struct ObjectHeader {
    id: u64,
    generation: u64,
    data_len: u64,
    allocated_len: u64,
    data_offset: u64,
    header_blocks: u32,
    extent_count: u16,
    extents: [Extent; MAX_EXTENTS],
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
            directory: spin::Mutex::new(vec![
                DirectoryEntry::default();
                sb.layout.directory_entries(sb.block_size)? as usize
            ]),
            index_cache: spin::Mutex::new(BTreeMap::new()),
            pending_sizes: spin::Mutex::new(BTreeMap::new()),
        };
        store.rebuild_allocation_state()?;
        Some(store)
    }

    fn format(dev: BlockDev) -> Option<Self> {
        let layout = Layout::for_device(dev.block_size, dev.total_blocks)?;
        let directory_entries = layout.directory_entries(dev.block_size)? as usize;
        let mut bitmap = vec![0; bitmap_len_bytes(dev.total_blocks)];
        bitmap_set_range(&mut bitmap, 0, layout.data_lba as u32, true);
        let store = Self {
            dev,
            layout,
            super_generation: 1,
            next_id: 1,
            bitmap: spin::Mutex::new(bitmap),
            directory: spin::Mutex::new(vec![DirectoryEntry::default(); directory_entries]),
            index_cache: spin::Mutex::new(BTreeMap::new()),
            pending_sizes: spin::Mutex::new(BTreeMap::new()),
        };
        if !store.persist_bitmap() {
            return None;
        }
        if !store.write_superblock_slot(0, 1) || !store.write_superblock_slot(1, 1) {
            return None;
        }
        if !store.dev.flush() {
            return None;
        }
        Some(store)
    }

    fn rebuild_allocation_state(&mut self) -> Option<()> {
        let mut rebuilt = vec![0; bitmap_len_bytes(self.dev.total_blocks)];
        bitmap_set_range(&mut rebuilt, 0, self.layout.data_lba as u32, true);
        let mut cache = BTreeMap::new();
        let both = self.read_blocks(
            self.layout.directory_bank_lba(0)?,
            self.layout.directory_blocks.checked_mul(2)?,
        )?;
        let bank_bytes = self.layout.directory_blocks as usize * self.dev.block_size as usize;
        let (first, second) = both.split_at(bank_bytes);
        let entries = self.directory.lock().len();
        for index in 0..entries {
            let offset = index * DIR_ENTRY_SIZE;
            let a = decode_directory(&first[offset..offset + DIR_ENTRY_SIZE]);
            let b = decode_directory(&second[offset..offset + DIR_ENTRY_SIZE]);
            let entry = newest_directory_entry(a, b)?;
            if entry.id == 0 {
                self.directory.lock()[index] = entry;
                continue;
            }
            if entry.flags & FLAG_ALLOCATED == 0 {
                return None;
            }
            self.directory.lock()[index] = entry;
            cache.insert(entry.id, index as u32);
            if entry.header_lba == 0 {
                continue;
            }
            let header = self.read_header(entry)?;
            if bitmap_range_used(&rebuilt, entry.header_lba as u32, entry.header_blocks) {
                return None;
            }
            bitmap_set_range(&mut rebuilt, entry.header_lba as u32, entry.header_blocks, true);
            for extent in header.extents() {
                if extent.lba < self.layout.data_lba
                    || extent
                        .lba
                        .checked_add(u64::from(extent.blocks))
                        .is_none_or(|end| end > self.dev.total_blocks as u64)
                    || bitmap_range_used(&rebuilt, extent.lba as u32, extent.blocks)
                {
                    return None;
                }
                bitmap_set_range(&mut rebuilt, extent.lba as u32, extent.blocks, true);
            }
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
        let generation = u64::from(
            self.read_directory(index).ok_or(objstore::ERR_IO_ERROR)?.generation.saturating_add(1),
        );
        let allocation = self.allocate_extent(1).ok_or(objstore::ERR_NO_SPACE)?;
        let header = ObjectHeader::empty(id, generation, self.dev.block_size);
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
        let allocation = self.read_header(entry);
        let tombstone = DirectoryEntry {
            generation: entry.generation.saturating_add(1),
            ..DirectoryEntry::default()
        };
        if !self.write_directory(index, tombstone) || !self.dev.flush() {
            return objstore::ERR_IO_ERROR;
        }
        self.index_cache.lock().remove(&id);
        if let Some(header) = allocation {
            self.free_object_allocation(entry.header_lba, header);
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
        let Some(header_lba) = self.allocate_extent(1) else {
            return objstore::ERR_NO_SPACE;
        };
        let Some(extents) = self.allocate_fragmented(data_blocks) else {
            self.free_extent(header_lba, 1);
            return objstore::ERR_NO_SPACE;
        };
        let hash = if size == 0 {
            content_hash(&[])
        } else {
            let Some(hash) = hash_memory(data_cap, size as usize) else {
                self.free_extent(header_lba, 1);
                self.free_extents(&extents);
                return objstore::ERR_IO_ERROR;
            };
            if !self.transfer_object(data_cap, &extents, size, true) {
                self.free_extent(header_lba, 1);
                self.free_extents(&extents);
                return objstore::ERR_IO_ERROR;
            }
            hash
        };
        let generation = old_header.generation.saturating_add(1);
        let mut extent_array = [EMPTY_EXTENT; MAX_EXTENTS];
        extent_array[..extents.len()].copy_from_slice(&extents);
        let header = ObjectHeader {
            id,
            generation,
            data_len: size as u64,
            allocated_len: data_blocks as u64 * self.dev.block_size as u64,
            data_offset: self.dev.block_size as u64,
            header_blocks: 1,
            extent_count: extents.len() as u16,
            extents: extent_array,
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
            self.free_extent(header_lba, 1);
            self.free_extents(&extents);
            return objstore::ERR_IO_ERROR;
        }
        self.free_object_allocation(old_entry.header_lba, old_header);
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
            && (!self.transfer_object(memory, header.extents(), size as u32, false)
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
        self.directory.lock().get(index as usize).copied()
    }

    fn write_directory(&self, index: u32, entry: DirectoryEntry) -> bool {
        if index as usize >= self.directory.lock().len() {
            return false;
        }
        let byte_offset = index as usize * DIR_ENTRY_SIZE;
        let bank = entry.generation & 1;
        let Some(bank_lba) = self.layout.directory_bank_lba(bank) else {
            return false;
        };
        let lba = bank_lba + (byte_offset / self.dev.block_size as usize) as u64;
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
        let allocation_blocks = header
            .extents()
            .iter()
            .try_fold(0u64, |total, extent| total.checked_add(u64::from(extent.blocks)))?;
        let allocation_bytes = allocation_blocks.checked_mul(self.dev.block_size as u64)?;
        (header.id == entry.id
            && header.generation as u32 == entry.generation
            && header.header_blocks == entry.header_blocks
            && header.data_offset == entry.header_blocks as u64 * self.dev.block_size as u64)
            .then_some(())
            .filter(|_| {
                header.data_len <= header.allocated_len
                    && header.allocated_len == allocation_bytes
                    && header.extents().iter().all(|extent| {
                        extent.lba >= self.layout.data_lba
                            && extent
                                .lba
                                .checked_add(u64::from(extent.blocks))
                                .is_some_and(|end| end <= self.dev.total_blocks as u64)
                    })
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
        let entries = self.directory.lock().len();
        for index in 0..entries {
            let entry = self.read_directory(index as u32)?;
            if entry.id == id {
                self.index_cache.lock().insert(id, index as u32);
                return Some(index as u32);
            }
        }
        None
    }

    fn find_free_directory(&self) -> Option<u32> {
        let entries = u32::try_from(self.directory.lock().len()).ok()?;
        (0..entries).find(|index| self.read_directory(*index).is_some_and(|entry| entry.id == 0))
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

    fn allocate_fragmented(&self, blocks: u32) -> Option<Vec<Extent>> {
        if blocks == 0 {
            return Some(Vec::new());
        }
        let mut bitmap = self.bitmap.lock();
        let mut remaining = blocks;
        let mut extents = Vec::new();
        let mut block = self.layout.data_lba as u32;
        while block < self.dev.total_blocks && remaining != 0 && extents.len() < MAX_EXTENTS {
            while block < self.dev.total_blocks && bitmap_range_used(&bitmap, block, 1) {
                block += 1;
            }
            let start = block;
            while block < self.dev.total_blocks
                && !bitmap_range_used(&bitmap, block, 1)
                && block - start < remaining
            {
                block += 1;
            }
            let count = block - start;
            if count != 0 {
                bitmap_set_range(&mut bitmap, start, count, true);
                extents.push(Extent {
                    lba: u64::from(start),
                    blocks: count,
                });
                remaining -= count;
            }
        }
        if remaining == 0 {
            return Some(extents);
        }
        for extent in &extents {
            bitmap_set_range(&mut bitmap, extent.lba as u32, extent.blocks, false);
        }
        None
    }

    fn free_extents(&self, extents: &[Extent]) {
        let mut bitmap = self.bitmap.lock();
        for extent in extents {
            if extent.lba >= self.layout.data_lba {
                bitmap_set_range(&mut bitmap, extent.lba as u32, extent.blocks, false);
            }
        }
    }

    fn free_object_allocation(&self, header_lba: u64, header: ObjectHeader) {
        self.free_extent(header_lba, header.header_blocks);
        self.free_extents(header.extents());
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
        let mut result = vec![0; bytes];
        let chunk_bytes = metadata_chunk_bytes(self.dev.block_size);
        let mut offset = 0usize;
        while offset < bytes {
            let count_bytes = (bytes - offset).min(chunk_bytes);
            let count = u32::try_from(count_bytes / self.dev.block_size as usize).ok()?;
            let memory = memory_alloc(count_bytes.div_ceil(PAGE_SIZE));
            if memory == 0
                || !self.dev.read_blocks_keep(
                    lba + (offset / self.dev.block_size as usize) as u64,
                    count,
                    memory,
                )
                || memory_map(memory, CHUNK_VADDR, false) != 0
            {
                if memory != 0 {
                    memory_close(memory);
                }
                return None;
            }
            unsafe {
                core::ptr::copy_nonoverlapping(
                    CHUNK_VADDR as *const u8,
                    result.as_mut_ptr().add(offset),
                    count_bytes,
                );
            }
            memory_unmap(memory);
            memory_close(memory);
            offset += count_bytes;
        }
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
        let chunk_bytes = metadata_chunk_bytes(self.dev.block_size);
        let mut offset = 0usize;
        while offset < total {
            let count_bytes = (total - offset).min(chunk_bytes);
            let count = (count_bytes / self.dev.block_size as usize) as u32;
            let memory = memory_alloc(count_bytes.div_ceil(PAGE_SIZE));
            if memory == 0 || memory_map(memory, CHUNK_VADDR, true) != 0 {
                if memory != 0 {
                    memory_close(memory);
                }
                return false;
            }
            let source_bytes = bytes.len().saturating_sub(offset).min(count_bytes);
            unsafe {
                core::ptr::write_bytes(CHUNK_VADDR as *mut u8, 0, count_bytes);
                if source_bytes != 0 {
                    core::ptr::copy_nonoverlapping(
                        bytes.as_ptr().add(offset),
                        CHUNK_VADDR as *mut u8,
                        source_bytes,
                    );
                }
            }
            memory_unmap(memory);
            let ok = self.dev.write_blocks(
                lba + (offset / self.dev.block_size as usize) as u64,
                count,
                memory,
            );
            memory_close(memory);
            if !ok {
                return false;
            }
            offset += count_bytes;
        }
        true
    }

    fn transfer_object(&self, memory: u64, extents: &[Extent], size: u32, write: bool) -> bool {
        if memory_map(memory, BUFFER_VADDR, !write) != 0 {
            return false;
        }
        let chunk_bytes = MAX_IO_BYTES.min((u16::MAX as usize) * self.dev.block_size as usize);
        let mut offset = 0usize;
        let mut ok = true;
        for extent in extents {
            let extent_bytes = extent.blocks as usize * self.dev.block_size as usize;
            let mut extent_offset = 0usize;
            while extent_offset < extent_bytes && offset < size as usize {
                let bytes =
                    (size as usize - offset).min(extent_bytes - extent_offset).min(chunk_bytes);
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
                let chunk_lba = extent.lba + (extent_offset / self.dev.block_size as usize) as u64;
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
                extent_offset += bytes;
            }
            if !ok {
                break;
            }
        }
        memory_unmap(memory);
        ok && offset == size as usize
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
            extent_count: 0,
            extents: [EMPTY_EXTENT; MAX_EXTENTS],
            data_hash: content_hash(&[]),
        }
    }

    fn extents(&self) -> &[Extent] {
        &self.extents[..self.extent_count as usize]
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
    put_u32(bytes, 28, crc32(&bytes[..28]) ^ DIR_CRC_SALT);
}

fn decode_directory(bytes: &[u8]) -> Option<DirectoryEntry> {
    if bytes.len() < DIR_ENTRY_SIZE {
        return None;
    }
    let id = get_u64(bytes, 0)?;
    if id == 0 && bytes.iter().all(|byte| *byte == 0) {
        return Some(DirectoryEntry::default());
    }
    if get_u32(bytes, 28)? != crc32(&bytes[..28]) ^ DIR_CRC_SALT {
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

fn newest_directory_entry(
    first: Option<DirectoryEntry>,
    second: Option<DirectoryEntry>,
) -> Option<DirectoryEntry> {
    match (first, second) {
        (Some(a), Some(b)) => Some(
            if a.generation >= b.generation {
                a
            } else {
                b
            },
        ),
        (Some(entry), None) | (None, Some(entry)) => Some(entry),
        (None, None) => Some(DirectoryEntry::default()),
    }
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
    put_u16(bytes, 56, header.extent_count);
    put_u16(bytes, 58, HASH_FNV1A64);
    put_u32(bytes, 60, header.header_blocks);
    put_u64(bytes, 80, header.data_hash);
    for (index, extent) in header.extents().iter().enumerate() {
        let offset = EXTENTS_OFFSET + index * EXTENT_SIZE;
        put_u64(bytes, offset, extent.lba);
        put_u32(bytes, offset + 8, extent.blocks);
    }
    put_u32(bytes, HEADER_CHECKSUM_OFFSET, 0);
    put_u32(bytes, HEADER_CHECKSUM_OFFSET, crc32(&bytes[..HEADER_LEN]));
}

fn decode_header(bytes: &[u8]) -> Option<ObjectHeader> {
    if bytes.len() < HEADER_LEN
        || get_u64(bytes, 0)? != HEADER_MAGIC
        || get_u16(bytes, 8)? != HEADER_VERSION
        || (get_u16(bytes, 10)? as usize) > bytes.len()
        || (get_u16(bytes, 10)? as usize) < HEADER_LEN
        || get_u16(bytes, 56)? as usize > MAX_EXTENTS
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
    let extent_count = get_u16(bytes, 56)?;
    let mut extents = [EMPTY_EXTENT; MAX_EXTENTS];
    for (index, extent) in extents.iter_mut().enumerate().take(extent_count as usize) {
        let offset = EXTENTS_OFFSET + index * EXTENT_SIZE;
        *extent = Extent {
            lba: get_u64(bytes, offset)?,
            blocks: get_u32(bytes, offset + 8)?,
        };
        if extent.blocks == 0 {
            return None;
        }
    }
    Some(ObjectHeader {
        id: get_u64(bytes, 16)?,
        generation: get_u64(bytes, 24)?,
        data_len: get_u64(bytes, 32)?,
        allocated_len: get_u64(bytes, 40)?,
        data_offset: get_u64(bytes, 48)?,
        header_blocks: get_u32(bytes, 60)?,
        extent_count,
        extents,
        data_hash: get_u64(bytes, 80)?,
    })
}

fn bitmap_len_bytes(total_blocks: u32) -> usize {
    (total_blocks as usize).div_ceil(8)
}

fn metadata_chunk_bytes(block_size: u32) -> usize {
    let block_size = block_size as usize;
    (METADATA_IO_BYTES / block_size).max(1) * block_size
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
