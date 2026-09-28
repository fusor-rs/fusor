//! The renderer/readiness contract, independent of executors and the browser.
//!
//! A boundary validates tracked source versions before synchronously publishing
//! owned patches. Async libraries implement `ReadLease`; renderers implement
//! `Publication`. This module does not execute or cache application requests.
use crate::{
    Effect, OwnerHandle, Registration, Signal, batch, effect, signal, untrack, versions::Versions,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundaryStatus {
    Detached,
    Pending,
    Ready,
    Error(String),
    Faulted(String),
    Disposed,
}

/// A read preparation lifetime, distinct from a component's DOM lifetime.
#[doc(hidden)]
pub trait ReadLease {
    fn cancel(&self);
}

/// Generated render work. Formatting, constructors, key comparisons and target
/// validation belong in preparation/validate, never in `apply`.
#[doc(hidden)]
pub trait Publication {
    fn validate(&self) -> Result<(), String>;
    fn apply(&mut self) -> Result<(), String>;
    fn finish(self: Box<Self>);
}

type Evaluate = dyn Fn(&Attempt) -> Result<Box<dyn Publication>, String>;

struct Inner {
    id: u64,
    status: Signal<BoundaryStatus>,
    wake: Signal<u64>,
    retry: Cell<u64>,
    epoch: Cell<u64>,
    attached: Cell<bool>,
    alive: Cell<bool>,
    driving: Cell<bool>,
    violation: RefCell<Option<String>>,
    rejected_retry: Cell<Option<u64>>,
    inputs: RefCell<Versions>,
    reads: RefCell<BTreeMap<u64, Rc<dyn ReadLease>>>,
}

thread_local! {
    static NEXT: Cell<u64> = const { Cell::new(0) };
    static EVALUATING: RefCell<Vec<Weak<Inner>>> = const { RefCell::new(Vec::new()) };
    // The length of EVALUATING, readable without a borrow on every write.
    static EVALUATION_DEPTH: Cell<usize> = const { Cell::new(0) };
    static PREPARING: RefCell<Option<OwnerHandle>> = const { RefCell::new(None) };
}

/// A stable handle. Attach it to exactly one live `Async` region. Read
/// status and put selection/retry controls outside that region.
#[derive(Clone)]
pub struct AsyncBoundary(Rc<Inner>);

impl AsyncBoundary {
    pub fn coherent() -> Self {
        let id = NEXT.with(|next| {
            let id = next.get().checked_add(1).expect("boundary id overflow");
            next.set(id);
            id
        });
        Self(Rc::new(Inner {
            id,
            status: signal(BoundaryStatus::Detached),
            wake: signal(0),
            retry: Cell::new(0),
            epoch: Cell::new(0),
            attached: Cell::new(false),
            alive: Cell::new(true),
            driving: Cell::new(false),
            violation: RefCell::new(None),
            inputs: RefCell::new(Versions::default()),
            rejected_retry: Cell::new(None),
            reads: RefCell::new(BTreeMap::new()),
        }))
    }

    pub fn status(&self) -> BoundaryStatus {
        let cycle = EVALUATING.with(|stack| {
            if stack
                .borrow()
                .iter()
                .filter_map(Weak::upgrade)
                .any(|inner| inner.id == self.0.id)
            {
                *self.0.violation.borrow_mut() =
                    Some("a coherent region cannot read its own boundary status".into());
                true
            } else {
                false
            }
        });
        if cycle {
            self.0.status.get_untracked()
        } else {
            self.0.status.get()
        }
    }

    pub fn pending(&self) -> bool {
        self.status() == BoundaryStatus::Pending
    }
    pub fn is_interactive(&self) -> bool {
        self.status() == BoundaryStatus::Ready
    }

    /// Explicitly retry failed reads. Compatible successes remain reusable.
    pub fn retry(&self) {
        if self.0.alive.get() {
            self.0
                .retry
                .set(self.0.retry.get().checked_add(1).expect("retry overflow"));
            self.0.wake.update(|n| *n += 1);
        }
    }

    #[doc(hidden)]
    pub fn attach(
        &self,
        owner: &OwnerHandle,
        evaluate: impl Fn(&Attempt) -> Result<Box<dyn Publication>, String> + 'static,
    ) -> Result<BoundaryMount, String> {
        if !self.0.alive.get() || owner.is_disposed() {
            return Err("cannot attach a disposed async boundary".into());
        }
        if self.0.attached.replace(true) {
            return Err("an async boundary can only attach once".into());
        }
        let inner = Rc::downgrade(&self.0);
        let cleanup = owner.on_cleanup(move || {
            if let Some(inner) = inner.upgrade() {
                dispose(&inner);
            }
        });
        let weak = Rc::downgrade(&self.0);
        let owner = owner.clone();
        let evaluate: Rc<Evaluate> = Rc::new(evaluate);
        let subscription = effect(move || {
            if let Some(inner) = weak
                .upgrade()
                .filter(|inner| inner.alive.get() && !owner.is_disposed())
            {
                Versions::exclude(|| {
                    inner.wake.get();
                });
                drive(&inner, &*evaluate);
            }
        });
        Ok(BoundaryMount {
            inner: Rc::downgrade(&self.0),
            subscription,
            _cleanup: cleanup,
        })
    }
}

/// Retained by the renderer. Dropping it cancels candidate reads and subscriptions.
#[doc(hidden)]
pub struct BoundaryMount {
    inner: Weak<Inner>,
    subscription: Effect,
    _cleanup: Registration,
}
impl Drop for BoundaryMount {
    fn drop(&mut self) {
        self.subscription.dispose();
        if let Some(inner) = self.inner.upgrade() {
            dispose(&inner);
        }
    }
}

fn dispose(inner: &Inner) {
    if !inner.alive.replace(false) {
        return;
    }
    let reads = inner.reads.take();
    for read in reads.into_values() {
        read.cancel();
    }
    inner.status.set(BoundaryStatus::Disposed);
}

/// Weak lifetime witness used by read declarations that outlive a mounted view.
#[doc(hidden)]
pub struct BoundaryLifetime(Weak<Inner>);
impl BoundaryLifetime {
    pub fn is_live(&self) -> bool {
        self.0.upgrade().is_some_and(|inner| inner.alive.get())
    }
}

/// One discovery pass in the current attempt. An unresolved read records pending
/// and returns normally; no exception, panic, or unwinding implements suspension.
#[doc(hidden)]
pub struct Attempt {
    inner: Weak<Inner>,
    epoch: u64,
    pending: Cell<bool>,
    reads: RefCell<BTreeMap<u64, Rc<dyn ReadLease>>>,
}
impl Attempt {
    #[doc(hidden)]
    pub fn boundary_lifetime(&self) -> BoundaryLifetime {
        BoundaryLifetime(self.inner.clone())
    }
    pub fn boundary_id(&self) -> u64 {
        self.inner.upgrade().map_or(0, |inner| inner.id)
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn retry_generation(&self) -> u64 {
        self.inner.upgrade().map_or(0, |inner| inner.retry.get())
    }
    pub fn pending(&self) {
        self.pending.set(true);
    }
    pub fn register(&self, id: u64, lease: Rc<dyn ReadLease>) {
        self.reads.borrow_mut().insert(id, lease);
    }
    pub fn notifier(&self) -> impl Fn() + 'static {
        let weak = self.inner.clone();
        let epoch = self.epoch;
        move || {
            if let Some(inner) = weak
                .upgrade()
                .filter(|inner| inner.alive.get() && inner.epoch.get() == epoch)
            {
                inner.wake.update(|n| *n += 1);
            }
        }
    }
}

struct Evaluation;
impl Drop for Evaluation {
    fn drop(&mut self) {
        EVALUATING.with(|stack| {
            stack.borrow_mut().pop();
        });
        EVALUATION_DEPTH.set(EVALUATION_DEPTH.get() - 1);
    }
}
struct Driving<'a>(&'a Cell<bool>);
impl Drop for Driving<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

