//! Fresh boot entropy and independently keyed ephemeral-port selection.
use charlotte_launch::sha256::hmac_sha256;
use zeroize::{
    Zeroize,
    Zeroizing,
};

const PORT_LABEL: &[u8] = b"CharlotteOS TCP/IP ephemeral port v1";

pub struct NetworkRandom {
    interface_seed: u64,
    port_key: [u8; 32],
    counter: u64,
}

impl NetworkRandom {
    /// No clock, MAC-address, fixed-seed or partial-fill fallback is allowed.
    pub fn initialize<E>(fill: impl FnOnce(&mut [u8]) -> Result<(), E>) -> Result<Self, E> {
        let mut entropy = Zeroizing::new([0; 40]);
        fill(&mut entropy[..])?;
        let interface_seed = u64::from_le_bytes(entropy[..8].try_into().unwrap());
        let port_key = entropy[8..].try_into().unwrap();
        Ok(Self {
            interface_seed,
            port_key,
            counter: 0,
        })
    }

    pub fn interface_seed(&self) -> u64 {
        self.interface_seed
    }

    pub fn ephemeral_port(&mut self) -> u16 {
        // The public domain label/counter need no secret allocation. The shared
        // helper erases hash state, key pads, intermediates and its result owner.
        let mut message = [0; PORT_LABEL.len() + core::mem::size_of::<u64>()];
        message[..PORT_LABEL.len()].copy_from_slice(PORT_LABEL);
        message[PORT_LABEL.len()..].copy_from_slice(&self.counter.to_le_bytes());
        self.counter = self.counter.checked_add(1).expect("network random counter exhausted");
        let digest: Zeroizing<[u8; 32]> = hmac_sha256(&self.port_key, &message);
        // The range contains exactly 2^14 ports, so masking is unbiased.
        49152 + (u16::from_le_bytes([digest[0], digest[1]]) & 0x3fff)
    }
}

#[cfg(test)]
mod contract;

impl Drop for NetworkRandom {
    fn drop(&mut self) {
        self.port_key.zeroize();
        self.interface_seed.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_sequence_matches_independent_hmac_vectors() {
        contract::check();
    }

    #[test]
    fn large_counters_keep_the_exact_little_endian_hmac_input() {
        let mut random = contract::seeded();
        for (counter, expected) in
            [(255, 58108), (65535, 58483), (u32::MAX as u64, 53431), (u64::MAX - 1, 51846)]
        {
            random.counter = counter;
            assert_eq!(random.ephemeral_port(), expected);
            assert_eq!(random.counter, counter + 1);
        }
    }

    #[test]
    #[should_panic(expected = "network random counter exhausted")]
    fn exhausted_counter_cannot_wrap_or_restart_the_port_sequence() {
        let mut random = contract::seeded();
        random.counter = u64::MAX;
        random.ephemeral_port();
    }

    #[test]
    fn unavailable_or_partial_entropy_cannot_initialize_network_state() {
        assert!(NetworkRandom::initialize(|_| Err(())).is_err());
        assert!(
            NetworkRandom::initialize(|bytes| {
                bytes[..8].fill(7);
                Err(())
            })
            .is_err()
        );
    }

    #[test]
    fn independent_boots_change_protocol_and_port_state() {
        let mut first = NetworkRandom::initialize(|bytes| {
            bytes.fill(1);
            Ok::<_, ()>(())
        })
        .unwrap();
        let mut second = NetworkRandom::initialize(|bytes| {
            bytes.fill(2);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_ne!(first.interface_seed(), second.interface_seed());
        let mut differences = 0;
        for _ in 0..1000 {
            let a = first.ephemeral_port();
            let b = second.ephemeral_port();
            assert!((49152..=65535).contains(&a));
            assert!((49152..=65535).contains(&b));
            differences += usize::from(a != b);
        }
        assert!(differences > 990);
    }

    #[test]
    fn port_key_is_independent_of_the_interface_seed() {
        let mut first = NetworkRandom::initialize(|bytes| {
            bytes.fill(1);
            Ok::<_, ()>(())
        })
        .unwrap();
        let mut second = NetworkRandom::initialize(|bytes| {
            bytes.fill(2);
            bytes[..8].fill(1);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(first.interface_seed(), second.interface_seed());
        assert_ne!(first.ephemeral_port(), second.ephemeral_port());
    }
}
