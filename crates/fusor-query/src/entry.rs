use super::{Clock, Inner, Key};
use crate::{Freshness, QueryState};
use fusor::{Effect, Owner, Signal, effect, signal, untrack};
use fusor_async::{Resource, ResourceData, ResourceState};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

/// One cached key: its read, the last data it produced, and how many
/// subscriptions currently use it.
pub(super) struct Entry<K, T, E> {
    owner: Owner,
    key: K,
    /// The read's key. `None` while unobserved, which cancels unfinished work.
    request_key: Signal<Option<K>>,
    resource: Resource<K, T, E>,
    pub(super) state: Signal<QueryState<K, T, E>>,
    cached: RefCell<Option<ResourceData<K, T>>>,
    updated: Cell<Option<Duration>>,
    invalid: Cell<bool>,
    observers: Cell<usize>,
    unused_since: Cell<Duration>,
    clock: Rc<Clock>,
    watch: RefCell<Option<Effect>>,
}

impl<K: Key, T: 'static, E: 'static> Entry<K, T, E> {
    pub(super) fn new(client: &Inner<K, T, E>, key: K) -> Rc<Self> {
        let owner = Owner::child(&client.owner.handle());
        let request_key = signal(None);
        let input = request_key.clone();
        let resource = Resource::shared(
            &owner.handle(),
            move || input.get(),
            client.load.clone(),
            client.spawn.clone(),
        );
        let entry = Rc::new(Self {
            owner,
            key,
            request_key,
            resource,
            state: signal(QueryState::Idle),
            cached: RefCell::new(None),
            updated: Cell::new(None),
            invalid: Cell::new(true),
            observers: Cell::new(0),
            unused_since: Cell::new((client.clock)()),
            clock: client.clock.clone(),
            watch: RefCell::new(None),
        });
        let weak = Rc::downgrade(&entry);
        let watch = effect(move || {
            if let Some(entry) = weak.upgrade() {
                let state = entry.resource.get();
                untrack(|| entry.publish(state));
            }
        });
        *entry.watch.borrow_mut() = Some(watch);
        entry.owner.commit();
        entry
    }

    fn publish(&self, state: ResourceState<K, T, E>) {
        let state = match state {
            ResourceState::Ready(data) => {
                drop(self.cached.replace(Some(data.clone())));
                self.updated.set(Some((self.clock)()));
                self.invalid.set(false);
                QueryState::Ready(data)
            }
            ResourceState::Loading { key, .. } => QueryState::Loading {
                key,
                previous: self.cached(),
            },
            ResourceState::Error { key, error, .. } => {
                self.invalid.set(true);
                QueryState::Error {
                    key,
                    error,
                    previous: self.cached(),
                }
            }
            ResourceState::Idle => self.cached().map_or(QueryState::Idle, QueryState::Ready),
            ResourceState::Disposed => {
                drop(self.cached.take());
                QueryState::Disposed
            }
        };
        drop(self.state.replace(state));
    }

    fn cached(&self) -> Option<ResourceData<K, T>> {
        self.cached.borrow().clone()
    }

    pub(super) fn acquire(&self, freshness: Freshness) {
        self.observers.set(self.observers.get() + 1);
        let fresh = !self.invalid.get()
            && self
                .updated
                .get()
                .is_some_and(|at| freshness.is_fresh((self.clock)().saturating_sub(at)));
        if !fresh && !self.resource.with(ResourceState::is_loading) {
            self.start();
        }
    }

    fn start(&self) {
        if self.request_key.get_untracked().is_some() {
            self.resource.refresh();
        } else {
            self.request_key.set(Some(self.key.clone()));
        }
    }

    pub(super) fn release(&self) {
        let remaining = self
            .observers
            .get()
            .checked_sub(1)
            .expect("query subscription released once");
        self.observers.set(remaining);
        if remaining == 0 {
            self.unused_since.set((self.clock)());
            // Disable the resource to cancel its generation; cached data is held
            // separately and can survive cancelled revalidation.
            self.request_key.set(None);
        }
    }

    pub(super) fn invalidate(&self) {
        self.invalid.set(true);
        if self.observers.get() != 0 {
            self.start();
        }
    }
}

impl<K, T, E> Entry<K, T, E> {
    pub(super) fn observers(&self) -> usize {
        self.observers.get()
    }

    /// When the last subscription left, or `None` while the entry is in use.
    pub(super) fn idle_since(&self) -> Option<Duration> {
        (self.observers.get() == 0).then(|| self.unused_since.get())
    }

    pub(super) fn expired(&self, now: Duration, retention: Duration) -> bool {
        self.idle_since()
            .is_some_and(|since| now.saturating_sub(since) >= retention)
    }

    pub(super) fn dispose(&self) {
        self.owner.dispose();
    }
}
