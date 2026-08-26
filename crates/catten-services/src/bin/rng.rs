//! VirtIO RNG driver and node-local cryptographic entropy service.
//!
//! The launch environment delegates exactly one MMIO region, interrupt, and
//! protected DMA domain. One polled split virtqueue obtains host entropy; IPC
//! replies move freshly allocated memory objects to callers.
#![no_std]
#![no_main]

extern crate alloc;

use core::sync::atomic::{
    Ordering,
    fence,
};

use catten_rt::{
    Context,
    config,
    owned::{
        DmaDomain,
        Endpoint,
        MmioRegion,
        OwnedMemory,
        SharedDmaMemory,
    },
};
use catten_services::{
    entropy,
    ns,
    virtio,
};
use catten_syscall::{
    DmaDirection,
    IpcRights,
    thread_exit,
};
use charlotte_launch::rng_status as status;

catten_rt::entry!(main);

const PAGE_SIZE: usize = 4_096;
const QUEUE_SIZE: u16 = 8;
const POLL_LIMIT: usize = 1_000_000;

#[inline]
unsafe fn mmio_r8(base: *mut u8, offset: usize) -> u8 {
    unsafe { core::ptr::read_volatile(base.add(offset)) }
}

#[inline]
unsafe fn mmio_r16(base: *mut u8, offset: usize) -> u16 {
    unsafe { core::ptr::read_volatile(base.add(offset).cast::<u16>()) }
}

#[inline]
unsafe fn mmio_r32(base: *mut u8, offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile(base.add(offset).cast::<u32>()) }
}

#[inline]
unsafe fn mmio_w8(base: *mut u8, offset: usize, value: u8) {
    unsafe { core::ptr::write_volatile(base.add(offset), value) }
}

#[inline]
unsafe fn mmio_w16(base: *mut u8, offset: usize, value: u16) {
    unsafe { core::ptr::write_volatile(base.add(offset).cast::<u16>(), value) }
}

#[inline]
unsafe fn mmio_w32(base: *mut u8, offset: usize, value: u32) {
    unsafe { core::ptr::write_volatile(base.add(offset).cast::<u32>(), value) }
}

fn ring_write(ring: &mut SharedDmaMemory<'_>, offset: usize, bytes: &[u8]) -> bool {
    ring.write_volatile_from(offset, bytes).is_ok()
}

fn ring_write_u16(ring: &mut SharedDmaMemory<'_>, offset: usize, value: u16) -> bool {
    ring_write(ring, offset, &value.to_le_bytes())
}

fn ring_write_u32(ring: &mut SharedDmaMemory<'_>, offset: usize, value: u32) -> bool {
    ring_write(ring, offset, &value.to_le_bytes())
}

fn ring_write_u64(ring: &mut SharedDmaMemory<'_>, offset: usize, value: u64) -> bool {
    ring_write(ring, offset, &value.to_le_bytes())
}

fn ring_read_u16(ring: &SharedDmaMemory<'_>, offset: usize) -> Option<u16> {
    let mut bytes = [0; 2];
    ring.read_volatile_into(offset, &mut bytes).ok()?;
    Some(u16::from_le_bytes(bytes))
}

fn ring_read_u32(ring: &SharedDmaMemory<'_>, offset: usize) -> Option<u32> {
    let mut bytes = [0; 4];
    ring.read_volatile_into(offset, &mut bytes).ok()?;
    Some(u32::from_le_bytes(bytes))
}

const fn avail_offset() -> usize {
    QUEUE_SIZE as usize * virtio::DESC_SIZE
}

const fn used_offset() -> usize {
    (avail_offset() + 6 + QUEUE_SIZE as usize * 2).next_multiple_of(PAGE_SIZE)
}

struct VirtioRng<'domain> {
    _mmio: catten_rt::owned::MappedMmio,
    ring: SharedDmaMemory<'domain>,
    entropy: SharedDmaMemory<'domain>,
    common: *mut u8,
    notify: *mut u8,
    avail_position: u16,
}

