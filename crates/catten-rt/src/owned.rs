//! Ownership-aware wrappers for memory and asynchronous kernel operations.
//!
//! The raw syscall crate intentionally mirrors the register ABI and therefore
//! uses copyable integers and addresses. This module is the safe application
//! layer: capability ownership is linear, mappings own their capability, DMA
//! consumes a CPU mapping, and an asynchronous read retains its mutable borrow
//! until the kernel has reached a terminal state.

extern crate alloc;

use alloc::vec::Vec;
use core::{
    marker::PhantomData,
    slice,
};

use catten_syscall::{
    self,
    DmaDirection,
    IpcRights,
    OpCode,
};
use charlotte_lifecycle::ThreadIdentity;

mod completion;
mod device;
mod ipc;
mod memory;

pub use completion::*;
pub use device::*;
pub use ipc::*;
pub use memory::*;

#[cfg(not(test))]
mod kernel {
    use catten_syscall::{
        self,
        DmaDirection,
        IpcRights,
        OpCode,
    };

    pub fn memory_alloc(pages: usize) -> u64 {
        catten_syscall::memory_alloc(pages)
    }

    pub fn memory_size(cap: u64) -> usize {
        catten_syscall::memory_size(cap)
    }

    pub fn memory_map_any(cap: u64, writable: bool) -> (u64, usize) {
        catten_syscall::memory_map_any(cap, writable)
    }

    pub fn memory_unmap(cap: u64) -> u64 {
        catten_syscall::memory_unmap(cap)
    }

    pub fn memory_close(cap: u64) -> u64 {
        catten_syscall::memory_close(cap)
    }

    pub fn dma_map_exclusive(domain: u64, memory: u64, direction: DmaDirection) -> u64 {
        catten_syscall::dma_map_exclusive(domain, memory, direction)
    }

    pub fn dma_map(domain: u64, memory: u64, direction: DmaDirection) -> u64 {
        // SAFETY: the owning caller keeps both capabilities live and pairs a
        // successful mapping with `dma_unmap` before releasing either one.
        unsafe { catten_syscall::dma_map(domain, memory, direction) }
    }

    pub fn dma_unmap(domain: u64, iova: u64) -> u64 {
        catten_syscall::dma_unmap(domain, iova)
    }

    pub fn device_mmio_map_any(cap: u64, writable: bool) -> (u64, usize) {
        catten_syscall::device_mmio_map_any(cap, writable)
    }

    pub fn device_mmio_unmap(cap: u64) -> u64 {
        catten_syscall::device_mmio_unmap(cap)
    }

    pub fn device_irq_bind_cq(cap: u64, cq: u32) -> u64 {
        catten_syscall::device_irq_bind_cq(cap, cq)
    }

    pub fn device_irq_ack(cap: u64) -> (u64, u64) {
        catten_syscall::device_irq_ack(cap)
    }

    pub fn device_close(cap: u64) -> u64 {
        catten_syscall::device_close(cap)
    }

    pub fn submit(op: OpCode) -> u64 {
        catten_syscall::submit(op)
    }

    pub fn submit_timer(timeout_ms: u64) -> u64 {
        catten_syscall::submit_timer(timeout_ms)
    }

    pub unsafe fn submit_read(buf_ptr: usize, buf_len: usize) -> u64 {
        unsafe { catten_syscall::submit_read(buf_ptr, buf_len) }
    }

    pub fn poll(cap: u64) -> (u64, u64) {
        catten_syscall::poll(cap)
    }

    pub fn wait(cap: u64) {
        catten_syscall::wait(cap);
    }

    pub fn wait_timeout(cap: u64, timeout_ms: u64) -> (u64, u64) {
        catten_syscall::wait_timeout(cap, timeout_ms)
    }

    pub fn cancel(cap: u64) {
        catten_syscall::cancel(cap);
    }

    pub fn close(cap: u64) {
        catten_syscall::close(cap);
    }

    pub fn ipc_scalar_send(connection: u64, opcode: u32, arg0: u64) -> u64 {
        catten_syscall::ipc_scalar_send(connection, opcode, arg0)
    }

    pub fn ipc_endpoint_create(interface: u64, version: u32, capacity: usize) -> u64 {
        catten_syscall::ipc_endpoint_create(interface, version, capacity)
    }

    pub fn ipc_connect(endpoint: u64, rights: IpcRights) -> u64 {
        catten_syscall::ipc_connect(endpoint, rights)
    }

    pub fn ipc_endpoint_bind_cq(endpoint: u64, cq: u32) -> u64 {
        catten_syscall::ipc_endpoint_bind_cq(endpoint, cq)
    }

    pub fn ipc_endpoint_resize(endpoint: u64, capacity: usize) -> u64 {
        catten_syscall::ipc_endpoint_resize(endpoint, capacity)
    }

    pub fn ipc_endpoint_status(endpoint: u64) -> (u64, u64, u64) {
        catten_syscall::ipc_endpoint_status(endpoint)
    }

    pub fn ipc_recv(endpoint: u64) -> catten_syscall::IpcMessage {
        catten_syscall::ipc_recv(endpoint)
    }

