#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
bash "$repo_root/scripts/tests/test-signing-policy.sh"
python3 "$repo_root/scripts/tests/test-boot-results.py"
python3 "$repo_root/scripts/tests/test-boot-profiles.py"
python3 "$repo_root/scripts/tests/test-sign-service-batch.py"
python3 "$repo_root/scripts/tests/test-boot-diagnostics.py"
python3 "$repo_root/scripts/tests/test-boot-images.py"
python3 "$repo_root/scripts/tests/test-secure-boot-results.py"
python3 "$repo_root/scripts/tests/test-management-startup-observations.py"
python3 "$repo_root/scripts/tests/test-management-runner-gates.py"
python3 "$repo_root/scripts/tests/test-management-rotation-results.py"
python3 "$repo_root/scripts/tests/test-management-validity-observations.py"
python3 "$repo_root/scripts/tests/test-management-chain-observations.py"
python3 "$repo_root/scripts/tests/test-management-failover-observations.py"
python3 "$repo_root/scripts/tests/test-management-drain-observations.py"
python3 "$repo_root/scripts/tests/test-qemu-raft-vote-trace.py"
python3 "$repo_root/scripts/tests/test-syscall-emitters.py"
python3 "$repo_root/scripts/tests/test-ci-runtime-inputs.py"
python3 "$repo_root/scripts/tests/test-tls-gcm-backend.py"
python3 "$repo_root/scripts/tests/test-tls-signature-lists.py"
python3 "$repo_root/scripts/tests/test-network-random.py"
python3 "$repo_root/scripts/tests/test-rng-ring-access.py"
python3 "$repo_root/scripts/tests/test-dependency-advisories.py"
python3 "$repo_root/scripts/tests/test-tooling-contracts.py"
python3 "$repo_root/scripts/tests/test-app-sdk.py"
python3 "$repo_root/scripts/tests/test-object-store-format.py"
python3 "$repo_root/scripts/tests/test-resource-policy.py"
python3 "$repo_root/scripts/tests/test-resource-inventory.py"
python3 "$repo_root/scripts/tests/test-boot-allocation-edges.py"
python3 "$repo_root/scripts/tests/test-selective-upstream-ports.py"
python3 "$repo_root/scripts/tests/test-bootstrap-storage.py"
python3 "$repo_root/scripts/tests/test-sensitive-buffer.py"
toolchain="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$repo_root/rust-toolchain.toml")"
host_cargo="$(rustup which --toolchain "$toolchain" cargo)"
host_rustc="$(rustup which --toolchain "$toolchain" rustc)"
host_rustdoc="$(rustup which --toolchain "$toolchain" rustdoc)"
host_work_dir="$(mktemp -d "${TMPDIR:-/tmp}/charlotte-host-tests.XXXXXX")"
trap 'rmdir "$host_work_dir"' EXIT

# Cargo discovers .cargo/config.toml from the invocation directory. CharlotteOS'
# root config builds core for freestanding targets, so invoke host tests from a
# temporary directory while retaining absolute manifest paths.
cd "$host_work_dir"

# The kernel binary disables the Cargo test harness. Exercise its independent
# allocators/collections directly as standalone host Rust unit-test crates.
mkdir -p "$repo_root/target/host-self-tests"

# Exercise the production operation-ID allocator at exhaustion and under
# concurrent allocation without mutating the kernel's boot-lifetime counter.
"$host_rustc" --edition=2024 --test \
    "$repo_root/crates/catten/src/completion/operation_ids.rs" \
    -o "$repo_root/target/host-self-tests/operation-id-tests"
"$repo_root/target/host-self-tests/operation-id-tests"

# Retirement-slot ownership, prepared table growth, charge/deallocation
# ordering, allocator-free publication and stale-handle rejection.
"$host_rustc" --edition=2024 --test \
    "$repo_root/crates/catten/src/klib/collections/id_table.rs" \
    -o "$repo_root/target/host-self-tests/id-table-tests"
"$repo_root/target/host-self-tests/id-table-tests"

# Deployment launch capacity includes unpublished reservations. Exercise the
# exact fixed registry with competing host threads before any publication.
"$host_rustc" --edition=2024 --test \
    "$repo_root/crates/catten/src/service/supervisor/deployments/storage.rs" \
    -o "$repo_root/target/host-self-tests/deployment-registry-tests"
"$repo_root/target/host-self-tests/deployment-registry-tests"

