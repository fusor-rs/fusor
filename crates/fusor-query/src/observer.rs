use super::{Entry, Key, QueryClient, QueryState};
use derive_where::derive_where;
use fusor::{Effect, OwnerHandle, Registration, Signal, batch, effect, signal, untrack};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Observer<K: Key, T: 'static, E: 'static> {
    client: QueryClient<K, T, E>,
    owner: OwnerHandle,
    disposed: Cell<bool>,
    entry: RefCell<Option<Rc<Entry<K, T, E>>>>,
    key: RefCell<Option<K>>,
    state: Signal<QueryState<K, T, E>>,
    wake: Signal<()>,
    effect: RefCell<Option<Effect>>,
    registrations: RefCell<Vec<Registration>>,
}

impl<K: Key, T: 'static, E: 'static> Observer<K, T, E> {
    /// Whether this subscription may hold an entry right now.
    fn can_observe(&self) -> bool {
        !self.disposed.get() && self.owner.is_active() && self.client.is_alive()
    }

    fn wake(&self) {
        self.wake.update(|_| ());
    }

    /// Follow the key once: hold its entry and show that entry's state.
    fn sync(&self, key: &impl Fn() -> Option<K>) {
        self.wake.get();
        if !self.client.track_alive() {
            self.dispose();
            return;
        }
        let next = key();
        if !self.owner.is_active() {
            return;
        }
        untrack(|| self.select(&next));
        if !self.client.is_alive() {
            self.dispose();
            return;
        }
        let state = self.state_for(next);
        if !self.disposed.get() {
            untrack(|| drop(self.state.replace(state)));
        }
    }

    /// Hold the entry for `next`, unless it is already held.
    fn select(&self, next: &Option<K>) {
        if *self.key.borrow() == *next && self.entry.borrow().is_some() {
            return;
        }
        self.release();
        if !self.can_observe() {
            return;
        }
        *self.key.borrow_mut() = next.clone();
        let entry = next.as_ref().and_then(|key| self.client.acquire(key));
        // Acquiring runs owner cleanup for evicted entries, which can dispose
        // this subscription, its owner or the client.
        if self.can_observe() {
            *self.entry.borrow_mut() = entry;
        } else if let Some(entry) = entry {
            entry.release();
        }
    }

    /// The state to show for `next`: its entry's, or why there is none.
    fn state_for(&self, next: Option<K>) -> QueryState<K, T, E> {
        let entry = self.entry.borrow().clone();
        match (entry, next) {
            (Some(entry), _) => entry.state.get(),
            (None, Some(key)) => QueryState::Capacity { key },
            (None, None) => QueryState::Idle,
        }
    }

    fn release(&self) {
        if let Some(entry) = self.entry.take() {
            entry.release();
        }
    }

    fn dispose(&self) {
        if self.disposed.replace(true) {
            return;
        }
        untrack(|| {
            batch(|| {
                self.effect.take();
                self.release();
                drop(self.state.replace(QueryState::Disposed));
            })
        });
    }
}

impl<K: Key, T: 'static, E: 'static> Drop for Observer<K, T, E> {
    fn drop(&mut self) {
        self.dispose();
    }
}

/// A read-only handle to one owner-bound subscription. Last-handle drop detaches
/// it; owner disposal detaches it even if handles remain in application state.
#[derive_where(Clone)]
pub struct Query<K: Key, T: 'static, E: 'static>(Rc<Observer<K, T, E>>);

impl<K: Key, T: 'static, E: 'static> Query<K, T, E> {
    pub(super) fn new(
        client: QueryClient<K, T, E>,
        owner: &OwnerHandle,
        key: impl Fn() -> Option<K> + 'static,
    ) -> Self {
        let inner = Rc::new(Observer {
            client,
            owner: owner.clone(),
            disposed: Cell::new(false),
            entry: RefCell::new(None),
            key: RefCell::new(None),
            state: signal(QueryState::Idle),
            wake: signal(()),
            effect: RefCell::new(None),
            registrations: RefCell::new(Vec::new()),
        });
        let weak = Rc::downgrade(&inner);
        let cleanup = owner.on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                inner.dispose();
            }
        });
        inner.registrations.borrow_mut().push(cleanup);
        // A disposed owner has already run the cleanup.
        if inner.disposed.get() {
            return Self(inner);
        }
        let weak = Rc::downgrade(&inner);
        let watcher = effect(move || {
            if let Some(inner) = weak.upgrade().filter(|inner| !inner.disposed.get()) {
                inner.sync(&key);
            }
        });
        // The first run disposes this subscription if the client is already gone.
        if inner.disposed.get() {
            watcher.dispose();
        } else {
            *inner.effect.borrow_mut() = Some(watcher);
        }
        let weak = Rc::downgrade(&inner);
        let activation = owner.on_activate(move || {
            if let Some(inner) = weak.upgrade() {
                inner.wake();
            }
        });
        inner.registrations.borrow_mut().push(activation);
        Self(inner)
    }

    pub fn get(&self) -> QueryState<K, T, E> {
        self.0.state.get()
    }

    pub fn with<R>(&self, read: impl FnOnce(&QueryState<K, T, E>) -> R) -> R {
        self.0.state.with(read)
    }

    pub fn dispose(&self) {
        self.0.dispose();
    }

    /// Explicitly refresh this key, or retry a capacity-limited subscription.
    pub fn refresh(&self) {
        if self.0.disposed.get() || !self.0.owner.is_active() {
            return;
        }
        untrack(|| {
            if self.0.entry.borrow().is_none() {
                self.0.wake();
                return;
            }
            let key = self.0.key.borrow().clone();
            if let Some(key) = key {
                self.0.client.invalidate(&key);
            }
        });
    }
}
