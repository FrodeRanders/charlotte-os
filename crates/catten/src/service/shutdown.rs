//! Reverse-dependency shutdown for kernel-supervised userspace services.
//!
//! The deployment agent drains application domains first. This coordinator
//! then applies the same rule to node services: stop ingress before control,
//! control before transports, and durable consumers before storage. Hardware
//! drivers are retained as an explicit remainder because aborting a domain is
//! not proof that its device has stopped DMA or completed durable flushes.

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

use crate::{
    cpu::scheduler::{
        monotonic_millis,
        system_scheduler::SYSTEM_SCHEDULER,
    },
    service::{
        bootstrap,
        launch::{
            self,
            SteadyState,
        },
        supervisor::{
            self,
            ServiceDomain,
        },
    },
};

const MAX_PHASE_GRACE_MS: u64 = 60_000;

static NODE_SHUTDOWN_COORDINATOR: spin::LazyLock<
    crate::cpu::multiprocessor::spin::mutex::Mutex<Option<NodeShutdownCoordinator>>,
> = spin::LazyLock::new(|| crate::cpu::multiprocessor::spin::mutex::Mutex::new(None));
static NODE_SHUTDOWN_DEADLINE_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeginNodeShutdownError {
    AlreadyInProgress,
    SteadyStateUnavailable,
    InvalidPhaseGrace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ShutdownPhase {
    DeploymentIngress,
    DeploymentControl,
    DeploymentAgent,
    HttpIngress,
    Time,
    ClusterCatalog,
    ReliableMessaging,
    Discovery,
    TcpIp,
    FrameRouter,
    ObjectStore,
}

const SHUTDOWN_PHASE_COUNT: usize = ShutdownPhase::ObjectStore as usize + 1;

/// How domains in one reverse-dependency phase actually retired.
///
/// An acknowledged retirement published `STATUS_READY` and exited before the
/// coordinator had to cross the phase deadline. A forced retirement required
/// the kernel to abort the domain's threads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShutdownPhaseOutcome {
    pub acknowledged: usize,
    pub unacknowledged: usize,
    pub forced: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceShutdownKind {
    NetworkDriver(&'static [u8]),
    BlockDriver(&'static [u8]),
    EntropyDriver,
}

pub(crate) struct DeviceShutdownDomain {
    kind: DeviceShutdownKind,
    domain: ServiceDomain,
    request_published: bool,
}

impl DeviceShutdownDomain {
    pub(crate) fn new(kind: DeviceShutdownKind, domain: ServiceDomain) -> Self {
        Self {
            kind,
            domain,
            request_published: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeShutdownProgress {
    Draining {
        phase: ShutdownPhase,
        remaining_domains: usize,
    },
    AwaitingDeviceQuiescence {
        device_domains: usize,
    },
    DeviceDomainsTransferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceShutdownProgress {
    Quiescing {
        remaining_domains: usize,
    },
    Complete,
    DeadlineExceeded {
        remaining_domains: usize,
    },
    UnverifiedExit {
        kind: DeviceShutdownKind,
    },
}

struct DomainRetirement {
    domain: ServiceDomain,
    request_published: bool,
    force_requested: bool,
}

pub(crate) struct ShutdownPhaseSpec {
    phase: ShutdownPhase,
    domains: Vec<DomainRetirement>,
    deadline_ms: Option<u64>,
}

impl ShutdownPhaseSpec {
    pub(crate) fn one(phase: ShutdownPhase, domain: ServiceDomain) -> Self {
        Self {
            phase,
            domains: alloc::vec![DomainRetirement {
                domain,
                request_published: false,
                force_requested: false,
            }],
            deadline_ms: None,
        }
    }
}

/// Owns every high-level service being retired and retains the hardware-root
/// domains for the subsequent explicit device-quiescence protocol.
#[must_use = "a node shutdown coordinator must be polled through service drain and device \
              quiescence"]
pub struct NodeShutdownCoordinator {
    node_deadline_ms: u64,
    phase_grace_ms: u64,
    phases: Vec<ShutdownPhaseSpec>,
    device_domains: Vec<DeviceShutdownDomain>,
    device_domains_transferred: bool,
    phase_outcomes: [ShutdownPhaseOutcome; SHUTDOWN_PHASE_COUNT],
}

impl NodeShutdownCoordinator {
    pub(crate) fn new(
        node_deadline_ms: u64,
        phase_grace_ms: u64,
        phases: Vec<ShutdownPhaseSpec>,
        device_domains: Vec<DeviceShutdownDomain>,
    ) -> Self {
        Self {
            node_deadline_ms,
            phase_grace_ms,
            phases,
            device_domains,
            device_domains_transferred: false,
            phase_outcomes: [ShutdownPhaseOutcome::default(); SHUTDOWN_PHASE_COUNT],
        }
    }

    /// Build the production reverse-dependency plan from the launched service
    /// set. Network/storage/entropy drivers are deliberately retained rather
    /// than treated as ordinary domains.
    pub fn from_steady_state(
        state: SteadyState,
        node_deadline_ms: u64,
        phase_grace_ms: u64,
    ) -> Self {
        let mut phases = Vec::new();
        if let Some(deployment) = state.deployment {
            phases
                .push(ShutdownPhaseSpec::one(ShutdownPhase::DeploymentIngress, deployment.ingress));
            phases.push(ShutdownPhaseSpec::one(
                ShutdownPhase::DeploymentControl,
                deployment.clusterctl,
            ));
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::DeploymentAgent, deployment.agent));
        }
        if let Some(appliance) = state.appliance {
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::HttpIngress, appliance.httpd));
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::Time, appliance.time));
        }
        if let Some(cluster) = state.cluster {
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::ClusterCatalog, cluster.dns));
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::ReliableMessaging, cluster.relmsg));
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::Discovery, cluster.disco));
        }
        if let Some(appliance) = state.appliance {
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::TcpIp, appliance.tcpip));
        }
        if let Some(network) = state.network {
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::FrameRouter, network.frouter));
        }
        if let Some(storage) = state.storage {
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::ObjectStore, storage.objstore));
        }

        let mut device_domains = Vec::new();
        if let Some(network) = state.network {
            device_domains.push(DeviceShutdownDomain::new(
                DeviceShutdownKind::NetworkDriver(network.driver_elf),
                network.driver,
            ));
        }
        if let Some(storage) = state.storage {
            device_domains.push(DeviceShutdownDomain::new(
                DeviceShutdownKind::BlockDriver(storage.driver_elf),
                storage.driver,
            ));
        }
        if let Some(entropy) = state.entropy {
            device_domains
                .push(DeviceShutdownDomain::new(DeviceShutdownKind::EntropyDriver, entropy));
        }

        Self::new(node_deadline_ms, phase_grace_ms, phases, device_domains)
    }

    pub fn poll(&mut self) -> NodeShutdownProgress {
        loop {
            let Some(current) = self.phases.first_mut() else {
                if self.device_domains_transferred {
                    return NodeShutdownProgress::DeviceDomainsTransferred;
                }
                return NodeShutdownProgress::AwaitingDeviceQuiescence {
                    device_domains: self.device_domains.len(),
                };
            };
            let now = monotonic_millis();
            let phase_deadline = *current.deadline_ms.get_or_insert_with(|| {
                now.saturating_add(self.phase_grace_ms).min(self.node_deadline_ms)
            });

            let mut index = 0;
            while index < current.domains.len() {
                let retirement = &mut current.domains[index];
                if supervisor::domain_exited(&retirement.domain) {
                    let retirement = current.domains.swap_remove(index);
                    let lifecycle_status =
                        bootstrap::lifecycle_status(retirement.domain.status_frame);
                    let outcome = &mut self.phase_outcomes[current.phase as usize];
                    if retirement.force_requested {
                        outcome.forced += 1;
                    } else if lifecycle_status == charlotte_launch::lifecycle::STATUS_READY {
                        outcome.acknowledged += 1;
                    } else {
                        outcome.unacknowledged += 1;
                    }
                    crate::logln!(
                        "[shutdown] {:?}: acknowledged={} unacknowledged={} forced={} \
                         lifecycle_status={}",
                        current.phase,
                        outcome.acknowledged,
                        outcome.unacknowledged,
                        outcome.forced,
                        lifecycle_status
                    );
                    supervisor::teardown_domain(retirement.domain);
                    continue;
                }
                if !retirement.request_published {
                    bootstrap::write_lifecycle_request(
                        retirement.domain.config_frame,
                        charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED,
                        charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
                        phase_deadline,
                    );
                    retirement.request_published = true;
                }
                if now >= phase_deadline && !retirement.force_requested {
                    bootstrap::write_lifecycle_request(
                        retirement.domain.config_frame,
                        charlotte_launch::lifecycle::STATE_FORCE_TERMINATING,
                        charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
                        phase_deadline,
                    );
                    retirement.force_requested = true;
                    SYSTEM_SCHEDULER.read().abort_as_threads(retirement.domain.asid);
                }
                index += 1;
            }

            if current.domains.is_empty() {
                self.phases.remove(0);
                continue;
            }
            return NodeShutdownProgress::Draining {
                phase: current.phase,
                remaining_domains: current.domains.len(),
            };
        }
    }

    /// Hardware-root domains become available only after every higher-level
    /// service has exited and been reclaimed.
    pub(crate) fn take_device_domains(&mut self) -> Option<Vec<DeviceShutdownDomain>> {
        if !self.phases.is_empty() || self.device_domains_transferred {
            return None;
        }
        self.device_domains_transferred = true;
        Some(core::mem::take(&mut self.device_domains))
    }

    /// Return the observed retirement outcomes for one phase.
    pub fn phase_outcome(&self, phase: ShutdownPhase) -> ShutdownPhaseOutcome {
        self.phase_outcomes[phase as usize]
    }
}