    pub fn ipc_recv_block(endpoint: u64) -> catten_syscall::IpcMessage {
        catten_syscall::ipc_recv_block(endpoint)
    }

    pub fn ipc_recv_authenticated(endpoint: u64) -> catten_syscall::IpcMessage {
        catten_syscall::ipc_recv_authenticated(endpoint)
    }

    pub fn ipc_recv_block_authenticated(endpoint: u64) -> catten_syscall::IpcMessage {
        catten_syscall::ipc_recv_block_authenticated(endpoint)
    }

    pub fn ipc_scalar_call(connection: u64, opcode: u32, arg0: u64) -> u64 {
        catten_syscall::ipc_scalar_call(connection, opcode, arg0)
    }

    pub fn ipc_scalar_send_move(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
        catten_syscall::ipc_scalar_send_move(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_move(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
        catten_syscall::ipc_scalar_call_move(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_borrow_read(
        connection: u64,
        opcode: u32,
        arg0: u64,
        memory: u64,
    ) -> u64 {
        catten_syscall::ipc_scalar_call_borrow_read(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_borrow_write(
        connection: u64,
        opcode: u32,
        arg0: u64,
        memory: u64,
    ) -> u64 {
        catten_syscall::ipc_scalar_call_borrow_write(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_copy(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
        catten_syscall::ipc_scalar_call_copy(connection, opcode, arg0, memory)
    }

    pub fn ipc_scalar_call_connection(
        connection: u64,
        opcode: u32,
        arg0: u64,
        endpoint: u64,
        rights: IpcRights,
    ) -> u64 {
        catten_syscall::ipc_scalar_call_connection(connection, opcode, arg0, endpoint, rights)
    }

    pub fn ipc_scalar_call_connection_copy(
        connection: u64,
        opcode: u32,
        arg0: u64,
        endpoint: u64,
        rights: IpcRights,
        memory: u64,
    ) -> u64 {
        catten_syscall::ipc_scalar_call_connection_copy(
            connection, opcode, arg0, endpoint, rights, memory,
        )
    }

    pub fn ipc_vector_send(connection: u64, opcode: u32, arg0: u64, descriptor: u64) -> u64 {
        catten_syscall::ipc_vector_send(connection, opcode, arg0, descriptor)
    }

    pub fn ipc_vector_call(connection: u64, opcode: u32, arg0: u64, descriptor: u64) -> u64 {
        catten_syscall::ipc_vector_call(connection, opcode, arg0, descriptor)
    }

    pub fn ipc_connection_watch_closed(connection: u64) -> u64 {
        catten_syscall::ipc_connection_watch_closed(connection)
    }

    pub fn ipc_reply_poll_with_memory(call: u64) -> (u64, u64, u64, u64) {
        catten_syscall::ipc_reply_poll_with_memory(call)
    }

    pub fn ipc_reply_wait_with_memory(call: u64) -> (u64, u64, u64, u64) {
        catten_syscall::ipc_reply_wait_with_memory(call)
    }

    pub fn ipc_reply(reply: u64, result: i64) -> u64 {
        catten_syscall::ipc_reply(reply, result)
    }

    pub fn ipc_reply_move(reply: u64, memory: u64, result: i64) -> u64 {
        catten_syscall::ipc_reply_move(reply, memory, result)
    }

    pub fn ipc_reply_connection(reply: u64, endpoint: u64, rights: IpcRights, result: i64) -> u64 {
        catten_syscall::ipc_reply_connection(reply, endpoint, rights, result)
    }

    pub fn ipc_close(cap: u64) -> u64 {
        catten_syscall::ipc_close(cap)
    }

    pub fn spawn_artifact_scoped(
        artifact: u64,
        artifact_len: usize,
        artifact_name: u64,
        descriptor: u64,
        descriptor_len: usize,
    ) -> u64 {
        catten_syscall::spawn_artifact_scoped(
            artifact,
            artifact_len,
            artifact_name,
            descriptor,
            descriptor_len,
        )
    }

    pub fn spawn_operational_connector(package: u64, package_len: usize, principal: u64) -> u64 {
        catten_syscall::spawn_operational_connector(package, package_len, principal)
    }

    pub fn spawn_artifact(artifact: u64, artifact_len: usize, artifact_name: u64) -> u64 {
        catten_syscall::spawn_artifact(artifact, artifact_len, artifact_name)
    }

    pub fn retire_artifact_named(principal: u64) -> u64 {
        catten_syscall::retire_artifact_named(principal)
    }

    pub fn retire_artifact_for_node_shutdown(principal: u64, deadline_ms: u64) -> u64 {
        catten_syscall::retire_artifact_for_node_shutdown(principal, deadline_ms)
    }

    pub fn force_retire_artifact_named(principal: u64) -> u64 {
        catten_syscall::force_retire_artifact_named(principal)
    }

    pub fn request_node_shutdown(envelope: u64, envelope_len: usize) -> u64 {
        catten_syscall::request_node_shutdown(envelope, envelope_len)
    }
}

#[cfg(test)]
mod kernel {
    extern crate std;

    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            MutexGuard,
        },
        vec::Vec,
    };

    use catten_syscall::{
        self,
        DmaDirection,
        IpcRights,
        OpCode,
    };

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Event {
        MemoryClose(u64),
        MemoryUnmap(u64),
        DmaUnmap(u64, u64),
        DeviceUnmap(u64),
        DeviceClose(u64),
        CompletionCancel(u64),
        CompletionWait(u64),
        CompletionClose(u64),
        IpcClose(u64),
        RetireArtifact(u64),
        RetireArtifactForNodeShutdown(u64, u64),
        ForceRetireArtifact(u64),
        RequestNodeShutdown(u64, usize),
    }

    pub struct State {
        pub memory_alloc: u64,
        pub memory_size: usize,
        pub memory_map_any: (u64, usize),
        pub memory_unmap: VecDeque<u64>,
        pub memory_close: VecDeque<u64>,
        pub dma_map: u64,
        pub dma_unmap: VecDeque<u64>,
        pub device_map: (u64, usize),
        pub device_unmap: VecDeque<u64>,
        pub device_bind: u64,
        pub device_ack: (u64, u64),
        pub submit: u64,
        pub poll: VecDeque<(u64, u64)>,
        pub wait_timeout: VecDeque<(u64, u64)>,
        pub ipc_send: u64,
        pub ipc_call: u64,
        pub ipc_endpoint: u64,
        pub ipc_connection: u64,
        pub ipc_bind: u64,
        pub ipc_receive: VecDeque<catten_syscall::IpcMessage>,
        pub ipc_send_move: u64,
        pub ipc_call_move: u64,
        pub ipc_vector_send: u64,
        pub ipc_vector_call: u64,
        pub ipc_reply: VecDeque<(u64, u64, u64, u64)>,
        pub ipc_reply_status: u64,
        pub connection_watch: u64,
        pub scoped_spawn: u64,
        pub retire_result: u64,
        pub node_shutdown_result: u64,
        pub events: Vec<Event>,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                memory_alloc: 10,
                memory_size: 4096,
                memory_map_any: (catten_syscall::memory_status::OK, 0x1000),
                memory_unmap: VecDeque::new(),
                memory_close: VecDeque::new(),
                dma_map: 0x2000,
                dma_unmap: VecDeque::new(),
                device_map: (catten_syscall::device_status::OK, 0x3000),
                device_unmap: VecDeque::new(),
                device_bind: catten_syscall::device_status::OK,
                device_ack: (catten_syscall::device_status::OK, 0),
                submit: 20,
                poll: VecDeque::new(),
                wait_timeout: VecDeque::new(),
                ipc_send: catten_syscall::ipc_status::OK,
                ipc_call: 30,
                ipc_endpoint: 40,
                ipc_connection: 41,
                ipc_bind: catten_syscall::ipc_status::OK,
                ipc_receive: VecDeque::new(),
                ipc_send_move: catten_syscall::ipc_status::OK,
                ipc_call_move: 30,
                ipc_vector_send: catten_syscall::ipc_status::OK,
                ipc_vector_call: 30,
                ipc_reply: VecDeque::new(),
                ipc_reply_status: catten_syscall::ipc_status::OK,
                connection_watch: 20,
                scoped_spawn: 2,
                retire_result: 0,
                node_shutdown_result: 0,
                events: Vec::new(),
            }
        }
    }

    static SERIAL: Mutex<()> = Mutex::new(());
    static STATE: Mutex<Option<State>> = Mutex::new(None);

    pub fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().expect("test serialization mutex poisoned")
    }

    pub fn reset() {
        *STATE.lock().expect("test kernel mutex poisoned") = Some(State::default());
    }

    pub fn update(f: impl FnOnce(&mut State)) {
        f(STATE
            .lock()
            .expect("test kernel mutex poisoned")
            .as_mut()
            .expect("test kernel is not initialized"));
    }

    pub fn events() -> Vec<Event> {
        STATE
            .lock()
            .expect("test kernel mutex poisoned")
            .as_ref()
            .expect("test kernel is not initialized")
            .events
            .clone()
    }

    fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
        f(STATE
            .lock()
            .expect("test kernel mutex poisoned")
            .as_mut()
            .expect("test kernel is not initialized"))
    }

    pub fn memory_alloc(_pages: usize) -> u64 {
        with_state(|state| state.memory_alloc)
    }

    pub fn memory_size(_cap: u64) -> usize {
        with_state(|state| state.memory_size)
    }

    pub fn memory_map_any(_cap: u64, _writable: bool) -> (u64, usize) {
        with_state(|state| state.memory_map_any)
    }

    pub fn memory_unmap(cap: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::MemoryUnmap(cap));
            state.memory_unmap.pop_front().unwrap_or(catten_syscall::memory_status::OK)
        })
    }

    pub fn memory_close(cap: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::MemoryClose(cap));
            state.memory_close.pop_front().unwrap_or(catten_syscall::memory_status::OK)
        })
    }

    pub fn dma_map_exclusive(_domain: u64, _memory: u64, _direction: DmaDirection) -> u64 {
        with_state(|state| state.dma_map)
    }

    pub fn dma_map(_domain: u64, _memory: u64, _direction: DmaDirection) -> u64 {
        with_state(|state| state.dma_map)
    }

    pub fn dma_unmap(domain: u64, iova: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::DmaUnmap(domain, iova));
            state.dma_unmap.pop_front().unwrap_or(catten_syscall::device_status::OK)
        })
    }

    pub fn device_mmio_map_any(_cap: u64, _writable: bool) -> (u64, usize) {
        with_state(|state| state.device_map)
    }

    pub fn device_mmio_unmap(cap: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::DeviceUnmap(cap));
            state.device_unmap.pop_front().unwrap_or(catten_syscall::device_status::OK)
        })
    }

    pub fn device_irq_bind_cq(_cap: u64, _cq: u32) -> u64 {
        with_state(|state| state.device_bind)
    }

    pub fn device_irq_ack(_cap: u64) -> (u64, u64) {
        with_state(|state| state.device_ack)
    }

    pub fn device_close(cap: u64) -> u64 {
        with_state(|state| state.events.push(Event::DeviceClose(cap)));
        catten_syscall::device_status::OK
    }

    pub fn submit(_op: OpCode) -> u64 {
        with_state(|state| state.submit)
    }

    pub fn submit_timer(_timeout_ms: u64) -> u64 {
        with_state(|state| state.submit)
    }

    pub unsafe fn submit_read(_buf_ptr: usize, _buf_len: usize) -> u64 {
        with_state(|state| state.submit)
    }

    pub fn poll(_cap: u64) -> (u64, u64) {
        with_state(|state| {
            state.poll.pop_front().unwrap_or((catten_syscall::completion_status::READY, 0))
        })
    }

    pub fn wait(cap: u64) {
        with_state(|state| state.events.push(Event::CompletionWait(cap)));
    }

    pub fn wait_timeout(_cap: u64, _timeout_ms: u64) -> (u64, u64) {
        with_state(|state| {
            state
                .wait_timeout
                .pop_front()
                .unwrap_or((catten_syscall::completion_status::PENDING_OR_TIMEOUT, 0))
        })
    }

    pub fn cancel(cap: u64) {
        with_state(|state| state.events.push(Event::CompletionCancel(cap)));
    }

    pub fn close(cap: u64) {
        with_state(|state| state.events.push(Event::CompletionClose(cap)));
    }

    pub fn ipc_scalar_send(_connection: u64, _opcode: u32, _arg0: u64) -> u64 {
        with_state(|state| state.ipc_send)
    }

    pub fn ipc_endpoint_create(_interface: u64, _version: u32, _capacity: usize) -> u64 {
        with_state(|state| state.ipc_endpoint)
    }

    pub fn ipc_connect(_endpoint: u64, _rights: IpcRights) -> u64 {
        with_state(|state| state.ipc_connection)
    }

    pub fn ipc_endpoint_bind_cq(_endpoint: u64, _cq: u32) -> u64 {
        with_state(|state| state.ipc_bind)
    }

    pub fn ipc_endpoint_resize(_endpoint: u64, capacity: usize) -> u64 {
        capacity as u64
    }

    pub fn ipc_endpoint_status(_endpoint: u64) -> (u64, u64, u64) {
        (0, 0, 0)
    }

    pub fn ipc_recv(_endpoint: u64) -> catten_syscall::IpcMessage {
        with_state(|state| {
            state.ipc_receive.pop_front().unwrap_or(catten_syscall::IpcMessage {
                status: catten_syscall::ipc_status::NO_MESSAGE,
                opcode: 0,
                arg0: 0,
                reply: 0,
                sender: 0,
                sender_generation: 0,
                sender_principal: 0,
                sender_roles: 0,
                interface: 0,
                version: 0,
                memory: 0,
                connection: 0,
            })
        })
    }

    pub fn ipc_recv_block(endpoint: u64) -> catten_syscall::IpcMessage {
        ipc_recv(endpoint)
    }

    pub fn ipc_recv_authenticated(endpoint: u64) -> catten_syscall::IpcMessage {
        ipc_recv(endpoint)
    }

    pub fn ipc_recv_block_authenticated(endpoint: u64) -> catten_syscall::IpcMessage {
        ipc_recv(endpoint)
    }

    pub fn ipc_scalar_call(_connection: u64, _opcode: u32, _arg0: u64) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_send_move(_connection: u64, _opcode: u32, _arg0: u64, _memory: u64) -> u64 {
        with_state(|state| state.ipc_send_move)
    }

    pub fn ipc_scalar_call_move(_connection: u64, _opcode: u32, _arg0: u64, _memory: u64) -> u64 {
        with_state(|state| state.ipc_call_move)
    }

    pub fn ipc_scalar_call_borrow_read(
        _connection: u64,
        _opcode: u32,
        _arg0: u64,
        _memory: u64,
    ) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_call_borrow_write(
        _connection: u64,
        _opcode: u32,
        _arg0: u64,
        _memory: u64,
    ) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_call_copy(_connection: u64, _opcode: u32, _arg0: u64, _memory: u64) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_call_connection(
        _connection: u64,
        _opcode: u32,
        _arg0: u64,
        _endpoint: u64,
        _rights: IpcRights,
    ) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_scalar_call_connection_copy(
        _connection: u64,
        _opcode: u32,
        _arg0: u64,
        _endpoint: u64,
        _rights: IpcRights,
        _memory: u64,
    ) -> u64 {
        with_state(|state| state.ipc_call)
    }

    pub fn ipc_vector_send(_connection: u64, _opcode: u32, _arg0: u64, _descriptor: u64) -> u64 {
        with_state(|state| state.ipc_vector_send)
    }

    pub fn ipc_vector_call(_connection: u64, _opcode: u32, _arg0: u64, _descriptor: u64) -> u64 {
        with_state(|state| state.ipc_vector_call)
    }

    pub fn ipc_connection_watch_closed(_connection: u64) -> u64 {
        with_state(|state| state.connection_watch)
    }

    pub fn ipc_reply_poll_with_memory(_call: u64) -> (u64, u64, u64, u64) {
        with_state(|state| state.ipc_reply.pop_front().unwrap_or((1, 0, 0, 0)))
    }

    pub fn ipc_reply_wait_with_memory(_call: u64) -> (u64, u64, u64, u64) {
        with_state(|state| {
            state.ipc_reply.pop_front().unwrap_or((catten_syscall::ipc_status::OK, 0, 0, 0))
        })
    }

    pub fn ipc_reply(_reply: u64, _result: i64) -> u64 {
        with_state(|state| state.ipc_reply_status)
    }

    pub fn ipc_reply_move(_reply: u64, _memory: u64, _result: i64) -> u64 {
        with_state(|state| state.ipc_reply_status)
    }

    pub fn ipc_reply_connection(
        _reply: u64,
        _endpoint: u64,
        _rights: IpcRights,
        _result: i64,
    ) -> u64 {
        with_state(|state| state.ipc_reply_status)
    }

    pub fn ipc_close(cap: u64) -> u64 {
        with_state(|state| state.events.push(Event::IpcClose(cap)));
        catten_syscall::ipc_status::OK
    }

    pub fn spawn_artifact_scoped(
        _artifact: u64,
        _artifact_len: usize,
        _artifact_name: u64,
        _descriptor: u64,
        _descriptor_len: usize,
    ) -> u64 {
        with_state(|state| state.scoped_spawn)
    }

    pub fn spawn_operational_connector(_package: u64, _package_len: usize, _principal: u64) -> u64 {
        with_state(|state| state.scoped_spawn)
    }

    pub fn spawn_artifact(_artifact: u64, _artifact_len: usize, _artifact_name: u64) -> u64 {
        with_state(|state| state.scoped_spawn)
    }

    pub fn retire_artifact_named(principal: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::RetireArtifact(principal));
            state.retire_result
        })
    }

    pub fn retire_artifact_for_node_shutdown(principal: u64, deadline_ms: u64) -> u64 {
        with_state(|state| {
            state.events.push(Event::RetireArtifactForNodeShutdown(principal, deadline_ms));
            state.retire_result
        })
    }

    pub fn force_retire_artifact_named(principal: u64) -> u64 {
        with_state(|state| state.events.push(Event::ForceRetireArtifact(principal)));
        0
    }

    pub fn request_node_shutdown(envelope: u64, envelope_len: usize) -> u64 {
        with_state(|state| {
            state.events.push(Event::RequestNodeShutdown(envelope, envelope_len));
            state.node_shutdown_result
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::collections::VecDeque;

    use catten_syscall::{
        self,
        DmaDirection,
        IpcRights,
        OpCode,
    };

    use super::{
        ArtifactLaunchError,
        CapabilityVector,
        Completion,
        Connection,
        DeployedArtifact,
        DmaDomain,
        Endpoint,
        IncomingMessage,
        MemoryError,
        MmioRegion,
        NodeShutdownRequestError,
        OwnedMemory,
        ReadOperation,
        kernel,
        launch_operational_connector,
        request_node_shutdown,
        spawn_scoped_artifact,
    };

    fn setup() -> std::sync::MutexGuard<'static, ()> {
        let guard = kernel::serial();
        kernel::reset();
        guard
    }

    #[test]
    fn mapping_unmap_failure_preserves_retryable_owner() {
        let _guard = setup();
        kernel::update(|state| {
            state.memory_unmap = VecDeque::from([
                catten_syscall::memory_status::UNMAP_FAILED,
                catten_syscall::memory_status::OK,
            ]);
        });

        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let mapping = memory.map_writable().expect("memory mapping");
        let (mapping, _) = mapping.unmap().expect_err("first unmap must fail");
        let memory = mapping.unmap().expect("retry must retain the owner");
        drop(memory);

        assert_eq!(
            kernel::events(),
            [
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryClose(10),
            ]
        );
    }

    #[test]
    fn dma_finish_failure_preserves_exclusive_transfer() {
        let _guard = setup();
        kernel::update(|state| {
            state.dma_unmap = VecDeque::from([
                catten_syscall::device_status::MAP_FAILED,
                catten_syscall::device_status::OK,
            ]);
        });

        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let domain = unsafe { DmaDomain::from_raw(7) };
        let transfer = memory.begin_dma(&domain, DmaDirection::DeviceRead).expect("DMA transfer");
        let (transfer, _) = transfer.finish().expect_err("first DMA unmap must fail");
        let memory = transfer.finish().expect("retry must retain DMA ownership");
        drop(memory);

        assert_eq!(
            kernel::events(),
            [
                kernel::Event::DmaUnmap(7, 0x2000),
                kernel::Event::DmaUnmap(7, 0x2000),
                kernel::Event::MemoryClose(10),
            ]
        );
    }

    #[test]
    fn shared_dma_drop_unmaps_device_before_cpu_and_close() {
        let _guard = setup();
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let domain = unsafe { DmaDomain::from_raw(7) };
        let shared = memory
            .map_shared_dma(&domain, DmaDirection::Bidirectional)
            .expect("shared DMA mapping");
        drop(shared);

        assert_eq!(
            kernel::events(),
            [
                kernel::Event::DmaUnmap(7, 0x2000),
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryClose(10),
            ]
        );
    }

    #[test]
    fn shared_dma_mapping_failure_restores_memory_owner() {
        let _guard = setup();
        kernel::update(|state| state.dma_map = 0);
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let domain = unsafe { DmaDomain::from_raw(7) };
        let error = memory
            .map_shared_dma(&domain, DmaDirection::DeviceWrite)
            .expect_err("DMA mapping must fail");
        assert_eq!(error.error(), MemoryError::DmaMapFailed);
        drop(error);
        assert_eq!(
            kernel::events(),
            [kernel::Event::MemoryUnmap(10), kernel::Event::MemoryClose(10)]
        );
    }

    #[test]
    fn shared_dma_cpu_unmap_failure_does_not_repeat_dma_unmap() {
        let _guard = setup();
        kernel::update(|state| {
            state.memory_unmap = VecDeque::from([
                catten_syscall::memory_status::UNMAP_FAILED,
                catten_syscall::memory_status::OK,
            ]);
        });
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let domain = unsafe { DmaDomain::from_raw(7) };
        let shared = memory
            .map_shared_dma(&domain, DmaDirection::Bidirectional)
            .expect("shared DMA mapping");
        let (shared, _) = shared.finish().expect_err("first CPU unmap must fail");
        let memory = shared.finish().expect("retry must preserve ownership");
        drop(memory);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::DmaUnmap(7, 0x2000),
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryClose(10),
            ]
        );
    }

    #[test]
    fn dropped_read_cancels_waits_then_closes() {
        let _guard = setup();
        let mut bytes = [0_u8; 4];
        drop(ReadOperation::submit(&mut bytes).expect("read submission"));
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::CompletionCancel(20),
                kernel::Event::CompletionWait(20),
                kernel::Event::CompletionClose(20),
            ]
        );
    }

    #[test]
    fn completion_timeout_keeps_capability_until_terminal() {
        let _guard = setup();
        kernel::update(|state| {
            state.wait_timeout = VecDeque::from([
                (catten_syscall::completion_status::PENDING_OR_TIMEOUT, 0),
                (catten_syscall::completion_status::READY, 42),
            ]);
        });
        let mut completion = Completion::submit(OpCode::Nop).expect("completion submission");
        assert_eq!(completion.wait_timeout(1), Ok(None));
        assert_eq!(completion.wait_timeout(1), Ok(Some(42)));
        drop(completion);
        assert_eq!(kernel::events(), [kernel::Event::CompletionClose(20)]);
    }

    #[test]
    fn failed_move_returns_memory_to_the_caller() {
        let _guard = setup();
        kernel::update(|state| state.ipc_send_move = catten_syscall::ipc_status::QUEUE_FULL);
        let connection = unsafe { Connection::from_raw(8) }.expect("connection");
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let (memory, _) = connection.send_move(1, 2, memory).expect_err("move must fail");
        drop(memory);
        drop(connection);
        assert_eq!(kernel::events(), [kernel::Event::MemoryClose(10), kernel::Event::IpcClose(8)]);
    }

    #[test]
    fn scoped_launch_rejects_lengths_without_leaking_inputs() {
        let _guard = setup();
        let artifact = unsafe { OwnedMemory::from_raw(11) }.expect("artifact memory");
        let descriptor = unsafe { OwnedMemory::from_raw(12) }.expect("descriptor memory");

        assert_eq!(
            spawn_scoped_artifact(artifact, 1, 1, descriptor, 0),
            Err(ArtifactLaunchError::InvalidLength)
        );
        assert_eq!(
            kernel::events(),
            [kernel::Event::MemoryClose(12), kernel::Event::MemoryClose(11)]
        );
    }

    #[test]
    fn scoped_launch_transfers_both_inputs_on_submission() {
        let _guard = setup();
        let artifact = unsafe { OwnedMemory::from_raw(11) }.expect("artifact memory");
        let descriptor = unsafe { OwnedMemory::from_raw(12) }.expect("descriptor memory");

        assert_eq!(
            spawn_scoped_artifact(
                artifact,
                1,
                1,
                descriptor,
                charlotte_launch::deployment::HEADER_LEN,
            ),
            Ok(2)
        );
        assert!(kernel::events().is_empty());
    }

    #[test]
    fn deployed_artifact_polls_cooperatively_then_forces_on_owner_drop() {
        let _guard = setup();
        kernel::update(|state| state.retire_result = 1);
        let mut artifact = DeployedArtifact {
            principal: 0x8000_1234,
            asid: 7,
            retired: false,
        };

        assert_eq!(artifact.poll_retire(), Ok(false));
        drop(artifact);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::RetireArtifact(0x8000_1234),
                kernel::Event::ForceRetireArtifact(0x8000_1234),
            ]
        );
    }

    #[test]
    fn completed_deployment_retirement_does_not_force_again() {
        let _guard = setup();
        let mut artifact = DeployedArtifact {
            principal: 0x8000_5678,
            asid: 8,
            retired: false,
        };

        assert_eq!(artifact.poll_retire(), Ok(true));
        drop(artifact);
        assert_eq!(kernel::events(), [kernel::Event::RetireArtifact(0x8000_5678)]);
    }

    #[test]
    fn node_shutdown_retirement_carries_the_enclosing_deadline() {
        let _guard = setup();
        kernel::update(|state| state.retire_result = 1);
        let mut artifact = DeployedArtifact {
            principal: 0x8000_9abc,
            asid: 9,
            retired: false,
        };

        assert_eq!(artifact.poll_node_shutdown(12_345), Ok(false));
        drop(artifact);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::RetireArtifactForNodeShutdown(0x8000_9abc, 12_345),
                kernel::Event::ForceRetireArtifact(0x8000_9abc),
            ]
        );
    }

    #[test]
    fn node_shutdown_request_transfers_the_signed_envelope() {
        let _guard = setup();
        let envelope = unsafe { OwnedMemory::from_raw(15) }.expect("shutdown envelope");

        assert_eq!(
            request_node_shutdown(envelope, charlotte_launch::shutdown::ENCODED_LEN),
            Ok(())
        );
        assert_eq!(
            kernel::events(),
            [kernel::Event::RequestNodeShutdown(15, charlotte_launch::shutdown::ENCODED_LEN,)]
        );
    }

    #[test]
    fn invalid_node_shutdown_request_retains_then_drops_the_envelope() {
        let _guard = setup();
        let envelope = unsafe { OwnedMemory::from_raw(16) }.expect("shutdown envelope");

        assert_eq!(
            request_node_shutdown(envelope, charlotte_launch::shutdown::ENCODED_LEN - 1),
            Err(NodeShutdownRequestError::InvalidEnvelope)
        );
        assert_eq!(kernel::events(), [kernel::Event::MemoryClose(16)]);
    }

    #[test]
    fn scoped_launch_kernel_rejection_still_consumes_both_inputs() {
        let _guard = setup();
        kernel::update(|state| state.scoped_spawn = 0);
        let artifact = unsafe { OwnedMemory::from_raw(11) }.expect("artifact memory");
        let descriptor = unsafe { OwnedMemory::from_raw(12) }.expect("descriptor memory");

        assert_eq!(
            spawn_scoped_artifact(
                artifact,
                1,
                1,
                descriptor,
                charlotte_launch::deployment::HEADER_LEN,
            ),
            Err(ArtifactLaunchError::Rejected)
        );
        assert!(kernel::events().is_empty());
    }

    #[test]
    fn operational_launch_rejects_lengths_without_leaking_package() {
        let _guard = setup();
        let package = unsafe { OwnedMemory::from_raw(11) }.expect("pickup memory");

        assert!(matches!(
            launch_operational_connector(package, 1, b"kafka"),
            Err(ArtifactLaunchError::InvalidLength)
        ));
        assert_eq!(kernel::events(), [kernel::Event::MemoryClose(11)]);
    }

    #[test]
    fn operational_launch_kernel_rejection_consumes_package() {
        let _guard = setup();
        kernel::update(|state| state.scoped_spawn = 0);
        let package = unsafe { OwnedMemory::from_raw(11) }.expect("pickup memory");

        assert!(matches!(
            launch_operational_connector(
                package,
                charlotte_launch::operations_pickup::PICKUP_HEADER_LEN,
                b"kafka",
            ),
            Err(ArtifactLaunchError::Rejected)
        ));
        assert!(kernel::events().is_empty());
    }

    #[test]
    fn pending_borrow_is_revoked_when_call_is_dropped() {
        let _guard = setup();
        let connection = unsafe { Connection::from_raw(8) }.expect("connection");
        let mut memory = OwnedMemory::allocate(1).expect("memory allocation");
        let call = connection.call_borrow_write(1, 2, &mut memory).expect("borrowed call");
        drop(call);
        drop(memory);
        drop(connection);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::IpcClose(30),
                kernel::Event::MemoryClose(10),
                kernel::Event::IpcClose(8),
            ]
        );
    }

    #[test]
    fn mmio_unmap_failure_preserves_retryable_mapping() {
        let _guard = setup();
        kernel::update(|state| {
            state.device_unmap = VecDeque::from([
                catten_syscall::device_status::MAP_FAILED,
                catten_syscall::device_status::OK,
            ]);
        });
        let mmio = unsafe { MmioRegion::from_raw(50) };
        let mapping = mmio.map(true).expect("MMIO mapping");
        let (mapping, _) = mapping.unmap().expect_err("first MMIO unmap must fail");
        let mmio = mapping.unmap().expect("retry must retain MMIO ownership");
        drop(mmio);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::DeviceUnmap(50),
                kernel::Event::DeviceUnmap(50),
                kernel::Event::DeviceClose(50),
            ]
        );
    }

    #[test]
    fn endpoint_and_connection_close_exactly_once() {
        let _guard = setup();
        let endpoint = Endpoint::create(1, 1, 4).expect("endpoint creation");
        let connection = endpoint.connect(IpcRights::CALL).expect("connection creation");
        drop(connection);
        drop(endpoint);
        assert_eq!(kernel::events(), [kernel::Event::IpcClose(41), kernel::Event::IpcClose(40)]);
    }

    #[test]
    fn incoming_message_owns_every_attachment() {
        let _guard = setup();
        kernel::update(|state| {
            state.ipc_receive.push_back(catten_syscall::IpcMessage {
                status: catten_syscall::ipc_status::OK,
                opcode: 7,
                arg0: 9,
                reply: 43,
                sender: 1,
                sender_generation: 2,
                sender_principal: 3,
                sender_roles: 4,
                interface: 5,
                version: 6,
                memory: 10,
                connection: 42,
            });
        });
        let endpoint = Endpoint::create(1, 1, 4).expect("endpoint creation");
        let message: IncomingMessage =
            endpoint.try_receive().expect("receive status").expect("queued message");
        assert_eq!(message.opcode, 7);
        drop(message);
        drop(endpoint);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::IpcClose(43),
                kernel::Event::MemoryClose(10),
                kernel::Event::IpcClose(42),
                kernel::Event::IpcClose(40),
            ]
        );
    }

    #[test]
    fn failed_reply_move_returns_memory_owner() {
        let _guard = setup();
        kernel::update(|state| {
            state.ipc_reply_status = catten_syscall::ipc_status::QUEUE_FULL;
            state.ipc_receive.push_back(catten_syscall::IpcMessage {
                status: catten_syscall::ipc_status::OK,
                opcode: 1,
                arg0: 0,
                reply: 43,
                sender: 0,
                sender_generation: 0,
                sender_principal: 0,
                sender_roles: 0,
                interface: 1,
                version: 1,
                memory: 0,
                connection: 0,
            });
        });
        let endpoint = Endpoint::create(1, 1, 4).expect("endpoint creation");
        let message = endpoint.try_receive().expect("receive status").expect("queued message");
        let reply = message.reply.expect("reply token");
        let memory = OwnedMemory::allocate(1).expect("memory allocation");
        let (memory, _) = reply.reply_move(memory, 4).expect_err("reply must fail");
        drop(memory);
        drop(endpoint);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::IpcClose(43),
                kernel::Event::MemoryClose(10),
                kernel::Event::IpcClose(40),
            ]
        );
    }

    #[test]
    fn reply_from_connection_borrow_preserves_its_owner() {
        let _guard = setup();
        kernel::update(|state| {
            state.ipc_receive.push_back(catten_syscall::IpcMessage {
                status: catten_syscall::ipc_status::OK,
                opcode: 1,
                arg0: 0,
                reply: 43,
                sender: 0,
                sender_generation: 0,
                sender_principal: 0,
                sender_roles: 0,
                interface: 1,
                version: 1,
                memory: 0,
                connection: 0,
            });
        });
        let endpoint = Endpoint::create(1, 1, 4).expect("endpoint creation");
        let message = endpoint.try_receive().expect("receive status").expect("queued message");
        let reply = message.reply.expect("reply token");
        let connection = unsafe { Connection::from_raw(42) }.expect("connection");
        reply
            .reply_connection_ref(connection.as_ref(), IpcRights::CALL, 7)
            .expect("reply delegation");
        drop(connection);
        drop(endpoint);
        assert_eq!(kernel::events(), [kernel::Event::IpcClose(42), kernel::Event::IpcClose(40)]);
    }

    #[test]
    fn failed_vector_call_rolls_descriptor_back_and_returns_moves() {
        let _guard = setup();
        let mut descriptor_page = [0_u8; 4096];
        kernel::update(|state| {
            state.memory_map_any =
                (catten_syscall::memory_status::OK, descriptor_page.as_mut_ptr() as usize);
            state.ipc_vector_call = 0;
        });
        let connection = unsafe { Connection::from_raw(8) }.expect("connection");
        let memory = unsafe { OwnedMemory::from_raw(11) }.expect("memory");
        let mut vector = CapabilityVector::new();
        vector.push_move(memory).expect("vector entry");
        let (vector, _) = connection.call_vector(1, 2, vector).expect_err("vector call must fail");
        drop(vector);
        drop(connection);
        assert_eq!(
            kernel::events(),
            [
                kernel::Event::MemoryUnmap(10),
                kernel::Event::MemoryClose(10),
                kernel::Event::MemoryClose(11),
                kernel::Event::IpcClose(8),
            ]
        );
    }
}
