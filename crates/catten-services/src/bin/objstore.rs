//! Persistent object store service — on-disk format v4.
//!
//! The mirrored directory banks contain only atomic locators. Every allocated
//! object has a fixed current header containing its generation, exact
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
    ShutdownRequest,
    config,
    owned::{
        Connection,
        ConnectionRef,
        Endpoint,
        IncomingMessage,
        IpcError,
        OwnedMemory,
        PendingCall,
        ReplyToken,
    },
};
use catten_services::{
    block,
    ns,
    objstore,
    sleep_ms,
    wait_for_registered_name_owned,
};
use catten_syscall::IpcRights;
use charlotte_launch::objstore_status as status;
use charlotte_protocol_objstore::disk::*;

const PAGE_SIZE: usize = 4096;
const MAX_IO_BYTES: usize = 512 * 1024;
const METADATA_IO_BYTES: usize = 32 * 1024;
/// Worst-case wait for the final object-store flush during shutdown.
const SHUTDOWN_FLUSH_TIMEOUT_MS: u64 = 5_000;

// Mutable status-page diagnostics consumed by the kernel boot verifier.
// These survive an EL0 thread exit and let the supervisor distinguish an IPC
// setup failure from a block-I/O or on-disk-format failure.
fn diag_stage(stage: u32) {
    config::write::<u32>(status::STAGE, stage);
}

fn diag_error(error: u32) {
    config::write::<u32>(status::STARTUP_ERROR, error);
}

fn startup_failed(fallback: u32) -> ! {
    if unsafe { config::read::<u32>(status::STARTUP_ERROR) } == 0 {
        diag_error(fallback);
    }
    config::write_u32_release(status::STATE, status::State::Failed as u32);
    catten_rt::domain_abort()
}

// One immutable first-failure record, published after every field is written.
// Diagnostic recording does not authorize retry or hardware cleanup.
fn io_failure(error: u32, op: u32, lba: u64, count: u32, phase: u32, ipc: u64, result: i64) {
    if unsafe { config::read_u32_acquire(status::FIRST_IO_ERROR) } != 0 {
        return;
    }
    let state = unsafe { config::read_u32_acquire(status::STATE) };
    config::write::<u32>(status::FAILURE_PHASE, phase);
    config::write::<u64>(status::FAILURE_STATUS, ipc);
    config::write::<i64>(status::FAILURE_RESULT, result);
    config::write::<u64>(status::FAILURE_LBA, lba);
    config::write::<u32>(status::FAILURE_COUNT, count);
    config::write::<u32>(status::FAILURE_STATE, state);
    config::write::<u32>(status::FAILURE_OP, op);
    config::write_u32_release(status::FIRST_IO_ERROR, error);
    catten_rt::logln!(
        concat!(
            "[objstore] first block failure state={} op={} lba={} count={} ",
            "phase={} status={:#x} result={}"
        ),
        state,
        op,
        lba,
        count,
        phase,
        ipc,
        result
    );
}

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

/// The service handles requests serially. Keep the validated locator/header
/// together from authorization through data access; failed reads never adopt
/// a default ownership policy or retry a different header.
struct StoredObject {
    index: u32,
    entry: DirectoryEntry,
    header: ObjectHeader,
}

struct BlockDev {
    conn: Connection,
    block_size: u32,
    total_blocks: u32,
}

impl BlockDev {
    fn connect(ns_conn: ConnectionRef<'_>) -> Option<Self> {
        diag_stage(10);
        let Some((generation, conn)) = wait_for_registered_name_owned(ns_conn, block::NAME) else {
            diag_error(0xc2);
            return None;
        };
        if generation < 1 {
            diag_error(0xc3);
            return None;
        }
        diag_stage(12);
        let Ok(info) = conn.call(block::OP_INFO, 0) else {
            diag_error(0xc4);
            return None;
        };
        let Ok(reply) = info.wait() else {
            diag_error(0xc5);
            return None;
        };
        diag_stage(13);
        let (block_size, total_blocks) = charlotte_protocol_block::unpack_info(reply.result);
        if !(512..=PAGE_SIZE as u32).contains(&block_size)
            || Layout::for_device(block_size, total_blocks).is_none()
        {
            diag_error(0xc6);
            return None;
        }
        Some(Self {
            conn,
            block_size,
            total_blocks,
        })
    }

