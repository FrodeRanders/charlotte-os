//! Minimal GICv3 Interrupt Translation Service (ITS) driver.
//!
//! The ITS lets PCI MSIs be delivered as LPIs: the guest maps a
//! `(device, event)` pair to an LPI with `MAPTI`, and a device write to
//! `GITS_TRANSLATER` raises the LPI. QEMU derives the device ID from the PCI
//! Requester ID of the write and treats the write *data* as the event ID, so
//! the MSI message for a device is simply `{ address: GITS_TRANSLATER,
//! data: event_id }`.
//!
//! QEMU `sbsa-ref` exposes an ITS at `0x44081000`; the `virt` board does not
//! (it uses a GICv2m frame instead, see [`super::allocate_v2m_msi`]).
//!
//! The tables and command queue each occupy one 4 KiB frame: a direct device
//! table of 512 entries (8 bytes each), a direct collection table, a 128-slot
//! command queue, and one interrupt translation table. One device (the NVMe)
//! is mapped with event 0 to LPI `LPI_BASE`.

use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use spin::{
    LazyLock,
    Mutex,
};

use super::lpi::LPI_BASE;
use crate::{
    cpu::isa::{
        aarch64::memory::{
            address::paddr::PAddr,
            paging::PAGE_SIZE,
        },
        interface::memory::address::PhysicalAddress,
    },
    memory::{
        KERNEL_AS,
        PHYSICAL_FRAME_ALLOCATOR,
    },
};

// ITS register offsets (relative to the ITS MMIO base).
const GITS_CTLR: usize = 0x0000;
const GITS_CBASER: usize = 0x0080;
const GITS_CWRITER: usize = 0x0088;
const GITS_CREADR: usize = 0x0090;
const GITS_BASER0: usize = 0x0100;
const GITS_BASER1: usize = 0x0108;
/// The MSI translation window sits 64 KiB above the ITS base; QEMU accepts a
/// write at `GITS_TRANSLATER` (offset 0x40) within it, deriving the device ID
/// from the PCI Requester ID and the event ID from the write data.
const GITS_TRANSLATER_WINDOW: usize = 0x10000;
const GITS_TRANSLATER: usize = 0x0040;

const GITS_CTLR_ENABLE: u32 = 1 << 0;

// QEMU's GITS_CBASER / GITS_BASER layouts: PhysicalAddress at bits [51:12]
// (BASER at [47:12], preserved read-only fields aside) and **Valid at bit 63**.
// GITS_CWRITER / GITS_CREADR carry their offset at bits [5:19] as the byte
// offset >> 5 (i.e. a command index).
const GITS_CBASER_VALID: u64 = 1 << 63;
const GITS_CBASER_SIZE: u64 = 0; // 1 page -> 128 command slots
const GITS_CBASER_PHYADDR: u64 = 0x0000_ffff_ffff_f000; // bits [51:12]

const GITS_BASER_VALID: u64 = 1 << 63;
const GITS_BASER_SIZE: u64 = 0; // 1 page
const GITS_BASER_PAGESIZE_4K: u64 = 0; // bits [9:8] = 0
const GITS_BASER_PHYADDR: u64 = 0x0000_ffff_ffff_f000; // bits [47:12]

// Command queue: one 4 KiB frame holds 128 32-byte command slots.
const GITS_CMDQ_ENTRY_SIZE: usize = 32;
const GITS_CMDQ_NR_ENTRIES: usize = PAGE_SIZE / GITS_CMDQ_ENTRY_SIZE;

// ITS commands (ARM GICv3 command queue encodings, as used by QEMU).
const GITS_CMD_MAPD: u64 = 0x08;
const GITS_CMD_MAPC: u64 = 0x09;
const GITS_CMD_SYNC: u64 = 0x05;
const GITS_CMD_MAPTI: u64 = 0x0a;
const GITS_CMD_INVALL: u64 = 0x0d;

// Command field masks (per ARM GICv3 spec, as encoded by Linux's ITS driver).
const CMD_EVENTID_MASK: u64 = 0xffff_ffff; // word 1, bits [31:0]
const CMD_SIZE_MASK: u64 = 0x1f; // word 1, bits [4:0]
const CMD_ITT_MASK: u64 = 0x0000_ffff_ffff_ff00; // word 2, bits [51:8]
const CMD_COLLECTION_MASK: u64 = 0xffff; // word 2, bits [15:0]
const CMD_VALID: u64 = 1 << 63; // word 2, bit 63

