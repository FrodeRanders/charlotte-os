use super::{
    KernelHeap,
    Local,
    Memory,
    Policy,
    Pool,
    Ratio,
    Scoped,
};

// Operator-editable build policy; dimensions and coupling are documented in
// docs/reference/resource-policy.md. Existing numerical defaults are preserved.
pub(super) const POLICY: Policy = Policy {
    name: "development-default",
    capabilities: Scoped {
        domain: [4096],
        pool: Pool {
            total: [65_536],
            ordinary: [49_152],
        },
    },
    completions: Scoped {
        domain: [1024],
        pool: Pool {
            total: [8192],
            ordinary: [6144],
        },
    },
    timers: Scoped {
        domain: [1024],
        pool: Pool {
            total: [8192],
            ordinary: [6144],
        },
    },
    watches: Scoped {
        domain: [1024],
        pool: Pool {
            total: [8192],
            ordinary: [6144],
        },
    },
    waiters: Scoped {
        domain: [1024],
        pool: Pool {
            total: [8192],
            ordinary: [6144],
        },
    },
    waiter_accounts: Pool {
        total: [8192],
        ordinary: [6144],
    },
    timer_accounts: Pool {
        total: [16_384],
        ordinary: [12_288],
    },
    completion_accounts: Pool {
        total: [24_576],
        ordinary: [18_432],
    },
    namespace_accounts: Pool {
        total: [32_768],
        ordinary: [24_576],
    },
    observer_lists: Pool {
        total: [8192],
        ordinary: [6144],
    },
    observer_backing: Pool {
        total: [16 * 1024 * 1024, 65_536],
        ordinary: [12 * 1024 * 1024, 49_152],
    },
    completion_nodes: Pool {
        total: [8 * 1024 * 1024, 20_480],
        ordinary: [6 * 1024 * 1024, 15_360],
    },
    ipc_nodes: Pool {
        total: [16 * 1024 * 1024, 98_304],
        ordinary: [12 * 1024 * 1024, 73_728],
    },
    device_nodes: Pool {
        total: [16 * 1024 * 1024, 139_264],
        ordinary: [12 * 1024 * 1024, 104_448],
    },
    iommu_nodes: Pool {
        total: [24 * 1024 * 1024, 262_144],
        ordinary: [18 * 1024 * 1024, 196_608],
    },
    mailbox_records: Scoped {
        domain: [512],
        pool: Pool {
            total: [8192],
            ordinary: [6144],
        },
    },
    mailbox_queues: Scoped {
        domain: [2, 1024 * 1024],
        pool: Pool {
            total: [1024, 8 * 1024 * 1024],
            ordinary: [768, 6 * 1024 * 1024],
        },
    },
    ipc_records: Scoped {
        domain: [512, 512, 512],
        pool: Pool {
            total: [8192, 8192, 8192],
            ordinary: [6144, 6144, 6144],
        },
    },
    endpoints: Scoped {
        domain: [64, 8192],
        pool: Pool {
            total: [1024, 32_768],
            ordinary: [768, 24_576],
        },
    },
    completion_queues: Scoped {
        domain: [32, 256 * 1024],
        pool: Pool {
            total: [2048, 4 * 1024 * 1024],
            ordinary: [1536, 3 * 1024 * 1024],
        },
    },
    root_metadata: Pool {
        total: [4 * 1024 * 1024, 16_384],
        ordinary: [3 * 1024 * 1024, 12_288],
    },
    namespace_nodes: Pool {
        total: [8 * 1024 * 1024, 40_960],
        ordinary: [6 * 1024 * 1024, 30_720],
    },
    authority_nodes: Pool {
        total: [16 * 1024 * 1024, 131_072],
        ordinary: [12 * 1024 * 1024, 98_304],
    },
    mailbox_nodes: Pool {
        total: [2 * 1024 * 1024, 16_384],
        ordinary: [1536 * 1024, 12_288],
    },
    deferred_nodes: Pool {
        total: [1024 * 1024, 16_384],
        ordinary: [768 * 1024, 12_288],
    },
    principal_history: Pool {
        total: [1024 * 1024, 4096],
        ordinary: [768 * 1024, 3072],
    },
    thread_metadata: Pool {
        total: [8 * 1024 * 1024, 4096],
        ordinary: [6 * 1024 * 1024, 3072],
    },
    thread_table: Pool {
        total: [32 * 1024 * 1024, 32_768],
        ordinary: [24 * 1024 * 1024, 24_576],
    },
    root_table: Pool {
        total: [16 * 1024 * 1024, 16_384],
        ordinary: [12 * 1024 * 1024, 12_288],
    },
    scratch_nodes: Pool {
        // Funded charges add 16 bytes per slot. Preserve the previous mixed
        // namespace/extent envelope at the existing count ceilings.
        total: [640 * 1024, 8192],
        ordinary: [480 * 1024, 6144],
    },
    snapshot_buffers: Pool {
        // Shared frame catalogs and primitive syscall bytes. Two maximum
        // pickup/plaintext pairs plus the former 3 MiB ordinary allowance fit.
        total: [16 * 1024 * 1024, 8192],
        ordinary: [12 * 1024 * 1024, 6144],
    },
    service_images: Pool {
        total: [16 * 1024 * 1024, 64],
        ordinary: [12 * 1024 * 1024, 48],
    },
    mapping_nodes: Pool {
        // Funded charges add 16 bytes per slot. Keep the count ceiling and
        // existing layout headroom.
        total: [1152 * 1024, 8192],
        ordinary: [864 * 1024, 6144],
    },
    memory_capability_nodes: Pool {
        total: [4 * 1024 * 1024, 32_768],
        ordinary: [3 * 1024 * 1024, 24_576],
    },
    memory: Memory {
        object_pages: 16_384,
        domain_object_pages: 16_384,
        objects: Scoped {
            domain: [1024],
            pool: Pool {
                total: [8192],
                ordinary: [6144],
            },
        },
        principal_pages: 16_384,
        principal_objects: 1024,
        heap_domain_pages: 1264,
        image_domain_pages: 16_384,
        private_table_domain_pages: 1024,
        dma_domain_pages: 1024,
        dma_unit_pages: 2048,
        object_ram: Ratio {
            numerator: 1,
            denominator: 4,
        },
        heap_ram: Ratio {
            numerator: 1,
            denominator: 4,
        },
        image_ram: Ratio {
            numerator: 1,
            denominator: 4,
        },
        stack_ram: Ratio {
            numerator: 1,
            denominator: 8,
        },
        private_table_ram: Ratio {
            numerator: 1,
            denominator: 16,
        },
        shared_table_ram: Ratio {
            numerator: 1,
            denominator: 64,
        },
        dma_ram: Ratio {
            numerator: 1,
            denominator: 32,
        },
        physical_reserve: Ratio {
            numerator: 1,
            denominator: 8,
        },
        ordinary_share: Ratio {
            numerator: 3,
            denominator: 4,
        },
        principal_thread_metadata: [1024 * 1024, 128],
        principal_thread_table: [16 * 1024 * 1024, 16_384],
        principal_root_table: [8 * 1024 * 1024, 8192],
        principal_kernel_buffer_bytes: 16 * 1024 * 1024,
        principal_namespace_account_bytes: 1024 * 1024,
        principal_reactor_account_bytes: 1024 * 1024,
        principal_metadata_bytes: 8 * 1024 * 1024,
    },
    kernel_heap: KernelHeap {
        initial_bytes: 8 * 1024 * 1024,
        growth_min_bytes: 64 * 1024 * 1024,
        growth_max_bytes: 256 * 1024 * 1024,
        ram_divisor: 64,
    },
    local: Local {
        waiters_per_source: 64,
        watches_per_endpoint: 128,
        watches_per_thread: 128,
        callbacks_per_completion: 128,
        endpoint_capacity: 4096,
        telemetry_working_bytes: 1024 * 1024,
    },
};