impl<'domain> VirtioRng<'domain> {
    fn open(ctx: &Context, domain: &'domain DmaDomain) -> Option<Self> {
        let mmio_cap = ctx.mmio_cap()?;
        // SAFETY: this launch-owned capability is adopted once at the runtime
        // ABI boundary and never used again as a raw handle.
        let mmio = unsafe { MmioRegion::from_raw(mmio_cap) }.map(true).ok()?;
        let base = mmio.as_ptr();
        let common = unsafe { base.add(virtio::MODERN_COMMON) };

        unsafe {
            mmio_w8(common, virtio::M_DEVICE_STATUS, 0);
            mmio_w8(common, virtio::M_DEVICE_STATUS, virtio::STATUS_ACKNOWLEDGE);
            mmio_w8(
                common,
                virtio::M_DEVICE_STATUS,
                virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER,
            );
            mmio_w32(common, virtio::M_DEVICE_FEATURE_SELECT, 1);
        }
        let required = virtio::FEATURE_VERSION_1 | virtio::FEATURE_ACCESS_PLATFORM;
        if unsafe { mmio_r32(common, virtio::M_DEVICE_FEATURE) } & required != required {
            return None;
        }
        unsafe {
            mmio_w32(common, virtio::M_DRIVER_FEATURE_SELECT, 1);
            mmio_w32(common, virtio::M_DRIVER_FEATURE, required);
            mmio_w8(
                common,
                virtio::M_DEVICE_STATUS,
                virtio::STATUS_ACKNOWLEDGE | virtio::STATUS_DRIVER | virtio::STATUS_FEATURES_OK,
            );
        }
        if unsafe { mmio_r8(common, virtio::M_DEVICE_STATUS) } & virtio::STATUS_FEATURES_OK == 0 {
            return None;
        }

        let mut ring = OwnedMemory::allocate(2)
            .ok()?
            .map_shared_dma(domain, DmaDirection::Bidirectional)
            .ok()?;
        let entropy = OwnedMemory::allocate(1)
            .ok()?
            .map_shared_dma(domain, DmaDirection::DeviceWrite)
            .ok()?;
        for offset in 0..ring.len() {
            ring.write_volatile(offset, 0).ok()?;
        }

        unsafe {
            mmio_w16(common, virtio::M_QUEUE_SELECT, 0);
            if mmio_r16(common, virtio::M_QUEUE_SIZE) < QUEUE_SIZE {
                return None;
            }
            mmio_w16(common, virtio::M_QUEUE_VECTOR, u16::MAX);
            mmio_w16(common, virtio::M_QUEUE_SIZE, QUEUE_SIZE);
        }
        let ring_iova = ring.iova();
        if !ring_write_u64(&mut ring, virtio::DESC_ADDR_LO, entropy.iova())
            || !ring_write_u32(&mut ring, virtio::DESC_LENGTH, entropy::MAX_REQUEST as u32)
            || !ring_write_u16(&mut ring, virtio::DESC_FLAGS, virtio::VRING_DESC_F_WRITE)
            || !ring_write_u16(&mut ring, avail_offset() + virtio::AVAIL_FLAGS, 1)
        {
            return None;
        }
        unsafe {
            mmio_w32(common, virtio::M_QUEUE_DESC, ring_iova as u32);
            mmio_w32(common, virtio::M_QUEUE_DESC + 4, (ring_iova >> 32) as u32);
            let avail = ring_iova + avail_offset() as u64;
            mmio_w32(common, virtio::M_QUEUE_DRIVER, avail as u32);
            mmio_w32(common, virtio::M_QUEUE_DRIVER + 4, (avail >> 32) as u32);
            let used = ring_iova + used_offset() as u64;
            mmio_w32(common, virtio::M_QUEUE_DEVICE, used as u32);
            mmio_w32(common, virtio::M_QUEUE_DEVICE + 4, (used >> 32) as u32);
            mmio_w16(common, virtio::M_QUEUE_ENABLE, 1);
            mmio_w8(
                common,
                virtio::M_DEVICE_STATUS,
                virtio::STATUS_ACKNOWLEDGE
                    | virtio::STATUS_DRIVER
                    | virtio::STATUS_FEATURES_OK
                    | virtio::STATUS_DRIVER_OK,
            );
        }
        let notify_offset = unsafe { mmio_r16(common, virtio::M_QUEUE_NOTIFY_OFF) };
        let notify = unsafe {
            base.add(
                virtio::MODERN_NOTIFY + notify_offset as usize * virtio::MODERN_NOTIFY_MULTIPLIER,
            )
        };
        Some(Self {
            _mmio: mmio,
            ring,
            entropy,
            common,
            notify,
            avail_position: 0,
        })
    }

