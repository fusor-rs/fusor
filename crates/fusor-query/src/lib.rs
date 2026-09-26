//! Shared reads with explicit freshness, retention and ownership.
//!
//! Each [`QueryClient`] is a typed query definition: it binds one loader to its
//! key space. Cloning it shares a cache; creating another client creates a distinct
//! query identity, even with identical Rust key/value types. Provide cloned clients
//! through typed owner context. No global registry, automatic retries or mutations.
mod entry;
mod observer;
mod state;
pub use observer::Query;
pub use state::{CacheInfo, Freshness, QueryOptions, QueryState};

use derive_where::derive_where;
use entry::Entry;
use fusor::{Owner, OwnerHandle, Registration, Signal, batch, signal, untrack};
use fusor_async::{CancellationToken, Loader, Spawner, boxed_loader};
use futures_util::future::LocalBoxFuture;
use std::{cell::RefCell, collections::BTreeMap, future::Future, rc::Rc, time::Duration};

/// What a query key needs: it is cloned into states and ordered in the cache.
pub trait Key: Clone + Ord + 'static {}
impl<K: Clone + Ord + 'static> Key for K {}

type Clock = dyn Fn() -> Duration;
type Entries<K, T, E> = BTreeMap<K, Rc<Entry<K, T, E>>>;

struct Inner<K, T, E> {
    owner: Owner,
    alive: Signal<bool>,
    options: QueryOptions,
    entries: RefCell<Entries<K, T, E>>,
    load: Rc<Loader<K, Result<T, E>>>,
    spawn: Rc<Spawner>,
    clock: Rc<Clock>,
    cleanup: RefCell<Option<Registration>>,
}

/// One loader, one typed key space, and one application/session lifetime.
/// Consumer owners only own subscriptions. The last consumer leaving cancels
/// unfinished work; completed data may remain until eviction or client disposal.
#[derive_where(Clone)]
pub struct QueryClient<K, T, E>(Rc<Inner<K, T, E>>);

