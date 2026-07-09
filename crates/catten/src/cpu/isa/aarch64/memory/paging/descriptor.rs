//! # AArch64 Translation Table Descriptor (VMSAv8-64, 4 KiB granule)
//!
//! This module models the stage 1 translation table descriptors used by the
//! kernel. We use the 4 KiB translation granule with a 48-bit address space,
//! giving a four level table hierarchy (levels 0-3) with 512 entries each,
//! matching the layout Limine configures before handing control to the kernel.
//!
//! There are two descriptor shapes that matter to us:
//! - **Table descriptors** (levels 0-2): point at the next level table. Bits
//!   `[1:0] = 0b11`.
//! - **Block descriptors** (levels 1-2): map a large/huge page directly. Bits
//!   `[1:0] = 0b01`.
//! - **Page descriptors** (level 3): map a 4 KiB page. Bits `[1:0] = 0b11`.
//!
//! Note that at level 3 the "page" encoding reuses the `0b11` value that means
//! "table" at the upper levels, while a `0b01` block encoding is *not* valid at
//! level 3. This asymmetry is handled by the walker, which knows the level of
//! each descriptor it inspects.
//!
//! See the ARM Architecture Reference Manual (ARM ARM), chapter D8
//! "The AArch64 Virtual Memory System Architecture".

use crate::cpu::isa::aarch64::memory::address::paddr::PAddr;

/// Descriptor bit `[0]`: the descriptor is valid.
const VALID: u64 = 1 << 0;
/// Descriptor bit `[1]`: distinguishes table/page (1) from block (0) at levels
/// where both are possible. Combined with [`VALID`] this yields the `0b11`
/// table/page encoding and the `0b01` block encoding.
const TABLE_OR_PAGE: u64 = 1 << 1;

/// Lower attribute: `AttrIndx[2:0]` selects a `MAIR_EL1` attribute field.
const ATTR_INDX_SHIFT: u64 = 2;
/// Lower attribute: Access Flag. If this is clear when the descriptor is used
/// the CPU takes an Access Flag fault, so we always set it on live mappings.
const AF: u64 = 1 << 10;
/// Lower attribute: Shareability field `[9:8]`. `0b11` is inner-shareable,
/// which is what we want for Normal cacheable memory on an SMP system.
const SH_INNER: u64 = 0b11 << 8;
/// Lower attribute: `AP[2:1]` access permissions field `[7:6]`.
/// - `AP[2]` (bit 7): 0 = read/write, 1 = read-only.
/// - `AP[1]` (bit 6): 0 = EL1 only, 1 = EL0 (user) accessible.
const AP_RO: u64 = 1 << 7;
const AP_EL0: u64 = 1 << 6;

/// Upper attribute: Privileged Execute Never.
const PXN: u64 = 1 << 53;
/// Upper attribute: Unprivileged Execute Never.
const UXN: u64 = 1 << 54;

/// The output address occupies bits `[47:12]` for the 4 KiB granule with up to
/// 48 bits of physical address. Larger physical address support (FEAT_LPA) uses
/// additional bits which we do not currently target on the `virt` machine.
const OUTPUT_ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

/// `MAIR_EL1` attribute index for Normal, Write-Back cacheable memory. Limine
/// programs `MAIR_EL1` so that index 0 is "Normal WB RW-allocate", so we reuse
/// that index for all ordinary kernel and user memory.
pub const MAIR_IDX_NORMAL: u64 = 0;
/// `MAIR_EL1` attribute index that Limine programs for the framebuffer. We do
/// not currently emit device mappings through this path; MMIO handling will add
/// a dedicated Device-nGnRnE index when the device layer needs it.
pub const MAIR_IDX_FRAMEBUFFER: u64 = 1;
/// `MAIR_EL1` attribute index used for strongly-ordered device memory
/// (Device-nGnRnE). Limine leaves attribute indices 2-7 set to `0x00`, which is
/// precisely the Device-nGnRnE encoding, so index 2 is usable for MMIO without
/// having to reprogram the attributes referenced by existing mappings.
pub const MAIR_IDX_DEVICE: u64 = 2;

/// A single translation table descriptor.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Descriptor(u64);

impl Descriptor {
    pub const fn empty() -> Self {
        Descriptor(0)
    }

    /// Whether the descriptor's valid bit is set.
    pub fn is_valid(&self) -> bool {
        self.0 & VALID != 0
    }

    /// Whether this descriptor (at an upper level) refers to a next-level
    /// table. Only meaningful at levels 0-2; at level 3 the same bit pattern
    /// denotes a page, so callers must account for the level.
    pub fn is_table(&self) -> bool {
        self.0 & (VALID | TABLE_OR_PAGE) == (VALID | TABLE_OR_PAGE)
    }

    /// Whether this descriptor (at levels 1-2) is a block mapping.
    pub fn is_block(&self) -> bool {
        self.0 & (VALID | TABLE_OR_PAGE) == VALID
    }

    /// Build a table descriptor pointing at the given next-level table frame.
    pub fn new_table(next_table: PAddr) -> Self {
        Descriptor(
            (<PAddr as Into<u64>>::into(next_table) & OUTPUT_ADDR_MASK) | VALID | TABLE_OR_PAGE,
        )
    }

    /// Build a leaf descriptor (page at level 3, or block at levels 1-2)
    /// mapping the given output frame with the supplied permissions.
    ///
    /// `is_page_level` selects the level 3 page encoding (`0b11`) when true and
    /// the block encoding (`0b01`) when false.
    pub fn new_leaf(
        frame: PAddr,
        writable: bool,
        user_accessible: bool,
        no_execute: bool,
        mair_index: u64,
        is_page_level: bool,
    ) -> Self {
        let mut bits = (<PAddr as Into<u64>>::into(frame) & OUTPUT_ADDR_MASK)
            | VALID
            | AF
            | SH_INNER
            | ((mair_index & 0b111) << ATTR_INDX_SHIFT);
        if is_page_level {
            bits |= TABLE_OR_PAGE;
        }
        if !writable {
            bits |= AP_RO;
        }
        if user_accessible {
            bits |= AP_EL0;
        }
        if no_execute {
            // Mark the mapping non-executable at both privilege levels. User
            // executable pages still require PXN so the kernel cannot execute
            // user code, so we set PXN unconditionally for non-kernel-code
            // mappings and only clear UXN for user-executable pages.
            bits |= UXN | PXN;
        } else if user_accessible {
            // Executable user page: allow EL0 execution but forbid EL1
            // execution of user memory (privileged execute never).
            bits |= PXN;
        }
        Descriptor(bits)
    }

    /// Extract the output address (next-level table or mapped frame).
    pub fn frame(&self) -> PAddr {
        PAddr::from(self.0 & OUTPUT_ADDR_MASK)
    }

    /// Clear the descriptor, marking it invalid.
    pub fn clear(&mut self) {
        self.0 = 0;
    }
}
