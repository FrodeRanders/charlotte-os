//! Historical SEC-12 review reproduction for ec8924fe7d319720d54fb1cb0a68ee785ea6540c.
//! Production create/metadata/policy bodies are copied from that revision;
//! std Mutex replaces spin Mutex. Disk I/O and allocation are host doubles.
//! Assertions demonstrate the observed defects, not the desired fixed behavior.
//! This is a standalone evidence artifact, not a workspace regression suite.
//! See ../2026-10-10-security-progress-verification.md for scope and commands.

use std::{
    cell::Cell,
    collections::BTreeMap,
    sync::Mutex,
};
mod objstore {
    pub const ERR_INVALID_ID: i64 = 3;
    pub const ERR_EXISTS: i64 = 5;
    pub const ERR_NO_SPACE: i64 = 2;
    pub const ERR_IO_ERROR: i64 = 4;
    const KERNEL_PRINCIPAL: u64 = 1;
    const ATTR_SHARED_READ: u32 = 1;
    pub const fn is_reserved_object(id: u64) -> bool {
        id >> 48 >= 0xfffe
    }
    pub fn may_read(id: u64, owner: u64, attributes: u32, principal: u64) -> bool {
        is_reserved_object(id)
            || attributes & ATTR_SHARED_READ != 0
            || owner == 0
            || owner == principal
            || principal == KERNEL_PRINCIPAL
    }
    pub fn may_read_scoped(
        id: u64,
        owner: u64,
        attributes: u32,
        principal: u64,
        scope_read: bool,
    ) -> bool {
        may_read(id, owner, attributes, principal) || scope_read
    }
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
    owner: u64,
    attributes: u32,
}
impl ObjectHeader {
    fn empty(_: u64, _: u64, _: u32, owner: u64, attributes: u32) -> Self {
        Self {
            owner,
            attributes,
        }
    }
}
struct Dev {
    block_size: u32,
}
impl Dev {
    fn flush(&self) -> bool {
        true
    }
}
const FLAG_ALLOCATED: u32 = 1;
struct ObjStore {
    next_id: u64,
    dev: Dev,
    index_cache: Mutex<BTreeMap<u64, u32>>,
    directory: Vec<DirectoryEntry>,
    headers: BTreeMap<u64, ObjectHeader>,
    fail_once: Cell<bool>,
}
impl ObjStore {
    fn new() -> Self {
        Self {
            next_id: 1,
            dev: Dev {
                block_size: 512,
            },
            index_cache: Mutex::new(BTreeMap::new()),
            directory: vec![DirectoryEntry::default(); 8],
            headers: BTreeMap::new(),
            fail_once: Cell::new(false),
        }
    }

    fn find_index(&self, id: u64) -> Option<u32> {
        self.index_cache.lock().unwrap().get(&id).copied()
    }

    fn find_free_directory(&self) -> Option<u32> {
        self.directory.iter().position(|e| e.id == 0).map(|n| n as u32)
    }

    fn read_directory(&self, i: u32) -> Option<DirectoryEntry> {
        self.directory.get(i as usize).copied()
    }

    fn allocate_extent(&self, _: u32) -> Option<u64> {
        Some(self.headers.len() as u64 + 1)
    }

    fn write_header(&mut self, lba: u64, h: ObjectHeader) -> bool {
        self.headers.insert(lba, h);
        true
    }

    fn read_header(&self, e: DirectoryEntry) -> Option<ObjectHeader> {
        if self.fail_once.replace(false) {
            None
        } else {
            self.headers.get(&e.header_lba).copied()
        }
    }

    fn persist_bitmap(&self) -> bool {
        true
    }

    fn persist_superblock(&self) -> bool {
        true
    }

    fn write_directory(&mut self, i: u32, e: DirectoryEntry) -> bool {
        self.directory[i as usize] = e;
        true
    }

    fn free_extent(&self, _: u64, _: u32) {}

    fn create(&mut self, requested: Option<u64>, owner: u64, attributes: u32) -> Result<u64, i64> {
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
        self.index_cache.lock().unwrap().insert(id, index);
        if requested.is_none() && !self.persist_superblock() {
            return Err(objstore::ERR_IO_ERROR);
        }
        Ok(id)
    }

    fn metadata(&self, id: u64) -> Option<(u64, u32)> {
        let entry = self.read_directory(self.find_index(id)?)?;
        let header = self.read_header(entry)?;
        Some((header.owner, header.attributes))
    }
}
fn main() {
    let mut store = ObjStore::new();
    let victim = 100u64;
    let attacker = 200u64;
    let id = 1u64;
    assert_eq!(store.create(Some(id), victim, 0), Ok(id));
    assert_eq!(store.metadata(id), Some((victim, 0)));
    assert!(!objstore::may_read_scoped(id, victim, 0, attacker, false));
    store.fail_once.set(true);
    let (owner, attributes) = store.metadata(id).unwrap_or((0, 0));
    assert!(objstore::may_read_scoped(id, owner, attributes, attacker, false));
    let header =
        store.read_header(store.read_directory(store.find_index(id).unwrap()).unwrap()).unwrap();
    assert_eq!(header.owner, victim);
    println!(
        "CONFIRMED: failed metadata read authorizes a foreign private object; second header read \
         succeeds"
    );
    assert_eq!(store.create(None, attacker, 0), Ok(id));
    assert_eq!(store.metadata(id), Some((attacker, 0)));
    assert_eq!(store.directory.iter().filter(|e| e.id == id).count(), 2);
    println!(
        "CONFIRMED: automatic creation aliases an existing foreign-owned ID; cache owner changes \
         and two live directory records share the ID"
    );
}
