//! Reactive, latest-key-wins data reads with explicit ownership.
//!
//! The core accepts a local task spawner. Enable `browser` for the browser
//! executor and cancellable GET adapter. There is no implicit cache or retry.
//! Only the synchronous key function tracks signals, never the async loader.
mod cancellation;
mod value;
pub use cancellation::{CancelRegistration, CancellationSource, CancellationToken};
pub use fusor::coherence::{AsyncBoundary, BoundaryStatus};
pub use value::{AsyncRead, AsyncValue};
#[cfg(feature = "browser")]
pub mod browser;
#[cfg(feature = "browser")]
pub mod fetch;

use cancellation::InFlight;
use derive_where::derive_where;
use fusor::{Effect, OwnerHandle, Registration, Signal, batch, effect, signal, untrack};
use futures_util::future::LocalBoxFuture;
use std::{
    cell::{Cell, RefCell},
    future::Future,
    rc::Rc,
};

/// A successful value and the key that actually produced it.
#[derive(Debug)]
#[derive_where(Clone; K)]
pub struct Data<K, T> {
    pub key: K,
    pub value: Rc<T>,
}

/// Previous data is explicitly labelled; it is never passed off as a new key's result.
#[derive(Debug)]
#[derive_where(Clone; K)]
pub enum ResourceState<K, T, E> {
    Idle,
    Loading {
        key: K,
        previous: Option<Data<K, T>>,
    },
    Ready(Data<K, T>),
    Error {
        key: K,
        error: Rc<E>,
        previous: Option<Data<K, T>>,
    },
    Disposed,
}
impl<K, T, E> ResourceState<K, T, E> {
    pub fn data(&self) -> Option<&Data<K, T>> {
        match self {
            Self::Ready(data) => Some(data),
            Self::Loading { previous, .. } | Self::Error { previous, .. } => previous.as_ref(),
            Self::Idle | Self::Disposed => None,
        }
    }
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }
}

/// A type-erased async operation: an input and its cancellation token in, a
/// local future of `O` out. Reads use `Loader<K, Result<T, E>>`.
pub type Loader<I, O> = dyn Fn(I, CancellationToken) -> LocalBoxFuture<'static, O>;
/// Schedules a local future on the current thread's executor, without blocking.
pub type Spawner = dyn Fn(LocalBoxFuture<'static, ()>);
/// Erase a loader's future type, so one loader can serve many operations.
pub fn boxed_loader<I, O, F: Future<Output = O> + 'static>(
    load: impl Fn(I, CancellationToken) -> F + 'static,
) -> Rc<Loader<I, O>> {
    Rc::new(move |input, cancel| Box::pin(load(input, cancel)))
}
/// Increment and return `counter`. Overflow is a bug, never a wraparound.
fn increment(counter: &Cell<u64>, name: &str) -> u64 {
    let next = counter
        .get()
        .checked_add(1)
        .unwrap_or_else(|| panic!("{name} overflow"));
    counter.set(next);
    next
}

struct Inner<K, T, E> {
    // Declared first: dropping `Inner` aborts and cancels the in-flight read
    // before any other field drops.
    in_flight: RefCell<Option<InFlight>>,
    owner: OwnerHandle,
    state: Signal<ResourceState<K, T, E>>,
    key: RefCell<Option<K>>,
    generation: Cell<u64>,
    disposed: Cell<bool>,
    subscription: RefCell<Option<Effect>>,
    registrations: RefCell<Vec<Registration>>,
    load: Rc<Loader<K, Result<T, E>>>,
    spawn: Rc<Spawner>,
}

