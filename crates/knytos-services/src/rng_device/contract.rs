//! Public-data fault controls, shared by host tests and the signed probe only.

use core::cell::Cell;

use super::rng_device::{
    Backend,
    Device,
    Error,
    Failure,
    Limits,
    Used,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Calls {
    prepare: usize,
    publish: usize,
    notify: usize,
    index: usize,
    entry: usize,
    copy: usize,
    pub(crate) backend_drops: usize,
    pub(crate) field_drops: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Mode {
    Healthy,
    Short,
    ShortThenTimeout,
    Prepare,
    Publish,
    Index,
    UnexpectedIndex,
    Timeout,
    Entry,
    Descriptor,
    ZeroLength,
    OverLength,
    Copy,
    #[cfg(test)]
    Panic,
    #[cfg(test)]
    CopyPanic,
}

pub(crate) struct Control {
    mode: Cell<Mode>,
    pub(crate) calls: Cell<Calls>,
    pub(crate) index: Cell<u16>,
    requested: Cell<u32>,
}

impl Control {
    pub(crate) fn new(mode: Mode) -> Self {
        Self {
            mode: Cell::new(mode),
            calls: Cell::new(Calls::default()),
            index: Cell::new(0),
            requested: Cell::new(0),
        }
    }

    fn bump(&self, change: impl FnOnce(&mut Calls)) {
        let mut calls = self.calls.get();
        change(&mut calls);
        self.calls.set(calls);
    }
}

struct Field<'a>(&'a Control);
impl Drop for Field<'_> {
    fn drop(&mut self) {
        self.0.bump(|c| c.field_drops += 1);
    }
}

pub(crate) struct Fake<'a, R = ()> {
    control: &'a Control,
    _field: Field<'a>,
    _resource: R,
}
impl<'a> Fake<'a> {
    pub(crate) fn new(control: &'a Control) -> Self {
        Self {
            control,
            _field: Field(control),
            _resource: (),
        }
    }
}
impl<R> Drop for Fake<'_, R> {
    fn drop(&mut self) {
        self.control.bump(|c| c.backend_drops += 1);
    }
}
impl<R> Backend for Fake<'_, R> {
    type Error = ();

    fn prepare(&mut self, _slot: usize, length: u32) -> Result<(), ()> {
        self.control.bump(|c| c.prepare += 1);
        self.control.requested.set(length);
        #[cfg(test)]
        if self.control.mode.get() == Mode::Panic {
            panic!("public fixture interrupt before publication");
        }
        if self.control.mode.get() == Mode::Prepare {
            Err(())
        } else {
            Ok(())
        }
    }

    fn publish(&mut self, next: u16) -> Result<(), ()> {
        self.control.bump(|c| c.publish += 1);
        let mode = self.control.mode.get();
        if mode != Mode::Timeout
            && !(mode == Mode::ShortThenTimeout && self.control.calls.get().publish > 1)
        {
            self.control.index.set(
                if mode == Mode::UnexpectedIndex {
                    next.wrapping_add(1)
                } else {
                    next
                },
            );
        }
        // Publication failure may have already changed the advertised index.
        if mode == Mode::Publish {
            Err(())
        } else {
            Ok(())
        }
    }

    fn notify(&mut self) {
        self.control.bump(|c| c.notify += 1);
    }

    fn used_index(&self) -> Result<u16, ()> {
        self.control.bump(|c| c.index += 1);
        if self.control.mode.get() == Mode::Index {
            Err(())
        } else {
            Ok(self.control.index.get())
        }
    }

    fn used_entry(&self, _slot: usize) -> Result<Used, ()> {
        self.control.bump(|c| c.entry += 1);
        let requested = self.control.requested.get();
        let mode = self.control.mode.get();
        if mode == Mode::Entry {
            return Err(());
        }
        Ok(Used {
            descriptor: u32::from(mode == Mode::Descriptor),
            length: match mode {
                Mode::ZeroLength => 0,
                Mode::OverLength => requested + 1,
                Mode::Short | Mode::ShortThenTimeout => requested.min(2),
                _ => requested,
            },
        })
    }

    fn copy_entropy(&self, destination: &mut [u8]) -> Result<(), ()> {
        self.control.bump(|c| c.copy += 1);
        destination.fill(0x3a);
        #[cfg(test)]
        if self.control.mode.get() == Mode::CopyPanic {
            panic!("public fixture interrupt after checked completion");
        }
        if self.control.mode.get() == Mode::Copy {
            Err(())
        } else {
            Ok(())
        }
    }
}

