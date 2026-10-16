//! Original weak observer and charge stay outside fallible empty-node allocation.

use core::mem::ManuallyDrop;

use super::*;

/// All partial fields remain inert on abandonment, including empty Box backing.
#[must_use]
struct PreparingEntry<C>(ManuallyDrop<Resources<C>>);

struct Resources<C> {
    observer: Option<Weak<dyn Observer>>,
    charge: Option<C>,
    storage: Option<Box<MaybeUninit<EntryNode<C>>>>,
}

impl<C> PreparingEntry<C> {
    fn new(observer: Weak<dyn Observer>, charge: C) -> Self {
        Self(ManuallyDrop::new(Resources {
            observer: Some(observer),
            charge: Some(charge),
            storage: None,
        }))
    }

    fn allocate(
        &mut self,
        allocate: impl FnOnce() -> Result<Box<MaybeUninit<EntryNode<C>>>, RegistrationError>,
    ) -> Result<(), RegistrationError> {
        assert!(self.0.storage.is_none(), "observer entry storage already prepared");
        self.0.storage = Some(allocate()?);
        Ok(())
    }

    fn complete(mut self) -> PreparedRegistration<C> {
        assert!(self.0.storage.is_some(), "observer entry initialization without storage");
        // This private Box is initialized once before list publication. No
        // allocation, callback, list lock or disposal remains in this transfer.
        let node = Box::write(
            self.0.storage.take().unwrap(),
            EntryNode {
                id: 0,
                observer: self.0.observer.take().unwrap(),
                next: None,
            },
        );
        PreparedRegistration(ManuallyDrop::new(Some(Entry {
            node,
            _charge: self.0.charge.take().unwrap(),
        })))
    }

    fn cancel(self) {
        self.cancel_with(|| {});
    }

    fn cancel_with(mut self, mut before_dispose: impl FnMut()) {
        before_dispose();
        drop(self.0.storage.take());
        before_dispose();
        drop(self.0.observer.take());
        before_dispose();
        drop(self.0.charge.take());
    }
}

pub(super) fn prepare<C>(
    observer: Weak<dyn Observer>,
    charge: C,
    allocate: impl FnOnce() -> Result<Box<MaybeUninit<EntryNode<C>>>, RegistrationError>,
) -> Result<PreparedRegistration<C>, RegistrationError> {
    let mut preparing = PreparingEntry::new(observer, charge);
    if let Err(error) = preparing.allocate(allocate) {
        // The allocator owns no payload and has returned. Dispose explicitly
        // with original admission retained through backing and weak release.
        preparing.cancel();
        return Err(error);
    }
    Ok(preparing.complete())
}

#[path = "entry_preparation/tests.rs"]
mod tests;

pub(super) fn test_preparation() {
    tests::run();
}
