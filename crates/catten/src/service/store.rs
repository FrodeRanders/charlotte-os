//! The service-ELF registry: where the kernel gets the userspace binaries
//! it loads.
//!
//! AArch64 embeds only the small set needed to reach naming, storage, UART,
//! and observation; other services live in the object store on the boot disk.
//! The x86_64 parity suite currently embeds its complete tested service set and
//! stages the same signed artifacts in the persistent object-store image. An
//! image produced by `scripts/make-nvme-image.py` can be attached through NVMe,
//! AHCI, or virtio-blk. The registry reads non-embedded services from the store
//! on first use and caches them for the rest of the boot. The userspace loader
//! enforces a valid cluster signature regardless of where the bytes came from,
//! so the store-sourced path is exactly as trusted as the embedded one. The
//! `hvf_compat` development configuration has no SMMU and therefore cannot
//! start the protected-DMA disk stack; it embeds the additional non-storage
//! services used by its reduced boot suite so lookup never waits for an object
//! store that cannot exist.

use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::{
    ipc,
    memory::KERNEL_ASID,
};

const OBJ_OP_READ: u32 = 4;
const OBJ_OP_WRITE: u32 = 3;
const OBJ_OP_FLUSH: u32 = 6;
const OBJ_OP_CREATE_AT: u32 = 8;
const OBJ_OP_SET_SIZE: u32 = 9;
const OBJ_ERR_OK: i64 = 0;
const OBJ_ERR_EXISTS: i64 = 5;

// This is a loader resource bound, not an object-store format limit. Keep it
// comfortably above the largest staged service while preventing corrupted or
// misconfigured store contents from consuming an unbounded amount of kernel
// heap before the ELF verifier can reject them.
const MAX_SERVICE_ELF_SIZE: usize = 4 * 1024 * 1024;

