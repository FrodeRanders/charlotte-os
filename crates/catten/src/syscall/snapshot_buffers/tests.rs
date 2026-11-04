//! Serialized native controls use actual object-backed copies. Counter-pressure
//! adapters qualify deterministic denial; they are not concurrent EL0 pressure.
use super::*;
use crate::{
    klib::admission::{
        self,
        Family,
        Reservation,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
    syscall::{
        self,
        TrapFrame,
        call_no,
    },
};

fn used() -> ([u64; 2], [u64; 2]) {
    admission::used::<SnapshotBuffers, 2>()
}
fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn close(handle: AddressSpaceHandle) {
    memory::close_user_address_space_handle(handle).unwrap();
}
fn input(handle: AddressSpaceHandle, len: usize) -> MemoryObjectCap {
    object::allocate(handle.id(), len.div_ceil(4096)).unwrap()
}
fn fill_budget(platform: bool, leave: usize) -> Reservation<SnapshotBuffers, 2> {
    let current = used();
    let (limit, taken) = if platform {
        (SnapshotBuffers::TOTAL[0], current.0[0])
    } else {
        (SnapshotBuffers::ORDINARY[0], current.1[0])
    };
    Reservation::reserve(platform, [limit - taken - leave as u64, 0]).unwrap()
}

pub(crate) fn run() {
    simultaneous_maxima();
    original_class_and_generation();
    rejection_and_prefix();
    syscall_consumption();
    crate::logln!(
        "[syscall snapshots] all six buffers: maxima, original class/generation, ordinary/total \
         denial before allocator entry, prefix cleanup and public input consumption passed; exact \
         byte/count recovery"
    );
}

fn simultaneous_maxima() {
    let before = used();
    let handle = root();
    let class = Class::capture(handle).unwrap();
    let maximum = charlotte_launch::operations_pickup::MAX_PICKUP_LEN;
    let profile = charlotte_launch::operations::MAX_PROFILE_LEN;
    let cap = input(handle, maximum);
    let descriptor = input(handle, charlotte_launch::deployment::MAX_DESCRIPTOR_LEN);
    let baseline = used();
    {
        let inputs = class
            .scoped(
                (cap, charlotte_launch::MAX_ARTIFACT_ELF_SIZE),
                (descriptor, charlotte_launch::deployment::MAX_DESCRIPTOR_LEN),
            )
            .unwrap();
        let bytes = inputs.image.len() + inputs.descriptor.len();
        assert_eq!(
            used(),
            (
                [baseline.0[0] + bytes as u64, baseline.0[1]],
                [baseline.1[0] + bytes as u64, baseline.1[1]]
            )
        );
        assert_eq!(inputs.image.capacity(), inputs.image.len());
        assert_eq!(inputs.descriptor.capacity(), inputs.descriptor.len());
    }
    assert_eq!(used(), baseline);
    // Actual private byte storage stands in for the former frame-catalog byte
    // allowance here. Existing frame-snapshot tests exercise actual catalogs.
    let allowance = 3 * 1024 * 1024;
    let mut mixed = Vec::<u8, _>::new_in(class.allocator.clone());
    mixed.try_reserve_exact(allowance).unwrap();
    mixed.resize(allowance, 0);
    let mut first = class.operational(cap, maximum).unwrap();
    let mut second = class.operational(cap, maximum).unwrap();
    first.plaintext.try_initialize_zeroed(profile).unwrap();
    second.plaintext.try_initialize_zeroed(profile).unwrap();
    first.plaintext.fill(0xa5);
    second.plaintext.fill(0x5a);
    assert_eq!(first.package.len(), maximum);
    assert_eq!(second.package.len(), maximum);
    let bytes = allowance + 2 * (maximum + profile);
    assert_eq!(bytes, 11_815_160);
    assert_eq!(
        used(),
        (
            [baseline.0[0] + bytes as u64, baseline.0[1]],
            [baseline.1[0] + bytes as u64, baseline.1[1]]
        )
    );
    drop((second, first, mixed));
    assert_eq!(used(), baseline);
    object::close_cap(handle.id(), descriptor).unwrap();
    object::close_cap(handle.id(), cap).unwrap();
    close(handle);
    assert_eq!(used(), before);
    crate::logln!(
        "[syscall snapshot maxima] image+descriptor={}+{}; two pickup+plaintext={}+{} each and \
         3145728 actual mixed bytes; peak additional=11815160; policy={}/{}; exact recovery",
        charlotte_launch::MAX_ARTIFACT_ELF_SIZE,
        charlotte_launch::deployment::MAX_DESCRIPTOR_LEN,
        maximum,
        profile,
        SnapshotBuffers::ORDINARY[0],
        SnapshotBuffers::TOTAL[0]
    );
}

fn original_class_and_generation() {
    let before = used();
    let handle = root();
    let ordinary = Class::capture(handle).unwrap();
    let cap = input(handle, 64);
    object::write_bytes(handle.id(), cap, &[0xab; 64]).unwrap();
    memory::budget::mark_platform(handle);
    let trusted = Class::capture(handle).unwrap();
    let first = ordinary.snapshot(cap, 64).unwrap();
    let second = trusted.snapshot(cap, 64).unwrap();
    assert_eq!(&*first, &[0xab; 64]);
    assert!(!first.allocator().platform());
    assert!(second.allocator().platform());
    let charged = used();
    assert_eq!(charged.1, [before.1[0] + 64, before.1[1]]);
    assert_eq!(charged.0, [before.0[0] + 128, before.0[1]]);
    close(handle);
    assert_eq!(used(), charged);
    assert!(Class::capture(handle).is_err());
    let fresh = root();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    let fresh_cap = input(fresh, 64);
    assert_eq!(
        ordinary.snapshot(fresh_cap, 64).err(),
        Some(MemoryObjectError::AddressSpaceMissing)
    );
    assert_eq!(trusted.snapshot(fresh_cap, 64).err(), Some(MemoryObjectError::AddressSpaceMissing));
    assert_eq!(used(), charged);
    drop((second, first));
    close(fresh);
    assert_eq!(used(), before);
}

fn rejection_and_prefix() {
    let before = used();
    for platform in [false, true] {
        let handle = root();
        if platform {
            memory::budget::mark_platform(handle);
        }
        let class = Class::capture(handle).unwrap();
        let cap = input(handle, 96);
        object::write_bytes(handle.id(), cap, &[0xcd; 96]).unwrap();
        let pressure = fill_budget(platform, 63);
        let charged = used();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            assert!(class.snapshot(cap, 64).is_err());
        }
        assert_eq!(used(), charged);
        drop(pressure);
        let baseline = used();
        let pressure = fill_budget(platform, 95);
        let charged = used();
        assert!(class.scoped((cap, 64), (cap, 32)).is_err());
        assert_eq!(used(), charged, "failed descriptor must dispose the image prefix");
        drop(pressure);
        let mut entered = 0;
        assert!(
            Class::scoped_with((cap, 64), (cap, 32), |(cap, len)| {
                entered += 1;
                if entered == 2 {
                    return Err(MemoryObjectError::FrameAllocFailed);
                }
                class.snapshot(cap, len)
            })
            .is_err()
        );
        assert_eq!(entered, 2);
        assert_eq!(used(), baseline);
        let inputs = class.scoped((cap, 64), (cap, 32)).unwrap();
        assert_eq!(&*inputs.image, &[0xcd; 64]);
        assert_eq!(&*inputs.descriptor, &[0xcd; 32]);
        drop(inputs);
        close(handle);
        assert_eq!(used(), before);
    }
}

