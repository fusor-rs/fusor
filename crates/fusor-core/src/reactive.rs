use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::{Rc, Weak},
};

mod graph;
mod memo;
pub mod versions;
use graph::{Observer, ObserverKind, Source, track};
pub use memo::{Memo, memo, memo_with_eq};

thread_local! {
    static CURRENT: RefCell<Option<Weak<Observer>>> = const { RefCell::new(None) };
    static QUEUE: RefCell<VecDeque<Weak<EffectInner>>> = const { RefCell::new(VecDeque::new()) };
    static BATCH_DEPTH: Cell<usize> = const { Cell::new(0) };
    static FLUSHING: Cell<bool> = const { Cell::new(false) };
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
    static NEXT_FLUSH: Cell<u64> = const { Cell::new(0) };
    static NEXT_WAVE: Cell<u64> = const { Cell::new(0) };
    static COMPUTING: Cell<usize> = const { Cell::new(0) };
    static RENDERING: Cell<bool> = const { Cell::new(false) };
    #[cfg(feature = "javascript")]
    static AFTER_FLUSH: RefCell<VecDeque<Box<dyn FnOnce()>>> = const { RefCell::new(VecDeque::new()) };
}

struct RenderGuard(bool);

impl RenderGuard {
    fn replace(rendering: bool) -> Self {
        Self(RENDERING.with(|current| current.replace(rendering)))
    }
}

impl Drop for RenderGuard {
    fn drop(&mut self) {
        RENDERING.with(|current| current.set(self.0));
    }
}

fn assert_not_computing() {
    assert_eq!(
        COMPUTING.with(Cell::get),
        0,
        "memo computations and equality functions must not write signals or create effects"
    );
}

struct SignalInner<T> {
    value: RefCell<T>,
    source: Rc<Source>,
    render_values: RefCell<Vec<(Rc<T>, versions::Versions)>>,
}

/// Shared, single-threaded reactive state. Cloning shares the same value.
pub struct Signal<T>(Rc<SignalInner<T>>);

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Create reactive state; reads inside an effect automatically subscribe to it.
pub fn signal<T>(value: T) -> Signal<T> {
    Signal(Rc::new(SignalInner {
        value: RefCell::new(value),
        source: Rc::new(Source::new(None)),
        render_values: RefCell::new(Vec::new()),
    }))
}

impl<T> Signal<T> {
    /// Read without cloning the value, tracking this dependency.
    /// Do not write this signal while the read closure holds its borrow.
    pub fn with<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        if let Some((value, inputs)) = self.render_value() {
            versions::Versions::exclude(|| track(&self.0.source));
            inputs.include();
            return read(&value);
        }
        track(&self.0.source);
        read(&self.0.value.borrow())
    }

    /// Read without subscribing the current effect.
    pub fn with_untracked<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        if let Some((value, _)) = self.render_value() {
            return read(&value);
        }
        read(&self.0.value.borrow())
    }

    /// A generated keyed row's candidate input. The override is synchronous,
    /// never published to observers, and validates against the collection's
    /// actual source versions. This is not historical storage for signals.
    ///
    /// [`Memo`] reads recompute against candidate inputs without changing their
    /// committed cache or subscriptions. The consuming evaluation tracks those
    /// inputs directly. Nested overrides restore the preceding value, including
    /// during unwinding. Do not mutate signals in `render`.
    /// Part of the contract versioned by [`crate::coherence::VERSION`].
    pub fn with_render_value<R>(
        &self,
        value: Rc<T>,
        inputs: versions::Versions,
        render: impl FnOnce() -> R,
    ) -> R {
        struct Pop<'a, T>(&'a Signal<T>);
        impl<T> Drop for Pop<'_, T> {
            fn drop(&mut self) {
                let value = self.0.0.render_values.borrow_mut().pop();
                drop(value);
            }
        }
        self.0.render_values.borrow_mut().push((value, inputs));
        let _rendering = RenderGuard::replace(true);
        let _pop = Pop(self);
        render()
    }

    /// Mutate state, then notify subscribers after releasing the mutable borrow.
    /// Always notifies; use `set` to skip unchanged values.
    /// The mutation closure must not read or write this same signal.
    pub fn update<R>(&self, update: impl FnOnce(&mut T) -> R) -> R {
        assert_not_computing();
        crate::coherence::mutation("signal write");
        let result = update(&mut self.0.value.borrow_mut());
        notify(&self.0.source);
        result
    }

    /// Replace the value and notify subscribers, returning the previous value.
    /// Unlike `set`, this needs no `PartialEq` and always notifies. Dropping
    /// the result retires the old value after notification.
    pub fn replace(&self, value: T) -> T {
        self.update(|current| std::mem::replace(current, value))
    }

    fn render_value(&self) -> Option<(Rc<T>, versions::Versions)> {
        RENDERING
            .with(Cell::get)
            .then(|| self.0.render_values.borrow().last().cloned())
            .flatten()
    }
}