    fn read_blocks_keep(&self, lba: u64, count: u32, memory: &mut OwnedMemory) -> bool {
        diag_stage(40);
        config::write::<u32>(status::BLOCK_OP, block::OP_READ);
        let pending = self.conn.call_borrow_write(
            block::OP_READ,
            charlotte_protocol_block::pack_lba_count(lba, count),
            memory,
        );
        self.finish_block(block::OP_READ, lba, count, 0xd1, pending)
    }

    fn write_blocks(&self, lba: u64, count: u32, memory: &OwnedMemory) -> bool {
        diag_stage(41);
        config::write::<u32>(status::BLOCK_OP, block::OP_WRITE);
        let pending = self.conn.call_borrow_read(
            block::OP_WRITE,
            charlotte_protocol_block::pack_lba_count(lba, count),
            memory,
        );
        self.finish_block(block::OP_WRITE, lba, count, 0xd2, pending)
    }

    fn flush(&self) -> bool {
        diag_stage(42);
        config::write::<u32>(status::BLOCK_OP, block::OP_FLUSH);
        self.finish_block(block::OP_FLUSH, 0, 0, 0xd3, self.conn.call(block::OP_FLUSH, 0))
    }

    fn finish_block(
        &self,
        op: u32,
        lba: u64,
        count: u32,
        error: u32,
        pending: Result<PendingCall<'_>, IpcError>,
    ) -> bool {
        let pending = match pending {
            Ok(pending) => pending,
            Err(reason) => {
                let code = match reason {
                    IpcError::Status(code) => code,
                    _ => u64::MAX,
                };
                config::write::<i64>(status::BLOCK_RESULT, i64::MIN);
                io_failure(error, op, lba, count, 1, code, i64::MIN);
                return false;
            }
        };
        let reply = match pending.wait() {
            Ok(reply) => reply,
            Err(reason) => {
                let code = match reason {
                    IpcError::Status(code) => code,
                    _ => u64::MAX,
                };
                config::write::<i64>(status::BLOCK_RESULT, i64::MIN);
                io_failure(error, op, lba, count, 2, code, i64::MIN);
                return false;
            }
        };
        config::write::<i64>(status::BLOCK_RESULT, reply.result);
        if reply.result != 0 || reply.memory.is_some() || reply.connection.is_some() {
            io_failure(error, op, lba, count, 3, 0, reply.result);
            return false;
        }
        true
    }
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
        diag_stage(20);
        let Some(expected) = Layout::for_device(dev.block_size, dev.total_blocks) else {
            diag_error(0xe1);
            return None;
        };
        let selected = catten_services::objstore_mount::select(
            |slot| {
                diag_stage(21 + slot as u32);
                read_superblock(&dev, slot)
            },
            |a, b| {
                if a.generation >= b.generation {
                    a
                } else {
                    b
                }
            },
        )
        .map_err(|_| diag_error(0xe5))
        .ok()?;
        let Some(sb) = selected else {
            diag_stage(23);
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
            diag_stage(24);
            diag_error(0xe6);
            return None;
        }

