//! The reference userspace UART driver (architecture doc §10, Phase 8).
//!
//! This is the first complete userspace driver: it runs in an isolated EL0
//! protection domain and owns a PL011 UART through *delegated* device
//! capabilities only — an MMIO register window and an interrupt — plus a
//! bootstrap connection to the name service. It never names a physical
//! address or an interrupt vector; the supervisor grants those as
//! capabilities in the config page.
//!
//! Flow:
//!
//! 1. map the delegated MMIO region into its own address space as device memory (a real EL0 device
//!    mapping under the driver's own page table);
//! 2. create a console endpoint and register it by name;
//! 3. bind both the endpoint's readiness and the interrupt to the default completion queue, then
//!    serve from one `CQ_WAIT` — the unified shard wait of §7: the same wait releases for console
//!    requests and for device interrupts alike;
//! 4. on `OP_WRITE`, transmit the byte through the PL011 transmit FIFO (a direct EL0 MMIO write);
//!    on a device interrupt, acknowledge it and count it.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    console,
    ns,
    pl011,
    wait_reply,
};
use catten_syscall::{
    IpcRights,
    cq_wait,
    device_irq_ack,
    device_irq_bind_cq,
    device_mmio_map_any,
    device_mmio_unmap,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_scalar_call_connection,
    ipc_status,
    thread_exit,
};
use charlotte_launch::uart_status as status;

const REPLY_SPINS: u64 = 50_000_000;

static MMIO_BASE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[inline]
unsafe fn uart_put(byte: u8) {
    let base = MMIO_BASE.load(core::sync::atomic::Ordering::Relaxed);
    let fr = (base + pl011::FR) as *const u32;
    let dr = (base + pl011::DR) as *mut u32;
    // Wait for room in the transmit FIFO, then write the byte.
    while unsafe { core::ptr::read_volatile(fr) } & pl011::FR_TXFF != 0 {
        core::hint::spin_loop();
    }
    unsafe {
        core::ptr::write_volatile(dr, byte as u32);
    }
}