# Shutdown bookkeeping stays inline during construction, phase advancement,
# one-time device transfer and abandonment, including nested owner retention.
"$host_rustc" --edition=2024 --test \
    "$repo_root/crates/catten/src/service/shutdown/storage.rs" \
    -o "$repo_root/target/host-self-tests/shutdown-storage-tests"
"$repo_root/target/host-self-tests/shutdown-storage-tests"

# Thread retirement storage is prepared before publication; linking/filtering
# and explicit node release run through a host allocation tracer.
"$host_rustc" --edition=2024 --test \
    "$repo_root/crates/catten/src/klib/collections/retirement_list.rs" \
    -o "$repo_root/target/host-self-tests/retirement-list-tests"
"$repo_root/target/host-self-tests/retirement-list-tests"

# Allocation-lifetime admission includes failed preparation, allocator clones,
# final weak release and deallocation before refund of the private holder.
"$host_rustc" --edition=2024 --test --crate-name charged_allocator_tests - \
    -o "$repo_root/target/host-self-tests/charged-allocator-tests" <<EOF
#![feature(allocator_api)]
#![allow(dead_code)]
extern crate alloc;
#[path = "$repo_root/crates/catten/src/klib/charged_allocator.rs"]
mod charged_allocator;
EOF
"$repo_root/target/host-self-tests/charged-allocator-tests"

# Compile the actual observer handles, layout admission and policy. This host
# mutex adapter supplies serialization; native campaigns qualify IRQ/guard use.
CATTEN_RESOURCE_POLICY_FILE="$repo_root/crates/catten/src/resource_policy/defaults.rs" \
"$host_rustc" --edition=2024 --test --crate-name observer_owner_tests - \
    -o "$repo_root/target/host-self-tests/observer-owner-tests" <<EOF
#![feature(allocator_api)]
#![allow(dead_code, unused_attributes)]
extern crate alloc;
extern crate self as charlotte_lifecycle;
extern crate self as catten_syscall;
#[path = "$repo_root/crates/catten-syscall/src/lib.rs"]
mod syscall_format;
pub use syscall_format::*;
#[path = "$repo_root/crates/charlotte-lifecycle/src/resources.rs"]
pub mod resources;
#[path = "$repo_root/crates/catten/src/resource_policy.rs"]
mod resource_policy;
mod cpu { pub mod multiprocessor { pub mod spin { pub mod mutex {
    pub struct Mutex<T>(std::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub const fn new(value: T) -> Self { Self(std::sync::Mutex::new(value)) }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> { self.0.lock().unwrap() }
        pub fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> { self.0.try_lock().ok() }
    }
}}}}
mod klib {
    #[path = "$repo_root/crates/catten/src/klib/admission.rs"]
    pub mod admission;
    #[path = "$repo_root/crates/catten/src/klib/charged_allocator.rs"]
    pub mod charged_allocator;
}
mod observer {
    #[path = "$repo_root/crates/catten/src/klib/observer/backing.rs"]
    pub mod backing;
    #[path = "$repo_root/crates/catten/src/klib/observer/owned.rs"]
    pub mod owned;
}
EOF
"$repo_root/target/host-self-tests/observer-owner-tests" --test-threads=1

# Production ledger growth and retaining ownership, with real buffer-free tracing.
CATTEN_RESOURCE_POLICY_FILE="$repo_root/crates/catten/src/resource_policy/defaults.rs" \
"$host_rustc" --edition=2024 --test --crate-name iommu_ledger_tests - \
    -o "$repo_root/target/host-self-tests/iommu-ledger-tests" <<EOF
#![feature(allocator_api)]
#![allow(dead_code, unused_attributes)]
extern crate alloc;
extern crate self as charlotte_lifecycle;
extern crate self as catten_syscall;
#[path = "$repo_root/crates/catten-syscall/src/lib.rs"]
mod syscall_format;
pub use syscall_format::*;
#[path = "$repo_root/crates/charlotte-lifecycle/src/resources.rs"]
pub mod resources;
#[path = "$repo_root/crates/catten/src/resource_policy.rs"]
mod resource_policy;
mod cpu { pub mod multiprocessor { pub mod spin { pub mod mutex {
    pub struct Mutex<T>(std::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub const fn new(value: T) -> Self { Self(std::sync::Mutex::new(value)) }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> { self.0.lock().unwrap() }
        pub fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> { self.0.try_lock().ok() }
    }
}}}}
mod klib {
    #[path = "$repo_root/crates/catten/src/klib/admission.rs"]
    pub mod admission;
}
#[path = "$repo_root/crates/catten/src/device/dma_common.rs"]
mod dma_common;
mod tables {
    use crate::dma_common::Error;
    struct Region { base: usize, pages: usize }
    #[path = "$repo_root/crates/catten/src/device/dma_tables/ledger.rs"]
    mod ledger;
}
EOF
"$repo_root/target/host-self-tests/iommu-ledger-tests" --test-threads=1