    fn fill(&mut self, output: &mut [u8]) -> bool {
        if output.is_empty() || output.len() > entropy::MAX_REQUEST {
            return false;
        }
        let mut filled = 0;
        while filled < output.len() {
            let requested = output.len() - filled;
            if !ring_write_u32(&mut self.ring, virtio::DESC_LENGTH, requested as u32) {
                return false;
            }
            let next = self.avail_position.wrapping_add(1);
            let slot = self.avail_position as usize % QUEUE_SIZE as usize;
            if !ring_write_u16(&mut self.ring, avail_offset() + virtio::AVAIL_RING + slot * 2, 0) {
                return false;
            }
            fence(Ordering::Release);
            if !ring_write_u16(&mut self.ring, avail_offset() + virtio::AVAIL_IDX, next) {
                return false;
            }
            unsafe { mmio_w32(self.notify, 0, 0) };
            let completed = (0..POLL_LIMIT)
                .any(|_| ring_read_u16(&self.ring, used_offset() + virtio::USED_IDX) == Some(next));
            if !completed {
                return false;
            }
            fence(Ordering::Acquire);
            let used_slot = self.avail_position as usize % QUEUE_SIZE as usize;
            let Some(used_len) =
                ring_read_u32(&self.ring, used_offset() + virtio::USED_RING + used_slot * 8 + 4)
            else {
                return false;
            };
            self.avail_position = next;
            let used_len = used_len as usize;
            if used_len == 0 || used_len > requested {
                return false;
            }
            if self.entropy.read_volatile_into(0, &mut output[filled..filled + used_len]).is_err() {
                return false;
            }
            filled += used_len;
        }
        true
    }
}

impl Drop for VirtioRng<'_> {
    fn drop(&mut self) {
        unsafe { mmio_w8(self.common, virtio::M_DEVICE_STATUS, 0) };
    }
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    unsafe { thread_exit() }
}

fn main(ctx: Context) -> ! {
    let ns_connection = ctx.bootstrap_connection().unwrap_or_else(|| fail(0xe001));
    let dma_cap = config::dma_domain_cap().unwrap_or_else(|| fail(0xe002));
    // SAFETY: the launch environment retains ownership of this borrowed DMA
    // authority for the process lifetime.
    let dma_domain = unsafe { DmaDomain::from_raw(dma_cap) };
    let mut device = VirtioRng::open(&ctx, &dma_domain).unwrap_or_else(|| fail(0xe003));

    let endpoint =
        Endpoint::create(entropy::INTERFACE, entropy::VERSION, 32).unwrap_or_else(|_| fail(0xe004));
    let registration = ns_connection
        .call_connection(
            ns::OP_REGISTER,
            entropy::NAME,
            &endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
        .unwrap_or_else(|_| fail(0xe005));
    if !registration.wait().is_ok_and(|reply| reply.result >= 1)
        || endpoint.bind_completion_queue(0).is_err()
    {
        fail(0xe005);
    }
    config::write::<u32>(status::STAGE, 1);
    catten_rt::logln!("[rng] serving host-backed VirtIO entropy");
    let mut bytes_served = 0u64;

    loop {
        let mut message = match endpoint.receive() {
            Ok(message) => message,
            Err(catten_rt::owned::ReceiveError::EndpointClosed) => unsafe { thread_exit() },
            Err(_) => continue,
        };
        let Some(reply) = message.reply.take() else {
            continue;
        };
        if message.opcode != entropy::OP_FILL {
            let _ = reply.reply(entropy::ERR_BAD_OPCODE);
            continue;
        }
        let Ok(length) = usize::try_from(message.arg0) else {
            let _ = reply.reply(entropy::ERR_INVALID);
            continue;
        };
        if length == 0 || length > entropy::MAX_REQUEST {
            let _ = reply.reply(entropy::ERR_INVALID);
            continue;
        }
        let Ok(memory) = OwnedMemory::allocate(length.div_ceil(PAGE_SIZE)) else {
            let _ = reply.reply(entropy::ERR_MEMORY);
            continue;
        };
        let Ok(mut mapping) = memory.map_writable() else {
            let _ = reply.reply(entropy::ERR_MEMORY);
            continue;
        };
        if !device.fill(&mut mapping.as_mut_slice()[..length]) {
            let _ = reply.reply(entropy::ERR_DEVICE);
            continue;
        }
        let Ok(memory) = mapping.unmap() else {
            let _ = reply.reply(entropy::ERR_MEMORY);
            continue;
        };
        if reply.reply_move(memory, length as i64).is_ok() {
            bytes_served = bytes_served.saturating_add(length as u64);
            config::write::<u64>(status::BYTES, bytes_served);
        }
    }
}
