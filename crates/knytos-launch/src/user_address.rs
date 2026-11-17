//! Shared virtual-address contract for the current four-level user page tables.
//! Never canonicalize an untrusted integer before validating this range.

pub const PAGE_SIZE: usize = 4096;
/// Both architectures use the lower, non-sign-extending 47-bit user window.
/// Wider hardware addressing does not widen the current four-level ABI.
pub const USER_END: usize = 1usize << 47;

pub fn valid_range(start: usize, len: usize) -> bool {
    start >= PAGE_SIZE && len != 0 && start.checked_add(len).is_some_and(|end| end <= USER_END)
}

/// Exclusive per-domain stack arena, including one guard page per slot.
pub const STACK_BASE: usize = 0x0100_0000;
pub const STACK_STRIDE: usize = (crate::MAX_USER_STACK_PAGES + 1) * PAGE_SIZE;
pub const STACK_END: usize = STACK_BASE + crate::MAX_USER_THREADS * STACK_STRIDE;

pub fn overlaps_stack(start: usize, len: usize) -> bool {
    start.checked_add(len).is_none_or(|end| start < STACK_END && STACK_BASE < end)
}

/// Application-selected mappings cannot occupy current or future stacks.
pub fn valid_application_pages(start: usize, pages: usize) -> bool {
    valid_pages(start, pages)
        && pages.checked_mul(PAGE_SIZE).is_some_and(|len| !overlaps_stack(start, len))
}

pub fn valid_pages(start: usize, pages: usize) -> bool {
    start.is_multiple_of(PAGE_SIZE)
        && pages.checked_mul(PAGE_SIZE).is_some_and(|len| valid_range(start, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applications_cannot_map_stacks_or_bridge_their_guards() {
        assert!(valid_application_pages(STACK_BASE - PAGE_SIZE, 1));
        assert!(!valid_application_pages(STACK_BASE - PAGE_SIZE, 2));
        assert!(!valid_application_pages(STACK_BASE, 1));
        assert!(!valid_application_pages(STACK_END - PAGE_SIZE, 1));
        assert!(valid_application_pages(STACK_END, 1));
        assert!(!valid_application_pages(PAGE_SIZE, usize::MAX));
        for slot in 0..crate::MAX_USER_THREADS {
            assert!(!valid_application_pages(STACK_BASE + slot * STACK_STRIDE, 1));
        }
    }

    #[test]
    fn mapping_ranges_reject_aliases_kernel_null_and_overflow() {
        assert!(valid_pages(PAGE_SIZE, 1));
        assert!(valid_pages(USER_END - PAGE_SIZE, 1));
        for base in [0, 1, USER_END, 1usize << 48, 0xffff_8000_0000_0000, usize::MAX] {
            assert!(!valid_pages(base, 1));
        }
        assert!(!valid_pages(USER_END - PAGE_SIZE, 2));
        assert!(!valid_pages(PAGE_SIZE, usize::MAX));
        assert!(!valid_pages(PAGE_SIZE, 0));
        assert!(!valid_range(usize::MAX - PAGE_SIZE, PAGE_SIZE * 2));
    }
}