/// Number of event IDs per device: `1 << (size + 1)`. Two slots cover event 0.
const ITS_EVENT_SIZE: u64 = 0;

/// The collection used for all physical LPIs; QEMU treats its target as a
/// processor number, and 0 addresses logical processor 0.
const ITS_COLLECTION: u16 = 0;

struct ItsState {
    base: usize,
    cmdq: PAddr,
    /// Kept alive so the ITS's device table mapping stays resident.
    #[allow(dead_code)]
    device_table: PAddr,
    /// Kept alive so the ITS's collection table mapping stays resident.
    #[allow(dead_code)]
    collection_table: PAddr,
    itt: PAddr,
    cmd_write: usize,
}

static ITS_STATE: LazyLock<Mutex<Option<ItsState>>> = LazyLock::new(|| Mutex::new(None));
/// First allocated device ID, used to size the (single) NVMe ITE.
static NEXT_LPI: AtomicU32 = AtomicU32::new(LPI_BASE);

unsafe fn mmio_read32(base: usize, offset: usize) -> u32 {
    unsafe {
        core::ptr::read_volatile(PAddr::from(base as u64).into_hhdm_ptr::<u32>().byte_add(offset))
    }
}

unsafe fn mmio_write32(base: usize, offset: usize, value: u32) {
    unsafe {
        core::ptr::write_volatile(
            PAddr::from(base as u64).into_hhdm_mut::<u32>().byte_add(offset),
            value,
        )
    }
}

unsafe fn mmio_write64(base: usize, offset: usize, value: u64) {
    unsafe {
        core::ptr::write_volatile(
            PAddr::from(base as u64).into_hhdm_mut::<u64>().byte_add(offset),
            value,
        )
    }
}

/// The ITS MMIO base, discovered from the ACPI MADT (a GIC ITS entry), if the
/// platform exposes one.
pub fn its_base() -> Option<usize> {
    #[cfg(feature = "acpi")]
    {
        crate::environment::acpi::sdt::discovery::madt_its_base().map(|b| b as usize)
    }
    #[cfg(not(feature = "acpi"))]
    {
        None
    }
}

fn alloc_zeroed_frame() -> PAddr {
    let frame = PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame().expect("ITS table frame");
    unsafe { core::ptr::write_bytes(frame.into_hhdm_mut::<u8>(), 0, PAGE_SIZE) };
    frame
}

fn map_mmio(base: usize) {
    let mut kas = KERNEL_AS.lock();
    let _ = kas.map_mmio_region(base, 0x2_0000);
}

/// Enable the ITS: map its MMIO, configure the command queue and the device /
/// collection tables, and enable it. Idempotent.
fn ensure_initialized() {
    let mut guard = ITS_STATE.lock();
    if guard.is_some() {
        return;
    }
    let Some(base) = its_base() else {
        return;
    };
    map_mmio(base);

    let cmdq = alloc_zeroed_frame();
    let device_table = alloc_zeroed_frame();
    let collection_table = alloc_zeroed_frame();
    let itt = alloc_zeroed_frame();

    unsafe {
        mmio_write32(base, GITS_CTLR, 0);
        mmio_write64(base, GITS_CBASER, 0);

        // Device table: 512 entries (8 bytes) in one 4 KiB frame, direct.
        let baser0 = (u64::from(device_table) & GITS_BASER_PHYADDR)
            | GITS_BASER_PAGESIZE_4K
            | GITS_BASER_SIZE
            | GITS_BASER_VALID;
        mmio_write64(base, GITS_BASER0, baser0);
        // Collection table: 512 entries (8 bytes) in one 4 KiB frame, direct.
        let baser1 = (u64::from(collection_table) & GITS_BASER_PHYADDR)
            | GITS_BASER_PAGESIZE_4K
            | GITS_BASER_SIZE
            | GITS_BASER_VALID;
        mmio_write64(base, GITS_BASER1, baser1);

        // Command queue: one 4 KiB frame, 128 slots.
        let cbaser = (u64::from(cmdq) & GITS_CBASER_PHYADDR) | GITS_CBASER_SIZE | GITS_CBASER_VALID;
        mmio_write64(base, GITS_CBASER, cbaser);

        mmio_write32(base, GITS_CTLR, GITS_CTLR_ENABLE);
    }

    *guard = Some(ItsState {
        base,
        cmdq,
        device_table,
        collection_table,
        itt,
        cmd_write: 0,
    });
}

