//! Fresh boot entropy and independently keyed ephemeral-port selection.
use hmac::{
    Hmac,
    Mac,
};
use sha2::Sha256;
use zeroize::Zeroize;

pub struct NetworkRandom {
    interface_seed: u64,
    port_key: [u8; 32],
    counter: u64,
}

impl NetworkRandom {
    /// No clock, MAC-address, fixed-seed or partial-fill fallback is allowed.
    pub fn initialize<E>(fill: impl FnOnce(&mut [u8]) -> Result<(), E>) -> Result<Self, E> {
        let mut entropy = [0; 40];
        if let Err(error) = fill(&mut entropy) {
            entropy.zeroize();
            return Err(error);
        }
        let interface_seed = u64::from_le_bytes(entropy[..8].try_into().unwrap());
        let port_key = entropy[8..].try_into().unwrap();
        entropy.zeroize();
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
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.port_key).expect("fixed-size HMAC key");
        mac.update(b"CharlotteOS TCP/IP ephemeral port v1");
        mac.update(&self.counter.to_le_bytes());
        self.counter = self.counter.checked_add(1).expect("network random counter exhausted");
        let digest = mac.finalize().into_bytes();
        // The range contains exactly 2^14 ports, so masking is unbiased.
        49152 + (u16::from_le_bytes([digest[0], digest[1]]) & 0x3fff)
    }
}

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