/// Retires hardware-root domains only after their adapters publish the
/// stronger device-quiesced acknowledgement and exit. A deadline never turns
/// into an unconditional thread abort: preserving the IOMMU/device grants is
/// safer than reclaiming memory while DMA state is unknown.
#[must_use = "a device shutdown coordinator must be retained until quiescence is verified"]
pub struct DeviceShutdownCoordinator {
    deadline_ms: u64,
    domains: Vec<DeviceShutdownDomain>,
}

impl DeviceShutdownCoordinator {
    pub(crate) fn new(deadline_ms: u64, domains: Vec<DeviceShutdownDomain>) -> Self {
        Self {
            deadline_ms,
            domains,
        }
    }

    pub fn poll(&mut self) -> DeviceShutdownProgress {
        let now = monotonic_millis();
        let mut index = 0;
        while index < self.domains.len() {
            let device = &mut self.domains[index];
            if supervisor::domain_exited(&device.domain) {
                if bootstrap::lifecycle_status(device.domain.status_frame)
                    != charlotte_launch::lifecycle::STATUS_DEVICE_QUIESCED
                {
                    return DeviceShutdownProgress::UnverifiedExit {
                        kind: device.kind,
                    };
                }
                let device = self.domains.swap_remove(index);
                supervisor::teardown_domain(device.domain);
                continue;
            }
            if !device.request_published {
                bootstrap::write_lifecycle_request(
                    device.domain.config_frame,
                    charlotte_launch::lifecycle::STATE_DRAIN_REQUESTED,
                    charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
                    self.deadline_ms,
                );
                device.request_published = true;
            }
            index += 1;
        }

        if self.domains.is_empty() {
            DeviceShutdownProgress::Complete
        } else if now >= self.deadline_ms {
            DeviceShutdownProgress::DeadlineExceeded {
                remaining_domains: self.domains.len(),
            }
        } else {
            DeviceShutdownProgress::Quiescing {
                remaining_domains: self.domains.len(),
            }
        }
    }
}

