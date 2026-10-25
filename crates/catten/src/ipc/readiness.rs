//! Endpoint/reply readiness and owning CQ/close-watch subscriptions. Identity
//! resolution and publication share the existing IPC registry; waits retain
//! their original scheduler/admission and borrowed-resource contracts.
use super::*;

mod waiter_source;
pub(crate) mod waiter_tests;

/// Debugger-visible cooperative admission retries: readable wait, reply wait.
/// Diagnostic counters only; they grant no authority and do not drive policy.
#[unsafe(no_mangle)]
pub static IPC_WAIT_ADMISSION_RETRIES: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

pub fn wait_readable(receiver: AddressSpaceId, endpoint_cap: CapabilityId) -> Result<(), IpcError> {
    let endpoint_id = {
        let ipc = IPC.read();
        receive_endpoint_id(&ipc, receiver, endpoint_cap)?
    };
    if endpoint_is_readable_or_closed(endpoint_id)? {
        return Ok(());
    }

    let tid = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
        .read()
        .get_lp_scheduler()
        .lock()
        .get_tid()
        .ok_or(IpcError::NoMessage)?;
    let observable = EndpointObservable {
        endpoint: endpoint_id,
    };
    let generation = loop {
        if endpoint_is_readable_or_closed(endpoint_id)? {
            return Ok(());
        }
        let registration = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .block_thread_with_constraint_generation(
                tid,
                &observable,
                crate::cpu::scheduler::threads::MigrationConstraint::EndpointWait,
            );
        match registration {
            Ok(generation) => break generation,
            Err(crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed) => {
                // Preserve the untimed receive contract under pressure without
                // parking after a rejected registration or killing the service.
                IPC_WAIT_ADMISSION_RETRIES[0].fetch_add(1, Ordering::Relaxed);
                crate::cpu::scheduler::yield_lp();
            }
            Err(_) => return Err(IpcError::NoMessage),
        }
    };

    // Lost-wake guard: if a sender enqueued after the fast-path check but
    // before observer registration completed, re-admit the thread immediately.
    let readable = match endpoint_is_readable_or_closed(endpoint_id) {
        Ok(readable) => readable,
        Err(error) => {
            // The endpoint was removed while this thread was parked; re-admit
            // it before returning so it is not left Blocked forever.
            let _ = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
                .read()
                .submit_woken_thread(tid, generation);
            return Err(error);
        }
    };
    if readable {
        let _ = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .submit_woken_thread(tid, generation);
    }

    crate::cpu::scheduler::yield_lp();
    Ok(())
}

pub(super) fn receive_endpoint_id(
    ipc: &IpcRegistry,
    receiver: AddressSpaceId,
    endpoint_cap: CapabilityId,
) -> Result<EndpointId, IpcError> {
    let (endpoint_id, rights) = match ipc.cap(receiver, endpoint_cap)? {
        Capability::Endpoint {
            endpoint,
            rights,
        } => (endpoint, rights),
        _ => return Err(IpcError::WrongType),
    };
    if !rights.contains(ConnectionRights::RECEIVE) {
        return Err(IpcError::PermissionDenied);
    }
    let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    if endpoint.closed {
        return Err(IpcError::EndpointClosed);
    }
    if endpoint.closing {
        return Err(IpcError::Pending);
    }
    Ok(endpoint_id)
}

pub(super) fn endpoint_is_readable_or_closed(endpoint_id: EndpointId) -> Result<bool, IpcError> {
    let ipc = IPC.read();
    let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
    Ok(endpoint_available(&ipc, endpoint))
}

pub(super) fn endpoint_available(ipc: &IpcRegistry, endpoint: &Endpoint) -> bool {
    endpoint.closed
        || (!endpoint.closing
            && endpoint.queue.front().is_some_and(|message| {
                message.reply.is_none_or(|token| {
                    ipc.reply_tokens
                        .get(&token)
                        .is_some_and(|token| !token.completing && !token.cleanup_failed)
                })
            }))
}

pub(super) struct EndpointObservable {
    endpoint: EndpointId,
}

struct EndpointCloseCompletionObserver {
    asid: AddressSpaceId,
    cap: crate::completion::CompletionCap,
    completion: crate::completion::CompletionWeak,
}

impl Observer for EndpointCloseCompletionObserver {
    fn notify(self: Arc<Self>) {
        if let Some(completion) = self.completion.upgrade() {
            let _ = crate::completion::complete_registered(
                self.asid,
                self.cap,
                completion,
                crate::completion::OpResult::Ok(REPLY_ENDPOINT_CLOSED),
            );
        }
    }
}