// The embedded bootstrap set. `CATTEN_{ARCH}_SERVICE_BUNDLE` points at the
// staged, signed bundle (the build pipeline signs every ELF in it).
#[cfg(target_arch = "aarch64")]
macro_rules! bootstrap_elf {
    ($name:literal, $file:literal) => {
        ($name, include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/", $file, ".elf")))
    };
}

#[cfg(target_arch = "x86_64")]
macro_rules! bootstrap_elf {
    ($name:literal, $file:literal) => {
        ($name, include_bytes!(concat!(env!("CATTEN_X86_64_SERVICE_BUNDLE"), "/", $file, ".elf")))
    };
}

#[cfg(target_arch = "aarch64")]
const BOOTSTRAP_ELFS: &[(&[u8], &[u8])] = &[
    bootstrap_elf!(b"ns", "ns"),
    bootstrap_elf!(b"nvme", "nvme"),
    bootstrap_elf!(b"objstore", "objstore"),
    bootstrap_elf!(b"uart", "uart"),
    // The system observer is started by the kernel itself during boot,
    // before the disk stack is up, so it must be embedded too.
    bootstrap_elf!(b"observe", "observe"),
    // The grant controller mediates application bootstrap authority before
    // store-backed services are available.
    bootstrap_elf!(b"grantctl", "grantctl"),
    #[cfg(feature = "shutdown_test")]
    bootstrap_elf!(b"shutdown_probe", "shutdown_probe"),
    // HVF cannot provide the SMMU required by the NVMe driver's protected DMA
    // domain. Keep its explicitly non-storage compatibility suite useful
    // without weakening DMA isolation or blocking on an unavailable store.
    #[cfg(feature = "hvf_compat")]
    bootstrap_elf!(b"cclient", "cclient"),
    #[cfg(feature = "hvf_compat")]
    bootstrap_elf!(b"sitas-user", "sitas-user"),
    #[cfg(feature = "hvf_compat")]
    bootstrap_elf!(b"echo", "echo"),
    #[cfg(feature = "hvf_compat")]
    bootstrap_elf!(b"client", "client"),
    #[cfg(feature = "hvf_compat")]
    bootstrap_elf!(b"servicemgr", "servicemgr"),
];

// x86_64 embeds the same service set exercised by its storage, lifecycle, and
// networking suites. The runner also stages these signed artifacts on the
// separate persistent block image used by store-backed service loading.
#[cfg(target_arch = "x86_64")]
const BOOTSTRAP_ELFS: &[(&[u8], &[u8])] = &[
    bootstrap_elf!(b"ns", "ns"),
    bootstrap_elf!(b"observe", "observe"),
    bootstrap_elf!(b"grantctl", "grantctl"),
    bootstrap_elf!(b"nvme", "nvme"),
    bootstrap_elf!(b"objstore", "objstore"),
    bootstrap_elf!(b"nvme_client", "nvme_client"),
    bootstrap_elf!(b"objstore_client", "objstore_client"),
    bootstrap_elf!(b"echo", "echo"),
    bootstrap_elf!(b"raft", "raft"),
    bootstrap_elf!(b"client", "client"),
    bootstrap_elf!(b"servicemgr", "servicemgr"),
    bootstrap_elf!(b"ahci", "ahci"),
    bootstrap_elf!(b"virtio_blk", "virtio_blk"),
    bootstrap_elf!(b"net", "net"),
    bootstrap_elf!(b"e1000e", "e1000e"),
    bootstrap_elf!(b"nclient", "nclient"),
    bootstrap_elf!(b"disco", "disco"),
    bootstrap_elf!(b"frouter", "frouter"),
    bootstrap_elf!(b"dns", "dns"),
    bootstrap_elf!(b"agent", "agent"),
    bootstrap_elf!(b"greet", "greet"),
    #[cfg(feature = "shutdown_test")]
    bootstrap_elf!(b"shutdown_probe", "shutdown_probe"),
    bootstrap_elf!(b"relmsg", "relmsg"),
    bootstrap_elf!(b"rclient", "rclient"),
    bootstrap_elf!(b"tcpip", "tcpip"),
    bootstrap_elf!(b"tcpclient", "tcpclient"),
    bootstrap_elf!(b"httpd", "httpd"),
    bootstrap_elf!(b"time", "time"),
    bootstrap_elf!(b"s3", "s3"),
    bootstrap_elf!(b"kafka", "kafka"),
    bootstrap_elf!(b"rng", "rng"),
    #[cfg(feature = "s3_test")]
    bootstrap_elf!(b"s3_smoke", "s3_smoke"),
    #[cfg(feature = "kafka_test")]
    bootstrap_elf!(b"kafka_smoke", "kafka_smoke"),
    bootstrap_elf!(b"fs", "fs"),
    bootstrap_elf!(b"clusterctl", "clusterctl"),
    bootstrap_elf!(b"deployd", "deployd"),
];

/// Loaded, store-sourced service images, keyed by the artifact name.
static STORE_ELFS: spin::Mutex<alloc::vec::Vec<(&'static [u8], &'static [u8])>> =
    spin::Mutex::new(alloc::vec::Vec::new());

/// Serialize cache misses without holding a spin lock across IPC waits. This
/// avoids duplicate object-store reads when several boot tasks request the
/// same image concurrently while still allowing the active reader and the
/// objstore service to run on any LP.
static STORE_READER_ACTIVE: AtomicBool = AtomicBool::new(false);

struct StoreReaderGuard;

impl Drop for StoreReaderGuard {
    fn drop(&mut self) {
        STORE_READER_ACTIVE.store(false, Ordering::Release);
    }
}

/// Kernel-owned IPC capabilities acquired while resolving a store object.
/// Store loading has several fallible reply steps; tying cleanup to scope
/// prevents an error or malformed reply from leaking authority into ASID 0.
struct KernelIpcCap(u64);

impl KernelIpcCap {
    fn raw(&self) -> u64 {
        self.0
    }
}

impl Drop for KernelIpcCap {
    fn drop(&mut self) {
        let _ = ipc::close_cap(KERNEL_ASID, self.0);
    }
}

struct KernelMemoryCap(crate::memory::object::MemoryObjectCap);

impl KernelMemoryCap {
    fn raw(&self) -> crate::memory::object::MemoryObjectCap {
        self.0
    }
}

impl Drop for KernelMemoryCap {
    fn drop(&mut self) {
        let _ = crate::memory::object::close_cap(KERNEL_ASID, self.0);
    }
}

fn cached_elf(name: &[u8]) -> Option<&'static [u8]> {
    STORE_ELFS
        .lock()
        .iter()
        .find_map(|(cached_name, image)| (*cached_name == name).then_some(*image))
}

