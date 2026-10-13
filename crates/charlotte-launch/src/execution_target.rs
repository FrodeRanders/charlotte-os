//! Signed executable requirements and kernel-reported node capabilities.

/// Bump whenever the userspace/kernel register or runtime contract changes.
/// Eligibility requires exact equality; there is no older-ABI decoder.
pub const RUNTIME_ABI: u32 = 1;
pub const ENCODED_LEN: usize = 16;

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Architecture {
    X86_64 = 62,
    Aarch64 = 183,
}

impl Architecture {
    pub const fn from_machine(machine: u16) -> Option<Self> {
        match machine {
            62 => Some(Self::X86_64),
            183 => Some(Self::Aarch64),
            _ => None,
        }
    }
}

/// Integer instruction extensions that need no additional context state.
pub const AARCH64_LSE: u64 = 1;
pub const AARCH64_CRC32: u64 = 2;
pub const X86_CX16: u64 = 1;
pub const X86_POPCNT: u64 = 2;
pub const KNOWN_FEATURES: u64 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ExecutionTarget {
    pub architecture: Architecture,
    pub runtime_abi: u32,
    pub features: u64,
}

impl ExecutionTarget {
    pub const fn baseline(architecture: Architecture) -> Self {
        Self {
            architecture,
            runtime_abi: RUNTIME_ABI,
            features: 0,
        }
    }

    pub const fn valid(self) -> bool {
        self.runtime_abi != 0 && self.features & !KNOWN_FEATURES == 0
    }

    /// `self` describes available execution capabilities, `required` demands.
    pub const fn supports(self, required: Self) -> bool {
        self.valid()
            && required.valid()
            && self.architecture as u16 == required.architecture as u16
            && self.runtime_abi == required.runtime_abi
            && self.features & required.features == required.features
    }

    pub fn from_pressure(packed: u64, features: u64) -> Option<Self> {
        if packed & 0xffff_0000 != 0 {
            return None;
        }
        let value = Self {
            architecture: Architecture::from_machine(packed as u16)?,
            runtime_abi: (packed >> 32) as u32,
            features,
        };
        value.valid().then_some(value)
    }

    pub fn encode(self) -> Option<[u8; ENCODED_LEN]> {
        if !self.valid() {
            return None;
        }
        let mut bytes = [0; ENCODED_LEN];
        bytes[..2].copy_from_slice(&(self.architecture as u16).to_le_bytes());
        bytes[4..8].copy_from_slice(&self.runtime_abi.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.features.to_le_bytes());
        Some(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != ENCODED_LEN || bytes[2..4] != [0, 0] {
            return None;
        }
        let target = Self {
            architecture: Architecture::from_machine(u16::from_le_bytes(
                bytes[..2].try_into().ok()?,
            ))?,
            runtime_abi: u32::from_le_bytes(bytes[4..8].try_into().ok()?),
            features: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
        };
        target.valid().then_some(target)
    }

    pub fn elf_architecture(image: &[u8]) -> Option<Architecture> {
        if image.len() < 64 || image[..4] != *b"\x7fELF" || image[4] != 2 || image[5] != 1 {
            return None;
        }
        Architecture::from_machine(u16::from_le_bytes(image[18..20].try_into().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_compatibility_requires_architecture_abi_and_all_features() {
        let mut node = ExecutionTarget::baseline(Architecture::Aarch64);
        let mut demand = node;
        assert!(node.supports(demand));
        demand.features = AARCH64_LSE;
        assert!(!node.supports(demand));
        node.features = KNOWN_FEATURES;
        assert!(node.supports(demand));
        demand.architecture = Architecture::X86_64;
        assert!(!node.supports(demand));
        demand.architecture = Architecture::Aarch64;
        demand.runtime_abi += 1;
        assert!(!node.supports(demand));
    }

    #[test]
    fn target_codec_rejects_unknown_architecture_features_and_noncanonical_fields() {
        let target = ExecutionTarget::baseline(Architecture::X86_64);
        let bytes = target.encode().unwrap();
        assert_eq!(ExecutionTarget::decode(&bytes), Some(target));
        for index in [0, 2, 4, 15] {
            let mut invalid = bytes;
            invalid[index] = if index == 4 {
                0
            } else {
                255
            };
            assert!(ExecutionTarget::decode(&invalid).is_none());
        }
        assert!(ExecutionTarget::decode(&bytes[..15]).is_none());
    }
}