impl<T: Clone> Signal<T> {
    /// Clone the current value and automatically track the read.
    pub fn get(&self) -> T {
        self.with(Clone::clone)
    }

    /// Clone the current value without dependency tracking.
    pub fn get_untracked(&self) -> T {
        self.with_untracked(Clone::clone)
    }
}

impl<T: PartialEq> Signal<T> {
    /// Replace the value, notifying only when it actually changes.
    /// Retire the old value after notification, outside the internal borrow.
    /// Inside a batch, notification queues observers until the batch completes.
    pub fn set(&self, value: T) {
        assert_not_computing();
        crate::coherence::mutation("signal write");
        let retired = {
            let mut current = self.0.value.borrow_mut();
            if *current == value {
                None
            } else {
                Some(std::mem::replace(&mut *current, value))
            }
        };
        if retired.is_some() {
            notify(&self.0.source);
        }
        drop(retired);
    }
}

/// A lazy computation. Reads track its underlying signals in the consuming effect.
/// It is deliberately not a cache: each `get` evaluates the function once.
pub struct Derived<T>(Rc<dyn Fn() -> T>);

impl<T> Clone for Derived<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub fn derived<T>(compute: impl Fn() -> T + 'static) -> Derived<T> {
    Derived(Rc::new(compute))
}

impl<T> Derived<T> {
    pub fn get(&self) -> T {
        (self.0)()
    }
}

struct EffectInner {
    observer: Rc<Observer>,
    active: Cell<bool>,
    queued: Cell<bool>,
    last_flush: Cell<u64>,
    flush_runs: Cell<u32>,
    callback: RefCell<Box<dyn FnMut()>>,
    lifecycle: RefCell<Vec<crate::Registration>>,
}

impl EffectInner {
    fn unsubscribe(&self) {
        self.observer.unsubscribe();
    }

    fn run(self: &Rc<Self>, initial: bool) {
        if !self.active.get() {
            return;
        }
        // Invalidated memos are checked lazily before deciding whether the
        // effect's observed values actually changed.
        if !initial && !untrack(|| self.observer.changed()) {
            return;
        }
        // Recollect on every run, so conditional reads shed stale dependencies.
        let _run = self.observer.begin();
        let _tracking = TrackingGuard::replace(Some(Rc::downgrade(&self.observer)));
        (self.callback.borrow_mut())();
    }
}

/// Owns a subscription. Dropping it detaches every dependency.
#[must_use = "retain the effect handle for as long as the subscription should live"]
pub struct Effect(Rc<EffectInner>);

impl Effect {
    #[cfg(feature = "dom")]
    pub(crate) fn initializer(&self) -> impl FnOnce() + 'static {
        let weak = Rc::downgrade(&self.0);
        move || {
            if let Some(inner) = weak.upgrade() {
                batch(|| inner.run(true));
            }
        }
    }

    /// Stop reacting. Safe to call more than once.
    pub fn dispose(&self) {
        self.0.active.set(false);
        self.0.unsubscribe();
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        self.dispose();
    }
}

/// Run immediately, then rerun when any signal read by the callback changes.
/// Effects are synchronous outside a batch; queued reruns are deduplicated.
fn allocate_effect(callback: impl FnMut() + 'static) -> Effect {
    assert_not_computing();
    Effect(Rc::new_cyclic(|weak| EffectInner {
        observer: Observer::new(ObserverKind::Effect(weak.clone())),
        active: Cell::new(true),
        queued: Cell::new(false),
        last_flush: Cell::new(0),
        flush_runs: Cell::new(0),
        callback: RefCell::new(Box::new(callback)),
        lifecycle: RefCell::new(Vec::new()),
    }))
}

#[cfg(feature = "dom")]
pub(crate) fn prepared_effect(callback: impl FnMut() + 'static) -> Effect {
    allocate_effect(callback)
}