fn encode_command(state: &mut ItsState, words: [u64; 4]) {
    let slot = state.cmd_write % GITS_CMDQ_NR_ENTRIES;
    let ptr = unsafe { state.cmdq.into_hhdm_mut::<u64>().add(slot * 4) };
    unsafe {
        core::ptr::write_volatile(ptr.add(0), words[0]);
        core::ptr::write_volatile(ptr.add(1), words[1]);
        core::ptr::write_volatile(ptr.add(2), words[2]);
        core::ptr::write_volatile(ptr.add(3), words[3]);
    }
    // Make the command writes observable to the ITS before posting the write
    // pointer.
    unsafe { core::arch::asm!("dsb ishst", options(nomem, nostack, preserves_flags)) };
    let next = (state.cmd_write + 1) % GITS_CMDQ_NR_ENTRIES;
    state.cmd_write = next;
    // The ITS command queue offset (GITS_CWRITER bits [5:19]) is the byte
    // offset of the next slot divided by 32, i.e. the command index.
    unsafe {
        mmio_write64(state.base, GITS_CWRITER, (next * GITS_CMDQ_ENTRY_SIZE) as u64);
    }
    // Wait for the ITS to consume the command (GITS_CREADR offset bits [5:19]).
    loop {
        let creadr = unsafe { mmio_read32(state.base, GITS_CREADR) };
        if (creadr as usize) >> 5 == next {
            break;
        }
        core::hint::spin_loop();
    }
}

fn mapd(state: &mut ItsState, device_id: u32) {
    let itt = u64::from(state.itt);
    let w0 = GITS_CMD_MAPD | ((device_id as u64) << 32);
    let w1 = ITS_EVENT_SIZE & CMD_SIZE_MASK;
    let w2 = (itt & CMD_ITT_MASK) | CMD_VALID;
    encode_command(state, [w0, w1, w2, 0]);
}

fn mapc(state: &mut ItsState) {
    let mut w2 = (ITS_COLLECTION as u64) & CMD_COLLECTION_MASK;
    // QEMU treats the MAPC target (word 2 bits [51:16]) as a processor number;
    // 0 addresses logical processor 0. w2's target field is already zero.
    w2 |= CMD_VALID;
    encode_command(state, [GITS_CMD_MAPC, 0, w2, 0]);
}

fn mapti(state: &mut ItsState, device_id: u32, event_id: u32, intid: u32) {
    let w0 = GITS_CMD_MAPTI | ((device_id as u64) << 32);
    let w1 = ((event_id as u64) & CMD_EVENTID_MASK) | ((intid as u64) << 32);
    let w2 = (ITS_COLLECTION as u64) & CMD_COLLECTION_MASK;
    encode_command(state, [w0, w1, w2, 0]);
}

fn invall(state: &mut ItsState) {
    encode_command(state, [GITS_CMD_INVALL, 0, 0, 0]);
    // Sync so the invalidation is visible before further commands.
    encode_command(state, [GITS_CMD_SYNC, 0, 0, 0]);
}

/// Allocate one MSI for `device_id` (the PCI Requester ID): map device + event
/// 0 to a fresh LPI via the ITS, configure the LPI in the redistributor's
/// property table, and return the message the device programs into MSI-X.
pub fn allocate_msi(device_id: u32) -> Option<
    crate::device_management::drivers::busses::pci_express::ecam::capabilities::standard::msi::MsiMessage,
>{
    use crate::device_management::drivers::busses::pci_express::ecam::capabilities::standard::msi::MsiMessage;

    ensure_initialized();
    let mut guard = ITS_STATE.lock();
    let state = guard.as_mut()?;
    if device_id as usize >= 512 {
        return None;
    }
    let intid = NEXT_LPI.fetch_add(1, Ordering::Relaxed);
    if intid < LPI_BASE {
        return None;
    }

    mapd(state, device_id);
    mapc(state);
    mapti(state, device_id, 0, intid);
    // Enable the LPI in the property table, then flush the redistributor's
    // config cache so the delivery path observes it.
    super::lpi::set_lpi_enabled(intid, true);
    invall(state);

    Some(MsiMessage {
        address: (state.base + GITS_TRANSLATER_WINDOW + GITS_TRANSLATER) as u64,
        data: 0,
        intid,
    })
}

/// Whether an ITS is present and initialized (MSI delivery is possible).
pub fn available() -> bool {
    its_base().is_some()
}
