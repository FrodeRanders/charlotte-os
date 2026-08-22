//! Convenient serial logging for EL0 programs.
//!
//! Wraps the [`catten_syscall::el0_log_str`] syscall behind a [`logln!`]
//! macro, so services can emit human-readable lines without packing bytes
//! into raw numbers. Output is rendered on the kernel serial log as
//! `[EL0 LOG] lp=.. asid=.. <text>`.

/// A fixed-capacity, `core::fmt::Write`-compatible buffer that a [`logln!`]
/// invocation formats into before flushing to the kernel log.
pub struct LogBuffer {
    buffer: [u8; 256],
    len: usize,
}

impl LogBuffer {
    /// Create an empty log buffer.
    pub const fn new() -> Self {
        Self {
            buffer: [0; 256],
            len: 0,
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.buffer[..self.len]
    }
}

impl core::fmt::Write for LogBuffer {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let bytes = text.as_bytes();
        let remaining = &mut self.buffer[self.len..];
        let copied = bytes.len().min(remaining.len());
        remaining[..copied].copy_from_slice(&bytes[..copied]);
        self.len += copied;
        Ok(())
    }
}

/// Flush a filled [`LogBuffer`] to the kernel serial log.
pub fn flush(buffer: &LogBuffer) {
    catten_syscall::el0_log_str(buffer.as_slice().as_ptr(), buffer.len);
}

/// Format a message and emit it as a single serial log line.
///
/// The kernel appends the trailing newline, so callers do not include one.
#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buffer = $crate::log::LogBuffer::new();
        let _ = core::write!(&mut buffer, $($arg)*);
        $crate::log::flush(&buffer);
    }};
}