fn frame(handle: AddressSpaceHandle, cap: u64, len: usize) -> TrapFrame {
    let mut regs = [0; 19];
    regs[1] = cap;
    regs[2] = len as u64;
    TrapFrame {
        regs,
        elr_el1: 0,
        spsr_el1: 0,
        sp_el0: 0,
        lp_id: 0,
        asid: handle.id(),
    }
}

fn syscall_consumption() {
    let before = used();
    let handle = root();
    let previous = crate::service::supervisor::DEPLOYMENT_AGENT_ASID.lock().replace(handle);
    for denied in [false, true] {
        for number in [
            call_no::SPAWN_ARTIFACT_SCOPED,
            call_no::SPAWN_OPERATIONAL_CONNECTOR,
            call_no::REQUEST_NODE_SHUTDOWN,
        ] {
            let len = if number == call_no::REQUEST_NODE_SHUTDOWN {
                charlotte_launch::shutdown::ENCODED_LEN
            } else {
                512
            };
            let cap = input(handle, len);
            let descriptor = input(handle, charlotte_launch::deployment::MAX_DESCRIPTOR_LEN);
            let mut frame = frame(handle, cap, len);
            if number == call_no::SPAWN_ARTIFACT_SCOPED {
                frame.regs[3] = descriptor;
                frame.regs[4] = charlotte_launch::deployment::MAX_DESCRIPTOR_LEN as u64;
            }
            let baseline = used();
            let pressure = denied.then(|| fill_budget(false, 0));
            syscall::syscall_dispatch(&mut frame, number);
            assert_eq!(
                frame.regs[0],
                if number == call_no::REQUEST_NODE_SHUTDOWN {
                    u64::MAX
                } else {
                    0
                }
            );
            assert!(object::info(handle.id(), cap).is_err());
            if number == call_no::SPAWN_ARTIFACT_SCOPED {
                assert_eq!(frame.regs[1], 0);
                assert!(object::info(handle.id(), descriptor).is_err());
            } else {
                object::close_cap(handle.id(), descriptor).unwrap();
            }
            drop(pressure);
            assert_eq!(used(), baseline);
        }
    }
    *crate::service::supervisor::DEPLOYMENT_AGENT_ASID.lock() = previous;
    // Upgrade authorization/connection checks precede consuming its image.
    let endpoint = crate::ipc::endpoint_create(handle.id(), 0x5131, 1, 1).unwrap();
    let connection =
        crate::ipc::connection_mint(handle.id(), endpoint, crate::ipc::ConnectionRights::CALL)
            .unwrap();
    let previous = crate::service::supervisor::LIVE_UPGRADE_MANAGER_ASID.lock().replace(handle);
    for denied in [false, true] {
        let cap = input(handle, 512);
        let mut frame = frame(handle, cap, 512);
        frame.regs[4] = connection;
        let baseline = used();
        let pressure = denied.then(|| fill_budget(false, 0));
        syscall::syscall_dispatch(&mut frame, call_no::SPAWN_UPGRADE);
        assert_eq!(frame.regs[0], 0);
        assert!(object::info(handle.id(), cap).is_err());
        drop(pressure);
        assert_eq!(used(), baseline);
    }
    *crate::service::supervisor::LIVE_UPGRADE_MANAGER_ASID.lock() = previous;
    close(handle);
    assert_eq!(used(), before);
    crate::logln!(
        "[syscall snapshot consumption] four public handlers, invalid content and quota denial: \
         original status/input consumption, zero scoped receipt and exact refund passed"
    );
}