# Production object catalogs: original class, retained capacity and real free ordering.
CATTEN_RESOURCE_POLICY_FILE="$repo_root/crates/catten/src/resource_policy/defaults.rs" \
"$host_rustc" --edition=2024 --test --crate-name frame_vector_tests - \
    -o "$repo_root/target/host-self-tests/frame-vector-tests" <<EOF
#![feature(allocator_api)]
#![allow(dead_code, unused_attributes)]
extern crate alloc;
extern crate self as charlotte_lifecycle;
extern crate self as catten_syscall;
#[path = "$repo_root/crates/catten-syscall/src/lib.rs"]
mod syscall_format;
pub use syscall_format::*;
#[path = "$repo_root/crates/charlotte-lifecycle/src/resources.rs"]
pub mod resources;
#[path = "$repo_root/crates/catten/src/resource_policy.rs"]
mod resource_policy;
mod cpu { pub mod multiprocessor { pub mod spin { pub mod mutex {
    pub struct Mutex<T>(std::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub const fn new(value: T) -> Self { Self(std::sync::Mutex::new(value)) }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> { self.0.lock().unwrap() }
        pub fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> { self.0.try_lock().ok() }
    }
}}}}
mod klib {
    #[path = "$repo_root/crates/catten/src/klib/admission.rs"]
    pub mod admission;
}
mod memory { pub mod object {
    type PAddr=usize;
    const MAX_MEMORY_OBJECT_PAGES: usize=crate::resource_policy::POLICY.memory.object_pages as usize;
    #[derive(Debug, PartialEq, Eq)] enum MemoryObjectError { ResourceLimit, FrameAllocFailed }
    pub mod backing {
        use super::*;
        #[path = "$repo_root/crates/catten/src/memory/object/backing/frame_vector.rs"]
        pub mod frame_vector;
    }
}}

EOF
"$repo_root/target/host-self-tests/frame-vector-tests" --test-threads=1

# Root catalogs retain backing and both byte charges until confirmed disposal.
python3 "$repo_root/scripts/tests/test-frame-tracking.py"

# Production byte allocator: complete growth overlap, real deallocation and original class.
CATTEN_RESOURCE_POLICY_FILE="$repo_root/crates/catten/src/resource_policy/defaults.rs" \
"$host_rustc" --edition=2024 --test --crate-name byte_allocator_tests - \
    -o "$repo_root/target/host-self-tests/byte-allocator-tests" <<EOF
#![feature(allocator_api)]
#![allow(dead_code, unused_attributes)]
extern crate alloc;
extern crate self as charlotte_lifecycle;
extern crate self as catten_syscall;
#[path = "$repo_root/crates/catten-syscall/src/lib.rs"]
mod syscall_format;
pub use syscall_format::*;
#[path = "$repo_root/crates/charlotte-lifecycle/src/resources.rs"]
pub mod resources;
#[path = "$repo_root/crates/catten/src/resource_policy.rs"]
mod resource_policy;
#[macro_export] macro_rules! logln { (\$message:literal) => {}; }
mod cpu { pub mod multiprocessor { pub mod spin { pub mod mutex {
    pub struct Mutex<T>(std::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub const fn new(value:T)->Self { Self(std::sync::Mutex::new(value)) }
        pub fn lock(&self)->std::sync::MutexGuard<'_,T> { self.0.lock().unwrap() }
        pub fn try_lock(&self)->Option<std::sync::MutexGuard<'_,T>> { self.0.try_lock().ok() }
    }
}}}}
mod klib {
    #[path = "$repo_root/crates/catten/src/klib/admission.rs"] pub mod admission;
}
#[path = "$repo_root/crates/catten/src/klib/admission/byte_allocator_tests.rs"]
mod byte_allocator_tests;

EOF
"$repo_root/target/host-self-tests/byte-allocator-tests" --test-threads=1


# Production fixed service-image cache: byte/count ownership and publication.
CATTEN_RESOURCE_POLICY_FILE="$repo_root/crates/catten/src/resource_policy/defaults.rs" \
"$host_rustc" --edition=2024 --test --crate-name service_image_cache_tests - \
    -o "$repo_root/target/host-self-tests/service-image-cache-tests" <<EOF
