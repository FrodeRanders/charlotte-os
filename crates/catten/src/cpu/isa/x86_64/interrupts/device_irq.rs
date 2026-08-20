//! x86_64 device-interrupt routing.
//!
//! Maps a Global System Interrupt (GSI) from the device-capability layer to an
//! IOAPIC pin plus a dynamically allocated vector, and delivers it to the bound
//! completion queue. This is the x86 counterpart of the AArch64 GIC SPI path:
//! `arch_enable_irq` programs the IOAPIC redirection table, the dynamic ISR
//! dispatches the vector to [`ih_device_interrupt`], which hands the GSI to the
//! architecture-independent [`deliver_interrupt`](crate::device::deliver_interrupt).

use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use spin::LazyLock;

use crate::cpu::{
    interrupt_routing::InterruptHandler,
    isa::{
        constants::interrupt_vectors::FIXED_INTERRUPT_VECTOR_COUNT,
        interface::interrupts::{
            DynInterruptDispatcherIfce,
            ExternalInterruptControllerIfce,
        },
        interrupts::{
            dynamic::{
                DYN_IH_MATRIX,
                DYN_VECS_PER_LP,
            },
            ioapic::IoApic,
        },
        lp::{
            InterruptVectorNum,
            LpId,
        },
    },
    multiprocessor::spin::mutex::Mutex,
};

/// The system's first I/O APIC controller. QEMU `q35` exposes one.
static IOAPIC: LazyLock<Mutex<IoApic>> = LazyLock::new(|| {
    let (address, gsi_base) =
        crate::environment::acpi::sdt::discovery::madt_ioapic().unwrap_or((0xfec0_0000, 0));
    IOAPIC_GSI_BASE.store(gsi_base, Ordering::Relaxed);
    let mut ioapic = IoApic::new(address);
    ioapic.map_mmio();
    Mutex::new(ioapic)
});

/// GSI base of the discovered IOAPIC (0 when there is a single controller).
static IOAPIC_GSI_BASE: AtomicU32 = AtomicU32::new(0);

/// Maps a dynamic-vector offset (0..[`DYN_VECS_PER_LP`]) to the GSI routed
/// through it. `u32::MAX` marks an unused slot.
static VECTOR_TO_INTID: [AtomicU32; DYN_VECS_PER_LP as usize] =
    [const { AtomicU32::new(u32::MAX) }; DYN_VECS_PER_LP as usize];

/// Maps a GSI to its allocated dynamic-vector offset, so a re-arm after the
/// driver acknowledges can unmask the existing route instead of allocating a
/// fresh vector each time. `u32::MAX` marks an unrouted GSI.
const MAX_GSI: usize = 256;
static GSI_TO_OFFSET: [AtomicU32; MAX_GSI] = [const { AtomicU32::new(u32::MAX) }; MAX_GSI];

/// Monotonic round-robin cursor over the dynamic-vector space. Collisions on a
/// long-lived system would wrap and overwrite; the current scale never does.
static NEXT_DYN_VECTOR: AtomicU32 = AtomicU32::new(0);

/// Translate a GSI to its IOAPIC redirection-table pin index.
fn gsi_to_pin(gsi: u32) -> u32 {
    gsi - IOAPIC_GSI_BASE.load(Ordering::Relaxed)
}

/// Translate a dynamic-vector offset to the architectural IDT vector it was
/// registered under (the first dynamic vector is `FIXED_INTERRUPT_VECTOR_COUNT
/// - 1`, matching [`register_dynamic_isr_gates`]).
fn offset_to_vector(offset: u32) -> InterruptVectorNum {
    (FIXED_INTERRUPT_VECTOR_COUNT - 1 + offset as u8) as InterruptVectorNum
}

/// The device-interrupt handler installed at every routed dynamic vector. The
/// dynamic ISR passes the vector offset as its argument; the offset maps to the
/// GSI routed through it.
#[unsafe(no_mangle)]
pub extern "C" fn ih_device_interrupt(offset: InterruptVectorNum) {
    let intid = VECTOR_TO_INTID[offset as usize].load(Ordering::Acquire);
    if intid != u32::MAX {
        if crate::device::vt_d::handle_interrupt(intid) {
            return;
        }
        crate::device::deliver_interrupt(intid);
    }
}

