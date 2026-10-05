//! Real EL0 scoped applications, configured roots and adversarial grant IPC.
use charlotte_launch::security_probe_status as status;

use crate::{
    cpu::scheduler::{
        monotonic_millis,
        sleep_millis,
    },
    logln,
    service::{
        bootstrap::{
            self,
            ManifestEntry,
            ManifestValue,
        },
        supervisor::{
            self,
            ProfileLaunchError,
            ServiceDomain,
        },
    },
};

const IMAGE: &[u8] = include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/probe.elf"));
const POLICY: &[u8] = include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/admitted.cdep"));
const ALTERNATE: &[u8] =
    include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/alternate.cdep"));
const ARTIFACT_KEY_HEX: &[u8] =
    include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/artifact.pub"));
const DEPLOYMENT_KEY_HEX: &[u8] =
    include_bytes!(concat!(env!("CATTEN_SECURITY_TEST_DIR"), "/deployment.pub"));

/// Kernel/runtime-boundary owner for a test domain, not a userspace capability.
/// Explicit stop waits and reaps. Failure-path Drop only requests an abort;
/// it must not block while another verifier is reporting a panic.
struct ProbeDomain(Option<ServiceDomain>);
impl ProbeDomain {
    fn get(&self) -> &ServiceDomain {
        self.0.as_ref().unwrap()
    }

    fn stop(mut self) {
        bootstrap::write_lifecycle_request(
            self.get().config_frame,
            charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED,
            charlotte_launch::lifecycle::REASON_DEPLOYMENT_RETIRED,
            monotonic_millis().saturating_add(5_000),
        );
        supervisor::wait_domain_exit(self.get(), 5_000);
        supervisor::teardown_domain(self.0.take().unwrap()).expect("security probe reclamation");
    }
}
impl Drop for ProbeDomain {
    fn drop(&mut self) {
        if let Some(domain) = self.0.take() {
            if !crate::memory::address_space_handle_is_current(domain.address_space) {
                return;
            }
            if supervisor::domain_exited(&domain) {
                // Failure-path Drop must not poll/wait under unknown guards.
                // A current, exited fixture root is retained for inspection.
                crate::logln!(
                    "[security] retaining exited probe on exceptional Drop asid={}",
                    domain.asid
                );
            } else {
                crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
                    .read()
                    .abort_as_threads(domain.asid);
            }
        }
    }
}

fn public_key(bytes: &[u8]) -> [u8; 32] {
    let text = core::str::from_utf8(bytes).expect("test public key UTF-8").trim();
    assert!(text.is_ascii() && text.len() == 64, "test public key length/encoding");
    let mut key = [0; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte =
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("test public-key hex");
    }
    key
}

fn word(domain: &ServiceDomain, offset: usize) -> u32 {
    let base: *const u8 = domain.status_frame.into();
    unsafe { crate::self_test::status_u32(base, offset) }
}

fn wait_for(domain: &ServiceDomain, predicate: impl Fn(&ServiceDomain) -> bool, what: &str) {
    let deadline = crate::self_test::results::Deadline::after_millis(30_000);
    while !predicate(domain) {
        assert_ne!(
            word(domain, status::STAGE),
            status::FAILED,
            "security probe failed check {}",
            word(domain, status::FAILURE)
        );
        assert!(!supervisor::domain_exited(domain), "security probe exited before {what}");
        deadline.assert_pending(what);
        sleep_millis(10);
    }
}