/// A shared handle to one owned read, with no `Clone` bound on data or errors.
/// Dropping its last handle cancels the read and detaches its key subscription.
/// Cloned handles cannot extend the owner's lifetime.
#[derive_where(Clone)]
pub struct Resource<K, T, E>(Rc<Inner<K, T, E>>);
impl<K: Clone + PartialEq + 'static, T: 'static, E: 'static> Resource<K, T, E> {
    /// `spawn` must schedule the future on a local executor. The loader is first
    /// called when the scheduled future is polled after owner activation.
    /// Return `None` from `key` to disable loading and clear previous data.
    pub fn new<F: Future<Output = Result<T, E>> + 'static>(
        owner: &OwnerHandle,
        key: impl Fn() -> Option<K> + 'static,
        load: impl Fn(K, CancellationToken) -> F + 'static,
        spawn: impl Fn(LocalBoxFuture<'static, ()>) + 'static,
    ) -> Self {
        Self::shared(owner, key, boxed_loader(load), Rc::new(spawn))
    }

    /// Like [`Self::new`], with a loader and spawner shared by other reads,
    /// such as the entries of one query cache.
    pub fn shared(
        owner: &OwnerHandle,
        key: impl Fn() -> Option<K> + 'static,
        load: Rc<Loader<K, Result<T, E>>>,
        spawn: Rc<Spawner>,
    ) -> Self {
        let inner = Rc::new(Inner {
            in_flight: RefCell::new(None),
            owner: owner.clone(),
            state: signal(ResourceState::Idle),
            key: RefCell::new(None),
            generation: Cell::new(0),
            disposed: Cell::new(false),
            subscription: RefCell::new(None),
            registrations: RefCell::new(Vec::new()),
            load,
            spawn,
        });
        let weak = Rc::downgrade(&inner);
        let cleanup = owner.on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                inner.dispose();
            }
        });
        inner.registrations.borrow_mut().push(cleanup);
        if inner.disposed.get() {
            return Self(inner);
        }
        let weak = Rc::downgrade(&inner);
        let subscription = effect(move || {
            let next = key();
            if let Some(inner) = weak.upgrade() {
                untrack(|| inner.set_key(next));
            }
        });
        if inner.disposed.get() {
            subscription.dispose();
        } else {
            *inner.subscription.borrow_mut() = Some(subscription);
        }
        let weak = Rc::downgrade(&inner);
        // Register only while prepared: an active owner already started in the effect.
        if !owner.is_active() {
            let activation = owner.on_activate(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.reload();
                }
            });
            inner.registrations.borrow_mut().push(activation);
        }
        Self(inner)
    }
    pub fn get(&self) -> ResourceState<K, T, E> {
        self.0.state.get()
    }
    pub fn with<R>(&self, read: impl FnOnce(&ResourceState<K, T, E>) -> R) -> R {
        self.0.state.with(read)
    }
    /// Reload the current key. No-op for a disabled or disposed resource.
    pub fn refresh(&self) {
        untrack(|| self.0.reload());
    }
    /// Permanently stop this resource, even while its owner remains alive.
    pub fn dispose(&self) {
        self.0.dispose();
    }
}

impl<K, T, E> Inner<K, T, E> {
    fn invalidate(&self) -> u64 {
        let generation = increment(&self.generation, "resource generation");
        // Dropping the in-flight read aborts it and cancels its token.
        drop(self.in_flight.take());
        generation
    }
    fn dispose(&self) {
        if self.disposed.replace(true) {
            return;
        }
        batch(|| {
            self.invalidate();
            drop(self.subscription.take());
            self.publish(ResourceState::Disposed);
        });
    }
    // Prepare the complete state before borrowing, notify (or queue in a batch),
    // then retire payloads outside the borrow. Caller-written update closures keep
    // their non-reentrant contract; framework-owned replacement does not drop there.
    fn publish(&self, next: ResourceState<K, T, E>) {
        drop(self.state.replace(next));
    }
    fn is_stopped(&self) -> bool {
        self.disposed.get() || self.owner.is_disposed()
    }
    /// Only the latest generation publishes, and only while the owner is active.
    fn is_current(&self, generation: u64) -> bool {
        !self.disposed.get() && self.owner.is_active() && self.generation.get() == generation
    }
}

impl<K: Clone + 'static, T: 'static, E: 'static> Inner<K, T, E> {
    /// Load `next` unless it equals the current key.
    fn set_key(self: &Rc<Self>, next: Option<K>)
    where
        K: PartialEq,
    {
        if self.is_stopped() || *self.key.borrow() == next {
            return;
        }
        let retired_key = self.key.replace(next);
        self.restart();
        // Key destructors see the completed transition, including cancellation and
        // scheduling. Cancellation callbacks retain their existing pre-publication order.
        drop(retired_key);
    }
    /// Cancel any in-flight read and load the current key again, unless stopped.
    fn reload(self: &Rc<Self>) {
        if !self.is_stopped() {
            self.restart();
        }
    }
    fn restart(self: &Rc<Self>) {
        let next = self.key.borrow().clone();
        batch(|| {
            let generation = self.invalidate();
            // Cancellation callbacks may dispose this resource or start another load.
            if self.disposed.get() || self.generation.get() != generation {
                return;
            }
            let Some(key) = next else {
                self.publish(ResourceState::Idle);
                return;
            };
            if self.owner.is_active() {
                self.spawn_load(key, generation);
            }
        });
    }
    fn spawn_load(self: &Rc<Self>, key: K, generation: u64) {
        let previous = self.state.with_untracked(|state| state.data().cloned());
        let loading = ResourceState::Loading {
            key: key.clone(),
            previous: previous.clone(),
        };
        let weak = Rc::downgrade(self);
        let load = self.load.clone();
        let (in_flight, work) = InFlight::start(move |token| async move {
            let result = load(key.clone(), token).await;
            let Some(inner) = weak.upgrade().filter(|i| i.is_current(generation)) else {
                return;
            };
            // Remove completed handles before notifying subscribers. Notifications
            // may immediately dispose this resource or start another generation.
            if let Some(in_flight) = inner.in_flight.take() {
                in_flight.complete();
            }
            let next = match result {
                Ok(value) => ResourceState::Ready(Data {
                    key,
                    value: Rc::new(value),
                }),
                Err(error) => ResourceState::Error {
                    key,
                    error: Rc::new(error),
                    previous,
                },
            };
            inner.publish(next);
            // On success, the unused previous data is retired here,
            // after Ready has been published, never in an update closure.
        });
        *self.in_flight.borrow_mut() = Some(in_flight);
        self.publish(loading);
        (self.spawn)(work);
    }
}