/// Resolve a service ELF by its artifact name.
///
/// The bootstrap set is embedded; anything else is read from the object
/// store (blocking until the store service is up) and cached. The returned
/// bytes are the signed ELF that the loader verifies before mapping.
pub fn service_elf(name: &'static [u8]) -> Option<&'static [u8]> {
    for (candidate, image) in BOOTSTRAP_ELFS {
        if *candidate == name {
            // Bootstrap images are immutable kernel input. Keep signature
            // enforcement at the loader boundary, which verifies every
            // domain immediately before mapping it. Re-verifying here made
            // lookup spuriously fallible and duplicated the security check.
            return Some(image);
        }
    }
    if let Some(image) = cached_elf(name) {
        return Some(image);
    }

    // Acquire a yield-friendly single-flight gate. Recheck the cache after
    // every wait because the previous reader may have loaded this same image.
    let _reader = loop {
        if STORE_READER_ACTIVE
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            break StoreReaderGuard;
        }
        crate::cpu::scheduler::yield_lp();
        if let Some(image) = cached_elf(name) {
            return Some(image);
        }
    };
    if let Some(image) = cached_elf(name) {
        return Some(image);
    }

    // The store read blocks until the objstore service is up; the cache and
    // its spin lock remain free while IPC and block I/O make progress.
    let image = read_from_store(name)?;
    if !verified_for_name(name, &image) {
        crate::logln!("[store] refusing ELF whose signed identity is not {:?}", name);
        return None;
    }
    let image: &'static [u8] = alloc::vec::Vec::leak(image);
    STORE_ELFS.lock().push((name, image));
    Some(image)
}

fn verified_for_name(name: &[u8], image: &[u8]) -> bool {
    charlotte_launch::signature_note::verify_elf_for_name(
        image,
        &charlotte_launch::CLUSTER_PUBLIC_KEY,
        name,
    ) == charlotte_launch::signature_note::VerifyOutcome::Valid
}

/// Outcome of an idempotent embedded-service seed pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeedReport {
    /// Valid signed artifacts that were already present and left untouched.
    pub retained: usize,
    /// Missing or invalid artifacts restored from the trusted boot bundle.
    pub written: usize,
}

/// Populate a newly formatted object store from the signed bootstrap bundle.
///
/// Existing artifacts are retained when they carry a valid cluster signature
/// for their logical name. This preserves a newer artifact installed through
/// `clusterctl`; only missing, empty, corrupt, or incorrectly signed objects
/// are repaired from the immutable boot bundle. The pass is safe to repeat on
/// every boot and flushes once after all required writes.
pub fn seed_embedded_services() -> Result<SeedReport, &'static str> {
    let obj_conn = connect_objstore().ok_or("object-store connection unavailable")?;
    let mut report = SeedReport {
        retained: 0,
        written: 0,
    };

    for (name, image) in BOOTSTRAP_ELFS {
        if read_from_store_connection(obj_conn.raw(), name)
            .is_some_and(|stored| verified_for_name(name, &stored))
        {
            report.retained += 1;
            continue;
        }
        if !verified_for_name(name, image) {
            return Err("embedded service signature is invalid");
        }
        write_store_artifact(obj_conn.raw(), name, image)?;
        report.written += 1;
    }

    if report.written != 0 {
        let result = scalar_result(obj_conn.raw(), OBJ_OP_FLUSH, 0)
            .ok_or("object-store flush call failed")?;
        if result != OBJ_ERR_OK {
            return Err("object-store flush rejected");
        }
    }
    Ok(report)
}

fn connect_objstore() -> Option<KernelIpcCap> {
    let name_service = crate::service::supervisor::node_name_service();
    let kernel_ns = KernelIpcCap(
        ipc::connection_delegate(
            name_service.domain.asid,
            name_service.endpoint_cap,
            KERNEL_ASID,
            crate::ipc::ConnectionRights::CALL,
        )
        .ok()?,
    );
    let lookup = KernelIpcCap(
        ipc::scalar_call(KERNEL_ASID, kernel_ns.raw(), 2, charlotte_launch::OBJSTORE_NAME).ok()?,
    );
    ipc::wait_reply(KERNEL_ASID, lookup.raw()).ok()?;
    let reply = ipc::poll_reply(KERNEL_ASID, lookup.raw()).ok().flatten()?;
    let _unexpected_memory = reply.memory.map(KernelMemoryCap);
    reply.cap.map(KernelIpcCap)
}