fn drive(inner: &Rc<Inner>, evaluate: &Evaluate) {
    if inner.rejected_retry.get() == Some(inner.retry.get()) {
        return;
    }
    if inner.driving.replace(true) {
        return;
    }
    let _driving = Driving(&inner.driving);
    inner.status.set(BoundaryStatus::Pending);
    let versions = inner.inputs.borrow().clone();
    if !untrack(|| versions.is_current()) || inner.epoch.get() == 0 {
        inner
            .epoch
            .set(inner.epoch.get().checked_add(1).expect("attempt overflow"));
        let reads = inner.reads.take();
        for read in reads.into_values() {
            read.cancel();
        }
    }
    inner.violation.take();
    let attempt = Attempt {
        inner: Rc::downgrade(inner),
        epoch: inner.epoch.get(),
        pending: Cell::new(false),
        reads: RefCell::new(BTreeMap::new()),
    };
    EVALUATING.with(|stack| stack.borrow_mut().push(Rc::downgrade(inner)));
    EVALUATION_DEPTH.set(EVALUATION_DEPTH.get() + 1);
    let guard = Evaluation;
    let (result, inputs) = Versions::capture(|| evaluate(&attempt));
    drop(guard);
    let old = inner.reads.replace(attempt.reads.take());
    let removed: Vec<_> = old
        .into_iter()
        .filter(|(id, _)| !inner.reads.borrow().contains_key(id))
        .map(|(_, lease)| lease)
        .collect();
    for lease in removed {
        lease.cancel();
    }
    *inner.inputs.borrow_mut() = inputs.clone();
    if !inner.alive.get() {
        return;
    }
    let result = match inner.violation.take() {
        Some(error) => {
            inner.rejected_retry.set(Some(inner.retry.get()));
            Err(error)
        }
        None => result,
    };
    let mut publication = match result {
        Ok(publication) => publication,
        Err(error) => {
            inner.status.set(BoundaryStatus::Error(error));
            return;
        }
    };
    if attempt.pending.get() || !untrack(|| inputs.is_current()) {
        inner.status.set(BoundaryStatus::Pending);
        return;
    }
    if let Err(error) = publication.validate() {
        inner.status.set(BoundaryStatus::Error(error));
        return;
    }
    if !inner.alive.get() || !untrack(|| inputs.is_current()) {
        inner.status.set(BoundaryStatus::Pending);
        return;
    }
    batch(|| {
        if let Err(error) = publication.apply() {
            inner.status.set(BoundaryStatus::Faulted(error));
            return;
        }
        // Keep the guard through activation. Synchronous activation effects may
        // invalidate these inputs and must never be overwritten by Ready.
        publication.finish();
        if inner.alive.get() {
            inner.status.set(if untrack(|| inputs.is_current()) {
                BoundaryStatus::Ready
            } else {
                BoundaryStatus::Pending
            });
        }
    });
}

/// Record a write inside a coherent evaluation. Every signal write checks this,
/// so the idle check inlines into callers.
#[inline]
pub(crate) fn mutation(operation: &str) -> bool {
    EVALUATION_DEPTH.get() != 0 && evaluating_mutation(operation)
}

#[cold]
fn evaluating_mutation(operation: &str) -> bool {
    EVALUATING.with(|stack| {
        let inner = stack.borrow().last().and_then(Weak::upgrade);
        if let Some(inner) = inner {
            *inner.violation.borrow_mut() = Some(format!(
                "coherent render evaluation must be pure: {operation}"
            ));
            true
        } else {
            false
        }
    })
}

pub(crate) fn preparing_owner() -> Option<OwnerHandle> {
    PREPARING.with(|owner| owner.borrow().clone())
}

#[doc(hidden)]
pub fn prepare_state<R>(owner: OwnerHandle, make: impl FnOnce(OwnerHandle) -> R) -> R {
    struct Restore(Option<OwnerHandle>);
    impl Drop for Restore {
        fn drop(&mut self) {
            PREPARING.with(|owner| *owner.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(PREPARING.with(|previous| previous.replace(Some(owner.clone()))));
    make(owner)
}