impl<K: Key, T: 'static, E: 'static> QueryClient<K, T, E> {
    /// `clock` must be monotonic. `spawn` schedules local futures without blocking.
    /// The client's owner is a child of `parent`; a retained clone cannot revive it.
    pub fn new<F: Future<Output = Result<T, E>> + 'static>(
        parent: &OwnerHandle,
        options: QueryOptions,
        load: impl Fn(K, CancellationToken) -> F + 'static,
        spawn: impl Fn(LocalBoxFuture<'static, ()>) + 'static,
        clock: impl Fn() -> Duration + 'static,
    ) -> Self {
        let inner = Rc::new(Inner {
            owner: Owner::child(parent),
            alive: signal(true),
            options,
            entries: RefCell::new(BTreeMap::new()),
            load: boxed_loader(load),
            spawn: Rc::new(spawn),
            clock: Rc::new(clock),
            cleanup: RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        let cleanup = inner.owner.handle().on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                batch(|| {
                    inner.alive.set(false);
                    for entry in inner.entries.take().into_values() {
                        entry.dispose();
                    }
                });
            }
        });
        *inner.cleanup.borrow_mut() = Some(cleanup);
        inner.owner.commit();
        Self(inner)
    }

    /// Observe a reactive key. Loading waits for both client and view activation.
    /// Cloning the returned handle shares one subscription, not an extra observer.
    pub fn observe(
        &self,
        owner: &OwnerHandle,
        key: impl Fn() -> Option<K> + 'static,
    ) -> Query<K, T, E> {
        Query::new(self.clone(), owner, key)
    }

    /// Mark a key stale and refresh it immediately if it has active observers.
    /// Returns false if it is absent. Inactive entries reload when next observed.
    pub fn invalidate(&self, key: &K) -> bool {
        untrack(|| {
            let Some(entry) = self.entry(key) else {
                return false;
            };
            entry.invalidate();
            true
        })
    }

    /// Release expired inactive entries. Retention is lazy, also checked whenever
    /// a new subscription selects a key. No background timers or hidden refetches.
    pub fn collect(&self) -> usize {
        untrack(|| {
            let now = (self.0.clock)();
            let retention = self.0.options.retention;
            let mut expired = Vec::new();
            self.0.entries.borrow_mut().retain(|_, entry| {
                let keep = !entry.expired(now, retention);
                if !keep {
                    expired.push(entry.clone());
                }
                keep
            });
            // Dispose outside the borrow: disposal runs owner cleanup callbacks.
            for entry in &expired {
                entry.dispose();
            }
            expired.len()
        })
    }

    /// Counts describe this cache, without tracking a reactive dependency.
    pub fn info(&self) -> CacheInfo {
        let entries = self.0.entries.borrow();
        CacheInfo {
            entries: entries.len(),
            observers: entries.values().map(|entry| entry.observers()).sum(),
        }
    }

    /// End this identity permanently, cancelling reads and clearing cached data.
    /// Use a new client for a new login/tenant. Already cloned application values
    /// remain ordinary Rust values and cannot be retroactively revoked.
    pub fn dispose(&self) {
        untrack(|| self.0.owner.dispose());
    }

    /// Whether the client is alive, subscribing the running effect to disposal.
    fn track_alive(&self) -> bool {
        self.0.alive.get()
    }

    fn is_alive(&self) -> bool {
        self.0.alive.get_untracked()
    }

    fn entry(&self, key: &K) -> Option<Rc<Entry<K, T, E>>> {
        self.0.entries.borrow().get(key).cloned()
    }

    /// Subscribe to the entry for `key`, creating it if there is room. `None`
    /// when the client is disposed or every entry is in use.
    fn acquire(&self, key: &K) -> Option<Rc<Entry<K, T, E>>> {
        self.collect();
        if !self.is_alive() {
            return None;
        }
        let entry = match self.entry(key) {
            Some(entry) => entry,
            None => self.insert(key)?,
        };
        entry.acquire(self.0.options.freshness);
        Some(entry)
    }

    /// Add an entry for `key`. A full cache first evicts its longest-unused
    /// idle entry; with none to evict, nothing is added.
    fn insert(&self, key: &K) -> Option<Rc<Entry<K, T, E>>> {
        if self.0.entries.borrow().len() >= self.0.options.capacity.get() {
            self.evict()?.dispose();
            // Disposal runs owner cleanup callbacks, which can dispose this client.
            if !self.is_alive() {
                return None;
            }
        }
        let entry = Entry::new(&self.0, key.clone());
        self.0
            .entries
            .borrow_mut()
            .insert(key.clone(), entry.clone());
        Some(entry)
    }

    fn evict(&self) -> Option<Rc<Entry<K, T, E>>> {
        let mut entries = self.0.entries.borrow_mut();
        let (_, oldest) = entries
            .iter()
            .filter_map(|(key, entry)| Some((entry.idle_since()?, key)))
            .min_by_key(|(since, _)| *since)?;
        let oldest = oldest.clone();
        entries.remove(&oldest)
    }
}

#[cfg(feature = "browser")]
pub mod browser {
    //! Browser executor and monotonic performance clock. Enable the async crate's
    //! `browser` feature separately when using its Fetch adapter.
    use super::*;
    pub fn client<K, T, E, F>(
        parent: &OwnerHandle,
        options: QueryOptions,
        load: impl Fn(K, CancellationToken) -> F + 'static,
    ) -> QueryClient<K, T, E>
    where
        K: Key,
        T: 'static,
        E: 'static,
        F: Future<Output = Result<T, E>> + 'static,
    {
        let performance = web_sys::window()
            .and_then(|w| w.performance())
            .expect("query client requires browser performance clock");
        QueryClient::new(
            parent,
            options,
            load,
            wasm_bindgen_futures::spawn_local,
            move || Duration::from_secs_f64(performance.now() / 1000.0),
        )
    }
}