fn scalar_result(connection: u64, opcode: u32, arg0: u64) -> Option<i64> {
    let call = KernelIpcCap(ipc::scalar_call(KERNEL_ASID, connection, opcode, arg0).ok()?);
    ipc::wait_reply(KERNEL_ASID, call.raw()).ok()?;
    let reply = ipc::poll_reply(KERNEL_ASID, call.raw()).ok().flatten()?;
    let _unexpected_connection = reply.cap.map(KernelIpcCap);
    let _unexpected_memory = reply.memory.map(KernelMemoryCap);
    Some(reply.result)
}

fn move_bytes_result(connection: u64, opcode: u32, arg0: u64, bytes: &[u8]) -> Option<i64> {
    let memory = crate::memory::object::allocate_with_bytes(KERNEL_ASID, bytes).ok()?;
    let call =
        match ipc::scalar_call_with_memory_move(KERNEL_ASID, connection, opcode, arg0, memory) {
            Ok(call) => KernelIpcCap(call),
            Err(_) => {
                let _ = crate::memory::object::close_cap(KERNEL_ASID, memory);
                return None;
            }
        };
    ipc::wait_reply(KERNEL_ASID, call.raw()).ok()?;
    let reply = ipc::poll_reply(KERNEL_ASID, call.raw()).ok().flatten()?;
    let _unexpected_connection = reply.cap.map(KernelIpcCap);
    let _unexpected_memory = reply.memory.map(KernelMemoryCap);
    Some(reply.result)
}

fn write_store_artifact(connection: u64, name: &[u8], image: &[u8]) -> Result<(), &'static str> {
    let object_id = charlotte_launch::artifact_object_id(name);
    let create = scalar_result(connection, OBJ_OP_CREATE_AT, object_id)
        .ok_or("object-store create call failed")?;
    if create != OBJ_ERR_OK && create != OBJ_ERR_EXISTS {
        return Err("object-store create rejected");
    }
    let size = (image.len() as u64).to_le_bytes();
    if move_bytes_result(connection, OBJ_OP_SET_SIZE, object_id, &size) != Some(OBJ_ERR_OK) {
        return Err("object-store resize rejected");
    }
    if move_bytes_result(connection, OBJ_OP_WRITE, object_id, image) != Some(OBJ_ERR_OK) {
        return Err("object-store write rejected");
    }
    Ok(())
}

/// Read one object (the signed ELF) from the object store by its derived
/// cluster-wide artifact id. Retries until the objstore service is up.
fn read_from_store(name: &'static [u8]) -> Option<alloc::vec::Vec<u8>> {
    let obj_conn = connect_objstore()?;
    read_from_store_connection(obj_conn.raw(), name)
}

fn read_from_store_connection(connection: u64, name: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    let object_id = charlotte_launch::artifact_object_id(name);
    let read =
        KernelIpcCap(ipc::scalar_call(KERNEL_ASID, connection, OBJ_OP_READ, object_id).ok()?);
    ipc::wait_reply(KERNEL_ASID, read.raw()).ok()?;
    let reply = ipc::poll_reply(KERNEL_ASID, read.raw()).ok().flatten()?;
    let _unexpected_connection = reply.cap.map(KernelIpcCap);
    let memory = reply.memory.map(KernelMemoryCap);
    if reply.result < 0 {
        return None;
    }
    let memory = memory?;
    let size = reply.result as usize;
    if size == 0 || size > MAX_SERVICE_ELF_SIZE {
        crate::logln!("[store] refusing {size}-byte service ELF for {:?}", name);
        return None;
    }
    // Snapshot through the memory object's direct-map frames. Reusing one
    // temporary virtual address here used to make correctness depend on TLB
    // state when successive cache misses ran on different LPs.
    crate::memory::object::snapshot_bytes(KERNEL_ASID, memory.raw(), size).ok()
}
