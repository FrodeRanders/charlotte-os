//! Self-test: Phase 1 userspace NVMe block device driver + object store.
#![cfg(target_arch = "aarch64")]

use crate::{
    ipc::ConnectionRights,
    logln,
    service::supervisor::{
        self,
        DriverGrant,
        NameServiceHandle,
    },
};

#[cfg(target_arch = "aarch64")]
const NVME_ELF: &[u8] = include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/nvme.elf"));
#[cfg(target_arch = "aarch64")]
const OBJSTORE_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/objstore.elf"));
const OBJSTORE_CLIENT_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/objstore_client.elf"));
const ECHO_ELF: &[u8] = include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/echo.elf"));
const ELF_SIZE_KEY: u64 = charlotte_launch::manifest_key(b"elf_size");
const OBJSTORE_TEST_DONE_NAME: u64 = u64::from_le_bytes(*b"objdone\0");
#[cfg(target_arch = "aarch64")]
const NVME_CLIENT_ELF: &[u8] =
    include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/nvme_client.elf"));

#[cfg(target_arch = "aarch64")]
static TEST_STATE: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<NameServiceHandle>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));

pub fn test_el0_nvme() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 userspace NVMe block device driver and object store...");
        let name_service = supervisor::node_name_service();
        *TEST_STATE.lock() = Some(name_service);
        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Nvme,
            verify_el0_nvme,
        );
        logln!("[nvme] verifier deferred");
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 NVMe + objstore test (AArch64 only).");
    }
}