/// Read one byte from the receive FIFO, or `None` if it is empty. A real
/// device read (MMIO) from EL0.
#[inline]
unsafe fn uart_get() -> Option<u8> {
    let base = MMIO_BASE.load(core::sync::atomic::Ordering::Relaxed);
    let fr = (base + pl011::FR) as *const u32;
    let dr = (base + pl011::DR) as *const u32;
    if unsafe { core::ptr::read_volatile(fr) } & pl011::FR_RXFE != 0 {
        None
    } else {
        Some((unsafe { core::ptr::read_volatile(dr) } & 0xff) as u8)
    }
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1); // started

    let ns_connection = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let mmio_cap = match ctx.mmio_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let irq_cap = match ctx.irq_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(status::STAGE, 2); // grants received

    // Map the delegated device register window as EL0 device memory.
    let (mmio_map_status, uart_mmio_vaddr) = device_mmio_map_any(mmio_cap, true);
    if mmio_map_status != 0 {
        unsafe { thread_exit() };
    }
    MMIO_BASE.store(uart_mmio_vaddr, core::sync::atomic::Ordering::Relaxed);
    config::write::<u32>(status::STAGE, 3); // MMIO mapped

    // Register the console endpoint by name.
    let endpoint = ipc_endpoint_create(console::INTERFACE, console::VERSION, 8);
    if endpoint == 0 {
        unsafe { thread_exit() };
    }
    let register = ipc_scalar_call_connection(
        ns_connection,
        ns::OP_REGISTER,
        console::NAME,
        endpoint,
        IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
    );
    if register == 0 {
        unsafe { thread_exit() };
    }
    let (generation, _) = unsafe { wait_reply(register, REPLY_SPINS) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 4); // registered

    // Unified shard wait: route both endpoint readiness and the device
    // interrupt to the default completion queue.
    if ipc_endpoint_bind_cq(endpoint, 0) != 0 {
        unsafe { thread_exit() };
    }
    if device_irq_bind_cq(irq_cap, 0) != 0 {
        unsafe { thread_exit() };
    }
    // Unmask the PL011 receive interrupt so real received data raises the
    // delegated interrupt (the self-test drives it via a software-pended SPI).
    unsafe {
        let base = MMIO_BASE.load(core::sync::atomic::Ordering::Relaxed);
        core::ptr::write_volatile((base + pl011::IMSC) as *mut u32, pl011::IMSC_RXIM);
    }
    config::write::<u32>(status::STAGE, 5); // serving

    let mut irq_count: u32 = 0;
    let mut served: u32 = 0;
    // A retained reply token for an in-flight deferred read (0 = none), and the
    // read result to hand back once a device interrupt completes it.
    let mut pending_read: u64 = 0;

    loop {
        // Block on the single wait point.
        cq_wait(1, 0);

        // Clear the device-side level condition before asking the kernel to
        // re-arm the GIC source. Clearing only when a deferred read exists
        // leaves an interrupt asserted after restart; re-arming it then
        // creates an IRQ/CQ wake storm that can starve endpoint requests.
        unsafe {
            let base = MMIO_BASE.load(core::sync::atomic::Ordering::Relaxed);
            core::ptr::write_volatile((base + pl011::ICR) as *mut u32, pl011::IMSC_RXIM);
        }

        // Drain device interrupts: acknowledge and re-arm the source,
        // counting coalesced deliveries.
        let (status, consumed) = device_irq_ack(irq_cap);
        if status == 0 && consumed > 0 {
            irq_count = irq_count.saturating_add(consumed as u32);
            config::write::<u32>(status::IRQ_COUNT, irq_count);

            // A device interrupt completes any retained deferred read: read
            // the receive register (real EL0 MMIO) and reply. Encoding: byte
            // in bits 0..8, interrupt count in bits 8.. so the caller can see
            // the reply was interrupt-driven.
            if pending_read != 0 {
                let byte = unsafe { uart_get() }.unwrap_or(0) as i64;
                let result = byte | ((irq_count as i64) << 8);
                ipc_reply(pending_read, result);
                pending_read = 0;
                config::write::<u32>(status::READ_ARMED, 0);
            }
        }

        // Drain every ready console request.
        loop {
            let message = ipc_recv(endpoint);
            if message.status == ipc_status::NO_MESSAGE {
                break;
            }
            if message.status == ipc_status::ENDPOINT_CLOSED {
                unsafe { thread_exit() };
            }
            if !message.is_ok() {
                break;
            }

            match message.opcode {
                console::OP_WRITE => {
                    unsafe {
                        uart_put((message.arg0 & 0xff) as u8);
                    }
                    served = served.saturating_add(1);
                    config::write::<u32>(status::SERVED, served);
                    if message.reply != 0 {
                        ipc_reply(message.reply, 0);
                    }
                }
                console::OP_STATUS => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, irq_count as i64);
                    }
                }
                console::OP_READ_DEFERRED => {
                    // Retain the reply token instead of replying now; a device
                    // interrupt completes it (architecture doc §7.2). Only one
                    // outstanding read is supported. Publishing READ_ARMED lets
                    // the test drive the interrupt only once the token is held.
                    if message.reply == 0 {
                        // No reply authority: nothing to defer.
                    } else if pending_read != 0 {
                        ipc_reply(message.reply, -1);
                    } else {
                        pending_read = message.reply;
                        config::write::<u32>(status::READ_ARMED, 1);
                    }
                }
                console::OP_SHUTDOWN => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, 0);
                    }
                    unsafe {
                        device_mmio_unmap(mmio_cap);
                        catten_syscall::device_close(irq_cap);
                        thread_exit();
                    }
                }
                console::OP_CRASH => {
                    // Model a crashed driver: exit without releasing device
                    // capabilities or completing the retained deferred read.
                    // The service manager must reclaim the device authority and
                    // reconcile the outstanding operation on teardown.
                    unsafe { thread_exit() };
                }
                _ => {
                    if message.reply != 0 {
                        ipc_reply(message.reply, -1);
                    }
                }
            }
        }
    }
}

catten_rt::entry!(main);
