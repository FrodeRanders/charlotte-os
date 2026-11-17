//! Hardware completion requirements at DMA backing retirement boundaries.

pub const fn vtd_draining_command(capabilities: u64) -> Option<u64> {
    if capabilities & (3 << 54) != 3 << 54 {
        return None;
    }
    Some((1 << 63) | (1 << 60) | (1 << 49) | (1 << 48))
}

pub const fn amd_completion_command(address: u64, epoch: u64) -> Option<[u64; 2]> {
    if address & 7 != 0 || address >> 52 != 0 || epoch == 0 {
        return None;
    }
    // Preserve Store Address[51:3], request coherent Store and strict queue
    // completion (f=1). The address is not shifted within the low 64-bit word.
    Some([(1 << 60) | address | 5, epoch])
}

pub const fn completion_matches(observed: u64, expected: u64) -> bool {
    expected != 0 && observed == expected
}

pub const fn smmu_queue_has_space(producer: u32, consumer: u32, entries: u32) -> bool {
    entries.is_power_of_two()
        && entries <= u32::MAX / 2
        && producer < entries * 2
        && consumer < entries * 2
        && producer != consumer ^ entries
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vtd_requires_both_drains_before_memory_reuse() {
        assert_eq!(vtd_draining_command(0), None);
        assert_eq!(vtd_draining_command(1 << 54), None);
        assert_eq!(vtd_draining_command(1 << 55), None);
        let command = vtd_draining_command(3 << 54).unwrap();
        assert_ne!(command & (1 << 49), 0);
        assert_ne!(command & (1 << 48), 0);
    }
    #[test]
    fn amd_completion_is_exact_and_contains_the_full_store_address() {
        let address = 0xabc_d123_4560;
        let command = amd_completion_command(address, 7).unwrap();
        assert_eq!(command[0] & 0x000f_ffff_ffff_fff8, address);
        assert_eq!(command[0] >> 60, 1);
        assert_eq!(command[0] & 7, 5);
        assert_eq!(command[1], 7);
        assert!(!completion_matches(6, 7));
        assert!(!completion_matches(0, 0));
        assert!(completion_matches(7, 7));
        assert_eq!(amd_completion_command(address + 1, 7), None);
        assert_eq!(amd_completion_command(1 << 52, 7), None);
    }
    #[test]
    fn command_timeout_does_not_allow_overwrite_of_unconsumed_smmu_work() {
        assert!(smmu_queue_has_space(0, 0, 256));
        assert!(smmu_queue_has_space(255, 0, 256));
        assert!(!smmu_queue_has_space(256, 0, 256));
        assert!(smmu_queue_has_space(256, 1, 256));
        assert!(!smmu_queue_has_space(0, 256, 256));
        assert!(!smmu_queue_has_space(0, 1 << 24, 256));
    }
}
