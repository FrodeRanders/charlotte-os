//! FIPS 180-4 SHA-256, hand-rolled so the kernel and the EL0 services share
//! one allocation-free implementation. Owned state and compression scratch use
//! zeroize because HMAC callers also hash key material here.
//!
//! Used by the ELF signature-note verification (`super::signature_note`):
//! the artifact hash that a cluster signature covers.

use zeroize::{
    Zeroize,
    ZeroizeOnDrop,
    Zeroizing,
};

/// The SHA-256 state: 8 state words plus the 64-byte input block buffer.
///
/// `update` accepts arbitrarily split input; `finalize` pads per FIPS 180-4
/// (the 0x80 byte, zeros, and the 64-bit bit length) and returns the digest.
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    total_len: u64,
}

impl Zeroize for Sha256 {
    fn zeroize(&mut self) {
        self.state.zeroize();
        self.buffer.zeroize();
        self.buffer_len.zeroize();
        self.total_len.zeroize();
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for Sha256 {}

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub const fn new() -> Self {
        Self {
            state: [
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ],
            buffer: [0u8; 64],
            buffer_len: 0,
            total_len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);
        if self.buffer_len != 0 {
            let take = core::cmp::min(64 - self.buffer_len, data.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&data[..take]);
            self.buffer_len += take;
            data = &data[take..];
            if self.buffer_len == 64 {
                Self::compress(&mut self.state, &self.buffer);
                self.buffer.zeroize();
                self.buffer_len = 0;
            }
        }
        while data.len() >= 64 {
            Self::compress(&mut self.state, data[..64].try_into().unwrap());
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    /// Consume and erase this state, returning the digest to its caller. A
    /// secret-derived digest needs its own zeroizing owner at that boundary.
    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buffer_len != 56 {
            self.update(&[0x00]);
        }
        self.update(&bit_len.to_be_bytes());
        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = Zeroizing::new([0u32; 64]);
        for (index, word) in w.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes(block[index * 4..index * 4 + 4].try_into().unwrap());
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16].wrapping_add(s0).wrapping_add(w[index - 7]).wrapping_add(s1);
        }

        let mut work = Zeroizing::new(*state);

        for index in 0..64 {
            let s1 = work[4].rotate_right(6) ^ work[4].rotate_right(11) ^ work[4].rotate_right(25);
            let ch = (work[4] & work[5]) ^ (!work[4] & work[6]);
            let temp1 = work[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = work[0].rotate_right(2) ^ work[0].rotate_right(13) ^ work[0].rotate_right(22);
            let maj = (work[0] & work[1]) ^ (work[0] & work[2]) ^ (work[1] & work[2]);
            let temp2 = s0.wrapping_add(maj);
            work[7] = work[6];
            work[6] = work[5];
            work[5] = work[4];
            work[4] = work[3].wrapping_add(temp1);
            work[3] = work[2];
            work[2] = work[1];
            work[1] = work[0];
            work[0] = temp1.wrapping_add(temp2);
        }

        for (state, work) in state.iter_mut().zip(work.iter()) {
            *state = state.wrapping_add(*work);
        }
    }
}

/// Digest of a whole byte string.
pub fn digest(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()
}

/// HMAC-SHA-256 with owned, erased key normalization, pads and intermediate
/// digests. The result keeps its zeroizing owner through further derivation.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut block = Zeroizing::new([0u8; 64]);
    if key.len() > block.len() {
        let hashed_key = Zeroizing::new(digest(key));
        block[..32].copy_from_slice(&hashed_key[..]);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = Zeroizing::new([0x36u8; 64]);
    let mut outer_pad = Zeroizing::new([0x5cu8; 64]);
    for index in 0..block.len() {
        inner_pad[index] ^= block[index];
        outer_pad[index] ^= block[index];
    }
    let mut inner = Sha256::new();
    inner.update(&inner_pad[..]);
    inner.update(message);
    let inner_digest = Zeroizing::new(inner.finalize());
    let mut outer = Sha256::new();
    outer.update(&outer_pad[..]);
    outer.update(&inner_digest[..]);
    Zeroizing::new(outer.finalize())
}

/// Digest of `data` with the byte range `[skip_start, skip_start + skip_len)`
/// treated as zeros.
///
/// The signature note's descriptor bytes are the signature itself, so a
/// signature cannot cover them; both the signer and every verifier hash the
/// artifact with those bytes zeroed instead. This avoids copying the image.
pub fn digest_skipping(data: &[u8], skip_start: usize, skip_len: usize) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(&data[..skip_start.min(data.len())]);
    let zeros = [0u8; 64];
    let mut remaining = skip_len;
    while remaining > 0 {
        let take = core::cmp::min(remaining, zeros.len());
        hasher.update(&zeros[..take]);
        remaining -= take;
    }
    let after = skip_start.saturating_add(skip_len).min(data.len());
    hasher.update(&data[after..]);
    hasher.finalize()
}

/// Self-test vector (FIPS 180-4): `sha256("abc")`.
pub const TEST_VECTOR_ABC: [u8; 32] = [
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
];

#[cfg(test)]
mod erasure_tests {
    use core::{
        mem::MaybeUninit,
        ptr,
    };

    use super::*;

    #[test]
    fn unfinished_drop_erases_named_fields_in_retained_storage() {
        for len in [65, 1, 55, 63, 64, 127, 129, 0] {
            let input = [0x36; 129];
            let mut storage = MaybeUninit::new(Sha256::new());
            let value = storage.as_mut_ptr();
            // Retain the allocation after destruction and read only initialized
            // scalar/array fields. Never read padding, adopt a second owner or
            // inspect freed memory. The fixture bytes are public.
            unsafe {
                (*value).update(&input[..len]);
                let state = ptr::addr_of!((*value).state);
                let buffer = ptr::addr_of!((*value).buffer);
                let buffer_len = ptr::addr_of!((*value).buffer_len);
                let total_len = ptr::addr_of!((*value).total_len);
                ptr::drop_in_place(value);
                assert_eq!(ptr::read_volatile(state), [0; 8]);
                assert_eq!(ptr::read_volatile(buffer), [0; 64]);
                assert_eq!(ptr::read_volatile(buffer_len), 0);
                assert_eq!(ptr::read_volatile(total_len), 0);
            }
        }
    }
}
