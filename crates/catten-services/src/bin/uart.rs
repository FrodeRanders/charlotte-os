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
//! 1. map the delegated MMIO region into its own address space as device
//!    memory (a real EL0 device mapping under the driver's own page table);
//! 2. create a console endpoint and register it by name;
//! 3. bind both the endpoint's readiness and the interrupt to the default
//!    completion queue, then serve from one `CQ_WAIT` — the unified shard
//!    wait of §7: the same wait releases for console requests and for device
//!    interrupts alike;
//! 4. on `OP_WRITE`, transmit the byte through the PL011 transmit FIFO
//!    (a direct EL0 MMIO write); on a device interrupt, acknowledge it and
//!    count it.
#![no_std]
#![no_main]

extern crate alloc;

use catten_rt::{
    Args,
    Input,
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
    device_mmio_map,
    device_mmio_unmap,
    ipc_endpoint_bind_cq,
    ipc_endpoint_create,
    ipc_recv,
    ipc_reply,
    ipc_scalar_call_connection,
    ipc_status,
    thread_exit,
};

const REPLY_SPINS: u64 = 50_000_000;

/// The user virtual address at which the driver maps its device register
/// window. Chosen above the program image, runtime pages, and the long-name
/// scratch page.
const UART_MMIO_VADDR: usize = 0x0000_0000_0040_0000;

/// Config-page output words (driver domain).
const STAGE_OFFSET: usize = 0; // u32 progress marker
const IRQ_COUNT_OFFSET: usize = 8; // u32 interrupts acknowledged
const SERVED_OFFSET: usize = 12; // u32 write requests served

#[inline]
unsafe fn uart_put(byte: u8) {
    let fr = (UART_MMIO_VADDR + pl011::FR) as *const u32;
    let dr = (UART_MMIO_VADDR + pl011::DR) as *mut u32;
    // Wait for room in the transmit FIFO, then write the byte.
    while unsafe { core::ptr::read_volatile(fr) } & pl011::FR_TXFF != 0 {
        core::hint::spin_loop();
    }
    unsafe {
        core::ptr::write_volatile(dr, byte as u32);
    }
}

fn cmain(_args: Args, _input: Input<0>) -> ! {
    config::write::<u32>(STAGE_OFFSET, 1); // started

    let ns_connection = match config::bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let mmio_cap = match config::mmio_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let irq_cap = match config::irq_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    config::write::<u32>(STAGE_OFFSET, 2); // grants received

    // Map the delegated device register window as EL0 device memory.
    if unsafe { device_mmio_map(mmio_cap, UART_MMIO_VADDR, true) } != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 3); // MMIO mapped

    // Register the console endpoint by name.
    let endpoint = unsafe { ipc_endpoint_create(console::INTERFACE, console::VERSION, 8) };
    if endpoint == 0 {
        unsafe { thread_exit() };
    }
    let register = unsafe {
        ipc_scalar_call_connection(
            ns_connection,
            ns::OP_REGISTER,
            console::NAME,
            endpoint,
            IpcRights::SEND | IpcRights::CALL | IpcRights::MINT_CONNECTION,
        )
    };
    if register == 0 {
        unsafe { thread_exit() };
    }
    let (generation, _) = unsafe { wait_reply(register, REPLY_SPINS) };
    if generation < 1 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 4); // registered

    // Unified shard wait: route both endpoint readiness and the device
    // interrupt to the default completion queue.
    if unsafe { ipc_endpoint_bind_cq(endpoint, 0) } != 0 {
        unsafe { thread_exit() };
    }
    if unsafe { device_irq_bind_cq(irq_cap, 0) } != 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(STAGE_OFFSET, 5); // serving

    let mut irq_count: u32 = 0;
    let mut served: u32 = 0;

    loop {
        // Block on the single wait point.
        unsafe {
            cq_wait(1, 0);
        }

        // Drain device interrupts: acknowledge and re-arm the source,
        // counting coalesced deliveries.
        let (status, consumed) = unsafe { device_irq_ack(irq_cap) };
        if status == 0 && consumed > 0 {
            irq_count = irq_count.saturating_add(consumed as u32);
            config::write::<u32>(IRQ_COUNT_OFFSET, irq_count);
        }

        // Drain every ready console request.
        loop {
            let message = unsafe { ipc_recv(endpoint) };
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
                    config::write::<u32>(SERVED_OFFSET, served);
                    if message.reply != 0 {
                        unsafe {
                            ipc_reply(message.reply, 0);
                        }
                    }
                }
                console::OP_STATUS => {
                    if message.reply != 0 {
                        unsafe {
                            ipc_reply(message.reply, irq_count as i64);
                        }
                    }
                }
                console::OP_SHUTDOWN => {
                    if message.reply != 0 {
                        unsafe {
                            ipc_reply(message.reply, 0);
                        }
                    }
                    unsafe {
                        device_mmio_unmap(mmio_cap);
                        catten_syscall::device_close(irq_cap);
                        thread_exit();
                    }
                }
                _ => {
                    if message.reply != 0 {
                        unsafe {
                            ipc_reply(message.reply, -1);
                        }
                    }
                }
            }
        }
    }
}

catten_rt::entry!(cmain);