fn spawn(artifact_key: &[u8; 32], deployment_key: &[u8; 32], mode: u64) -> ProbeDomain {
    let manifest = [
        ManifestEntry {
            key: status::FOREIGN_THREAD_KEY,
            flags: 0,
            value: ManifestValue::Unsigned(supervisor::node_name_service().domain.tid as u64),
        },
        ManifestEntry {
            key: status::MODE_KEY,
            flags: 0,
            value: ManifestValue::Unsigned(mode),
        },
        ManifestEntry {
            key: status::ALTERNATE_KEY,
            flags: 0,
            value: ManifestValue::Bytes(ALTERNATE),
        },
        ManifestEntry {
            key: status::DEPLOYMENT_KEY,
            flags: 0,
            value: ManifestValue::Bytes(deployment_key),
        },
    ];
    ProbeDomain(Some(
        supervisor::try_spawn_security_probe(
            IMAGE,
            POLICY,
            deployment_key,
            artifact_key,
            &manifest,
        )
        .expect("scoped probe launch"),
    ))
}

pub fn test_el0_security() {
    crate::self_test::results::spawn_verifier(crate::self_test::results::TestId::Security, verify);
}

extern "C" fn verify() {
    assert!(
        crate::service::launch::steady_state().appliance.is_some(),
        "security test needs tcpip"
    );
    let artifact_key = public_key(ARTIFACT_KEY_HEX);
    let deployment_key = public_key(DEPLOYMENT_KEY_HEX);
    assert_ne!(artifact_key, deployment_key);
    assert_ne!(artifact_key, charlotte_launch::CLUSTER_PUBLIC_KEY);
    assert_ne!(deployment_key, charlotte_launch::CLUSTER_PUBLIC_KEY);
    assert!(matches!(
        supervisor::try_spawn_with_deployment_descriptor(
            IMAGE,
            POLICY,
            &artifact_key,
            &artifact_key
        ),
        Err(ProfileLaunchError::InvalidDeploymentDescriptor)
    ));
    assert!(matches!(
        supervisor::try_spawn_with_deployment_descriptor(
            IMAGE,
            POLICY,
            &deployment_key,
            &deployment_key
        ),
        Err(ProfileLaunchError::DescriptorArtifactMismatch)
    ));
    logln!("[security] independent roots and wrong-role root rejection verified");

    let flood = spawn(&artifact_key, &deployment_key, 1);
    wait_for(
        flood.get(),
        |domain| word(domain, status::REQUESTS) >= 64,
        "cancellation traffic start",
    );
    let mut previous_generation = 0;
    let mut previous_domain: Option<crate::memory::AddressSpaceHandle> = None;
    for _ in 0..2 {
        let probe = spawn(&artifact_key, &deployment_key, 0);
        if let Some(retired) = previous_domain {
            assert!(!crate::memory::launch_descriptor_matches(
                retired.id(),
                retired.generation() as u64,
                &charlotte_launch::sha256::digest(POLICY)
            ));
        }
        wait_for(
            probe.get(),
            |domain| word(domain, status::STAGE) == status::PASSED,
            "adversarial application grant checks",
        );
        assert_eq!(word(probe.get(), status::CHECKS), status::EXPECTED_CHECKS);
        let base: *const u8 = probe.get().status_frame.into();
        let generation = unsafe {
            core::ptr::read_volatile(base.add(status::PUBLICATION_GENERATION).cast::<u64>())
        };
        assert!(generation > previous_generation, "republication must advance service generation");
        previous_generation = generation;
        let retired = probe.get().address_space;
        let digest = charlotte_launch::sha256::digest(POLICY);
        assert!(crate::memory::launch_descriptor_matches(
            retired.id(),
            retired.generation() as u64,
            &digest
        ));
        probe.stop();
        previous_domain = Some(retired);
        assert!(!crate::memory::launch_descriptor_matches(
            retired.id(),
            retired.generation() as u64,
            &digest
        ));
        logln!(
            "[security] probe passed; retired policy fenced, publication generation={generation}"
        );
    }
    wait_for(
        flood.get(),
        |domain| word(domain, status::REQUESTS) >= 512,
        "bounded cancellation stress",
    );
    let submitted = word(flood.get(), status::REQUESTS);
    flood.stop();
    logln!("[security] concurrent cancellation traffic retired after {submitted} requests");
    crate::self_test::results::pass(crate::self_test::results::TestId::Security);
}
