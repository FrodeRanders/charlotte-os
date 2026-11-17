//! Fallible access to the kernel's cryptographic random source.
//!
//! Security-sensitive code must propagate [`Unavailable`] and fail closed;
//! this module deliberately provides no userspace fallback.

/// The kernel could not provide cryptographically secure random data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Unavailable;

/// Fill `output` exclusively from kernel-provided random words.
pub fn fill(output: &mut [u8]) -> Result<(), Unavailable> {
    for chunk in output.chunks_mut(core::mem::size_of::<u64>()) {
        let word = catten_syscall::random_u64().ok_or(Unavailable)?.to_ne_bytes();
        chunk.copy_from_slice(&word[..chunk.len()]);
    }
    Ok(())
}

/// Return one kernel-provided cryptographically random word.
pub fn word() -> Result<u64, Unavailable> {
    catten_syscall::random_u64().ok_or(Unavailable)
}