#![feature(allocator_api)]
#![allow(dead_code, unused_attributes)]
extern crate alloc;
extern crate self as charlotte_lifecycle;
extern crate self as catten_syscall;
#[path = "$repo_root/crates/catten-syscall/src/lib.rs"]
mod syscall_format;
pub use syscall_format::*;
#[path = "$repo_root/crates/charlotte-lifecycle/src/resources.rs"]
pub mod resources;
#[path = "$repo_root/crates/catten/src/resource_policy.rs"]
mod resource_policy;
#[macro_export] macro_rules! logln { (\$message:literal) => {}; }
mod cpu { pub mod multiprocessor { pub mod spin { pub mod mutex {
    pub struct Mutex<T>(std::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub const fn new(value:T)->Self { Self(std::sync::Mutex::new(value)) }
        pub fn lock(&self)->std::sync::MutexGuard<'_,T> { self.0.lock().unwrap() }
        pub fn try_lock(&self)->Option<std::sync::MutexGuard<'_,T>> { self.0.try_lock().ok() }
    }
}}}}
mod klib {
    #[path = "$repo_root/crates/catten/src/klib/admission.rs"] pub mod admission;
}
#[path = "$repo_root/crates/catten/src/service/store/image_cache.rs"]
mod image_cache;

EOF
"$repo_root/target/host-self-tests/service-image-cache-tests" --test-threads=1


# Production queued attachment retirement capacity: real deallocation/refund order.
CATTEN_RESOURCE_POLICY_FILE="$repo_root/crates/catten/src/resource_policy/defaults.rs" \
"$host_rustc" --edition=2024 --test --crate-name attachment_storage_tests - \
    -o "$repo_root/target/host-self-tests/attachment-storage-tests" <<EOF
#![feature(allocator_api)]
#![allow(dead_code, unused_attributes)]
extern crate alloc;
extern crate self as charlotte_lifecycle;
extern crate self as catten_syscall;
#[path = "$repo_root/crates/catten-syscall/src/lib.rs"]
mod syscall_format;
pub use syscall_format::*;
#[path = "$repo_root/crates/charlotte-lifecycle/src/resources.rs"]
pub mod resources;
#[path = "$repo_root/crates/catten/src/resource_policy.rs"]
mod resource_policy;
#[macro_export] macro_rules! logln { (\$message:literal) => {}; }
mod cpu { pub mod multiprocessor { pub mod spin { pub mod mutex {
    pub struct Mutex<T>(std::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub const fn new(value:T)->Self { Self(std::sync::Mutex::new(value)) }
        pub fn lock(&self)->std::sync::MutexGuard<'_,T> { self.0.lock().unwrap() }
        pub fn try_lock(&self)->Option<std::sync::MutexGuard<'_,T>> { self.0.try_lock().ok() }
    }
}}}}
mod klib {
    #[path = "$repo_root/crates/catten/src/klib/admission.rs"] pub mod admission;
}
mod memory {
    pub mod kernel_buffers {
        use core::{alloc::AllocError, sync::atomic::{AtomicU64,Ordering}};
        pub static USED: AtomicU64 = AtomicU64::new(0);
        pub static LIMIT: AtomicU64 = AtomicU64::new(u64::MAX);
        #[derive(Clone,Copy,Debug)] pub struct Sponsor(pub bool);
        impl Sponsor { pub fn unidentified()->Self { Self(false) } }
        impl crate::klib::admission::ByteSponsor for Sponsor {
            fn reserve(&self,n:u64)->Result<(),AllocError> {
                if self.0 { USED.fetch_update(Ordering::AcqRel,Ordering::Acquire,|v|
                    v.checked_add(n).filter(|n| *n <= LIMIT.load(Ordering::Acquire)))
                    .map_err(|_|AllocError)?; } Ok(())
            }
            fn release(&self,n:u64) { if self.0 { assert!(USED.fetch_sub(n,Ordering::AcqRel)>=n); } }
        }
    }
    pub mod object {
    pub static DROPS: core::sync::atomic::AtomicUsize=core::sync::atomic::AtomicUsize::new(0);
    #[derive(Debug)] pub struct RetiredMemory(pub u64);
    impl Drop for RetiredMemory {
        fn drop(&mut self) { DROPS.fetch_add(1,core::sync::atomic::Ordering::AcqRel); }
    }
    impl RetiredMemory { pub fn release(self)->Result<(),()> { Ok(()) } }
    #[derive(Debug)] pub enum MemoryObjectError { UnknownCapability, Other }
    pub fn try_retire_cap(_:usize,_:u64)->Result<RetiredMemory,MemoryObjectError> { Err(MemoryObjectError::UnknownCapability) }
}}
mod ipc {
    use alloc::vec::Vec;
    type BufferAllocator=crate::klib::admission::ByteAllocator<crate::klib::admission::IpcNodes,crate::memory::kernel_buffers::Sponsor>;
    type Buffer<T> = Vec<T,BufferAllocator>;
    fn empty_buffer_allocator()->BufferAllocator { BufferAllocator::with_sponsor(false,crate::memory::kernel_buffers::Sponsor::unidentified()) }
    impl From<bool> for BufferAllocator { fn from(p:bool)->Self { Self::with_sponsor(p,crate::memory::kernel_buffers::Sponsor::unidentified()) } }
    fn buffer<T>(allocator:impl Into<BufferAllocator>)->Buffer<T> { Vec::new_in(allocator.into()) }
    type AddressSpaceId=usize;
    type MemoryObjectCap=u64;
    const CAP_VECTOR_MAX:usize=crate::CAP_VECTOR_MAX;
    #[derive(Debug,PartialEq,Eq)] enum IpcError { ResourceLimit }
    #[path = "$repo_root/crates/catten/src/ipc/attachments.rs"] pub mod attachments;
}
EOF
"$repo_root/target/host-self-tests/attachment-storage-tests" --test-threads=1

