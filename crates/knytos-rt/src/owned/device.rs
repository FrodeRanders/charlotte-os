//! MMIO regions and interrupt ownership.
//!
//! Child module of [`crate::owned`].

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceError {
    MappingFailed(u64),
    Status(u64),
}

/// An owned MMIO capability before it is mapped into the process.
#[must_use = "dropping an MMIO capability closes it"]
#[derive(Debug)]
pub struct MmioRegion {
    cap: Option<u64>,
}

impl MmioRegion {
    /// Adopt a uniquely owned MMIO capability supplied at launch.
    ///
    /// # Safety
    /// `cap` must identify an MMIO grant owned by this address space and must
    /// not be used through the raw syscall API after adoption.
    pub const unsafe fn from_raw(cap: u64) -> Self {
        Self {
            cap: Some(cap),
        }
    }

    pub(super) fn raw_handle(&self) -> u64 {
        self.cap.expect("MMIO capability already consumed")
    }

    pub fn map(mut self, writable: bool) -> Result<MappedMmio, (Self, DeviceError)> {
        let (status, base) = kernel::device_mmio_map_any(self.raw_handle(), writable);
        if status != catten_syscall::device_status::OK {
            return Err((self, DeviceError::MappingFailed(status)));
        }
        let cap = self.cap.take().expect("MMIO capability already consumed");
        Ok(MappedMmio {
            cap: Some(cap),
            base,
        })
    }
}

impl Drop for MmioRegion {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::device_close(cap);
        }
    }
}

/// An active MMIO mapping. Device-register access remains `unsafe` because
/// register width, alignment, volatility, and ordering are device-specific.
#[must_use = "dropping an MMIO mapping unmaps and closes it"]
#[derive(Debug)]
pub struct MappedMmio {
    cap: Option<u64>,
    base: usize,
}

impl MappedMmio {
    pub const fn as_ptr(&self) -> *mut u8 {
        self.base as *mut u8
    }

    pub fn unmap(mut self) -> Result<MmioRegion, (Self, DeviceError)> {
        let cap = self.cap.expect("MMIO capability already consumed");
        let status = kernel::device_mmio_unmap(cap);
        if status != catten_syscall::device_status::OK {
            return Err((self, DeviceError::Status(status)));
        }
        let _ = self.cap.take();
        Ok(MmioRegion {
            cap: Some(cap),
        })
    }
}

impl Drop for MappedMmio {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::device_mmio_unmap(cap);
            let _ = kernel::device_close(cap);
        }
    }
}

/// An owned interrupt capability.
#[must_use = "dropping an interrupt capability masks and closes it"]
#[derive(Debug)]
pub struct Interrupt {
    cap: Option<u64>,
}

impl Interrupt {
    /// Adopt a uniquely owned interrupt capability supplied at launch.
    ///
    /// # Safety
    /// `cap` must identify an interrupt grant owned by this address space and
    /// must not be used through the raw syscall API after adoption.
    pub const unsafe fn from_raw(cap: u64) -> Self {
        Self {
            cap: Some(cap),
        }
    }

    pub(super) fn raw_handle(&self) -> u64 {
        self.cap.expect("interrupt capability already consumed")
    }

    pub fn bind_completion_queue(&self, cq: u32) -> Result<(), DeviceError> {
        let status = kernel::device_irq_bind_cq(self.raw_handle(), cq);
        if status == catten_syscall::device_status::OK {
            Ok(())
        } else {
            Err(DeviceError::Status(status))
        }
    }

    pub fn acknowledge(&self) -> Result<u64, DeviceError> {
        let (status, consumed) = kernel::device_irq_ack(self.raw_handle());
        if status == catten_syscall::device_status::OK {
            Ok(consumed)
        } else {
            Err(DeviceError::Status(status))
        }
    }
}

impl Drop for Interrupt {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            let _ = kernel::device_close(cap);
        }
    }
}
