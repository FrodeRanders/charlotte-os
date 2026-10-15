//! Real observer owners exercise local disposal guards and exact CQ wakes.
//! Private serialized namespaces do not establish physical root quiescence.

use alloc::boxed::Box;
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    klib::observer::{
        CallOnNotify,
        WaitSponsor,
        registration::ObserverList,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

const CLIENT: usize = 0x5e32;

struct DisposalProbe(Mutex<Option<Box<dyn FnOnce() + Send>>>);

impl Observer for DisposalProbe {
    fn notify(self: Arc<Self>) {
        panic!("subscription disposal fabricated producer completion");
    }
}

impl Drop for DisposalProbe {
    fn drop(&mut self) {
        self.0.get_mut().take().unwrap()();
    }
}

fn observation(
    account: &Arc<watch_budget::DomainBudget>,
    action: impl FnOnce() + Send + 'static,
) -> (EventObservation, crate::klib::observer::registration::ListRef<watch_budget::Charge>) {
    let observer: Arc<dyn Observer> = Arc::new(DisposalProbe(Mutex::new(Some(Box::new(action)))));
    let source = ObserverList::try_new(1, false).unwrap();
    let registration = source
        .register(Arc::downgrade(&observer), watch_budget::reserve(account, false).unwrap())
        .unwrap();
    (
        EventObservation {
            _observer: observer,
            _registration: registration,
        },
        source,
    )
}

fn install(
    submission: &EventSubmission,
    owner: EventObservation,
    worker: bool,
) -> Result<bool, SubmitError> {
    submission.install_observation(owner._observer, owner._registration, worker)
}

fn transition_detachment() {
    open_address_space(CLIENT, 8);
    let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![7])).unwrap();
    let captured = completion_of(CLIENT, cap).unwrap();
    let account = watch_admission(CLIENT).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let mut sources = Vec::new();
    for worker in [false, true] {
        let weak = Arc::downgrade(&captured);
        let counter = hits.clone();
        let (owner, source) = observation(&account, move || {
            let _registry = COMPLETIONS.write();
            let completion = weak.upgrade().unwrap();
            let inner = completion.inner.lock();
            assert!(matches!(inner.state, OpState::Completed(OpResult::Ok(7))));
            assert_eq!(inner.buffer.as_deref(), Some(&[7][..]));
            counter.fetch_add(1, Ordering::SeqCst);
        });
        if worker {
            captured.set_worker_observation(owner).finish();
        } else {
            captured.set_event_observation(owner._observer, owner._registration).finish();
        }
        sources.push(source);
    }
    let transition;
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        transition = captured.complete(OpResult::Ok(7)).unwrap();
        assert_eq!(transition.result, OpResult::Ok(7));
        assert!(captured.complete(OpResult::Err(99)).is_none());
        assert_eq!(account.used(), 2);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }
    drop(transition);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(account.used(), 0);
    assert!(sources.iter().all(|source| source.registered() == 0));
    assert_eq!(poll(CLIENT, cap).unwrap().unwrap().buffer.unwrap(), [7]);
    close(CLIENT, cap).unwrap();
    close_address_space(CLIENT);
}

