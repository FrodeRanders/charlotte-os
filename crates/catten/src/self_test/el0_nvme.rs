//! Self-test: the userspace NVMe block device driver + object store.
//!
//! The deepest end-to-end storage test. A deferred verifier runs after the
//! scheduler is active and drives the whole stack through real protection
//! domains: PCI topology discovery → a userspace driver granted the BAR0
//! MMIO window, its completion IRQ and a protected-DMA (SMMU) domain → the
//! node name service → an object store on top of the block device → a
//! persistent object client → and finally durable Raft recovery.
//!
//! Phases (each must succeed for the verifier to call
//! [`crate::self_test::results::pass`] for `TestId::Nvme`):
//!
//! 1. **Discovery / grants** — the verifier looks up the first NVMe controller in the published PCI
//!    topology and spawns `nvme.elf` with `DriverGrant` capabilities (MMIO base, interrupt, SMMU
//!    requester id, MSI-X address). Platforms without an MSI-capable GIC or an SMMU stream report
//!    the test unsupported (`results::fail`) instead of faulting. Outcome: the driver domain
//!    initializes the controller from EL0 using only its capabilities.
//! 2. **Block I/O round trip** — a client sends a 12 KiB write, flush and read through the driver;
//!    the driver submits NVMe commands, and the device's MSI-X completion is delivered as an
//!    interrupt to the driver's completion queue. Outcome: the round trip verifies, the MSI-X
//!    completion counter is nonzero, and the SMMU reports no translation faults.
//! 3. **Object store** — `objstore.elf` connects to the block device and formats/mounts; an object
//!    client then persists a **2 MiB + 4 KiB** object (exercising PRP-list DMA at scale) and
//!    verifies it reads back with the exact size and payload length. Outcome: the persistent object
//!    round trip verifies.
//! 4. **Durability** — [`super::el0_service::verify_persistent_upgrade`] and
//!    [`super::el0_raft::test_persistent_raft`] restart domains that depend on the object store.
//!    Outcome: term/vote state survives a process restart on the NVMe-backed store.
//!
//! Why: this is the proof that the whole driver model — capability grants,
//! userspace MMIO, SMMU-protected DMA, and real GIC ITS/MSI interrupt delivery
//! (on sbsa-ref) — works together, and that the object store is actually
//! durable. It is also the test that forced the GIC security (`DS=1`), LPI
//! delivery, SPI priority and EL0-heap fixes documented in
//! `docs/platforms/sbsa-ref.md`.
//!
//! Expected outcome: the verifier logs
//! `SUCCESS: storage stack and persistent Raft recovery verified` and the
//! authoritative coordinator reports `TestId::Nvme` passed.

use crate::{
    ipc::ConnectionRights,
    logln,
    service::supervisor::{
        self,
        DriverGrant,
        NameServiceHandle,
    },
};

const ELF_SIZE_KEY: u64 = charlotte_launch::manifest_key(b"elf_size");
const OBJSTORE_TEST_DONE_NAME: u64 = u64::from_le_bytes(*b"objdone\0");
/// `objstore::NAME` from the object-store protocol (`name(b"obj")`), used for
/// the deferred registration lookup below.
const OBJSTORE_NAME: u64 = u64::from_le_bytes(*b"obj\0\0\0\0\0");
/// `ns::OP_LOOKUP` (short-name lookup; the name service defers the reply
/// until the service registers).
const NS_OP_LOOKUP: u32 = 2;

static TEST_STATE: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<NameServiceHandle>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

pub fn test_el0_nvme() {
    logln!("Testing EL0 userspace NVMe block device driver and object store...");
    let name_service = supervisor::node_name_service();
    *TEST_STATE.lock() = Some(name_service);
    let _vtid = crate::self_test::results::spawn_verifier(
        crate::self_test::results::TestId::Nvme,
        verify_el0_nvme,
    );
    logln!("[nvme] verifier deferred");
}

struct BlockDevice {
    driver: &'static [u8],
    mmio_base: usize,
    mmio_pages: usize,
    intid: u32,
    requester_id: u32,
    msi_address: Option<u64>,
}

