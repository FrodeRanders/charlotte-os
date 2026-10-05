//! Socket admission and remote-owner lifetime policy.

pub const UNUSED_SOCKET_TIMEOUT_MS: u64 = 5_000;
pub const PLATFORM_SOCKET_RESERVE: usize = 16;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SocketOwner {
    pub address_space: u64,
    pub generation: u64,
    pub principal: u64,
}

impl SocketOwner {
    pub fn from_message(message: &catten_syscall::IpcMessage) -> Self {
        Self {
            address_space: message.sender,
            generation: message.sender_generation,
            principal: message.sender_principal,
        }
    }
}

pub struct SocketLifetime {
    pub platform: bool,
    created_ms: u64,
}

impl SocketLifetime {
    pub fn new(platform: bool, created_ms: u64) -> Self {
        Self {
            platform,
            created_ms,
        }
    }

    /// Even open listeners/UDP sockets are reclaimed when the exact owner dies.
    pub fn reclaim(&self, owner_alive: bool, activated: bool, now_ms: u64) -> bool {
        !owner_alive
            || (!activated && now_ms.saturating_sub(self.created_ms) >= UNUSED_SOCKET_TIMEOUT_MS)
    }
}

pub fn can_admit(capacity: usize, total: usize, ordinary: usize, platform: bool) -> bool {
    let reserve = PLATFORM_SOCKET_RESERVE.min(capacity);
    total < capacity && (platform || ordinary < capacity.saturating_sub(reserve))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_pressure_preserves_platform_slots_at_every_capacity() {
        for capacity in 0usize..=1024 {
            let ordinary = capacity.saturating_sub(PLATFORM_SOCKET_RESERVE);
            assert!(!can_admit(capacity, ordinary, ordinary, false));
            assert_eq!(can_admit(capacity, ordinary, ordinary, true), ordinary < capacity);
            assert!(!can_admit(capacity, capacity, ordinary, true));
        }
    }

    #[test]
    fn owner_death_reclaims_unconfigured_listener_and_datagram_resources() {
        for platform in [false, true] {
            let lease = SocketLifetime::new(platform, 10);
            for activated in [false, true] {
                assert!(lease.reclaim(false, activated, 11));
                assert!(!lease.reclaim(true, activated, 11));
            }
            assert!(!lease.reclaim(true, false, 5009));
            assert!(lease.reclaim(true, false, 5010));
            assert!(!lease.reclaim(true, true, u64::MAX));
        }
    }

    #[test]
    fn recycled_domain_or_same_principal_does_not_inherit_socket_authority() {
        let original = SocketOwner {
            address_space: 5,
            generation: 1,
            principal: 7,
        };
        assert_ne!(
            original,
            SocketOwner {
                generation: 2,
                ..original
            }
        );
        assert_ne!(
            original,
            SocketOwner {
                address_space: 6,
                ..original
            }
        );
    }
}
