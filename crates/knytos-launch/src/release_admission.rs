//! Complete release admission results captured by the catalog apply operation.

use crate::{
    release::MAX_DESCRIPTORS,
    sha256,
};

pub const MAGIC: &[u8; 8] = b"CRLADM01";
pub const HEADER_LEN: usize = 56;
pub const MAX_LEN: usize = HEADER_LEN + MAX_DESCRIPTORS * 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    release_digest: [u8; 32],
    generation: u64,
    count: usize,
    components: [u64; MAX_DESCRIPTORS],
}

impl Receipt {
    pub fn new(envelope: &[u8], generation: u64, components: &[u64]) -> Option<Self> {
        if crate::release::decode(envelope)?.descriptors().count() != components.len()
            || !valid_generation(generation)
            || components.is_empty()
            || components.len() > MAX_DESCRIPTORS
            || components.iter().any(|value| !valid_generation(*value))
        {
            return None;
        }
        let mut receipt = Self {
            release_digest: sha256::digest(envelope),
            generation,
            count: components.len(),
            components: [0; MAX_DESCRIPTORS],
        };
        receipt.components[..components.len()].copy_from_slice(components);
        Some(receipt)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn digest(&self) -> [u8; 32] {
        self.release_digest
    }

    pub fn components(&self) -> &[u64] {
        &self.components[..self.count]
    }

    pub fn encoded_len(&self) -> usize {
        HEADER_LEN + self.count * 8
    }

    pub fn matches(&self, envelope: &[u8]) -> bool {
        crate::release::decode(envelope).is_some_and(|release| {
            release.descriptors().count() == self.count
                && sha256::digest(envelope) == self.release_digest
        })
    }

    pub fn encode(&self, output: &mut [u8]) -> Option<usize> {
        let len = self.encoded_len();
        let output = output.get_mut(..len)?;
        output.fill(0);
        output[..8].copy_from_slice(MAGIC);
        output[8..40].copy_from_slice(&self.release_digest);
        output[40..48].copy_from_slice(&self.generation.to_le_bytes());
        output[48..50].copy_from_slice(&(self.count as u16).to_le_bytes());
        for (bytes, generation) in
            output[HEADER_LEN..].as_chunks_mut::<8>().0.iter_mut().zip(self.components())
        {
            bytes.copy_from_slice(&generation.to_le_bytes());
        }
        Some(len)
    }

    pub fn decode(input: &[u8]) -> Option<Self> {
        if input.get(..8)? != MAGIC || input.get(50..HEADER_LEN)?.iter().any(|byte| *byte != 0) {
            return None;
        }
        let count = usize::from(u16::from_le_bytes(input.get(48..50)?.try_into().ok()?));
        if count == 0 || count > MAX_DESCRIPTORS || input.len() != HEADER_LEN + count * 8 {
            return None;
        }
        let generation = u64::from_le_bytes(input[40..48].try_into().ok()?);
        if !valid_generation(generation) {
            return None;
        }
        let mut receipt = Self {
            release_digest: input[8..40].try_into().ok()?,
            generation,
            count,
            components: [0; MAX_DESCRIPTORS],
        };
        for (slot, bytes) in receipt.components[..count]
            .iter_mut()
            .zip(input[HEADER_LEN..].as_chunks::<8>().0.iter())
        {
            *slot = u64::from_le_bytes(*bytes);
            if !valid_generation(*slot) {
                return None;
            }
        }
        Some(receipt)
    }
}

fn valid_generation(value: u64) -> bool {
    value > 0 && value <= i64::MAX as u64
}

/// Errors are exact negative scalar results; success requires the complete
/// current receipt. Earlier positive scalar successes are never accepted.
pub fn decode_result(bytes: &[u8]) -> Option<Result<Receipt, i64>> {
    if bytes.len() == 8 {
        let code = i64::from_le_bytes(bytes.try_into().ok()?);
        return (code < 0).then_some(Err(code));
    }
    Receipt::decode(bytes).map(Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(count: usize) -> Receipt {
        Receipt {
            release_digest: [7; 32],
            generation: 2,
            count,
            components: core::array::from_fn(|index| {
                if index < count {
                    3
                } else {
                    0
                }
            }),
        }
    }
    #[test]
    fn exact_header_bounds_reserved_and_generation_fields() {
        for count in [1, MAX_DESCRIPTORS] {
            let expected = fixture(count);
            let mut bytes = [0; MAX_LEN + 1];
            let len = expected.encode(&mut bytes).unwrap();
            assert_eq!(Receipt::decode(&bytes[..len]), Some(expected));
            for end in 0..len {
                assert!(Receipt::decode(&bytes[..end]).is_none());
            }
            assert!(Receipt::decode(&bytes[..len + 1]).is_none());
            for offset in [0, 50, 55] {
                let mut bad = bytes;
                bad[offset] ^= 1;
                assert!(Receipt::decode(&bad[..len]).is_none());
            }
            for offset in [40, HEADER_LEN] {
                for generation in [0, i64::MAX as u64 + 1, u64::MAX] {
                    let mut bad = bytes;
                    bad[offset..offset + 8].copy_from_slice(&generation.to_le_bytes());
                    assert!(Receipt::decode(&bad[..len]).is_none());
                }
            }
            for count in [0, MAX_DESCRIPTORS as u16 + 1, u16::MAX] {
                let mut bad = bytes;
                bad[48..50].copy_from_slice(&count.to_le_bytes());
                assert!(Receipt::decode(&bad[..len]).is_none());
            }
        }
    }
    #[test]
    fn scalar_success_and_extended_errors_reject() {
        for value in [0, 1, i64::MAX] {
            assert!(decode_result(&value.to_le_bytes()).is_none());
        }
        assert_eq!(decode_result(&(-7i64).to_le_bytes()), Some(Err(-7)));
        let mut bytes = [0; 9];
        bytes[..8].copy_from_slice(&(-7i64).to_le_bytes());
        assert!(decode_result(&bytes).is_none());
    }
}