fn wait_for_block_device() -> BlockDevice {
    #[cfg(not(feature = "hvf_compat"))]
    {
        // Normal boot publishes the immutable topology before the scheduler
        // starts, so absence here is a real discovery failure rather than a
        // reason to guess platform addresses.
        let topo = &crate::device_management::topology::DEVICE_TOPOLOGY;
        if let Some((bar0, irq, requester_id, msi_address)) =
            crate::device_management::drivers::busses::pci_express::topology::lookup_first_nvme(
                &topo.pcie,
            )
        {
            logln!("[nvme] PCI topology: BAR0={:#x} intid={}", bar0, irq);
            return BlockDevice {
                driver: b"nvme",
                mmio_base: bar0 as usize,
                mmio_pages: 2,
                intid: irq,
                requester_id,
                msi_address,
            };
        }
        if let Some((abar, irq, requester_id, msi_address)) =
            crate::device_management::drivers::busses::pci_express::topology::lookup_first_virtio_blk(
                &topo.pcie,
            )
        {
            logln!("[virtio-blk] PCI topology: BAR4={:#x} intid={}", abar, irq);
            return BlockDevice {
                driver: b"virtio_blk",
                mmio_base: abar as usize,
                mmio_pages: 4,
                intid: irq,
                requester_id,
                msi_address,
            };
        }
        if let Some((abar, irq, requester_id, msi_address)) =
            crate::device_management::drivers::busses::pci_express::topology::lookup_first_ahci(
                &topo.pcie,
            )
        {
            logln!("[ahci] PCI topology: ABAR={:#x} intid={}", abar, irq);
            return BlockDevice {
                driver: b"ahci",
                mmio_base: abar as usize,
                mmio_pages: 2,
                intid: irq,
                requester_id,
                msi_address,
            };
        }
        panic!("[block] no NVMe, AHCI, or virtio-blk controller in the published PCI topology");
    }
    #[cfg(feature = "hvf_compat")]
    {
        // HVF cannot safely map the QEMU ECAM window, so this development mode
        // retains the known fixed test-device placement.
        let bar0: usize = 0x1000_0000;
        let intid: u32 = 44;
        logln!("[nvme] HVF fallback: BAR0={:#x} intid={}", bar0, intid);
        BlockDevice {
            driver: b"nvme",
            mmio_base: bar0,
            mmio_pages: 2,
            intid,
            requester_id: 0x10,
            msi_address: None,
        }
    }
}