/// Route `gsi` to `target_lp` on a freshly allocated dynamic vector and install
/// the device-interrupt handler for it, then unmask the source. Idempotent:
/// once a GSI is routed, a later call (e.g. the driver's interrupt acknowledge)
/// only re-arms the existing route.
pub fn enable_irq(gsi: u32, target_lp: LpId) {
    // MSI interrupts are already routed by `allocate_msi` (their vector and
    // handler were installed there); there is no IOAPIC pin to program.
    if gsi >= crate::device::MSI_INTID_BASE {
        return;
    }
    let slot = gsi as usize;
    if slot >= MAX_GSI {
        return;
    }
    let mut offset = GSI_TO_OFFSET[slot].load(Ordering::Acquire);
    if offset == u32::MAX {
        offset = NEXT_DYN_VECTOR.fetch_add(1, Ordering::Relaxed) % DYN_VECS_PER_LP as u32;
        VECTOR_TO_INTID[offset as usize].store(gsi, Ordering::Release);
        GSI_TO_OFFSET[slot].store(offset, Ordering::Release);
        let handler: InterruptHandler = ih_device_interrupt;
        DYN_IH_MATRIX.set_dyn_ih(target_lp, offset as InterruptVectorNum, handler);
        let vector = offset_to_vector(offset);
        let mut ioapic = IOAPIC.lock();
        ioapic
            .setup_ext_int(target_lp, vector, gsi_to_pin(gsi), false, true, false)
            .expect("failed to route a GSI through the IOAPIC");
        return;
    }
    // Already routed: just re-arm (unmask) the source.
    let mut ioapic = IOAPIC.lock();
    let _ = ioapic.set_ext_int_mask_state(gsi_to_pin(gsi), false);
}

/// Mask (disable) a routed GSI. MSI interrupts have no IOAPIC pin to mask.
pub fn disable_irq(gsi: u32) {
    if gsi >= crate::device::MSI_INTID_BASE {
        return;
    }
    let mut ioapic = IOAPIC.lock();
    let _ = ioapic.set_ext_int_mask_state(gsi_to_pin(gsi), true);
}

/// There is no separate software-clearable pending bit for a wired IOAPIC
/// interrupt; the LAPIC EOI (signalled by the dynamic ISR epilogue) clears the
/// in-service vector.
pub fn clear_irq_pending(_gsi: u32) {}

/// The architectural IDT vector a GSI is currently routed through, if any.
/// Used by self-tests to inject a synthetic delivery on that vector.
pub fn gsi_vector(gsi: u32) -> Option<InterruptVectorNum> {
    let slot = gsi as usize;
    if slot >= MAX_GSI {
        return None;
    }
    let offset = GSI_TO_OFFSET[slot].load(Ordering::Acquire);
    (offset != u32::MAX).then(|| offset_to_vector(offset))
}

/// Whether the kernel's MSI mechanism is available. On x86_64, MSI is delivered
/// directly to the LAPIC via the message address, so it is always available.
pub fn msi_available() -> bool {
    true
}

/// Allocate one MSI vector and return the address/data pair the PCI function
/// writes to raise it. The vector is delivered directly to the LAPIC (the
/// message address encodes the target APIC id); the returned `intid` is the
/// synthetic identifier the device-capability layer routes through.
pub fn allocate_msi(
    _device_id: u32,
) -> Option<
    crate::device_management::drivers::busses::pci_express::ecam::capabilities::standard::msi::MsiMessage,
>{
    use crate::{
        cpu::isa::interrupts::x2apic::X2Apic,
        device_management::drivers::busses::pci_express::ecam::capabilities::standard::msi::MsiMessage,
    };

    let target_lp = crate::cpu::isa::lp::ops::get_lp_id();
    let dest = X2Apic::physical_apic_id(target_lp)?;
    let offset = NEXT_DYN_VECTOR.fetch_add(1, Ordering::Relaxed) % DYN_VECS_PER_LP as u32;
    let intid = crate::device::MSI_INTID_BASE + offset;
    VECTOR_TO_INTID[offset as usize].store(intid, Ordering::Release);
    let handler: InterruptHandler = ih_device_interrupt;
    DYN_IH_MATRIX.set_dyn_ih(target_lp, offset as InterruptVectorNum, handler);
    let vector = offset_to_vector(offset);
    Some(MsiMessage {
        address: 0xfee0_0000 | ((dest as u64) << 12),
        data: vector as u32,
        intid,
    })
}