impl Drop for DeviceShutdownCoordinator {
    fn drop(&mut self) {
        if !self.domains.is_empty() {
            crate::logln!(
                "[shutdown] retaining {} unverified hardware-root domain(s)",
                self.domains.len()
            );
            core::mem::forget(core::mem::take(&mut self.domains));
        }
    }
}

impl Drop for NodeShutdownCoordinator {
    fn drop(&mut self) {
        for phase in &mut self.phases {
            for retirement in &mut phase.domains {
                if !retirement.force_requested && !supervisor::domain_exited(&retirement.domain) {
                    bootstrap::write_lifecycle_request(
                        retirement.domain.config_frame,
                        charlotte_launch::lifecycle::STATE_FORCE_TERMINATING,
                        charlotte_launch::lifecycle::REASON_NODE_SHUTDOWN,
                        self.node_deadline_ms,
                    );
                    SYSTEM_SCHEDULER.read().abort_as_threads(retirement.domain.asid);
                    retirement.force_requested = true;
                }
            }
        }
        // Hardware-root domains cannot be safely aborted until their adapter
        // has acknowledged device reset/quiescence and IOMMU invalidation.
        // There is no generic device-quiescence owner yet, so an exceptional
        // coordinator drop deliberately leaves those domains running rather
        // than converting uncertainty into unsafe teardown.
        if !self.device_domains_transferred && !self.device_domains.is_empty() {
            crate::logln!(
                "[shutdown] coordinator dropped with {} hardware-root domain(s) still live",
                self.device_domains.len()
            );
            core::mem::forget(core::mem::take(&mut self.device_domains));
        }
    }
}