fn wait_for_nvme() -> (usize, u32, u32, Option<u64>) {
    #[cfg(not(feature = "hvf_compat"))]
    {
        // Normal boot publishes the immutable topology before the scheduler
        // starts, so absence here is a real discovery failure rather than a
        // reason to guess platform addresses.
        let topo = &crate::device_management::topology::DEVICE_TOPOLOGY;
        let (bar0, irq, requester_id, msi_address) =
            crate::device_management::drivers::busses::pci_express::topology::lookup_first_nvme(
                &topo.pcie,
            )
            .expect("[nvme] no NVMe controller in the published PCI topology");
        logln!("[nvme] PCI topology: BAR0={:#x} intid={}", bar0, irq);
        (bar0 as usize, irq, requester_id, msi_address)
    }
    #[cfg(feature = "hvf_compat")]
    {
        // HVF cannot safely map the QEMU ECAM window, so this development mode
        // retains the known fixed test-device placement.
        let bar0: usize = 0x1000_0000;
        let intid: u32 = 44;
        logln!("[nvme] HVF fallback: BAR0={:#x} intid={}", bar0, intid);
        (bar0, intid, 0x10, None)
    }
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_nvme() {
    let ns = TEST_STATE.lock().as_ref().copied().expect("[nvme] test state missing");
    logln!("[nvme] verifier running, discovering NVMe...");

    // MSI-X setup relies on the kernel's GICv2m MSI allocator, which only
    // works where the MADT publishes a GICv2m frame at the address the kernel
    // uses (QEMU virt). On sbsa-ref MSI goes through the GIC ITS (not yet
    // supported); the gicv2m probe would fault. Report unsupported before
    // touching the frame.
    if !crate::cpu::isa::interrupts::gic::msi_available() {
        logln!("[nvme] SKIP: no supported GICv2m MSI frame in the ACPI MADT; NVMe test not run.");
        crate::self_test::results::fail(crate::self_test::results::TestId::Nvme);
        return;
    }

    let (bar0, intid, requester_id, msi_address) = wait_for_nvme();

    // Protected DMA requires an SMMU to exist and to map this requester to a
    // stream. On platforms without one (e.g. HVF, where the ECAM window and
    // SMMU are absent), the driver cannot be granted a DMA domain. Report the
    // test as unsupported rather than panicking inside the driver spawn, which
    // would abort the verifier before it reports and leave the boot waiting on
    // a result that never arrives.
    if crate::device::smmu::stream_id(requester_id).is_err() {
        logln!("[nvme] SKIP: protected DMA unavailable (no SMMU); NVMe test not run.");
        crate::self_test::results::fail(crate::self_test::results::TestId::Nvme);
        return;
    }

    // --- Spawn NVMe driver ---
    let driver = supervisor::spawn_driver_with_name_service(
        NVME_ELF,
        &ns,
        ConnectionRights::CALL,
        DriverGrant {
            mmio_phys_base: bar0,
            mmio_pages: 2,
            intid,
            dma_requester_id: Some(requester_id),
            dma_msi_address: msi_address,
        },
    );
    let driver_cfg: *const u32 = {
        let base: *mut u8 = driver.status_frame.into();
        base as *const u32
    };
    logln!("[nvme] driver spawned (asid={})", driver.asid);

    // Start the client immediately. Its single deferred lookup must be woken
    // by the driver's later registration; verifier ordering must not mask a
    // broken name-service synchronization path.
    let client = supervisor::spawn_with_name_service(NVME_CLIENT_ELF, &ns, ConnectionRights::CALL);
    let client_cfg: *const u32 = {
        let base: *mut u8 = client.status_frame.into();
        base as *const u32
    };
    let deadline = crate::self_test::results::Deadline::after_millis(30_000);
    while unsafe { core::ptr::read_volatile(client_cfg) } != 0x900d {
        let state = unsafe { core::ptr::read_volatile(client_cfg) };
        assert!(state < 0xdea0, "[nvme] I/O verifier failed: {:#x}", state);
        deadline.assert_pending("EL0 nvme I/O client sentinel");
        // Yield rather than blocking on a timer: the interrupt-driven driver
        // shares scheduler capacity, and a timer wait that is not delivered
        // would leave the deadline unchecked (observed as a silent hang).
        crate::cpu::scheduler::yield_lp();
    }
    let sentinel_ptr: *const u32 = unsafe { driver_cfg.add(5) };
    assert_eq!(
        unsafe { core::ptr::read_volatile(sentinel_ptr) },
        0x900d,
        "[nvme] client completed before driver registration"
    );
    logln!("[nvme] 12 KiB PRP-list write/flush/read round trip verified");
    let irq_count = unsafe { core::ptr::read_volatile(driver_cfg.add(20)) };
    assert!(irq_count > 0, "[nvme] MSI-X completion interrupt was not delivered");
    logln!("[nvme] MSI-X delivered {} completion interrupt(s)", irq_count);
    assert_eq!(
        crate::device::smmu::fault_count(),
        0,
        "[nvme] valid DMA traffic caused an SMMU fault"
    );
    logln!("[nvme] SMMU domain completed the transfer without translation faults");

    // --- Spawn object store via name service (deferred lookup) ---
    let objstore = supervisor::spawn_with_name_service(OBJSTORE_ELF, &ns, ConnectionRights::CALL);
    let obj_cfg: *const u32 = {
        let base: *mut u8 = objstore.status_frame.into();
        base as *const u32
    };
    logln!("[nvme] objstore spawned (asid={})", objstore.asid);

    // Wait for object store to register
    let deadline = crate::self_test::results::Deadline::after_millis(30_000);
    while unsafe { core::ptr::read_volatile(obj_cfg.add(1)) } != 0x900d {
        deadline.assert_pending("EL0 nvme objstore registration");
        crate::cpu::scheduler::yield_lp();
    }

    logln!("[nvme] NVMe driver and object store both initialised and registered.");
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
        OBJSTORE_CLIENT_ELF,
        &ns,
        ECHO_ELF,
        ELF_SIZE_KEY,
    );
    let object_cfg: *const u32 = {
        let base: *mut u8 = object_client.status_frame.into();
        base as *const u32
    };
    let deadline = crate::self_test::results::Deadline::after_millis(60_000);
    let completion = loop {
        match crate::ipc::poll_reply(crate::memory::KERNEL_ASID, completion_lookup) {
            Ok(Some(reply)) => break reply,
            Ok(None) => {}
            Err(_) => panic!("[nvme] object-client completion reply error"),
        }
        deadline.assert_pending("EL0 nvme object-client completion");
        crate::cpu::scheduler::yield_lp();
    };
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
    assert_eq!(unsafe { core::ptr::read_volatile(object_cfg.add(2)) }, ECHO_ELF.len() as u32);
    logln!("[nvme] 2 MiB + 4 KiB persistent object round trip verified.");
    // The completion service reports that the client finished its work, not
    // that the scheduler has reaped its initial thread. Wait for that separate
    // lifecycle event before releasing the address space.
    supervisor::wait_domain_exit(&object_client, 30_000);
    supervisor::teardown_domain(object_client);
    crate::self_test::el0_service::verify_persistent_upgrade(&ns);
    crate::self_test::el0_raft::test_persistent_raft(&ns);
    logln!("[nvme] SUCCESS: storage stack and persistent Raft recovery verified.");
    crate::self_test::results::pass(crate::self_test::results::TestId::Nvme);
}
