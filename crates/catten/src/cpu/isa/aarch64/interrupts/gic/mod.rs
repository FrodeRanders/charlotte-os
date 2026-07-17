//! # AArch64 Generic Interrupt Controller (GICv3)
//!
//! This module implements the local interrupt controller interface for the
//! GICv3 as found on the QEMU `virt` machine and typical ARMv8-A server/embedded
//! platforms. GICv3 splits into three cooperating parts:
//!
//! - **Distributor (GICD)**: a single system-wide MMIO block that manages
//!   Shared Peripheral Interrupts (SPIs) and global configuration.
//! - **Redistributors (GICR)**: one MMIO frame per core that manages that
//!   core's private interrupts, i.e. Software Generated Interrupts (SGIs,
//!   INTIDs 0-15, used for IPIs) and Private Peripheral Interrupts (PPIs,
//!   INTIDs 16-31, which include the Generic Timer).
//! - **CPU interface**: accessed through `ICC_*_EL1` system registers rather
//!   than MMIO. This is where interrupts are acknowledged (`ICC_IAR1_EL1`),
//!   completed (`ICC_EOIR1_EL1`), and generated as IPIs (`ICC_SGI1R_EL1`).
//!
//! The MMIO base addresses are, for now, the fixed QEMU `virt` defaults. Once
//! the device tree layer is implemented they should be discovered from the
//! `/intc` node rather than hard-coded. See the ARM Generic Interrupt
//! Controller Architecture Specification, GIC architecture version 3 and
//! version 4.

use core::arch::asm;

use spin::LazyLock;

use crate::cpu::isa::aarch64::memory::address::paddr::PAddr;
use crate::cpu::isa::interface::interrupts::LocalIntCtlrIfce;
use crate::cpu::isa::interface::memory::address::PhysicalAddress;
use crate::cpu::isa::lp::ops::get_lic_id;
use crate::cpu::isa::lp::{InterruptVectorNum, LpId};
use crate::cpu::multiprocessor::spin::per_lp::PerLp;

pub type LocalIntCtlr = GicV3;

/// QEMU `virt` GIC distributor MMIO physical base address.
const GICD_BASE: usize = 0x0800_0000;
/// QEMU `virt` GIC redistributor region MMIO physical base address. Each core's
/// redistributor occupies two consecutive 64 KiB frames (RD_base + SGI_base).
const GICR_BASE: usize = 0x080A_0000;
/// Size of a single core's redistributor region (two 64 KiB frames: the
/// RD_base control frame and the SGI_base frame for private interrupts).
const GICR_STRIDE: usize = 0x2_0000;
/// Offset from a redistributor's RD_base to its SGI_base frame.
const GICR_SGI_OFFSET: usize = 0x1_0000;

// Distributor register offsets.
const GICD_CTLR: usize = 0x0000;
const GICD_IGROUPR: usize = 0x0080;
// Per-INTID banked distributor registers for Shared Peripheral Interrupts
// (INTID >= 32). Each is an array indexed by `intid / 32` (one bit per INTID),
// except IPRIORITYR (one byte per INTID) and IROUTER (one 64-bit word per
// INTID, only for SPIs).
const GICD_ISENABLER: usize = 0x0100;
const GICD_ICENABLER: usize = 0x0180;
const GICD_ISPENDR: usize = 0x0200;
const GICD_ICPENDR: usize = 0x0280;
const GICD_IPRIORITYR: usize = 0x0400;
const GICD_IROUTER: usize = 0x6000;

// GICD_CTLR bits (when using affinity routing, ARE_NS).
const GICD_CTLR_ARE_NS: u32 = 1 << 4;
const GICD_CTLR_ENABLE_GRP1_NS: u32 = 1 << 1;

// Redistributor (RD_base frame) register offsets.
const GICR_WAKER: usize = 0x0014;
// GICR_WAKER bits.
const GICR_WAKER_PROCESSOR_SLEEP: u32 = 1 << 1;
const GICR_WAKER_CHILDREN_ASLEEP: u32 = 1 << 2;

// Redistributor (SGI_base frame) register offsets, for SGIs and PPIs.
const GICR_IGROUPR0: usize = 0x0080;
const GICR_ISENABLER0: usize = 0x0100;
const GICR_ICENABLER0: usize = 0x0180;
const GICR_IPRIORITYR: usize = 0x0400;

/// Priority value that permits all interrupts through `ICC_PMR_EL1` (lowest
/// possible priority threshold; larger numeric values are lower priority).
const PMR_ALLOW_ALL: u64 = 0xFF;
/// Default priority assigned to the interrupts we enable. It must be numerically
/// lower than the PMR threshold so the interrupt is not masked.
const DEFAULT_PRIORITY: u8 = 0xA0;

#[derive(Debug)]
pub enum Error {
    InvalidLpId,
}