/// Atomically transfer the published service set into the shutdown state
/// machine. This is a kernel-internal authority boundary; a replicated and
/// authenticated cluster intent still has to select when it is called.
pub fn begin_node_shutdown(
    node_deadline_ms: u64,
    phase_grace_ms: u64,
) -> Result<(), BeginNodeShutdownError> {
    if phase_grace_ms > MAX_PHASE_GRACE_MS {
        return Err(BeginNodeShutdownError::InvalidPhaseGrace);
    }
    let mut coordinator = NODE_SHUTDOWN_COORDINATOR.lock();
    if coordinator.is_some() {
        return Err(BeginNodeShutdownError::AlreadyInProgress);
    }
    let state = launch::take_steady_state_for_shutdown()
        .ok_or(BeginNodeShutdownError::SteadyStateUnavailable)?;
    *coordinator =
        Some(NodeShutdownCoordinator::from_steady_state(state, node_deadline_ms, phase_grace_ms));
    Ok(())
}

/// Transfer the steady-state service set and start the kernel worker that
/// retains shutdown ownership after the requesting deployment agent reaches
/// its own retirement phase.
pub fn start_node_shutdown_worker(
    node_grace_ms: u64,
    phase_grace_ms: u64,
) -> Result<(), BeginNodeShutdownError> {
    let deadline_ms = monotonic_millis().saturating_add(node_grace_ms);
    begin_node_shutdown(deadline_ms, phase_grace_ms)?;
    NODE_SHUTDOWN_DEADLINE_MS.store(deadline_ms, Ordering::Release);
    crate::cpu::scheduler::spawn_thread_on_lp(
        crate::memory::KERNEL_ASID,
        node_shutdown_worker,
        crate::cpu::isa::lp::ops::get_lp_id(),
    );
    Ok(())
}

extern "C" fn node_shutdown_worker() {
    let deadline_ms = NODE_SHUTDOWN_DEADLINE_MS.load(Ordering::Acquire);
    loop {
        match poll_node_shutdown() {
            Some(NodeShutdownProgress::Draining {
                ..
            }) => {
                crate::cpu::scheduler::yield_lp();
            }
            Some(NodeShutdownProgress::AwaitingDeviceQuiescence {
                ..
            }) => break,
            Some(NodeShutdownProgress::DeviceDomainsTransferred) | None => {
                crate::logln!("[shutdown] coordinator ownership was lost before device drain");
                return;
            }
        }
    }

    let Some(mut devices) = begin_device_shutdown(deadline_ms) else {
        crate::logln!("[shutdown] device coordinator was unavailable");
        return;
    };
    loop {
        match devices.poll() {
            DeviceShutdownProgress::Complete => break,
            DeviceShutdownProgress::Quiescing {
                ..
            } => crate::cpu::scheduler::yield_lp(),
            DeviceShutdownProgress::DeadlineExceeded {
                remaining_domains,
            } => {
                crate::logln!(
                    "[shutdown] refusing power-off: {} device domain(s) did not quiesce",
                    remaining_domains
                );
                loop {
                    crate::cpu::scheduler::yield_lp();
                }
            }
            DeviceShutdownProgress::UnverifiedExit {
                kind,
            } => {
                crate::logln!("[shutdown] refusing power-off: {:?} exited without proof", kind);
                loop {
                    crate::cpu::scheduler::yield_lp();
                }
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    crate::cpu::isa::power::power_off();
    #[cfg(not(target_arch = "aarch64"))]
    {
        crate::logln!("[shutdown] device quiescence complete; platform power-off unavailable");
        loop {
            crate::cpu::scheduler::yield_lp();
        }
    }
}

pub fn poll_node_shutdown() -> Option<NodeShutdownProgress> {
    NODE_SHUTDOWN_COORDINATOR.lock().as_mut().map(NodeShutdownCoordinator::poll)
}

/// Inspect how a production shutdown phase retired without taking ownership
/// away from the global coordinator.
pub fn node_shutdown_phase_outcome(phase: ShutdownPhase) -> Option<ShutdownPhaseOutcome> {
    NODE_SHUTDOWN_COORDINATOR.lock().as_ref().map(|coordinator| coordinator.phase_outcome(phase))
}

/// Transfer the hardware-root domains to the device shutdown layer only once
/// every higher-level service has been reclaimed.
pub fn begin_device_shutdown(deadline_ms: u64) -> Option<DeviceShutdownCoordinator> {
    let domains = NODE_SHUTDOWN_COORDINATOR
        .lock()
        .as_mut()
        .and_then(NodeShutdownCoordinator::take_device_domains)?;
    Some(DeviceShutdownCoordinator::new(deadline_ms, domains))
}
