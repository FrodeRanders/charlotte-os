//! Reverse-dependency shutdown for kernel-supervised userspace services.
//!
//! The deployment agent drains application domains first. This coordinator
//! then applies the same rule to node services: stop ingress before control,
//! control before transports, and durable consumers before storage. Hardware
//! drivers are retained as an explicit remainder because aborting a domain is
//! not proof that its device has stopped DMA or completed durable flushes.

extern crate alloc;

use alloc::vec::Vec;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeginNodeShutdownError {
    AlreadyInProgress,
    SteadyStateUnavailable,
    InvalidPhaseGrace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    ObjectStore,
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
    device_domains: Vec<ServiceDomain>,
    device_domains_transferred: bool,
}

impl NodeShutdownCoordinator {
    pub(crate) fn new(
        node_deadline_ms: u64,
        phase_grace_ms: u64,
        phases: Vec<ShutdownPhaseSpec>,
        device_domains: Vec<ServiceDomain>,
    ) -> Self {
        Self {
            node_deadline_ms,
            phase_grace_ms,
            phases,
            device_domains,
            device_domains_transferred: false,
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
        if let Some(storage) = state.storage {
            phases.push(ShutdownPhaseSpec::one(ShutdownPhase::ObjectStore, storage.objstore));
        }

        let mut device_domains = Vec::new();
        if let Some(network) = state.network {
            // frouter owns no device, but must remain until a NIC-specific
            // quiescence protocol has stopped ingress and drained RX/TX.
            device_domains.push(network.frouter);
            device_domains.push(network.driver);
        }
        if let Some(storage) = state.storage {
            device_domains.push(storage.driver);
        }
        if let Some(entropy) = state.entropy {
            device_domains.push(entropy);
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
    pub fn take_device_domains(&mut self) -> Option<Vec<ServiceDomain>> {
        if !self.phases.is_empty() || self.device_domains_transferred {
            return None;
        }
        self.device_domains_transferred = true;
        Some(core::mem::take(&mut self.device_domains))
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

pub fn poll_node_shutdown() -> Option<NodeShutdownProgress> {
    NODE_SHUTDOWN_COORDINATOR.lock().as_mut().map(NodeShutdownCoordinator::poll)
}

/// Transfer the hardware-root domains to the device shutdown layer only once
/// every higher-level service has been reclaimed.
pub fn take_device_shutdown_domains() -> Option<Vec<ServiceDomain>> {
    NODE_SHUTDOWN_COORDINATOR.lock().as_mut().and_then(NodeShutdownCoordinator::take_device_domains)
}