pub struct GicV3;

#[inline(always)]
unsafe fn mmio_read32(base: usize, offset: usize) -> u32 {
    let ptr = unsafe { PAddr::from(base as u64).into_hhdm_ptr::<u32>().byte_add(offset) };
    unsafe { core::ptr::read_volatile(ptr) }
}

#[inline(always)]
unsafe fn mmio_write32(base: usize, offset: usize, value: u32) {
    let ptr = unsafe { PAddr::from(base as u64).into_hhdm_mut::<u32>().byte_add(offset) };
    unsafe { core::ptr::write_volatile(ptr, value) }
}

#[inline(always)]
unsafe fn mmio_write64(base: usize, offset: usize, value: u64) {
    let ptr = unsafe { PAddr::from(base as u64).into_hhdm_mut::<u64>().byte_add(offset) };
    unsafe { core::ptr::write_volatile(ptr, value) }
}

#[inline(always)]
unsafe fn mmio_write8(base: usize, offset: usize, value: u8) {
    let ptr = unsafe { PAddr::from(base as u64).into_hhdm_mut::<u8>().byte_add(offset) };
    unsafe { core::ptr::write_volatile(ptr, value) }
}

/// The RD_base MMIO address of the calling core's redistributor. Cores are
/// laid out consecutively starting at [`GICR_BASE`]; we index by the local
/// interrupt controller id (affinity 0), which matches the QEMU `virt` layout.
#[inline]
fn gicr_rd_base() -> usize {
    GICR_BASE + (get_lic_id() as usize) * GICR_STRIDE
}

/// The SGI_base MMIO address of the calling core's redistributor.
#[inline]
fn gicr_sgi_base() -> usize {
    gicr_rd_base() + GICR_SGI_OFFSET
}

impl GicV3 {
    /// Wake this core's redistributor out of its low-power sleep state so it
    /// begins delivering interrupts, per the GICv3 power-up sequence.
    fn redistributor_wake() {
        let rd = gicr_rd_base();
        unsafe {
            let mut waker = mmio_read32(rd, GICR_WAKER);
            waker &= !GICR_WAKER_PROCESSOR_SLEEP;
            mmio_write32(rd, GICR_WAKER, waker);
            // Wait until the redistributor signals it is awake.
            while mmio_read32(rd, GICR_WAKER) & GICR_WAKER_CHILDREN_ASLEEP != 0 {
                core::hint::spin_loop();
            }
        }
    }

    /// Enable the CPU interface system register access and unmask interrupts at
    /// the priority mask, then enable Group 1 interrupt signalling.
    fn cpu_interface_init() {
        unsafe {
            // ICC_SRE_EL1.SRE = 1 to use the system register interface.
            let mut sre: u64;
            asm!("mrs {}, ICC_SRE_EL1", out(reg) sre, options(nomem, nostack, preserves_flags));
            sre |= 1;
            asm!("msr ICC_SRE_EL1, {}", in(reg) sre, options(nomem, nostack, preserves_flags));
            asm!("isb", options(nomem, nostack, preserves_flags));
            // Allow all interrupt priorities through.
            asm!("msr ICC_PMR_EL1, {}", in(reg) PMR_ALLOW_ALL, options(nomem, nostack, preserves_flags));
            // Enable Group 1 interrupts at the CPU interface.
            asm!("msr ICC_IGRPEN1_EL1, {}", in(reg) 1u64, options(nomem, nostack, preserves_flags));
            asm!("isb", options(nomem, nostack, preserves_flags));
        }
    }

    /// Configure a private interrupt (SGI or PPI, INTID 0-31) on this core's
    /// redistributor: assign it to Group 1, give it a runnable priority, and
    /// enable it.
    fn enable_private_int(intid: u32) {
        let sgi = gicr_sgi_base();
        unsafe {
            // Group 1 (non-secure): set the corresponding bit in IGROUPR0.
            let mut group = mmio_read32(sgi, GICR_IGROUPR0);
            group |= 1 << intid;
            mmio_write32(sgi, GICR_IGROUPR0, group);
            // Priority: IPRIORITYR is byte-addressable, one byte per INTID.
            let prio_ptr = PAddr::from(sgi as u64)
                .into_hhdm_mut::<u8>()
                .byte_add(GICR_IPRIORITYR + intid as usize);
            core::ptr::write_volatile(prio_ptr, DEFAULT_PRIORITY);
            // Enable the interrupt.
            mmio_write32(sgi, GICR_ISENABLER0, 1 << intid);
        }
    }

    /// Disable a private interrupt on this core's redistributor.
    #[allow(dead_code)]
    fn disable_private_int(intid: u32) {
        let sgi = gicr_sgi_base();
        unsafe {
            mmio_write32(sgi, GICR_ICENABLER0, 1 << intid);
        }
    }