/// Return a completion capability that becomes ready when the endpoint named
/// by `connection_cap` closes. Registration and the closed-state check share
/// the IPC lock, closing the lost-wake race between those operations.
pub fn watch_connection_closed(
    asid: AddressSpaceId,
    connection_cap: CapabilityId,
) -> Result<crate::completion::CompletionCap, IpcError> {
    let endpoint_id = {
        let ipc = IPC.read();
        match ipc.cap(asid, connection_cap)? {
            Capability::Connection {
                endpoint,
                ..
            } => endpoint,
            _ => return Err(IpcError::WrongType),
        }
    };
    let mut submission =
        crate::completion::EventSubmission::new(asid).map_err(|_| IpcError::QueueFull)?;
    let cap = submission.cap();
    let completion = submission.completion().clone();
    let observer: Arc<dyn Observer> = Arc::try_new(EndpointCloseCompletionObserver {
        asid,
        cap,
        completion: Arc::downgrade(&completion),
    })
    .map_err(|_| IpcError::ResourceLimit)?;

    let registration = {
        let ipc = IPC.read();
        // Revalidate after staging; a revoked/reused connection must not
        // authorize registration against the previously resolved endpoint.
        if !matches!(ipc.cap(asid, connection_cap)?, Capability::Connection { endpoint, .. } if endpoint == endpoint_id)
        {
            return Err(IpcError::UnknownCapability);
        }
        let endpoint = ipc.endpoints.get(&endpoint_id).ok_or(IpcError::UnknownCapability)?;
        if endpoint.closed {
            None
        } else {
            Some(
                endpoint
                    .close_observers
                    .register(Arc::downgrade(&observer), submission.take_charge())
                    .map_err(|_| IpcError::ResourceLimit)?,
            )
        }
    };
    if let Some(registration) = registration {
        if submission
            .install_watch_observation(observer, registration)
            .map_err(|_| IpcError::UnknownCapability)?
        {
            let _ = crate::completion::complete_registered(
                asid,
                cap,
                completion,
                crate::completion::OpResult::Cancelled,
            );
        }
    } else {
        observer.notify();
    }
    Ok(submission.commit())
}

impl Observable for EndpointObservable {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        waiter_source::Source::Endpoint(self.endpoint).register(observer, sponsor)
    }
}

pub fn wait_reply(caller: AddressSpaceId, call_cap: CapabilityId) -> Result<(), IpcError> {
    let call_id = pending_call_id(caller, call_cap)?;
    let observable = PendingCallObservable {
        call: call_id,
    };
    loop {
        if pending_call_is_ready(call_id)? {
            return Ok(());
        }

        let tid = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .get_lp_scheduler()
            .lock()
            .get_tid()
            .ok_or(IpcError::NoMessage)?;
        let registration = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
            .read()
            .block_thread_with_constraint_generation(
                tid,
                &observable,
                crate::cpu::scheduler::threads::MigrationConstraint::GeneralWait,
            );
        let generation = match registration {
            Ok(generation) => generation,
            Err(crate::cpu::scheduler::system_scheduler::Error::WaitRegistrationFailed) => {
                // A borrowed PendingCall must retain its borrow until reply
                // or cancellation revokes the server's access. Do not let
                // admission pressure masquerade as a terminal wait result.
                IPC_WAIT_ADMISSION_RETRIES[1].fetch_add(1, Ordering::Relaxed);
                crate::cpu::scheduler::yield_lp();
                continue;
            }
            Err(_) => return Err(IpcError::NoMessage),
        };

        // Close the check/register race. Notifications are hints rather than
        // proof that this particular pending call completed, so after every
        // wake the loop checks the call and parks again when necessary.
        let ready = match pending_call_is_ready(call_id) {
            Ok(ready) => ready,
            Err(error) => {
                // The call was removed while this thread was parked. Its
                // observers may have been dropped without a wake, so re-admit
                // the thread before returning rather than leaving it Blocked.
                let _ = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
                    .read()
                    .submit_woken_thread(tid, generation);
                return Err(error);
            }
        };
        if ready {
            let _ = crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER
                .read()
                .submit_woken_thread(tid, generation);
        }
        crate::cpu::scheduler::yield_lp();
    }
}

/// Park on the exact pending-call observable with an owned deadline watchdog.
/// `false` means timeout; waiter/watchdog admission rejection is ResourceLimit.
/// Neither outcome observes the result or consumes the pending capability.
pub fn wait_reply_timeout(
    caller: AddressSpaceId,
    call_cap: CapabilityId,
    timeout_ms: u64,
) -> Result<bool, IpcError> {
    let call_id = pending_call_id(caller, call_cap)?;
    if pending_call_is_ready(call_id)? {
        return Ok(true);
    }
    let observable = PendingCallObservable {
        call: call_id,
    };
    let waited = crate::cpu::scheduler::try_block_until(&observable, timeout_ms, || {
        // Removed calls wake the waiter too; propagate that error after waking.
        pending_call_is_ready(call_id).unwrap_or(true)
    });
    if pending_call_is_ready(call_id)? {
        return Ok(true);
    }
    waited.map_err(|_| IpcError::ResourceLimit)
}

