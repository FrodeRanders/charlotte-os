//! # Page Table Entry

use spin::LazyLock;

use crate::cpu::isa::x86_64::memory::address::paddr::PAddr;

/// PTE component indexes and masks
const PRESENT_BIT_INDEX: u64 = 0;
const WRITABLE_BIT_INDEX: u64 = 1;
const USER_ACCESSIBLE_BIT_INDEX: u64 = 2;
const PAT_INDEX_0: u64 = 3;
const PAT_INDEX_1: u64 = 4;
const PAT_INDEX_2_STANDARD: u64 = 7; // only for PTEs pointing to a 4 KiB page
const PAT_INDEX_2_LARGE_HUGE: u64 = 12; // only for PTEs pointing to a 2 MiB or 1 GiB page
const MAX_PAT_INDEX: u8 = 0b111;
const ACCESSED_BIT_INDEX: u64 = 5;
const DIRTY_BIT_INDEX: u64 = 6;
const PAGE_SIZE_BIT_INDEX: u64 = 7; // only for PTEs pointing to a 2 MiB or 1 GiB page
const GLOBAL_BIT_INDEX: u64 = 8;

static FRAME_ADDR_MASK: LazyLock<u64> =
    LazyLock::new(|| 0xfffffffffffff000 & *super::super::address::PADDR_MASK as u64);
const EXECUTE_DISABLE_BIT_INDEX: u64 = 63;

