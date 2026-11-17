// Protocol dimensions, not workload policy: RFC 8446 section 5.2. Input
// storage also covers Rustls's conservative pre-negotiation record allowance.
const TLS_PLAINTEXT_BYTES: usize = 16_384;
const TLS_CIPHERTEXT_RECORD_BYTES: usize = TLS_PLAINTEXT_BYTES + 256 + 5;
const PRENEGOTIATION_RECORD_BYTES: usize = TLS_PLAINTEXT_BYTES + 2048 + 5;

/// Co-vary credential, record and operation budgets here. These are boot-time
/// inputs, copied into an immutable configuration before accepting any client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_roots: usize,
    pub max_enrollments: usize,
    pub max_chain_certificates: usize,
    pub max_certificate_bytes: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_plaintext_bytes: usize,
    pub send_fragment_bytes: usize,
    pub handshake_ms: u64,
    pub session_ms: u64,
    pub work_per_step: usize,
}

impl Limits {
    pub const DEFAULT: Self = Self {
        max_roots: 8,
        max_enrollments: 64,
        max_chain_certificates: 4,
        max_certificate_bytes: 8 * 1024,
        max_input_bytes: 64 * 1024,
        max_output_bytes: 64 * 1024,
        max_plaintext_bytes: 16 * 1024,
        send_fragment_bytes: 4096,
        handshake_ms: 5_000,
        session_ms: 60_000,
        work_per_step: 32,
    };

    pub fn validate(self) -> Result<Self, super::Error> {
        // TLS 1.3 peers may send a full 16 KiB plaintext record plus the
        // protocol's ciphertext allowance. Fragmented handshakes also retain
        // bounded internal Rustls storage (its current 65,535-byte limit).
        if self.max_roots == 0
            || self.max_enrollments == 0
            || self.max_chain_certificates == 0
            || self.max_certificate_bytes == 0
            || self.max_input_bytes < PRENEGOTIATION_RECORD_BYTES
            || self.max_output_bytes < TLS_CIPHERTEXT_RECORD_BYTES
            || self.max_plaintext_bytes == 0
            || self.max_plaintext_bytes > TLS_PLAINTEXT_BYTES
            || !(32..=TLS_PLAINTEXT_BYTES).contains(&self.send_fragment_bytes)
            || self.handshake_ms == 0
            || self.session_ms < self.handshake_ms
            || self.work_per_step == 0
            || self.max_chain_certificates.checked_mul(self.max_certificate_bytes).is_none()
        {
            return Err(super::Error::InvalidLimits);
        }
        // Rustls's fragment setting includes the five-byte record header.
        // Each TLS 1.3 AES-GCM fragment also needs its inner content type and
        // authentication tag. Smaller fragments increase aggregate wire bytes.
        let fragments = self.max_plaintext_bytes.div_ceil(self.send_fragment_bytes - 5);
        let required = fragments
            .checked_mul(5 + 1 + 16)
            .and_then(|overhead| self.max_plaintext_bytes.checked_add(overhead))
            .ok_or(super::Error::InvalidLimits)?;
        if required > self.max_output_bytes {
            return Err(super::Error::InvalidLimits);
        }
        Ok(self)
    }
}
