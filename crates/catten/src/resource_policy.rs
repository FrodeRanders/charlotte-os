//! Compile-time resource capacity policy. See docs/reference/resource-policy.md.
//! Admission and original-charge ownership remain in their resource families.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pool<const N: usize> {
    pub total: [u64; N],
    pub ordinary: [u64; N],
}
impl<const N: usize> Pool<N> {
    const fn valid(self) -> bool {
        let mut i = 0;
        while i < N {
            if self.ordinary[i] == 0
                || self.ordinary[i] >= self.total[i]
                || self.total[i] > isize::MAX as u64
            {
                return false;
            }
            i += 1;
        }
        true
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Scoped<const N: usize> {
    pub domain: [u64; N],
    pub pool: Pool<N>,
}
impl<const N: usize> Scoped<N> {
    const fn valid(self) -> bool {
        if !self.pool.valid() {
            return false;
        }
        let mut i = 0;
        while i < N {
            if self.domain[i] == 0 || self.domain[i] > self.pool.ordinary[i] {
                return false;
            }
            i += 1;
        }
        true
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ratio {
    pub numerator: u64,
    pub denominator: u64,
}
impl Ratio {
    const fn valid(self) -> bool {
        self.numerator > 0 && self.numerator < self.denominator
    }

    /// Widen before multiplication: large discovered RAM must not wrap policy.
    pub const fn of(self, value: u64) -> u64 {
        ((value as u128 * self.numerator as u128) / self.denominator as u128) as u64
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Memory {
    pub object_pages: u64,
    pub domain_object_pages: u64,
    pub objects: Scoped<1>,
    pub principal_pages: u64,
    pub principal_objects: u64,
    pub heap_domain_pages: u64,
    pub image_domain_pages: u64,
    pub private_table_domain_pages: u64,
    pub dma_domain_pages: u64,
    pub dma_unit_pages: u64,
    pub object_ram: Ratio,
    pub heap_ram: Ratio,
    pub image_ram: Ratio,
    pub stack_ram: Ratio,
    pub private_table_ram: Ratio,
    pub shared_table_ram: Ratio,
    pub dma_ram: Ratio,
    pub physical_reserve: Ratio,
    pub ordinary_share: Ratio,
    /// Principal thread-context bytes, bundle count.
    pub principal_thread_metadata: [u64; 2],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KernelHeap {
    pub initial_bytes: usize,
    pub growth_min_bytes: usize,
    pub growth_max_bytes: usize,
    pub ram_divisor: u64,
}
impl KernelHeap {
    /// Clamp before rounding, including when reported RAM cannot fit usize.
    /// Validated endpoints are aligned, so rounding stays within the ceiling.
    pub fn growth_reserve_bytes(self, usable_bytes: u64) -> usize {
        let scaled = usize::try_from(usable_bytes / self.ram_divisor).unwrap_or(usize::MAX);
        let bounded = scaled.clamp(self.growth_min_bytes, self.growth_max_bytes);
        bounded.div_ceil(2 * 1024 * 1024) * (2 * 1024 * 1024)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Local {
    pub waiters_per_source: usize,
    pub watches_per_endpoint: usize,
    pub watches_per_thread: usize,
    pub callbacks_per_completion: usize,
    pub endpoint_capacity: usize,
    pub telemetry_working_bytes: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Policy {
    pub name: &'static str,
    pub capabilities: Scoped<1>,
    pub completions: Scoped<1>,
    pub timers: Scoped<1>,
    pub watches: Scoped<1>,
    pub waiters: Scoped<1>,
    pub observer_lists: Pool<1>,
    pub mailbox_records: Scoped<1>,
    /// Queue sets, requested ring bytes.
    pub mailbox_queues: Scoped<2>,
    /// Connections, pending calls, reply tokens.
    pub ipc_records: Scoped<3>,
    /// Endpoints, queue backing slots.
    pub endpoints: Scoped<2>,
    /// Completion queues, backing bytes.
    pub completion_queues: Scoped<2>,
    /// Metadata pools: requested bytes, allocations.
    pub root_metadata: Pool<2>,
    pub principal_history: Pool<2>,
    pub thread_metadata: Pool<2>,
    pub scratch_nodes: Pool<2>,
    pub snapshot_buffers: Pool<2>,
    pub mapping_nodes: Pool<2>,
    pub memory_capability_nodes: Pool<2>,
    pub memory: Memory,
    pub kernel_heap: KernelHeap,
    pub local: Local,
}
impl Policy {
    /// Compile-time consistency, not proof of boot capacity, fairness or RAM fit.
    /// Reasons are stable so the host harness checks each rejected relationship.
    pub const fn validate(self) -> Result<(), &'static str> {
        if self.name.is_empty() || self.name.len() > 64 {
            return Err("policy name");
        }
        if !self.capabilities.valid()
            || !self.completions.valid()
            || !self.timers.valid()
            || !self.watches.valid()
            || !self.waiters.valid()
            || !self.observer_lists.valid()
            || !self.mailbox_records.valid()
            || !self.mailbox_queues.valid()
            || !self.ipc_records.valid()
            || !self.endpoints.valid()
            || !self.completion_queues.valid()
            || !self.root_metadata.valid()
            || !self.principal_history.valid()
            || !self.thread_metadata.valid()
            || !self.scratch_nodes.valid()
            || !self.snapshot_buffers.valid()
            || !self.mapping_nodes.valid()
            || !self.memory_capability_nodes.valid()
            || !self.memory.objects.valid()
        {
            return Err("domain/ordinary/total ordering or machine range");
        }
        let m = self.memory;
        if !m.object_ram.valid()
            || !m.heap_ram.valid()
            || !m.image_ram.valid()
            || !m.stack_ram.valid()
            || !m.private_table_ram.valid()
            || !m.shared_table_ram.valid()
            || !m.dma_ram.valid()
            || !m.physical_reserve.valid()
            || !m.ordinary_share.valid()
        {
            return Err("RAM ratio");
        }
        if m.object_pages == 0
            || m.object_pages > m.domain_object_pages
            || m.domain_object_pages > m.principal_pages
            || m.principal_pages > isize::MAX as u64 / 4096
            || m.principal_objects < m.objects.domain[0]
            || m.heap_domain_pages == 0
            || m.heap_domain_pages > m.principal_pages
            || m.image_domain_pages == 0
            || m.image_domain_pages > m.principal_pages
            || m.private_table_domain_pages == 0
            || m.private_table_domain_pages > m.principal_pages
            || m.dma_domain_pages == 0
            || m.dma_unit_pages < m.dma_domain_pages
            || m.dma_unit_pages > isize::MAX as u64 / 4096
        {
            return Err("backing ceilings");
        }
        if m.object_pages > self.snapshot_buffers.ordinary[0] / core::mem::size_of::<usize>() as u64
        {
            return Err("object snapshot capacity");
        }
        let mut i = 0;
        while i < 2 {
            if m.principal_thread_metadata[i] == 0
                || m.principal_thread_metadata[i] > self.thread_metadata.ordinary[i]
            {
                return Err("principal thread metadata");
            }
            i += 1;
        }
        let l = self.local;
        if l.waiters_per_source == 0
            || l.waiters_per_source as u64 > self.waiters.domain[0]
            || l.watches_per_endpoint == 0
            || l.watches_per_endpoint as u64 > self.watches.domain[0]
            || l.watches_per_thread == 0
            || l.watches_per_thread as u64 > self.watches.domain[0]
            || l.callbacks_per_completion == 0
            || l.callbacks_per_completion as u64 > self.watches.domain[0]
            || l.endpoint_capacity == 0
            || l.endpoint_capacity > u32::MAX as usize
            || l.endpoint_capacity.checked_next_power_of_two().is_none()
            || l.endpoint_capacity.next_power_of_two() as u64 > self.endpoints.domain[1]
            || l.telemetry_working_bytes == 0
            || l.telemetry_working_bytes > isize::MAX as usize
        {
            return Err("local source/queue ceilings");
        }
        // Kernel arena is mapped using 2 MiB pages. Validate before allocation.
        let h = self.kernel_heap;
        if h.ram_divisor == 0
            || h.initial_bytes == 0
            || h.growth_min_bytes == 0
            || h.growth_min_bytes > h.growth_max_bytes
            || !h.initial_bytes.is_multiple_of(2 * 1024 * 1024)
            || !h.growth_min_bytes.is_multiple_of(2 * 1024 * 1024)
            || !h.growth_max_bytes.is_multiple_of(2 * 1024 * 1024)
            || h.initial_bytes.checked_add(h.growth_max_bytes).is_none()
            || h.initial_bytes.saturating_add(h.growth_max_bytes) > isize::MAX as usize
        {
            return Err("kernel heap layout");
        }
        Ok(())
    }
}

#[path = "resource_policy/defaults.rs"]
mod defaults;
pub(crate) const DEFAULT: Policy = defaults::POLICY;
// The selected file is trusted build source, never an application/manifest input.
mod selected {
    include!(env!("CATTEN_RESOURCE_POLICY_FILE"));
}
pub(crate) const POLICY: Policy = selected::POLICY;
const _: () = assert!(DEFAULT.validate().is_ok(), "invalid default resource policy");
const _: () = assert!(
    POLICY.validate().is_ok(),
    "invalid selected resource policy; run the resource-policy harness"
);

impl Memory {
    pub const fn frames_available(self, free: u64, usable: u64, request: u64) -> bool {
        let reserve = self.physical_reserve.of(usable);
        let reserve = if reserve == 0 {
            1
        } else {
            reserve
        };
        match free.checked_sub(request) {
            Some(remaining) => remaining >= reserve,
            None => false,
        }
    }
}
pub(crate) const fn frames_available(free: u64, usable: u64, request: u64) -> bool {
    POLICY.memory.frames_available(free, usable, request)
}

#[cfg(test)]
#[path = "resource_policy/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "resource_policy/larger.rs"]
mod larger;