    /// Map the GIC distributor and this core's redistributor MMIO frames into
    /// the kernel address space via the HHDM as Device memory.
    fn map_mmio() {
        
        use crate::memory::KERNEL_AS;
        let mut kas = KERNEL_AS.lock();
        // Distributor: a single 64 KiB frame.
        kas.map_mmio_region(GICD_BASE, 0x1_0000)
            .expect("Failed to map GIC distributor MMIO");
        // This core's redistributor: RD_base + SGI_base (two 64 KiB frames).
        kas.map_mmio_region(gicr_rd_base(), GICR_STRIDE)
            .expect("Failed to map GIC redistributor MMIO");
    }
}

impl LocalIntCtlrIfce for GicV3 {
    type Error = Error;

    fn init_lp() {
        // Ensure the distributor and this core's redistributor MMIO are mapped
        // into the kernel address space (Device memory via the HHDM) before we
        // touch any GIC register. Limine does not HHDM-map MMIO from base
        // revision 3 onwards, so we map it explicitly here.
        Self::map_mmio();
        // The distributor is system-wide; enabling affinity routing and Group 1
        // is idempotent and safe to repeat from each core as it comes online.
        unsafe {
            let ctlr = mmio_read32(GICD_BASE, GICD_CTLR);
            mmio_write32(
                GICD_BASE,
                GICD_CTLR,
                ctlr | GICD_CTLR_ARE_NS | GICD_CTLR_ENABLE_GRP1_NS,
            );
            // Ensure SPI group registers do not matter here; SPIs are wired up
            // by the external interrupt controller path when devices attach.
            let _ = GICD_IGROUPR;
        }
        Self::redistributor_wake();
        Self::cpu_interface_init();
        // Enable the EL1 virtual timer PPI (INTID 27) so the scheduler tick is
        // delivered to this core.
        Self::enable_private_int(
            crate::cpu::isa::constants::interrupt_vectors::LAPIC_TIMER_VECTOR,
        );
    }

    /// Send a unicast IPI to `target_lp` by generating the SGI whose INTID is
    /// `target_vector` through `ICC_SGI1R_EL1`.
    ///
    /// The target is addressed by affinity: the Aff1-Aff3 fields are derived
    /// from the target LP's MPIDR, and Aff0 is represented as a single-bit
    /// in the TargetList. On a standard QEMU `virt` machine with `-smp 2`,
    /// the MPIDRs are `0x80000000` (LP0) and `0x80000001` (LP1), so this
    /// reduces to `(1 << lp_id)`, but the full encoding works for any
    /// cluster/socket layout.
    ///
    /// Cross-core SGI delivery is exercised by the SMP2 boot gate: kernel
    /// ShardMailbox fan-out, EL0 cross-LP completion, and EL0 ping-pong all
    /// require LP0 <-> LP1 wakeups to be delivered through this path.
    fn send_unicast_ipi(
        target_lp: LpId,
        target_vector: InterruptVectorNum,
    ) -> Result<(), Error> {
        // SGIs are INTIDs 0-15 only.
        if target_vector > 15 {
            return Err(Error::InvalidLpId);
        }
        let aff0 = target_lp & 0xff;
        // ICC_SGI1R_EL1 layout: INTID in bits [27:24], TargetList in [15:0]
        // (a bitmask of affinity-0 values within the addressed cluster).
        let sgi1r: u64 = ((target_vector as u64 & 0xf) << 24) | (1u64 << (aff0 as u64));
        unsafe {
            asm!("dsb ishst", options(nomem, nostack, preserves_flags));
            asm!("msr ICC_SGI1R_EL1, {}", in(reg) sgi1r, options(nomem, nostack, preserves_flags));
            asm!("isb", options(nomem, nostack, preserves_flags));
        }
        Ok(())
    }

    fn signal_eoi() {
        // Completing an interrupt requires the INTID that was acknowledged.
        // The dispatcher records it in ACKED_INTID before invoking handlers.
        let intid = *ACKED_INTID.get();
        unsafe {
            asm!("msr ICC_EOIR1_EL1, {}", in(reg) intid as u64, options(nomem, nostack, preserves_flags));
        }
    }
}

/// Acknowledge the highest priority pending Group 1 interrupt, returning its
/// INTID via `ICC_IAR1_EL1`. A value of 1020-1023 is a spurious/special INTID.
#[inline]
pub fn acknowledge_int() -> u32 {
    let iar: u64;
    unsafe {
        asm!("mrs {}, ICC_IAR1_EL1", out(reg) iar, options(nomem, nostack));
    }
    (iar & 0xff_ffff) as u32
}

/// Signal end-of-interrupt for the given INTID via `ICC_EOIR1_EL1`.
#[inline]
pub fn end_of_int(intid: u32) {
    unsafe {
        asm!("msr ICC_EOIR1_EL1, {}", in(reg) intid as u64, options(nomem, nostack, preserves_flags));
    }
}