        let mut store = Self {
            dev,
            layout: sb.layout,
            super_generation: sb.generation,
            next_id: sb.next_id,
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
        diag_stage(30);
        let Some(layout) = Layout::for_device(dev.block_size, dev.total_blocks) else {
            diag_error(0xf0);
            return None;
        };
        let Some(directory_entries) = layout.directory_entries(dev.block_size) else {
            diag_error(0xf0);
            return None;
        };
        let directory_entries = directory_entries as usize;
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
            diag_error(0xf1);
            return None;
        }
        diag_stage(31);
        if !store.write_superblock_slot(0, 1) || !store.write_superblock_slot(1, 1) {
            diag_error(0xf2);
            return None;
        }
        diag_stage(32);
        if !store.dev.flush() {
            diag_error(0xf3);
            return None;
        }
        diag_stage(33);
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
            if entry.flags != FLAG_ALLOCATED {
                return None;
            }
            self.directory.lock()[index] = entry;
            // Duplicate IDs would make ownership depend on directory order.
            // Reject a disk produced by an earlier colliding allocator.
            if cache.insert(entry.id, index as u32).is_some() {
                return None;
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

    fn create(&mut self, requested: Option<u64>, owner: u64, attributes: u32) -> Result<u64, i64> {
        let slot = self.find_free_directory();
        let mut next = self.next_id;
        let attempts = self.directory.lock().len().saturating_add(1);
        let id = objstore::admit_create_id(requested, &mut next, attempts, slot.is_some(), |id| {
            self.find_index(id).is_some()
        })?;
        let index = slot.ok_or(objstore::ERR_NO_SPACE)?;
        self.next_id = next;
        let generation = u64::from(
            self.read_directory(index).ok_or(objstore::ERR_IO_ERROR)?.generation.saturating_add(1),
        );
        let allocation = self.allocate_extent(1).ok_or(objstore::ERR_NO_SPACE)?;
        let header = ObjectHeader::empty(id, generation, self.dev.block_size, owner, attributes);
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

    fn delete(&self, object: StoredObject) -> i64 {
        let StoredObject {
            index,
            entry,
            header,
        } = object;
        let tombstone = DirectoryEntry {
            generation: entry.generation.saturating_add(1),
            ..DirectoryEntry::default()
        };
        if !self.write_directory(index, tombstone) || !self.dev.flush() {
            return objstore::ERR_IO_ERROR;
        }
        self.index_cache.lock().remove(&entry.id);
        self.pending_sizes.lock().remove(&entry.id);
        self.free_object_allocation(entry.header_lba, header);
        if !self.persist_bitmap() || !self.dev.flush() {
            return objstore::ERR_IO_ERROR;
        }
        objstore::ERR_OK
    }

    fn set_size(&self, object: StoredObject, size_memory: OwnedMemory) -> i64 {
        let Ok(mapping) = size_memory.map_read_only() else {
            return objstore::ERR_IO_ERROR;
        };
        let Some(encoded_size) = mapping.as_slice().get(..core::mem::size_of::<u64>()) else {
            return objstore::ERR_IO_ERROR;
        };
        let size = u64::from_le_bytes(encoded_size.try_into().unwrap());
        let Ok(size) = u32::try_from(size) else {
            return objstore::ERR_TOO_LARGE;
        };
        self.pending_sizes.lock().insert(object.entry.id, size);
        objstore::ERR_OK
    }

    fn write(&self, object: StoredObject, data_memory: OwnedMemory) -> i64 {
        let StoredObject {
            index,
            entry: old_entry,
            header: old_header,
        } = object;
        let id = old_entry.id;
        let size = self.pending_sizes.lock().remove(&id).unwrap_or(PAGE_SIZE as u32);
        let Ok(data) = data_memory.map_read_only() else {
            return objstore::ERR_IO_ERROR;
        };
        let Some(data) = data.as_slice().get(..size as usize) else {
            return objstore::ERR_IO_ERROR;
        };

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
            let hash = content_hash(data);
            if !self.write_object(data, &extents) {
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
            // Ownership and sharing survive every rewrite.
            owner: old_header.owner,
            attributes: old_header.attributes,
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

    fn read_and_reply(&self, object: StoredObject, reply: ReplyToken) {
        let header = object.header;
        let Ok(size) = usize::try_from(header.data_len) else {
            let _ = reply.reply(objstore::ERR_TOO_LARGE);
            return;
        };
        let Ok(memory) = OwnedMemory::allocate(size.max(1).div_ceil(PAGE_SIZE)) else {
            let _ = reply.reply(objstore::ERR_IO_ERROR);
            return;
        };
        let Ok(mut mapping) = memory.map_writable() else {
            let _ = reply.reply(objstore::ERR_IO_ERROR);
            return;
        };
        if size != 0 {
            let output = &mut mapping.as_mut_slice()[..size];
            if !self.read_object(output, header.extents())
                || content_hash(output) != header.data_hash
            {
                let _ = reply.reply(objstore::ERR_IO_ERROR);
                return;
            }
        }
        let Ok(memory) = mapping.unmap() else {
            let _ = reply.reply(objstore::ERR_IO_ERROR);
            return;
        };
        let _ = reply.reply_move(memory, size as i64);
    }

    fn resolve(&self, id: u64) -> Result<StoredObject, i64> {
        let index = self.find_index(id).ok_or(objstore::ERR_NOT_FOUND)?;
        let entry = self.read_directory(index).ok_or(objstore::ERR_IO_ERROR)?;
        let (entry, header) =
            objstore::resolve_header(Some(entry), |entry| self.read_header(entry))?;
        Ok(StoredObject {
            index,
            entry,
            header,
        })
    }

    /// Read a small object entirely into memory, verifying its stored content
    /// hash. The caller bounds `id`/size to the scope table.
    fn read_object_bytes(&self, id: u64) -> Option<Vec<u8>> {
        let entry = self.read_directory(self.find_index(id)?)?;
        let header = self.read_header(entry)?;
        let size = usize::try_from(header.data_len).ok()?;
        if size == 0 || size > objstore::SCOPE_MAX_LEN {
            return None;
        }
        let mut bytes = vec![0u8; size];
        if !self.read_object(&mut bytes, header.extents())
            || content_hash(&bytes) != header.data_hash
        {
            return None;
        }
        Some(bytes)
    }

    /// Persist an accepted scope table as the reserved scope object. The
    /// caller updates its in-memory table only after this succeeds.
    fn install_scope(&mut self, bytes: &[u8], principal: u64) -> i64 {
        if bytes.len() > objstore::SCOPE_MAX_LEN {
            return objstore::ERR_TOO_LARGE;
        }
        if self.find_index(objstore::SCOPE_OBJECT_ID).is_none()
            && let Err(error) = self.create(Some(objstore::SCOPE_OBJECT_ID), principal, 0)
            && error != objstore::ERR_EXISTS
        {
            return error;
        }
        let pages = bytes.len().max(1).div_ceil(PAGE_SIZE);
        let Ok(memory) = OwnedMemory::allocate(pages) else {
            return objstore::ERR_IO_ERROR;
        };
        let Ok(mut mapping) = memory.map_writable() else {
            return objstore::ERR_IO_ERROR;
        };
        mapping.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
        let Ok(memory) = mapping.unmap() else {
            return objstore::ERR_IO_ERROR;
        };
        self.pending_sizes.lock().insert(objstore::SCOPE_OBJECT_ID, bytes.len() as u32);
        match self.resolve(objstore::SCOPE_OBJECT_ID) {
            Ok(object) => self.write(object, memory),
            Err(error) => error,
        }
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
        if entry.header_lba < self.layout.data_lba || entry.header_blocks != 1 {
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
            let mut memory = OwnedMemory::allocate(count_bytes.div_ceil(PAGE_SIZE)).ok()?;
            // The read is a borrow-write lend: the buffer must be unmapped in
            // this address space while the server writes it via DMA. Map only
            // after the lend completes.
            if !self.dev.read_blocks_keep(
                lba + (offset / self.dev.block_size as usize) as u64,
                count,
                &mut memory,
            ) {
                return None;
            }
            let mapping = memory.map_read_only().ok()?;
            result[offset..offset + count_bytes]
                .copy_from_slice(&mapping.as_slice()[..count_bytes]);
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
            let Ok(memory) = OwnedMemory::allocate(count_bytes.div_ceil(PAGE_SIZE)) else {
                return false;
            };
            let Ok(mut mapping) = memory.map_writable() else {
                return false;
            };
            let source_bytes = bytes.len().saturating_sub(offset).min(count_bytes);
            mapping.as_mut_slice()[..count_bytes].fill(0);
            if source_bytes != 0 {
                mapping.as_mut_slice()[..source_bytes]
                    .copy_from_slice(&bytes[offset..offset + source_bytes]);
            }
            let Ok(memory) = mapping.unmap() else {
                return false;
            };
            let ok = self.dev.write_blocks(
                lba + (offset / self.dev.block_size as usize) as u64,
                count,
                &memory,
            );
            if !ok {
                return false;
            }
            offset += count_bytes;
        }
        true
    }

    fn write_object(&self, source: &[u8], extents: &[Extent]) -> bool {
        let chunk_bytes = MAX_IO_BYTES.min((u16::MAX as usize) * self.dev.block_size as usize);
        let mut offset = 0usize;
        for extent in extents {
            let extent_bytes = extent.blocks as usize * self.dev.block_size as usize;
            let mut extent_offset = 0usize;
            while extent_offset < extent_bytes && offset < source.len() {
                let bytes =
                    (source.len() - offset).min(extent_bytes - extent_offset).min(chunk_bytes);
                let blocks = bytes.div_ceil(self.dev.block_size as usize) as u32;
                let Ok(chunk) = OwnedMemory::allocate(bytes.div_ceil(PAGE_SIZE)) else {
                    return false;
                };
                let Ok(mut mapping) = chunk.map_writable() else {
                    return false;
                };
                mapping.as_mut_slice().fill(0);
                mapping.as_mut_slice()[..bytes].copy_from_slice(&source[offset..offset + bytes]);
                let Ok(chunk) = mapping.unmap() else {
                    return false;
                };
                let chunk_lba = extent.lba + (extent_offset / self.dev.block_size as usize) as u64;
                if !self.dev.write_blocks(chunk_lba, blocks, &chunk) {
                    return false;
                }
                offset += bytes;
                extent_offset += bytes;
            }
        }
        offset == source.len()
    }

    fn read_object(&self, destination: &mut [u8], extents: &[Extent]) -> bool {
        let chunk_bytes = MAX_IO_BYTES.min((u16::MAX as usize) * self.dev.block_size as usize);
        let mut offset = 0usize;
        for extent in extents {
            let extent_bytes = extent.blocks as usize * self.dev.block_size as usize;
            let mut extent_offset = 0usize;
            while extent_offset < extent_bytes && offset < destination.len() {
                let bytes =
                    (destination.len() - offset).min(extent_bytes - extent_offset).min(chunk_bytes);
                let blocks = bytes.div_ceil(self.dev.block_size as usize) as u32;
                let Ok(mut chunk) = OwnedMemory::allocate(bytes.div_ceil(PAGE_SIZE)) else {
                    return false;
                };
                let chunk_lba = extent.lba + (extent_offset / self.dev.block_size as usize) as u64;
                if !self.dev.read_blocks_keep(chunk_lba, blocks, &mut chunk) {
                    return false;
                }
                let Ok(mapping) = chunk.map_read_only() else {
                    return false;
                };
                destination[offset..offset + bytes].copy_from_slice(&mapping.as_slice()[..bytes]);
                offset += bytes;
                extent_offset += bytes;
            }
        }
        offset == destination.len()
    }
}

fn read_superblock(
    dev: &BlockDev,
    slot: u64,
) -> Result<catten_services::objstore_mount::Slot<Superblock>, ()> {
    config::write::<u32>(status::DETAIL, 50 + slot as u32 * 10);
    let mut memory = OwnedMemory::allocate(1).map_err(|_| {
        io_failure(0xe3, block::OP_READ, slot, 1, 4, u64::MAX, i64::MIN);
    })?;
    if !dev.read_blocks_keep(slot, 1, &mut memory) {
        return Err(());
    }
    config::write::<u32>(status::DETAIL, 51 + slot as u32 * 10);
    let Ok(mapping) = memory.map_read_only() else {
        config::write::<u32>(status::DETAIL, 0xe50 + slot as u32);
        io_failure(0xe4, block::OP_READ, slot, 1, 5, u64::MAX, i64::MIN);
        return Err(());
    };
    config::write::<u32>(status::DETAIL, 52 + slot as u32 * 10);
    use catten_services::objstore_mount::Slot;
    let bytes = &mapping.as_slice()[..dev.block_size as usize];
    let result = match classify_superblock(bytes) {
        SuperblockState::Blank => Slot::Blank,
        SuperblockState::Current => Slot::Valid(decode_superblock(bytes).ok_or(())?),
        SuperblockState::Corrupt => Slot::Corrupt,
        SuperblockState::Unsupported => Slot::Unsupported,
    };
    config::write::<u32>(status::DETAIL, 53 + slot as u32 * 10);
    config::write::<u32>(status::DETAIL, 54 + slot as u32 * 10);
    config::write::<u32>(status::DETAIL, 55 + slot as u32 * 10);
    Ok(result)
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

fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn get_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}
fn get_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}
fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn reply_scalar(reply: Option<ReplyToken>, result: i64) {
    if let Some(reply) = reply {
        let _ = reply.reply(result);
    }
}

/// Install an operator scope table. Only the kernel and the signed
/// administration service may change policy; a malformed table is rejected
/// and leaves the previous table in force.
fn apply_scope(
    store: &mut ObjStore,
    scope: &mut objstore::ScopeTable,
    message: &mut IncomingMessage,
    principal: u64,
) -> i64 {
    if !objstore::reserved_mutator(principal) {
        return objstore::ERR_DENIED;
    }
    let Ok(length) = usize::try_from(message.arg0) else {
        return objstore::ERR_TOO_LARGE;
    };
    if !(objstore::SCOPE_HEADER_LEN..=objstore::SCOPE_MAX_LEN).contains(&length) {
        return objstore::ERR_TOO_LARGE;
    }
    let Some(memory) = message.memory.take() else {
        return objstore::ERR_IO_ERROR;
    };
    let parsed = {
        let Ok(mapping) = memory.map_read_only() else {
            return objstore::ERR_IO_ERROR;
        };
        let Some(bytes) = mapping.as_slice().get(..length) else {
            return objstore::ERR_IO_ERROR;
        };
        objstore::ScopeTable::parse(bytes)
    };
    let Some(parsed) = parsed else {
        return objstore::ERR_INVALID_ID;
    };
    let persisted = parsed.encode();
    let result = store.install_scope(&persisted, principal);
    if result == objstore::ERR_OK {
        *scope = parsed;
        catten_rt::logln!("[objstore] operator scope table applied ({} entries)", scope.len());
    }
    result
}

fn serve(ctx: &Context) -> ShutdownRequest {
    let ns_connection = ctx.bootstrap_connection().unwrap_or_else(|| startup_failed(0xb0));
    diag_stage(1);
    let dev = BlockDev::connect(ns_connection).unwrap_or_else(|| startup_failed(0xc0));
    diag_stage(2);
    config::write::<u32>(status::BLOCK_SIZE, dev.block_size);
    config::write::<u32>(status::TOTAL_BLOCKS, dev.total_blocks);
    let mut store = ObjStore::mount(dev).unwrap_or_else(|| startup_failed(0xe2));
    diag_stage(3);
    // Re-apply the persisted operator scope table. A malformed or missing
    // table fails closed to no grants.
    let mut scope = match store.read_object_bytes(objstore::SCOPE_OBJECT_ID) {
        Some(bytes) => match objstore::ScopeTable::parse(&bytes) {
            Some(table) => {
                catten_rt::logln!(
                    "[objstore] persisted scope table restored ({} entries)",
                    table.len()
                );
                table
            }
            None => {
                catten_rt::logln!("[objstore] persisted scope table malformed; denying all grants");
                objstore::ScopeTable::default()
            }
        },
        None => objstore::ScopeTable::default(),
    };

    let endpoint =
        Endpoint::create(objstore::INTERFACE, objstore::VERSION, 64).unwrap_or_else(|_| {
            diag_error(0xb1);
            startup_failed(0xb1)
        });
    let generation = ns_connection
        .call_connection(
            ns::OP_REGISTER,
            objstore::NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .and_then(|call| call.wait().map_err(|_| catten_rt::owned::IpcError::CreationFailed))
        .map(|reply| reply.result)
        .unwrap_or(0);
    if generation < 1 {
        diag_error(0xb2);
        startup_failed(0xb2);
    }
    diag_stage(4);
    config::write::<u32>(status::SENTINEL, 0x900d);
    config::write_u32_release(status::STATE, status::State::Serving as u32);

    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            config::write_u32_release(status::STATE, status::State::Draining as u32);
            // Closing the public endpoint first prevents any later mutation
            // from entering the store. Do not publish lifecycle readiness
            // until the complete on-disk commit chain is durable.
            drop(endpoint);
            // Bound the shutdown flush so a dead block device cannot hang the
            // node's shutdown indefinitely.
            let (start_ticks, frequency_hz) = catten_syscall::monotonic_clock();
            let frequency_hz = frequency_hz.max(1);
            while !store.dev.flush() {
                let (now_ticks, _) = catten_syscall::monotonic_clock();
                let elapsed_ms =
                    now_ticks.saturating_sub(start_ticks).saturating_mul(1000) / frequency_hz;
                if elapsed_ms >= SHUTDOWN_FLUSH_TIMEOUT_MS {
                    catten_rt::logln!(
                        "[objstore] flush did not complete before the shutdown deadline"
                    );
                    break;
                }
                catten_rt::logln!("[objstore] shutdown flush failed; retrying");
                sleep_ms(10);
            }
            drop(store);
            return request;
        }

        loop {
            let Some(mut message) =
                endpoint.try_receive().unwrap_or_else(|_| catten_rt::domain_abort())
            else {
                break;
            };
            // The sender principal is kernel-authenticated, never
            // caller-supplied. Ownership and sharing are persisted in the
            // object header; reserved artifact/executable objects are
            // mutated only by the kernel and the deployment administration
            // service, and ordinary objects are private to their creator.
            let id = message.arg0;
            let principal = message.sender_principal;
            let object = if matches!(
                message.opcode,
                objstore::OP_READ
                    | objstore::OP_INFO
                    | objstore::OP_DELETE
                    | objstore::OP_SET_SIZE
                    | objstore::OP_WRITE
            ) {
                match store.resolve(id) {
                    Ok(object) => Some(object),
                    Err(error) => {
                        reply_scalar(message.reply.take(), error);
                        continue;
                    }
                }
            } else {
                None
            };
            let may_create = objstore::may_create(id, principal);
            let scope_read = scope.allows(principal, id, objstore::SCOPE_RIGHT_READ);
            let scope_write = scope.allows(principal, id, objstore::SCOPE_RIGHT_WRITE);
            let may_mutate = object.as_ref().is_some_and(|object| {
                objstore::may_mutate_scoped(id, object.header.owner, principal, scope_write)
            });
            let may_read = object.as_ref().is_some_and(|object| {
                objstore::may_read_scoped(
                    id,
                    object.header.owner,
                    object.header.attributes,
                    principal,
                    scope_read,
                )
            });
            match message.opcode {
                objstore::OP_CREATE => {
                    let result = store.create(None, principal, 0).map(|id| id as i64).unwrap_or(0);
                    reply_scalar(message.reply.take(), result);
                }
                objstore::OP_CREATE_AT => {
                    let result = if may_create {
                        store.create(Some(id), principal, 0).map(|_| 0).unwrap_or_else(|e| e)
                    } else {
                        objstore::ERR_DENIED
                    };
                    reply_scalar(message.reply.take(), result);
                }
                objstore::OP_CREATE_AT_SHARED => {
                    let result = if may_create {
                        store
                            .create(Some(id), principal, objstore::ATTR_SHARED_READ)
                            .map(|_| 0)
                            .unwrap_or_else(|e| e)
                    } else {
                        objstore::ERR_DENIED
                    };
                    reply_scalar(message.reply.take(), result);
                }
                objstore::OP_DELETE => {
                    let result = if may_mutate {
                        store.delete(object.unwrap())
                    } else {
                        objstore::ERR_DENIED
                    };
                    reply_scalar(message.reply.take(), result);
                }
                objstore::OP_SET_SIZE => {
                    let result = if !may_mutate {
                        objstore::ERR_DENIED
                    } else {
                        message
                            .memory
                            .take()
                            .map(|memory| store.set_size(object.unwrap(), memory))
                            .unwrap_or(objstore::ERR_IO_ERROR)
                    };
                    reply_scalar(message.reply.take(), result);
                }
                objstore::OP_WRITE => {
                    let result = if !may_mutate {
                        objstore::ERR_DENIED
                    } else {
                        message
                            .memory
                            .take()
                            .map(|memory| store.write(object.unwrap(), memory))
                            .unwrap_or(objstore::ERR_IO_ERROR)
                    };
                    reply_scalar(message.reply.take(), result);
                }
                objstore::OP_READ => {
                    if let Some(reply) = message.reply.take() {
                        if may_read {
                            store.read_and_reply(object.unwrap(), reply);
                        } else {
                            let _ = reply.reply(objstore::ERR_DENIED);
                        }
                    }
                }
                objstore::OP_APPLY_SCOPE => {
                    let result = apply_scope(&mut store, &mut scope, &mut message, principal);
                    reply_scalar(message.reply.take(), result);
                }
                objstore::OP_RESIZE => {
                    reply_scalar(message.reply.take(), objstore::ERR_IO_ERROR);
                }
                objstore::OP_FLUSH => {
                    let result = if store.dev.flush() {
                        0
                    } else {
                        objstore::ERR_IO_ERROR
                    };
                    reply_scalar(message.reply.take(), result);
                }
                objstore::OP_INFO => {
                    let result = if !may_read {
                        objstore::ERR_DENIED
                    } else {
                        i64::try_from(object.unwrap().header.data_len)
                            .unwrap_or(objstore::ERR_TOO_LARGE)
                    };
                    reply_scalar(message.reply.take(), result);
                }
                _ => reply_scalar(message.reply.take(), -1),
            }
        }
        sleep_ms(10);
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}