#[cfg(not(test))]
pub(crate) fn check() {
    invalid_requests_do_not_touch_backend();
    complete_and_short_results_release_normally();
    timeout_and_late_completion_never_restart();
    backend_failures_retain_every_field();
    partial_success_does_not_authorize_replay();
    actual_owned_fields_follow_the_same_fence();
}

#[cfg_attr(test, test)]
fn invalid_requests_do_not_touch_backend() {
    assert!(Limits::new(0).is_none());
    assert_eq!(Limits::DEFAULT.completion_polls(), 1_000_000);
    let control = Control::new(Mode::Healthy);
    let mut device = Device::new(Fake::new(&control), Limits::new(3).unwrap());
    assert_eq!(device.fill(&mut []), Err(Error::InvalidLength));
    assert_eq!(device.fill(&mut [0; super::entropy::MAX_REQUEST + 1]), Err(Error::InvalidLength));
    assert_eq!(control.calls.get(), Calls::default());
    assert_eq!(device.retained_failure(), None);
    drop(device);
    assert_eq!(control.calls.get().backend_drops, 1);
    assert_eq!(control.calls.get().field_drops, 1);
}

#[cfg_attr(test, test)]
fn complete_and_short_results_release_normally() {
    for mode in [Mode::Healthy, Mode::Short] {
        let control = Control::new(mode);
        let mut device = Device::new(Fake::new(&control), Limits::new(3).unwrap());
        let mut bytes = [0; 5];
        assert_eq!(device.fill(&mut bytes), Ok(()));
        assert_eq!(bytes, [0x3a; 5]);
        assert_eq!(device.retained_failure(), None);
        assert_eq!(
            control.calls.get().publish,
            if mode == Mode::Short {
                3
            } else {
                1
            }
        );
        assert_eq!(device.fill(&mut bytes), Ok(()));
        drop(device);
        assert_eq!(control.calls.get().backend_drops, 1);
        assert_eq!(control.calls.get().field_drops, 1);
    }
}

#[cfg_attr(test, test)]
fn timeout_and_late_completion_never_restart() {
    let control = Control::new(Mode::Timeout);
    let mut device = Device::new(Fake::new(&control), Limits::new(3).unwrap());
    let mut bytes = [0xa5; 4];
    assert_eq!(device.fill(&mut bytes), Err(Error::Retained(Failure::Timeout)));
    assert_eq!(control.calls.get().publish, 1);
    assert_eq!(control.calls.get().notify, 1);
    assert_eq!(control.calls.get().index, 3);
    assert_eq!(bytes, [0xa5; 4]);
    let attempt = device.retained_attempt().unwrap();
    assert_eq!(
        (
            attempt.previous_index,
            attempt.expected_index,
            attempt.slot,
            attempt.requested,
            attempt.copied_prefix
        ),
        (0, 1, 0, 4, 0)
    );
    assert_eq!(attempt.checked_completion, None);
    // A late device update cannot clear the complete owner's terminal fence.
    control.mode.set(Mode::Healthy);
    control.index.set(1);
    let before = control.calls.get();
    assert_eq!(device.fill(&mut bytes), Err(Error::Retained(Failure::Timeout)));
    assert_eq!(control.calls.get(), before);
    drop(device);
    assert_eq!(control.calls.get(), before);
}