extern "C" fn verify_el0_nvme() {
    let ns = TEST_STATE.lock().as_ref().copied().expect("[nvme] test state missing");
    logln!("[nvme] verifier running, discovering NVMe...");

    // MSI(-X) setup relies on the kernel's MSI allocator, which requires an
    // available mechanism (the GICv2m/ITS on AArch64, the LAPIC on x86_64).
    if !crate::device::msi_available() {
        logln!("[nvme] SKIP: no supported MSI mechanism; NVMe test not run.");
        crate::self_test::results::fail(crate::self_test::results::TestId::Nvme);
        return;
    }

    let device = wait_for_block_device();

    // Protected DMA requires an IOMMU to exist and to map this requester to a
    // stream. On platforms without one, the driver cannot be granted a DMA
    // domain. Report the test as unsupported rather than panicking inside the
    // driver spawn, which would abort the verifier before it reports.
    if crate::device::stream_id(device.requester_id).is_err() {
        logln!("[nvme] SKIP: protected DMA unavailable; NVMe test not run.");
        crate::self_test::results::fail(crate::self_test::results::TestId::Nvme);
        return;
    }

    // --- Spawn the block driver (NVMe or AHCI) ---
    let driver = supervisor::spawn_driver_with_name_service(
        crate::service::store::service_elf(device.driver).expect("[el0_nvme] block driver elf"),
        &ns,
        ConnectionRights::CALL,
        DriverGrant {
            mmio_phys_base: device.mmio_base,
            mmio_pages: device.mmio_pages,
            intid: device.intid,
            dma_requester_id: Some(device.requester_id),
            dma_msi_address: device.msi_address,
        },
    );
    let driver_cfg: *const u32 = {
        let base: *mut u8 = driver.status_frame.into();
        base as *const u32
    };
    logln!("[nvme] driver spawned (asid={})", driver.asid);

    // The object store is embedded and must be up before any store-sourced
    // service is fetched (the client and object-client ELFs now come from
    // the store).
    // --- Spawn object store via name service (deferred lookup) ---
    let objstore = supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"objstore").expect("[el0_nvme] objstore.elf"),
        &ns,
        ConnectionRights::CALL,
    );
    logln!("[nvme] objstore spawned (asid={})", objstore.asid);
    logln!(
        "[nvme] driver stage={} raw_dw3={:#x} irq_count={}",
        unsafe { core::ptr::read_volatile(driver_cfg.add(1)) },
        unsafe { core::ptr::read_volatile(driver_cfg.add(17)) },
        unsafe { core::ptr::read_volatile(driver_cfg.add(20)) }
    );

    // The object store registers with the name service under its interface
    // name once its endpoint is up. A deferred lookup resolves exactly when
    // that registration lands, so block on the reply rather than polling the
    // shared status sentinel: the name service is the event source.
    let obj_ns = crate::ipc::connection_delegate(
        ns.domain.asid,
        ns.endpoint_cap,
        crate::memory::KERNEL_ASID,
        ConnectionRights::CALL,
    )
    .expect("[nvme] objstore registration name-service connection");
    let obj_lookup =
        crate::ipc::scalar_call(crate::memory::KERNEL_ASID, obj_ns, NS_OP_LOOKUP, OBJSTORE_NAME)
            .expect("[nvme] objstore registration lookup");
    let registered = crate::ipc::wait_reply_timeout(crate::memory::KERNEL_ASID, obj_lookup, 30_000)
        .expect("[nvme] objstore registration reply error");
    assert!(registered, "[nvme] objstore registration deadline expired");
    if let Ok(Some(reply)) = crate::ipc::poll_reply(crate::memory::KERNEL_ASID, obj_lookup)
        && let Some(connection) = reply.cap
    {
        let _ = crate::ipc::close_cap(crate::memory::KERNEL_ASID, connection);
    }
    crate::ipc::close_cap(crate::memory::KERNEL_ASID, obj_lookup)
        .expect("[nvme] objstore lookup close");
    crate::ipc::close_cap(crate::memory::KERNEL_ASID, obj_ns)
        .expect("[nvme] objstore name-service connection close");

    logln!("[nvme] NVMe driver and object store both initialised and registered.");

    // Start the client immediately. Its single deferred lookup must be woken
    // by the driver's later registration; verifier ordering must not mask a
    // broken name-service synchronization path.
    let client = supervisor::spawn_with_name_service(
        crate::service::store::service_elf(b"nvme_client").expect("[el0_nvme] nvme_client.elf"),
        &ns,
        ConnectionRights::CALL,
    );
    let client_cfg: *const u32 = {
        let base: *mut u8 = client.status_frame.into();
        base as *const u32
    };
    // The client writes its completion sentinel and then exits; its thread
    // exit is the completion event, so block on that instead of polling the
    // shared status frame.
    let client_exit = crate::completion::observe_thread_exit_with_generation(
        client.asid,
        client.tid,
        Some(client.generation),
    )
    .expect("[nvme] client exit observer");
    let exited = crate::completion::wait_timeout(client.asid, client_exit, 30_000)
        .expect("[nvme] client exit wait error");
    assert!(exited, "[nvme] I/O client did not exit within deadline");
    let state = unsafe { core::ptr::read_volatile(client_cfg) };
    assert_eq!(state, 0x900d, "[nvme] I/O verifier failed: {:#x}", state);
    let sentinel_ptr: *const u32 = unsafe { driver_cfg.add(5) };
    assert_eq!(
        unsafe { core::ptr::read_volatile(sentinel_ptr) },
        0x900d,
        "[nvme] client completed before driver registration"
    );
    logln!("[nvme] 12 KiB PRP-list write/flush/read round trip verified");
    let irq_count = unsafe { core::ptr::read_volatile(driver_cfg.add(20)) };
    // The NVMe driver completes through MSI-X; the AHCI driver polls its
    // command slot instead, so an interrupt counter is only meaningful for
    // NVMe.
    if device.driver == b"nvme" {
        assert!(irq_count > 0, "[nvme] MSI-X completion interrupt was not delivered");
        logln!("[nvme] MSI-X delivered {} completion interrupt(s)", irq_count);
    } else {
        logln!("[ahci] polled completion (no interrupt delivery)");
    }
    assert_eq!(crate::device::fault_count(), 0, "[nvme] valid DMA traffic caused a DMA fault");
    logln!("[nvme] DMA domain completed the transfer without translation faults");

    let completion_ns = crate::ipc::connection_delegate(
        ns.domain.asid,
        ns.endpoint_cap,
        crate::memory::KERNEL_ASID,
        ConnectionRights::CALL,
    )
    .expect("[nvme] object-client completion name-service connection");
    let completion_lookup = crate::ipc::scalar_call(
        crate::memory::KERNEL_ASID,
        completion_ns,
        2,
        OBJSTORE_TEST_DONE_NAME,
    )
    .expect("[nvme] object-client completion lookup");
    let object_client = supervisor::spawn_with_name_service_and_data(
        crate::service::store::service_elf(b"objstore_client")
            .expect("[el0_nvme] objstore_client.elf"),
        &ns,
        crate::service::store::service_elf(b"echo").expect("[el0_nvme] echo.elf"),
        ELF_SIZE_KEY,
    );
    let object_cfg: *const u32 = {
        let base: *mut u8 = object_client.status_frame.into();
        base as *const u32
    };
    let ready =
        crate::ipc::wait_reply_timeout(crate::memory::KERNEL_ASID, completion_lookup, 60_000)
            .expect("[nvme] object-client completion reply error");
    assert!(ready, "[nvme] object-client completion deadline expired");
    let completion = crate::ipc::poll_reply(crate::memory::KERNEL_ASID, completion_lookup)
        .expect("[nvme] object-client completion poll error")
        .expect("[nvme] object-client completion reply missing");
    assert!(completion.result >= 1, "[nvme] invalid object-client completion generation");
    if let Some(connection) = completion.cap {
        crate::ipc::close_cap(crate::memory::KERNEL_ASID, connection)
            .expect("[nvme] close object-client completion connection");
    }
    crate::ipc::close_cap(crate::memory::KERNEL_ASID, completion_lookup)
        .expect("[nvme] close object-client completion lookup");
    crate::ipc::close_cap(crate::memory::KERNEL_ASID, completion_ns)
        .expect("[nvme] close completion name-service connection");
    let state = unsafe { core::ptr::read_volatile(object_cfg) };
    assert_eq!(
        state,
        0x900d,
        "[nvme] large-object verifier failed: {:#x}, detail={:#x}",
        state,
        unsafe { core::ptr::read_volatile(object_cfg.add(1)) }
    );
    assert_eq!(unsafe { core::ptr::read_volatile(object_cfg.add(1)) }, 2 * 1024 * 1024 + 4096);
    assert_eq!(
        unsafe { core::ptr::read_volatile(object_cfg.add(2)) },
        crate::service::store::service_elf(b"echo").expect("[el0_nvme] echo.elf").len() as u32
    );
    logln!("[nvme] 2 MiB + 4 KiB persistent object round trip verified.");
    // The completion service reports that the client finished its work, not
    // that the scheduler has reaped its initial thread. Wait for that separate
    // lifecycle event before releasing the address space.
    supervisor::wait_domain_exit(&object_client, 30_000);
    supervisor::teardown_domain(object_client);
    // The persistent upgrade phase reloads the echo service ELF through the
    // service manager; the persistent Raft recovery phase uses the raft image.
    crate::self_test::el0_service::verify_persistent_upgrade(&ns);
    #[cfg(not(feature = "live_upgrade_test"))]
    crate::self_test::el0_raft::test_persistent_raft(&ns);
    logln!("[nvme] SUCCESS: storage stack verified.");
    crate::self_test::results::pass(crate::self_test::results::TestId::Nvme);
}