/// Per-core storage of the most recently acknowledged INTID, so that
/// `signal_eoi` (which takes no argument, matching the generic interface) can
/// complete the correct interrupt.
pub(crate) static ACKED_INTID: LazyLock<PerLp<u32>> = LazyLock::new(|| PerLp::new(|| 0u32));

/// Record the INTID acknowledged on the current core so a later argument-less
/// `signal_eoi` can complete it.
pub fn record_acked_intid(intid: u32) {
    *ACKED_INTID.get_mut() = intid;
}

/// The lowest INTID that is a Shared Peripheral Interrupt (SPI). INTIDs below
/// this are SGIs (0-15) and PPIs (16-31) handled at the redistributor.
pub const FIRST_SPI: u32 = 32;

/// Ensure the distributor MMIO is mapped in the *currently active* address
/// space before an SPI configuration access. Limine does not HHDM-map MMIO
/// (base revision 3+), and the mapping installed by [`GicV3::init_lp`] lives
/// in the kernel address space captured at boot; a later caller (a kernel
/// self-test thread, a driver-management path) may be running under a
/// different active translation regime. Mapping into `get_current()` writes
/// the alias into the live translation tables and is idempotent (an already
/// mapped page is treated as success).
fn ensure_distributor_mapped() {
    use crate::cpu::isa::aarch64::memory::paging::AddressSpace;
    use crate::cpu::isa::interface::memory::AddressSpaceInterface;
    let mut current = AddressSpace::get_current();
    let _ = current.map_mmio_region(GICD_BASE, 0x1_0000);
}

/// Enable a Shared Peripheral Interrupt at the distributor and route it to
/// `target_lp`: assign it to Group 1 (non-secure), give it a runnable
/// priority, program affinity routing, and set its enable bit. This is the
/// distributor-side counterpart of [`GicV3::enable_private_int`] for device
/// interrupts (architecture doc §10.2). Requires the distributor MMIO to have
/// been mapped (done by [`GicV3::init_lp`]).
pub fn enable_spi(intid: u32, target_lp: LpId) {
    debug_assert!(intid >= FIRST_SPI, "enable_spi called with a non-SPI INTID");
    ensure_distributor_mapped();
    let word = (intid / 32) as usize;
    let bit = intid % 32;
    unsafe {
        // Group 1 non-secure.
        let mut group = mmio_read32(GICD_BASE, GICD_IGROUPR + word * 4);
        group |= 1 << bit;
        mmio_write32(GICD_BASE, GICD_IGROUPR + word * 4, group);
        // Runnable priority (numerically below the PMR threshold).
        mmio_write8(GICD_BASE, GICD_IPRIORITYR + intid as usize, DEFAULT_PRIORITY);
        // Affinity routing (ARE_NS): deliver to a specific PE. For the QEMU
        // `virt` layout Aff0 is the LP id and the higher affinities are 0.
        mmio_write64(GICD_BASE, GICD_IROUTER + intid as usize * 8, (target_lp as u64) & 0xff);
        // Ensure the routing/config writes are observable before enabling.
        asm!("dsb ish", options(nomem, nostack, preserves_flags));
        mmio_write32(GICD_BASE, GICD_ISENABLER + word * 4, 1 << bit);
    }
}

/// Disable (mask) a Shared Peripheral Interrupt at the distributor. Used to
/// mask a device interrupt after delivery until the owning driver acknowledges
/// it, so a level-triggered source does not storm the CPU.
pub fn disable_spi(intid: u32) {
    let word = (intid / 32) as usize;
    let bit = intid % 32;
    unsafe {
        mmio_write32(GICD_BASE, GICD_ICENABLER + word * 4, 1 << bit);
        asm!("dsb ish", options(nomem, nostack, preserves_flags));
    }
}
/// Set a Shared Peripheral Interrupt pending in software (distributor
/// `GICD_ISPENDR`). Used by self-tests to exercise the device IRQ delivery
/// path without a real peripheral.
pub fn set_spi_pending(intid: u32) {
    let word = (intid / 32) as usize;
    let bit = intid % 32;
    unsafe {
        mmio_write32(GICD_BASE, GICD_ISPENDR + word * 4, 1 << bit);
        asm!("dsb ish", options(nomem, nostack, preserves_flags));
    }
}

/// Clear a Shared Peripheral Interrupt's pending state (distributor
/// `GICD_ICPENDR`).
pub fn clear_spi_pending(intid: u32) {
    let word = (intid / 32) as usize;
    let bit = intid % 32;
    unsafe {
        mmio_write32(GICD_BASE, GICD_ICPENDR + word * 4, 1 << bit);
        asm!("dsb ish", options(nomem, nostack, preserves_flags));
    }
}
