//! The service-ELF registry: where the kernel gets the userspace binaries
//! it loads.
//!
//! Only a small bootstrap set is embedded in the kernel image — the name
//! service, the disk stack (`ns`, `nvme`, `objstore`), the `uart` service
//! the earliest synchronous test needs, and the system observer the kernel
//! starts during boot — everything else lives in the object store on the
//! boot disk. An initial NVMe image produced by `scripts/make-nvme-image.py`
//! stages the signed ELFs, and the registry reads them from the store the
//! first time a test asks for one, then caches them for the rest of the
//! boot. The EL0 loader enforces a valid cluster signature on whatever
//! source the bytes come from, so the store-sourced path is exactly as
//! trusted as the embedded one. The `hvf_compat` development configuration
//! has no SMMU and therefore cannot start the protected-DMA disk stack; it
//! embeds the additional non-storage services used by its reduced boot suite
//! so service lookup never waits for an object store that cannot exist.

use core::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::{
    ipc,
    memory::KERNEL_ASID,
};

const OBJ_OP_READ: u32 = 4;

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

// x86_64 embeds the device-independent bootstrap services plus the NVMe
// storage stack (now reachable via direct DMA). The remaining services
// (servicemgr, raft, networking) are still AArch64-only.
#[cfg(target_arch = "x86_64")]
const BOOTSTRAP_ELFS: &[(&[u8], &[u8])] = &[
    bootstrap_elf!(b"ns", "ns"),
    bootstrap_elf!(b"observe", "observe"),
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
    bootstrap_elf!(b"nclient", "nclient"),
    bootstrap_elf!(b"disco", "disco"),
    bootstrap_elf!(b"frouter", "frouter"),
    bootstrap_elf!(b"dns", "dns"),
    bootstrap_elf!(b"agent", "agent"),
    bootstrap_elf!(b"greet", "greet"),
    bootstrap_elf!(b"relmsg", "relmsg"),
    bootstrap_elf!(b"tcpip", "tcpip"),
    bootstrap_elf!(b"tcpclient", "tcpclient"),
    bootstrap_elf!(b"httpd", "httpd"),
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

/// Read one object (the signed ELF) from the object store by its derived
/// cluster-wide artifact id. Retries until the objstore service is up.
fn read_from_store(name: &'static [u8]) -> Option<alloc::vec::Vec<u8>> {
    let name_service = crate::service::supervisor::node_name_service();
    let kernel_ns = match ipc::connection_delegate(
        name_service.domain.asid,
        name_service.endpoint_cap,
        KERNEL_ASID,
        crate::ipc::ConnectionRights::CALL,
    ) {
        Ok(conn) => KernelIpcCap(conn),
        Err(error) => {
            crate::logln!("[store] delegate failed: {error:?}");
            return None;
        }
    };

    // The name service defers this one call until objstore registers. Boot's
    // store-dependent phase runs in a scheduler-owned kernel thread, so use
    // the ordinary waitable IPC path rather than issuing and leaking repeated
    // polling calls under load.
    let lookup = KernelIpcCap(
        ipc::scalar_call(KERNEL_ASID, kernel_ns.raw(), 2, charlotte_launch::OBJSTORE_NAME).ok()?,
    );
    ipc::wait_reply(KERNEL_ASID, lookup.raw()).ok()?;
    let lookup_reply = ipc::poll_reply(KERNEL_ASID, lookup.raw()).ok().flatten()?;
    let _unexpected_memory = lookup_reply.memory.map(KernelMemoryCap);
    let obj_conn = KernelIpcCap(lookup_reply.cap?);

    let object_id = charlotte_launch::artifact_object_id(name);
    let read =
        KernelIpcCap(ipc::scalar_call(KERNEL_ASID, obj_conn.raw(), OBJ_OP_READ, object_id).ok()?);
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