/// The page table entry structure
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    fn pat_index_mask(high_bit_index: u64) -> u64 {
        (1 << PAT_INDEX_0) | (1 << PAT_INDEX_1) | (1 << high_bit_index)
    }

    fn decode_pat_index(&self, high_bit_index: u64) -> u8 {
        let low = (self.0 >> PAT_INDEX_0) & 1;
        let middle = ((self.0 >> PAT_INDEX_1) & 1) << 1;
        let high = ((self.0 >> high_bit_index) & 1) << 2;
        (low | middle | high) as u8
    }

    fn encode_pat_index(&mut self, pat_index: u8, high_bit_index: u64) {
        assert!(
            pat_index <= MAX_PAT_INDEX,
            "PAT index must fit in the three architectural PAT-selection bits"
        );

        let pat_index = u64::from(pat_index);
        let encoded = ((pat_index & 1) << PAT_INDEX_0)
            | (((pat_index >> 1) & 1) << PAT_INDEX_1)
            | (((pat_index >> 2) & 1) << high_bit_index);
        let mask = Self::pat_index_mask(high_bit_index);
        self.0 = (self.0 & !mask) | encoded;
    }

    pub fn new(
        present: bool,
        writable: bool,
        user_accessible: bool,
        pat_index: u8,
        global: bool,
        frame_addr: PAddr,
    ) -> Self {
        let mut pte = Self(0);
        pte.set_present(present)
            .set_writable(writable)
            .set_user_accessible(user_accessible)
            .set_pat_index_bits(pat_index)
            .set_global(global)
            .set_frame(frame_addr);
        pte
    }

    pub fn new_large_huge(
        present: bool,
        writable: bool,
        user_accessible: bool,
        pat_index: u8,
        global: bool,
        frame_addr: PAddr,
    ) -> Self {
        let mut pte = Self(0);
        pte.set_present(present)
            .set_writable(writable)
            .set_user_accessible(user_accessible)
            .set_pat_index_bits_large_huge(pat_index)
            .set_global(global)
            .set_frame(frame_addr)
            .set_page_size(true);
        pte
    }

    pub fn is_present(&self) -> bool {
        self.0 & (1 << PRESENT_BIT_INDEX) != 0
    }

    pub fn set_present(&mut self, present: bool) -> &mut Self {
        if present {
            self.0 |= 1 << PRESENT_BIT_INDEX;
        } else {
            self.0 &= !(1 << PRESENT_BIT_INDEX);
        }
        self
    }

    pub fn is_writable(&self) -> bool {
        self.0 & (1 << WRITABLE_BIT_INDEX) != 0
    }

    pub fn set_writable(&mut self, writable: bool) -> &mut Self {
        if writable {
            self.0 |= 1 << WRITABLE_BIT_INDEX;
        } else {
            self.0 &= !(1 << WRITABLE_BIT_INDEX);
        }
        self
    }

    pub fn is_user_accessible(&self) -> bool {
        self.0 & (1 << USER_ACCESSIBLE_BIT_INDEX) != 0
    }

    pub fn set_user_accessible(&mut self, user_accessible: bool) -> &mut Self {
        if user_accessible {
            self.0 |= 1 << USER_ACCESSIBLE_BIT_INDEX;
        } else {
            self.0 &= !(1 << USER_ACCESSIBLE_BIT_INDEX);
        }
        self
    }

    pub fn get_pat_index(&self) -> u8 {
        self.decode_pat_index(PAT_INDEX_2_STANDARD)
    }

    pub fn get_pat_index_large_huge(&self) -> u8 {
        self.decode_pat_index(PAT_INDEX_2_LARGE_HUGE)
    }

    pub fn set_pat_index_bits(&mut self, pat_index: u8) -> &mut Self {
        self.encode_pat_index(pat_index, PAT_INDEX_2_STANDARD);
        self
    }

    pub fn set_pat_index_bits_large_huge(&mut self, pat_index: u8) -> &mut Self {
        self.encode_pat_index(pat_index, PAT_INDEX_2_LARGE_HUGE);
        self
    }

    pub(crate) fn self_test_pat_encoding() {
        for expected in 0..=MAX_PAT_INDEX {
            let mut standard = Self(u64::MAX);
            let standard_unrelated = standard.0 & !Self::pat_index_mask(PAT_INDEX_2_STANDARD);
            standard.set_pat_index_bits(expected);
            assert_eq!(standard.get_pat_index(), expected);
            assert_eq!(
                standard.0 & !Self::pat_index_mask(PAT_INDEX_2_STANDARD),
                standard_unrelated,
                "standard-page PAT update changed unrelated PTE bits"
            );

            let mut large_huge = Self(u64::MAX);
            let large_huge_unrelated = large_huge.0 & !Self::pat_index_mask(PAT_INDEX_2_LARGE_HUGE);
            large_huge.set_pat_index_bits_large_huge(expected);
            assert_eq!(large_huge.get_pat_index_large_huge(), expected);
            assert_eq!(
                large_huge.0 & !Self::pat_index_mask(PAT_INDEX_2_LARGE_HUGE),
                large_huge_unrelated,
                "large/huge-page PAT update changed unrelated PTE bits"
            );
        }
    }

    pub fn is_accessed(&self) -> bool {
        self.0 & (1 << ACCESSED_BIT_INDEX) != 0
    }

    pub fn set_accessed(&mut self, accessed: bool) -> &mut Self {
        if accessed {
            self.0 |= 1 << ACCESSED_BIT_INDEX;
        } else {
            self.0 &= !(1 << ACCESSED_BIT_INDEX);
        }
        self
    }

    pub fn is_dirty(&self) -> bool {
        self.0 & (1 << DIRTY_BIT_INDEX) != 0
    }

    pub fn set_dirty(&mut self, dirty: bool) -> &mut Self {
        if dirty {
            self.0 |= 1 << DIRTY_BIT_INDEX;
        } else {
            self.0 &= !(1 << DIRTY_BIT_INDEX);
        }
        self
    }

    pub fn get_page_size(&self) -> bool {
        self.0 & (1 << PAGE_SIZE_BIT_INDEX) != 0
    }

    pub fn set_page_size(&mut self, page_size: bool) -> &mut Self {
        if page_size {
            self.0 |= 1 << PAGE_SIZE_BIT_INDEX;
        } else {
            self.0 &= !(1 << PAGE_SIZE_BIT_INDEX);
        }
        self
    }

    pub fn is_global(&self) -> bool {
        self.0 & (1 << GLOBAL_BIT_INDEX) != 0
    }

    pub fn set_global(&mut self, global: bool) -> &mut Self {
        if global {
            self.0 |= 1 << GLOBAL_BIT_INDEX;
        } else {
            self.0 &= !(1 << GLOBAL_BIT_INDEX);
        }
        self
    }

    pub fn try_get_frame(&self) -> Result<PAddr, super::super::Error> {
        Ok(PAddr::try_from((self.0 & *FRAME_ADDR_MASK) as usize)?)
    }

    pub fn set_frame(&mut self, frame: PAddr) -> &mut Self {
        self.0 =
            (self.0 & !*FRAME_ADDR_MASK) | ((<PAddr as Into<u64>>::into(frame)) & *FRAME_ADDR_MASK);
        self
    }

    pub fn is_execute_disabled(&self) -> bool {
        self.0 & (1 << EXECUTE_DISABLE_BIT_INDEX) != 0
    }

    pub fn set_execute_disabled(&mut self, execute_disabled: bool) -> &mut Self {
        if execute_disabled {
            self.0 |= 1 << EXECUTE_DISABLE_BIT_INDEX;
        } else {
            self.0 &= !(1 << EXECUTE_DISABLE_BIT_INDEX);
        }
        self
    }

    pub fn is_uncached(&self) -> bool {
        self.0 & (0b11 << PAT_INDEX_0) == 0
    }

    pub fn is_write_combining(&self) -> bool {
        ((self.0 & (0b11 << PAT_INDEX_0)) >> PAT_INDEX_0) == 0b01
    }
}
