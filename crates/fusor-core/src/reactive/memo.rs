use super::{
    COMPUTING, RENDERING, RenderGuard, TrackingGuard,
    graph::{Observer, ObserverKind, Source, track},
    untrack,
};
use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::{Rc, Weak},
};

/// A shared lazy cache of a pure computation over tracked signals and memos.
///
/// Reads validate dependencies synchronously, including inside a batch. Equal
/// results keep the previous cached value and suppress downstream effects that
/// have no other changed dependencies. Keep a handle alive while using the memo;
/// dropping the last handle releases its cache, captures, and subscriptions.
/// Computation and equality functions must be pure. Managed signal writes and
/// effect creation during evaluation panic before changing reactive state.
#[must_use = "memos are lazy; retain a handle and read it to use the computation"]
pub struct Memo<T>(Rc<MemoInner<T>>);

impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Create a cached derivation using `PartialEq` to suppress unchanged results.
pub fn memo<T: PartialEq + 'static>(compute: impl Fn() -> T + 'static) -> Memo<T> {
    memo_with_eq(compute, T::eq)
}

/// Create a cached derivation with an explicit equivalence relation.
/// Equal values retain the old cache. `|_, _| false` publishes every recomputation.
pub fn memo_with_eq<T: 'static>(
    compute: impl Fn() -> T + 'static,
    equal: impl Fn(&T, &T) -> bool + 'static,
) -> Memo<T> {
    Memo(Rc::<MemoInner<T>>::new_cyclic(|weak| MemoInner {
        graph: OnceCell::new(),
        weak_self: weak.clone(),
        observer_id: Observer::reserve_id(),
        value: RefCell::new(None),
        compute: Box::new(compute),
        equal: Box::new(equal),
        stale: Cell::new(true),
        needs_compute: Cell::new(true),
        running: Cell::new(false),
    }))
}

type Equal<T> = dyn Fn(&T, &T) -> bool;

struct MemoGraph {
    source: Rc<Source>,
    observer: Rc<Observer>,
}

struct MemoInner<T> {
    graph: OnceCell<MemoGraph>,
    weak_self: Weak<MemoInner<T>>,
    observer_id: u64,
    value: RefCell<Option<T>>,
    compute: Box<dyn Fn() -> T>,
    equal: Box<Equal<T>>,
    stale: Cell<bool>,
    needs_compute: Cell<bool>,
    running: Cell<bool>,
}

impl<T> Drop for MemoInner<T> {
    fn drop(&mut self) {
        if let Some(graph) = self.graph.get() {
            graph.observer.unsubscribe();
        }
        // Releasing the last handle may happen inside another computation.
        // A cached value's destructor is not a dependency of that caller.
        if let Some(value) = self.value.get_mut().take() {
            untrack(|| drop(value));
        }
    }
}

impl<T: 'static> MemoInner<T> {
    fn graph(&self) -> &MemoGraph {
        // Publish the complete graph before user computation can recurse or panic.
        self.graph.get_or_init(|| MemoGraph {
            source: Rc::new(Source::new(Some(self.weak_self.clone()))),
            observer: Observer::with_id(
                self.observer_id,
                ObserverKind::Memo(self.weak_self.clone()),
            ),
        })
    }
}

pub(super) trait MemoNode {
    fn refresh(&self);
    fn invalidate(&self) -> &Source;
}

struct EvaluationGuard<'a>(&'a Cell<bool>);

impl Drop for EvaluationGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
        COMPUTING.with(|depth| depth.set(depth.get() - 1));
    }
}

impl<T: 'static> MemoNode for MemoInner<T> {
    fn refresh(&self) {
        // Version validation must never publish a candidate into the cache.
        let _committed = RenderGuard::replace(false);
        assert!(
            !self.running.get(),
            "reactive cycle: a memo depends on itself"
        );
        if !self.stale.get() {
            return;
        }
        self.running.set(true);
        COMPUTING.with(|depth| depth.set(depth.get() + 1));
        let _evaluation = EvaluationGuard(&self.running);
        let graph = self.graph();
        if !self.needs_compute.get() && !untrack(|| graph.observer.changed()) {
            self.stale.set(false);
            return;
        }
        // Until publication succeeds, partial dependencies cannot validate the
        // previous cache. In particular, retry after a caught native panic.
        self.needs_compute.set(true);
        let next = {
            let _run = graph.observer.begin();
            let _tracking = TrackingGuard::replace(Some(Rc::downgrade(&graph.observer)));
            (self.compute)()
        };
        let equal = untrack(|| {
            self.value
                .borrow()
                .as_ref()
                .is_some_and(|old| (self.equal)(old, &next))
        });
        if equal {
            untrack(|| drop(next));
        } else {
            let old = self.value.replace(Some(next));
            graph.source.advance();
            // No reactive borrows are held while cached payloads are destroyed.
            untrack(|| drop(old));
        }
        self.needs_compute.set(false);
        self.stale.set(false);
    }

    fn invalidate(&self) -> &Source {
        self.stale.set(true);
        // Only an initialized graph's observer can propagate invalidation here.
        &self.graph.get().expect("memo graph initialized").source
    }
}

impl<T: 'static> Memo<T> {
    /// Borrow the current cached value and subscribe the consuming effect/memo.
    /// Do not change dependencies while this callback borrows the cache.
    pub fn with<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        if RENDERING.with(Cell::get) {
            return self.with_candidate(read);
        }
        self.0.refresh();
        // A successful refresh always initialized the graph before computation.
        track(&self.0.graph.get().expect("memo graph initialized").source);
        read(self.0.value.borrow().as_ref().expect("memo evaluated"))
    }

    fn with_candidate<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        struct Candidate<T>(Option<T>);
        impl<T> Drop for Candidate<T> {
            fn drop(&mut self) {
                untrack(|| drop(self.0.take()));
            }
        }
        let value = {
            assert!(
                !self.0.running.replace(true),
                "reactive cycle: a memo depends on itself"
            );
            COMPUTING.with(|depth| depth.set(depth.get() + 1));
            let _evaluation = EvaluationGuard(&self.0.running);
            // Keep the caller's tracking context: candidate inputs belong to
            // this evaluation, not to the memo's committed dependency graph.
            Candidate(Some((self.0.compute)()))
        };
        read(value.0.as_ref().expect("candidate evaluated"))
    }

    /// Read without subscribing the caller. The memo still tracks its own inputs.
    pub fn with_untracked<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        untrack(|| self.with(read))
    }
}

impl<T: Clone + 'static> Memo<T> {
    pub fn get(&self) -> T {
        self.with(Clone::clone)
    }
    pub fn get_untracked(&self) -> T {
        self.with_untracked(Clone::clone)
    }
}
