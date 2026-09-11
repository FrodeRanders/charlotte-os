//! Bounded polling for hardware handshakes.
//!
//! Firmware and devices can stop responding; a kernel that spins forever in a
//! handshake loses the information needed to diagnose or recover. Callers use
//! these helpers where a short wait is architectural and a stuck controller
//! must not hang the logical processor.

/// Poll `condition` up to `limit` times, spinning between attempts. Returns
/// whether the condition was observed. A `false` result means the caller must
/// decide how to proceed without the handshake; it must not assume success.
pub fn bounded_spin<F: FnMut() -> bool>(limit: u32, mut condition: F) -> bool {
    for _ in 0..limit {
        if condition() {
            return true;
        }
        core::hint::spin_loop();
    }
    condition()
}