pub fn effect(callback: impl FnMut() + 'static) -> Effect {
    let subscription = allocate_effect(callback);
    // Initial effects still run immediately inside a batch. Their writes flush
    // after the initial callback returns, avoiding recursive RefCell borrows.
    if let Some(owner) = crate::coherence::preparing_owner().filter(|owner| !owner.is_active()) {
        let weak = Rc::downgrade(&subscription.0);
        let activation = owner.on_activate(move || {
            if let Some(inner) = weak.upgrade() {
                batch(|| inner.run(true));
            }
        });
        let weak = Rc::downgrade(&subscription.0);
        let cleanup = owner.on_cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                inner.active.set(false);
                inner.unsubscribe();
            }
        });
        subscription
            .0
            .lifecycle
            .borrow_mut()
            .extend([activation, cleanup]);
    } else if !crate::coherence::mutation("effect creation") {
        batch(|| subscription.0.run(true));
    }
    subscription
}

fn notify(source: &Source) {
    source.advance();
    // Invalidate the entire reachable graph before executing any effects.
    source.notify();
    flush();
}

fn clear_queue() {
    #[cfg(feature = "javascript")]
    AFTER_FLUSH.with(|queue| queue.borrow_mut().clear());
    QUEUE.with(|queue| {
        for pending in queue
            .borrow_mut()
            .drain(..)
            .filter_map(|item| item.upgrade())
        {
            pending.queued.set(false);
        }
    });
}

struct FlushGuard;

impl Drop for FlushGuard {
    fn drop(&mut self) {
        FLUSHING.with(|flushing| flushing.set(false));
        if std::thread::panicking() {
            clear_queue();
        }
    }
}

fn flush() {
    if BATCH_DEPTH.with(Cell::get) > 0 || FLUSHING.with(|flushing| flushing.replace(true)) {
        return;
    }
    let _guard = FlushGuard;
    let epoch = NEXT_FLUSH.with(|next| {
        let epoch = next
            .get()
            .checked_add(1)
            .expect("reactive flush ID exhausted");
        next.set(epoch);
        epoch
    });
    loop {
        let next = QUEUE.with(|queue| queue.borrow_mut().pop_front());
        let Some(next) = next else {
            #[cfg(feature = "javascript")]
            {
                let callback = AFTER_FLUSH.with(|queue| queue.borrow_mut().pop_front());
                if let Some(callback) = callback {
                    untrack(callback);
                    continue;
                }
            }
            break;
        };
        if let Some(next) = next.upgrade() {
            next.queued.set(false);
            // Width and acyclic propagation depth are not feedback loops.
            // Limit repeated execution of each effect, without a per-flush map.
            let runs = if next.last_flush.replace(epoch) == epoch {
                next.flush_runs.get() + 1
            } else {
                1
            };
            next.flush_runs.set(runs);
            assert!(
                runs <= 10_000,
                "reactive cycle: one effect exceeded 10,000 runs in one flush"
            );
            next.run(false);
        }
    }
}

/// Schedule foreign notifications after ordinary reactive work has settled.
/// The queue never lends a borrow across callbacks; their writes join the next
/// wave of the current flush rather than recursively running an observer.
#[cfg(feature = "javascript")]
pub(crate) fn after_flush(callback: impl FnOnce() + 'static) {
    AFTER_FLUSH.with(|queue| queue.borrow_mut().push_back(Box::new(callback)));
}

struct TrackingGuard(Option<Weak<Observer>>);

impl TrackingGuard {
    fn replace(next: Option<Weak<Observer>>) -> Self {
        Self(CURRENT.with(|current| current.replace(next)))
    }
}

impl Drop for TrackingGuard {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.0.take()));
    }
}

/// Evaluate a closure without collecting dependencies in the current effect.
pub fn untrack<R>(read: impl FnOnce() -> R) -> R {
    let _guard = TrackingGuard::replace(None);
    read()
}

struct BatchGuard;

impl Drop for BatchGuard {
    fn drop(&mut self) {
        BATCH_DEPTH.with(|depth| depth.set(depth.get() - 1));
        if std::thread::panicking() {
            clear_queue();
        } else {
            flush();
        }
    }
}

/// Group synchronous writes into one notification per affected effect.
/// Nested batches flush at the outer boundary. This is not a rollback transaction.
pub fn batch<R>(update: impl FnOnce() -> R) -> R {
    BATCH_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _guard = BatchGuard;
    update()
}