/// One owned subscription in the existing admitted pending-call record.
/// The record owns cancellation; no separate scalar registry is needed.
#[derive(Debug)]
pub(super) struct ReplyCqInterest {
    _observer: Arc<ReplyCqObserver>,
    _registration: WaitRegistration,
}

#[derive(Debug)]
struct ReplyCqObserver(crate::completion::CqInterest);
impl Observer for ReplyCqObserver {
    fn notify(self: Arc<Self>) {
        self.0.wake();
    }
}

/// Bind one pending call's readiness to the caller's exact CQ. This does not
/// observe its result, adopt its attachments, or transfer any memory borrow.
pub fn bind_reply_cq(
    caller: AddressSpaceId,
    call_cap: CapabilityId,
    cq: crate::completion::CqId,
) -> Result<(), IpcError> {
    let tid =
        crate::cpu::scheduler::system_scheduler::get_thread_id().ok_or(IpcError::ResourceLimit)?;
    let sponsor = {
        let table = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE.read();
        let thread = table.get(tid).map_err(|_| IpcError::ResourceLimit)?;
        if thread.asid != caller {
            return Err(IpcError::PermissionDenied);
        }
        thread.wait_sponsor.clone()
    };
    bind_reply_cq_with(caller, call_cap, cq, &sponsor)
}

pub(super) fn bind_reply_cq_with(
    caller: AddressSpaceId,
    call_cap: CapabilityId,
    cq: crate::completion::CqId,
    sponsor: &WaitSponsor,
) -> Result<(), IpcError> {
    let call_id = pending_call_id(caller, call_cap)?;
    let source = {
        let ipc = IPC.read();
        let call = ipc.pending_calls.get(&call_id).ok_or(IpcError::UnknownCapability)?;
        if call.cq_interest.is_some() {
            return Err(IpcError::ReplyAlreadyUsed);
        }
        call.observers.clone()
    };
    let queue =
        crate::completion::capture_cq_interest(caller, cq).ok_or(IpcError::UnknownCapability)?;
    let observer = Arc::try_new(ReplyCqObserver(queue)).map_err(|_| IpcError::ResourceLimit)?;
    let weak = Arc::downgrade(&observer) as Weak<dyn Observer>;
    // Admit storage outside IPC. List close serializes against registration;
    // a reply winning that race is handled by the ready check below.
    let registration = match sponsor.register(&source, weak) {
        Ok(registration) => registration,
        Err(RegistrationError::Closed) => WaitRegistration::ready(),
        Err(_) => return Err(IpcError::ResourceLimit),
    };
    let mut interest = Some(ReplyCqInterest {
        _observer: observer.clone(),
        _registration: registration,
    });
    let ready = {
        let mut ipc = IPC.write();
        if !matches!(ipc.cap(caller, call_cap)?, Capability::PendingCall { call } if call == call_id)
        {
            return Err(IpcError::UnknownCapability);
        }
        let call = ipc.pending_calls.get_mut(&call_id).ok_or(IpcError::UnknownCapability)?;
        if call.cq_interest.is_some() {
            return Err(IpcError::ReplyAlreadyUsed);
        }
        call.cq_interest = interest.take();
        call.result.is_some()
    };
    if ready {
        observer.notify();
    }
    Ok(())
}

pub(super) fn pending_call_id(
    caller: AddressSpaceId,
    call_cap: CapabilityId,
) -> Result<PendingCallId, IpcError> {
    let ipc = IPC.read();
    let call_id = match ipc.cap(caller, call_cap)? {
        Capability::PendingCall {
            call,
        } => call,
        _ => return Err(IpcError::WrongType),
    };
    let call = ipc.pending_calls.get(&call_id).ok_or(IpcError::UnknownCapability)?;
    if call.caller != caller {
        return Err(IpcError::PermissionDenied);
    }
    Ok(call_id)
}

fn pending_call_is_ready(call_id: PendingCallId) -> Result<bool, IpcError> {
    let ipc = IPC.read();
    Ok(ipc.pending_calls.get(&call_id).ok_or(IpcError::UnknownCapability)?.result.is_some())
}

pub(super) struct PendingCallObservable {
    call: PendingCallId,
}

impl Observable for PendingCallObservable {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        waiter_source::Source::Reply(self.call).register(observer, sponsor)
    }
}

impl EndpointObservable {
    pub(super) fn new(endpoint: EndpointId) -> Self {
        Self {
            endpoint,
        }
    }
}

impl PendingCallObservable {
    pub(super) fn new(call: PendingCallId) -> Self {
        Self {
            call,
        }
    }
}
