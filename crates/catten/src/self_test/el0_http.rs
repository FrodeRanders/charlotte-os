//! Self-test: hardcoded HTTP keyhole serving observable node state.
//!
//! Spawns the `tcpip` service configured for the SLIRP user network
//! (10.0.2.15, gateway 10.0.2.2) and the `httpd` service, then waits for the
//! httpd to reach its listening stage. The actual request/response round trip
//! is performed by the host through `--http-test`: the run script curls the
//! hostfwd port after the self-test completes and verifies the JSON payload.
//!
//! Requires the net driver and the frouter (both spawned by the net self-test
//! under `virtio_net_test` with the frouter enabled).
#![cfg(target_arch = "aarch64")]

mod inner {
    use crate::{
        ipc::ConnectionRights,
        logln,
        service::{
            bootstrap::{
                ManifestEntry,
                ManifestValue,
            },
            supervisor::{
                self,
                NameServiceHandle,
                ServiceDomain,
            },
        },
    };

    const IP_KEY: u64 = charlotte_launch::manifest_key(b"ip");
    const GATEWAY_KEY: u64 = charlotte_launch::manifest_key(b"gateway");

    const TCPIP_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/tcpip.elf"));
    const HTTPD_ELF: &[u8] =
        include_bytes!(concat!(env!("CATTEN_AARCH64_SERVICE_BUNDLE"), "/httpd.elf"));

    static mut HTTP_NS: Option<NameServiceHandle> = None;

    fn spawn_binary(
        image: &[u8],
        ns: &NameServiceHandle,
        manifest: &[ManifestEntry<'_>],
    ) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(image);
        let conn = crate::ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            addr.asid,
            ConnectionRights::CALL,
        )
        .expect("http spawn conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        crate::service::bootstrap::write_manifest(addr.config_frame, manifest);
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr) };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        let generation = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .expect("http thread missing after spawn")
            .generation;
        ServiceDomain {
            asid: addr.asid,
            tid,
            generation,
            config_frame: addr.config_frame,
            status_frame: addr.status_frame,
        }
    }

    pub fn test_el0_http() {
        logln!("Testing EL0 HTTP keyhole (hardcoded state server)...");

        let name_service = supervisor::node_name_service();
        let ns_asid = name_service.domain.asid;
        logln!("[http] using node name service (asid={})", ns_asid);

        unsafe { HTTP_NS = Some(name_service) };

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Http,
            verify_el0_http,
        );
        logln!("[http] verifier deferred (waits for tcpip + httpd listening)");
    }

    extern "C" fn verify_el0_http() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { HTTP_NS.as_ref() }.expect("[http] test state missing");

        let tcpip_manifest = [
            ManifestEntry {
                key: IP_KEY,
                flags: 0,
                value: ManifestValue::Bytes(&[10, 0, 2, 15]),
            },
            ManifestEntry {
                key: GATEWAY_KEY,
                flags: 0,
                value: ManifestValue::Bytes(&[10, 0, 2, 2]),
            },
        ];
        let tcpip = spawn_binary(TCPIP_ELF, ns, &tcpip_manifest);
        logln!("[http] tcpip spawned (asid={})", tcpip.asid);
        let tcpip_cfg: *const u32 = {
            let base: *mut u8 = tcpip.status_frame.into();
            base as *const u32
        };

        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe { core::ptr::read_volatile(tcpip_cfg) } < 6 {
            deadline.assert_pending("EL0 http tcpip service startup");
            yield_lp();
        }
        logln!("[http] tcpip service reached serving stage.");

        let httpd = spawn_binary(HTTPD_ELF, ns, &[]);
        let httpd_cfg: *const u32 = {
            let base: *mut u8 = httpd.status_frame.into();
            base as *const u32
        };
        logln!("[http] httpd spawned (asid={})", httpd.asid);

        // Wait for the httpd to be listening (stage 6). The request/response
        // round trip is performed by the host via --http-test afterwards.
        let mut spins: u64 = 0;
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe { core::ptr::read_volatile(httpd_cfg) } < 6 {
            spins += 1;
            if spins.is_multiple_of(200_000) {
                let stage = unsafe { core::ptr::read_volatile(httpd_cfg) };
                let error = unsafe { core::ptr::read_volatile(httpd_cfg.add(2)) };
                logln!("[http] waiting: httpd stage={} error={:#x}", stage, error);
            }
            deadline.assert_pending("EL0 http httpd listening");
            yield_lp();
        }
        logln!("[http] httpd reached the listening stage.");

        logln!(
            "[http] SUCCESS: httpd is listening on port 80; host may curl the hostfwd port."
        );
        crate::self_test::results::pass(crate::self_test::results::TestId::Http);
    }
}

pub use inner::test_el0_http;