fn public_terminal(cancel_pending: bool, replace_queue: bool) {
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    let cap = submit(CLIENT, OpCode::Read, Some(alloc::vec![8])).unwrap();
    let captured = completion_of(CLIENT, cap).unwrap();
    let account = watch_admission(CLIENT).unwrap();
    let records = record_admission(CLIENT).unwrap();
    let expected = if cancel_pending {
        OpResult::Cancelled
    } else {
        OpResult::Ok(8)
    };
    let hits = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let successor_calls = Arc::new(AtomicUsize::new(0));
    let successor_token = Arc::new(Mutex::new(None));
    let successor_sponsor = WaitSponsor::new(false);
    let counter = successor_calls.clone();
    let successor_observer: Arc<dyn Observer> = CallOnNotify::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let mut sources = Vec::new();
    for worker in [false, true].into_iter().take(
        if replace_queue {
            1
        } else {
            2
        },
    ) {
        let weak = Arc::downgrade(&captured);
        let counter = hits.clone();
        let expected = expected.clone();
        let sponsor = successor_sponsor.clone();
        let observer = successor_observer.clone();
        let token = successor_token.clone();
        let (owner, source) = observation(&account, move || {
            {
                let registry = COMPLETIONS.write();
                let completion = weak.upgrade().unwrap();
                let inner = completion.inner.lock();
                assert!(matches!(&inner.state, OpState::Completed(result) if *result == expected));
                assert_eq!(inner.buffer.as_deref(), Some(&[8][..]));
                let queue = &registry[&CLIENT].cqs[&DEFAULT_CQ];
                assert_eq!(queue.ring_head, 1);
                assert_ne!(queue.work_generation, 0);
                // SAFETY: this private heap ring has no concurrent consumer;
                // the registry retains its whole backing through this read.
                let entry = unsafe { core::ptr::read((*queue.ring).entries.as_ptr()) };
                assert_eq!(entry.cookie, cap);
                assert_eq!(cq::fields_to_op_result(entry.status, entry.result), expected);
            }
            counter.fetch_add(1, Ordering::SeqCst);
            if replace_queue {
                open_cq(CLIENT, DEFAULT_CQ, 8).unwrap();
                let successor = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
                *token.lock() =
                    Some(sponsor.register(&successor.identity, Arc::downgrade(&observer)).unwrap());
            }
        });
        if worker {
            captured.set_worker_observation(owner).finish();
        } else {
            captured.set_event_observation(owner._observer, owner._registration).finish();
        }
        sources.push(source);
    }
    let counter = calls.clone();
    let disposals = hits.clone();
    let weak = Arc::downgrade(&captured);
    let notification: Arc<dyn Observer> = CallOnNotify::new(move || {
        let _registry = COMPLETIONS.write();
        let completion = weak.upgrade().unwrap();
        let _inner = completion.inner.lock();
        assert_eq!(
            disposals.load(Ordering::SeqCst),
            if replace_queue {
                1
            } else {
                2
            }
        );
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let callback = observe(CLIENT, cap, notification.clone()).unwrap();
    let cq_sponsor = WaitSponsor::new(false);
    let original = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let cq_token = cq_sponsor.register(&original.identity, Arc::downgrade(&notification)).unwrap();
    if cancel_pending {
        assert_eq!(captured.cancel(), CancelState::CancelRequested);
    }
    complete_registered(CLIENT, cap, captured.clone(), OpResult::Ok(8)).unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        if replace_queue {
            1
        } else {
            2
        }
    );
    assert_eq!(successor_calls.load(Ordering::SeqCst), 0);
    assert_eq!(successor_sponsor.used(), usize::from(replace_queue));
    assert_eq!(cq_sponsor.used(), 0);
    assert_eq!(account.used(), 0);
    assert!(sources.iter().all(|source| source.registered() == 0));
    assert_eq!(records.used(), 1);
    complete_registered(CLIENT, cap, captured.clone(), OpResult::Err(99)).unwrap();
    assert_eq!(successor_calls.load(Ordering::SeqCst), 0);
    let result = poll(CLIENT, cap).unwrap().unwrap();
    assert_eq!(result.result, expected);
    assert_eq!(result.buffer.unwrap(), [8]);
    close(CLIENT, cap).unwrap();
    drop((captured, callback, cq_token));
    assert_eq!(records.used(), 1, "notification capture still retains the original weak record");
    drop(notification);
    assert_eq!(records.used(), 0);
    drop(successor_token.lock().take());
    assert_eq!(successor_sponsor.used(), 0);
    close_address_space(CLIENT);
}

fn installations() {
    for worker in [false, true] {
        for mode in 0..5 {
            open_address_space(CLIENT, 8);
            let submission = EventSubmission::new(CLIENT).unwrap();
            let captured = submission.completion().clone();
            let account = watch_admission(CLIENT).unwrap();
            let hits = Arc::new(AtomicUsize::new(0));
            let mut sources = Vec::new();
            if mode == 1 {
                captured.cancel();
            } else if mode == 2 || mode == 3 {
                complete(CLIENT, submission.cap(), OpResult::Ok(0)).unwrap();
                if mode == 3 {
                    poll(CLIENT, submission.cap()).unwrap().unwrap();
                }
            } else if mode == 4 {
                open_address_space(CLIENT, 8);
            }
            for _ in 0..if mode == 0 {
                2
            } else {
                1
            } {
                let weak = Arc::downgrade(&captured);
                let counter = hits.clone();
                let (owner, source) = observation(&account, move || {
                    let _registry = COMPLETIONS.write();
                    let completion = weak.upgrade().unwrap();
                    let _inner = completion.inner.lock();
                    counter.fetch_add(1, Ordering::SeqCst);
                });
                let result = install(&submission, owner, worker);
                if mode == 4 {
                    assert_eq!(result, Err(SubmitError::UnknownAddressSpace));
                } else {
                    assert_eq!(result.unwrap(), mode == 1 && !worker);
                }
                sources.push(source);
            }
            let retained = mode == 0 || (mode == 1 && worker);
            assert_eq!(
                sources.iter().map(|source| source.registered()).sum::<usize>(),
                usize::from(retained)
            );
            assert_eq!(hits.load(Ordering::SeqCst), usize::from(mode == 0 || !retained));
            captured.release_event_observation();
            assert_eq!(
                hits.load(Ordering::SeqCst),
                if mode == 0 {
                    2
                } else {
                    1
                }
            );
            // EventSubmission still owns its original unpublished charge.
            assert_eq!(account.used(), 1);
            drop(submission);
            assert_eq!(account.used(), 0);
            close_address_space(CLIENT);
        }
    }
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    transition_detachment();
    public_terminal(false, false);
    public_terminal(true, false);
    public_terminal(false, true);
    installations();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[completion terminal disposal] heap-held owner detachment, post-publication reentrant \
         event/worker disposal, cancellation parity, idempotency, exact CQ replacement and ten \
         installation controls passed"
    );
}