#[cfg_attr(test, test)]
fn backend_failures_retain_every_field() {
    for (mode, failure) in [
        (Mode::Prepare, Failure::Prepare),
        (Mode::Publish, Failure::Publish),
        (Mode::Index, Failure::UsedIndex),
        (
            Mode::UnexpectedIndex,
            Failure::UnexpectedIndex {
                expected: 1,
                observed: 2,
            },
        ),
        (Mode::Entry, Failure::UsedEntry),
        (Mode::Descriptor, Failure::UnexpectedDescriptor(1)),
        (
            Mode::ZeroLength,
            Failure::InvalidUsedLength {
                requested: 4,
                observed: 0,
            },
        ),
        (
            Mode::OverLength,
            Failure::InvalidUsedLength {
                requested: 4,
                observed: 5,
            },
        ),
        (Mode::Copy, Failure::CopyAfterCompletion),
    ] {
        let control = Control::new(mode);
        let mut device = Device::new(Fake::new(&control), Limits::new(3).unwrap());
        let mut bytes = [0xa5; 4];
        assert_eq!(device.fill(&mut bytes), Err(Error::Retained(failure)));
        assert_eq!(device.retained_failure(), Some(failure));
        assert_eq!(
            device.retained_attempt().unwrap().checked_completion,
            (mode == Mode::Copy).then_some(Used {
                descriptor: 0,
                length: 4
            })
        );
        let before = control.calls.get();
        assert_eq!(device.fill(&mut bytes), Err(Error::Retained(failure)));
        assert_eq!(control.calls.get(), before);
        drop(device);
        assert_eq!(control.calls.get(), before);
    }
}

#[cfg_attr(test, test)]
fn partial_success_does_not_authorize_replay() {
    let control = Control::new(Mode::ShortThenTimeout);
    let mut device = Device::new(Fake::new(&control), Limits::new(3).unwrap());
    let mut bytes = [0xa5; 5];
    assert_eq!(device.fill(&mut bytes), Err(Error::Retained(Failure::Timeout)));
    assert_eq!(bytes, [0x3a, 0x3a, 0xa5, 0xa5, 0xa5]);
    assert_eq!(control.calls.get().publish, 2);
    let attempt = device.retained_attempt().unwrap();
    assert_eq!(
        (attempt.previous_index, attempt.expected_index, attempt.requested, attempt.copied_prefix),
        (1, 2, 3, 2)
    );
    let before = control.calls.get();
    assert_eq!(device.fill(&mut bytes), Err(Error::Retained(Failure::Timeout)));
    drop(device);
    assert_eq!(control.calls.get(), before);
}

#[cfg(not(test))]
pub(crate) fn rejecting_fill(destination: &mut [u8]) -> bool {
    let control = Control::new(Mode::Timeout);
    let mut device = Device::new(Fake::new(&control), Limits::new(3).unwrap());
    assert_eq!(device.fill(destination), Err(Error::Retained(Failure::Timeout)));
    false
}

#[cfg(not(test))]
fn actual_owned_fields_follow_the_same_fence() {
    use catten_rt::owned::{
        Endpoint,
        OwnedMemory,
    };
    use catten_syscall::IpcRights;
    for mode in [Mode::Healthy, Mode::Timeout] {
        let control = Control::new(mode);
        let endpoint =
            Endpoint::create(super::entropy::INTERFACE, super::entropy::VERSION, 1).unwrap();
        let connection = endpoint.connect(IpcRights::CALL).unwrap();
        let mut death = connection.as_ref().watch_closed().unwrap();
        let memory = OwnedMemory::allocate(1).unwrap();
        let backend = Fake {
            control: &control,
            _field: Field(&control),
            _resource: (endpoint, memory),
        };
        let mut device = Device::new(backend, Limits::new(3).unwrap());
        assert_eq!(device.fill(&mut [0; 4]).is_ok(), mode == Mode::Healthy);
        drop(device);
        // These are real owned fields in the signed probe. Retained CPU-only
        // fixture resources live until ordinary exact-root namespace retirement;
        // they are not DMA grants, cryptographic entropy or physical retry owners.
        assert_eq!(control.calls.get().backend_drops, usize::from(mode == Mode::Healthy));
        assert_eq!(death.wait_timeout(10).unwrap().is_some(), mode == Mode::Healthy);
    }
}
