//! The two faces of a name service, made explicit.
//!
//! A name service is both an immediate **catalog** and a waitable **event
//! broker**:
//!
//! - **Catalog**: `name` resolves to a target with a committed generation,
//!   or does not exist yet. Answers are immediate; a catalog never waits.
//! - **Event broker**: waiters park on named conditions and are resolved
//!   when the condition fires. The broker never polls and never assumes an
//!   ordering: the *publishing* side owns the fulfillment.
//!
//! The local name service implements both roles against its registry
//! (deferred lookups), and the replicated dns implements both against the
//! applied catalog (cluster events). "Wait for name X" therefore means the
//! same thing at either scope, and fulfillment is always defined by the
//! publishing side — a Raft commit cluster-wide, a registration locally —
//! never by polling order, spin counts, or boot timing.

/// An immediate name catalog.
pub trait Catalog {
    /// Resolve `name` to its target, if present.
    fn resolve(&self, name: &[u8]) -> Option<CatalogTarget>;
}

/// A resolved catalog entry.
#[derive(Debug, Clone, Copy)]
pub struct CatalogTarget {
    /// The committed generation of the entry.
    pub generation: u64,
    /// The owning connection, or 0 when the target is not connection-bound
    /// (e.g. cluster events).
    pub connection: u64,
}

/// A waitable event broker.
pub trait EventBroker {
    /// The parked waiter payload (typically a reply token).
    type Waiter;

    /// Park `waiter` on `event`. Returns `Some(waiter)` when the event has
    /// already fired (the caller resolves it immediately against the
    /// catalog), or `None` once parked. The broker replies to nothing: the
    /// caller answers resolved waiters so access policy and reply shape stay
    /// with the service.
    fn park(
        &mut self,
        event: &[u8],
        waiter: Self::Waiter,
        catalog: &dyn Catalog,
    ) -> Option<Self::Waiter>;

    /// Resolve every parked waiter whose event has fired, returning the
    /// `(event, waiter)` pairs for the caller to answer. Runs in catalog
    /// order.
    fn settle(&mut self, catalog: &dyn Catalog) -> alloc::vec::Vec<(alloc::vec::Vec<u8>, Self::Waiter)>;

    /// Take the waiters parked on `event`. Used by the firing side, which
    /// already knows which condition it just made true.
    fn fire(&mut self, event: &[u8]) -> alloc::vec::Vec<Self::Waiter>;
}

/// The shared keyed waitlist backing both the local name service's deferred
/// lookups and the replicated dns's cluster events.
#[derive(Debug)]
pub struct KeyedWaitlist<W> {
    waiters: alloc::collections::BTreeMap<alloc::vec::Vec<u8>, alloc::vec::Vec<W>>,
}

impl<W> Default for KeyedWaitlist<W> {
    fn default() -> Self {
        Self {
            waiters: alloc::collections::BTreeMap::new(),
        }
    }
}

impl<W> KeyedWaitlist<W> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.waiters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.waiters.is_empty()
    }
}

impl<W> EventBroker for KeyedWaitlist<W> {
    type Waiter = W;

    fn park(&mut self, event: &[u8], waiter: W, catalog: &dyn Catalog) -> Option<W> {
        if catalog.resolve(event).is_some() {
            return Some(waiter);
        }
        self.waiters.entry(event.to_vec()).or_default().push(waiter);
        None
    }

    fn settle(&mut self, catalog: &dyn Catalog) -> alloc::vec::Vec<(alloc::vec::Vec<u8>, W)> {
        let mut resolved = alloc::vec::Vec::new();
        let fired: alloc::vec::Vec<alloc::vec::Vec<u8>> = self
            .waiters
            .keys()
            .filter(|name| catalog.resolve(name).is_some())
            .cloned()
            .collect();
        for event in fired {
            if let Some(waiters) = self.waiters.remove(&event) {
                for waiter in waiters {
                    resolved.push((event.clone(), waiter));
                }
            }
        }
        resolved
    }

    fn fire(&mut self, event: &[u8]) -> alloc::vec::Vec<W> {
        self.waiters.remove(event).unwrap_or_default()
    }
}