# Diagnostic capture uses the production compile-time resource policy too.
# Compile both modules together rather than treating snapshot.rs as a crate root.
CATTEN_RESOURCE_POLICY_FILE="$repo_root/crates/catten/src/resource_policy/defaults.rs" \
"$host_rustc" --edition=2024 --test --crate-name snapshot_tests - \
    -o "$repo_root/target/host-self-tests/snapshot-tests" <<EOF
#![allow(dead_code)]
#![feature(allocator_api)]
extern crate alloc;
#[path = "$repo_root/crates/catten/src/resource_policy.rs"]
mod resource_policy;
#[path = "$repo_root/crates/catten/src/klib/collections/snapshot.rs"]
mod snapshot;
EOF
"$repo_root/target/host-self-tests/snapshot-tests"

# Scheduler FIFO links are inline in their existing exact thread owners.
# Exercise production splices/rejection/abandonment with allocation tracing.
"$host_rustc" --edition=2024 --test \
    "$repo_root/crates/catten/src/cpu/scheduler/lp_schedulers/run_queue.rs" \
    -o "$repo_root/target/host-self-tests/run-queue-tests"
"$repo_root/target/host-self-tests/run-queue-tests"

# Scratch metadata admission/release is architecture independent. Exercise the
# production allocator directly, including its local metadata-failure adapter.
"$host_rustc" --edition=2024 --test \
    "$repo_root/crates/catten/src/memory/object/scratch.rs" \
    -o "$repo_root/target/host-self-tests/scratch-window-tests"
"$repo_root/target/host-self-tests/scratch-window-tests"

manifests=()
for manifest_path in "$repo_root"/crates/*/Cargo.toml; do
    crate_dir="${manifest_path%/Cargo.toml}"
    if ! rg --quiet '#[[:space:]]*\[[[:space:]]*test[[:space:]]*\]' \
        "$crate_dir" --glob '*.rs'; then
        continue
    fi
    manifests+=("${manifest_path#"$repo_root"/}")
done

for manifest in "${manifests[@]}"; do
    # A package may legitimately disable test harnesses for its freestanding
    # binary targets while retaining host unit tests in its library. Cargo
    # applies `test = false` per target, so let it select the enabled targets.
    features=()
    if [ "$manifest" = "crates/charlotte-app/Cargo.toml" ]; then features=(--all-features); fi
    RUSTC="$host_rustc" RUSTDOC="$host_rustdoc" "$host_cargo" test --locked \
        --manifest-path "$repo_root/$manifest" "${features[@]}"
done

RUSTC="$host_rustc" RUSTDOC="$host_rustdoc" "$host_cargo" test --locked \
    --manifest-path "$repo_root/tools/cluster-sign/Cargo.toml"

RUSTC="$host_rustc" RUSTDOC="$host_rustdoc" "$host_cargo" run --locked --quiet \
    --manifest-path "$repo_root/tools/cluster-sign/Cargo.toml" \
    -- selftest
