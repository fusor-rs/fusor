//! Public integration contracts, exercised without a browser or private imports.
use fusor::{
    Effect, Owner, batch,
    coherence::{AsyncBoundary, BoundaryStatus, Publication, ReadLease, prepare_state},
    effect, memo,
    render::{Children, construct},
    signal,
    versions::Versions,
};
use std::{
    any::Any,
    cell::{Cell, RefCell},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};

struct Construction {
    owner: Owner,
    prepared: bool,
    state: Option<Rc<dyn Any>>,
}

impl fusor::render::Scope for Construction {
    fn owner(&self) -> fusor::OwnerHandle {
        self.owner.handle()
    }

    fn retain_state<T: 'static>(&mut self, state: T) -> Rc<T> {
        let state = Rc::new(state);
        self.state = Some(state.clone());
        state
    }

    fn prepares_effects(&self) -> bool {
        self.prepared
    }
}

#[test]
fn children_delivery_is_nested_typed_and_restored_after_unwind() {
    type Fragment = Children<u32, ()>;
    type Other = Children<String, ()>;
    let owner = Owner::new();
    owner.commit();
    let outer = Fragment::new(|owner| {
        assert!(owner.is_active(), "factory uses the placement owner");
        Ok(1)
    });
    outer.with(|| {
        Fragment::default().with(|| {
            assert_eq!(Fragment::take().prepare(&owner.handle()), Ok(None));
        });
        Other::new(|_| Ok("other".into())).with(|| {
            let failed = catch_unwind(AssertUnwindSafe(|| {
                Fragment::new(|_| Ok(2)).with(|| {
                    assert_eq!(Fragment::take().prepare(&owner.handle()), Ok(Some(2)));
                    panic!("construction failed");
                });
            }));
            assert!(failed.is_err());
            assert_eq!(Fragment::take().prepare(&owner.handle()), Ok(Some(1)));
            assert_eq!(
                Other::take().prepare(&owner.handle()),
                Ok(Some("other".into()))
            );
        });
    });
    assert_eq!(Fragment::take().prepare(&owner.handle()), Ok(None));
    assert_eq!(Other::take().prepare(&owner.handle()), Ok(None));
}
#[test]
fn ordinary_constructor_effects_are_immediate_and_explicit_preparation_defers() {
    let mut ordinary_scope = Construction {
        owner: Owner::new(),
        prepared: false,
        state: None,
    };
    let mut prepared_scope = Construction {
        owner: Owner::new(),
        prepared: true,
        state: None,
    };
    let value = signal(0);
    let ordinary = Rc::new(RefCell::new(Vec::new()));
    let prepared = Rc::new(RefCell::new(Vec::new()));
    let _ordinary = batch(|| {
        let (value, seen) = (value.clone(), ordinary.clone());
        let subscription = construct(&mut ordinary_scope, |_| {
            Ok::<_, ()>(effect(move || seen.borrow_mut().push(value.get())))
        })
        .unwrap();
        assert_eq!(
            *ordinary.borrow(),
            [0],
            "initial effects run even in a batch"
        );
        subscription
    });
    let _prepared = construct(&mut prepared_scope, |_| {
        let (value, seen) = (value.clone(), prepared.clone());
        Ok::<_, ()>(effect(move || seen.borrow_mut().push(value.get())))
    })
    .unwrap();
    value.set(1);
    assert_eq!(*ordinary.borrow(), [0, 1]);
    assert!(prepared.borrow().is_empty());
    prepared_scope.owner.commit();
    assert_eq!(*prepared.borrow(), [1]);
    value.set(2);
    assert_eq!(*prepared.borrow(), [1, 2]);
    prepared_scope.owner.dispose();
    value.set(3);
    assert_eq!(*ordinary.borrow(), [0, 1, 2, 3]);
    assert_eq!(*prepared.borrow(), [1, 2]);
}

#[test]
fn construction_preserves_ambient_tracking_and_nested_effect_dependencies() {
    let mut scope = Construction {
        owner: Owner::new(),
        prepared: false,
        state: None,
    };
    let input = signal(0);
    let nested = signal(0);
    let calls = Rc::new(Cell::new(0));
    let nested_calls = Rc::new(Cell::new(0));
    let _effect = effect({
        let (input, nested, calls, nested_calls) = (
            input.clone(),
            nested.clone(),
            calls.clone(),
            nested_calls.clone(),
        );
        move || {
            calls.set(calls.get() + 1);
            construct(&mut scope, |_| {
                input.get();
                let (nested, calls) = (nested.clone(), nested_calls.clone());
                Ok::<_, ()>(effect(move || {
                    nested.get();
                    calls.set(calls.get() + 1);
                }))
            })
            .unwrap();
        }
    });
    assert_eq!((calls.get(), nested_calls.get()), (1, 1));
    nested.set(1);
    assert_eq!((calls.get(), nested_calls.get()), (1, 2));
    input.set(1);
    assert_eq!((calls.get(), nested_calls.get()), (2, 3));
    nested.set(2);
    assert_eq!((calls.get(), nested_calls.get()), (2, 4));
}

#[test]
fn failed_candidate_never_starts_effects_and_nested_preparation_restores_on_unwind() {
    let parent = Owner::new();
    let discarded = Owner::child(&parent.handle());
    let trace = Rc::new(RefCell::new(Vec::new()));
    let effects = RefCell::new(Vec::<Effect>::new());
    prepare_state(parent.handle(), |_| {
        let failure = catch_unwind(AssertUnwindSafe(|| {
            prepare_state(discarded.handle(), |_| {
                let trace = trace.clone();
                effects
                    .borrow_mut()
                    .push(effect(move || trace.borrow_mut().push("discarded")));
                panic!("renderer validation rejected this candidate");
            });
        }));
        assert!(failure.is_err());
        let trace = trace.clone();
        effects
            .borrow_mut()
            .push(effect(move || trace.borrow_mut().push("parent")));
    });
    discarded.dispose();
    discarded.commit();
    let outside_trace = trace.clone();
    let _outside = effect(move || outside_trace.borrow_mut().push("outside"));
    assert_eq!(*trace.borrow(), ["outside"]);
    parent.commit();
    assert_eq!(*trace.borrow(), ["outside", "parent"]);
    assert_eq!(effects.borrow().len(), 2);
}

#[test]
fn preparation_with_an_active_owner_preserves_ordinary_effect_lifetime() {
    let owner = Owner::new();
    owner.commit();
    let source = signal(1);
    let trace = Rc::new(RefCell::new(Vec::new()));
    let _effect = prepare_state(owner.handle(), |_| {
        let (source, observed) = (source.clone(), trace.clone());
        effect(move || observed.borrow_mut().push(source.get()))
    });
    assert_eq!(*trace.borrow(), [1]);
    owner.dispose();
    source.set(2);
    assert_eq!(*trace.borrow(), [1, 2]);
}

#[test]
fn guarded_self_removal_invalidates_later_dispatch_without_borrowing_the_scene() {
    let scene = Rc::new(RefCell::new(Some(Owner::new())));
    let owner = scene.borrow().as_ref().unwrap().handle();
    scene.borrow().as_ref().unwrap().commit();
    let cleanup = Rc::new(Cell::new(0));
    let cleaned = cleanup.clone();
    let _cleanup = owner.on_cleanup(move || cleaned.set(cleaned.get() + 1));
    let calls = Rc::new(Cell::new(0));
    let (removed, called) = (scene.clone(), calls.clone());
    let mut callback = owner.guarded(move |()| {
        batch(|| {
            called.set(called.get() + 1);
            let retired = removed.borrow_mut().take();
            drop(retired);
        });
    });
    assert_eq!(callback(()), Some(()));
    assert_eq!(callback(()), None);
    assert!(scene.borrow().is_none());
    assert_eq!(calls.get(), 1);
    assert_eq!(cleanup.get(), 1);
}

struct Candidate {
    scene: Rc<Cell<i32>>,
    value: i32,
    trace: Rc<RefCell<Vec<&'static str>>>,
    valid: bool,
    apply_ok: bool,
}

#[test]
fn candidate_invalidation_can_dispose_the_boundary_before_reevaluation() {
    let owner = Rc::new(Owner::new());
    let boundary = AsyncBoundary::coherent();
    let input = signal(0);
    let calls = Rc::new(Cell::new(0));
    let _mount = boundary
        .attach(&owner.handle(), {
            let (owner, input, calls) = (owner.clone(), input.clone(), calls.clone());
            move |attempt| {
                input.get();
                calls.set(calls.get() + 1);
                let owner = owner.clone();
                attempt.on_invalidate(move || owner.dispose());
                Err("pending candidate".into())
            }
        })
        .unwrap();
    input.set(1);
    assert_eq!(calls.get(), 1);
    assert_eq!(boundary.status(), BoundaryStatus::Disposed);
}

impl Publication for Candidate {
    fn validate(&self) -> Result<(), String> {
        self.trace.borrow_mut().push("validate");
        self.valid
            .then_some(())
            .ok_or_else(|| "invalid target".into())
    }

    fn apply(&mut self) -> Result<(), String> {
        self.trace.borrow_mut().push("apply");
        if !self.apply_ok {
            return Err("renderer fault".into());
        }
        self.scene.set(self.value);
        Ok(())
    }

    fn finish(self: Box<Self>) {
        self.trace.borrow_mut().push("finish");
    }
}

impl Drop for Candidate {
    fn drop(&mut self) {
        self.trace.borrow_mut().push("drop");
    }
}

#[test]
fn renderer_validation_and_publication_are_distinct_from_owner_activation() {
    let owner = Owner::new();
    let boundary = AsyncBoundary::coherent();
    let input = signal(1);
    let valid = Rc::new(Cell::new(false));
    let scene = Rc::new(Cell::new(0));
    let trace = Rc::new(RefCell::new(Vec::new()));
    let mount = boundary
        .attach(&owner.handle(), {
            let (input, valid, scene, trace) =
                (input.clone(), valid.clone(), scene.clone(), trace.clone());
            move |_| {
                trace.borrow_mut().push("prepare");
                Ok(Box::new(Candidate {
                    scene: scene.clone(),
                    value: input.get(),
                    trace: trace.clone(),
                    valid: valid.get(),
                    apply_ok: true,
                }))
            }
        })
        .unwrap();
    assert!(!owner.handle().is_active());
    assert_eq!(
        boundary.status(),
        BoundaryStatus::Error("invalid target".into())
    );
    assert_eq!(scene.get(), 0);
    assert_eq!(*trace.borrow(), ["prepare", "validate", "drop"]);
    owner.commit();
    assert_eq!(
        scene.get(),
        0,
        "commit is not renderer validation/publication"
    );
    valid.set(true);
    trace.borrow_mut().clear();
    boundary.retry();
    assert_eq!(boundary.status(), BoundaryStatus::Ready);
    assert_eq!(scene.get(), 1);
    assert_eq!(
        *trace.borrow(),
        ["prepare", "validate", "apply", "finish", "drop"]
    );
    input.set(2);
    assert_eq!(scene.get(), 2);
    drop(mount);
    input.set(3);
    assert_eq!(scene.get(), 2);
    assert_eq!(boundary.status(), BoundaryStatus::Disposed);
    assert!(
        boundary
            .attach(&owner.handle(), |_| unreachable!())
            .is_err()
    );
}

#[test]
fn publication_failure_skips_finish_and_reports_faulted() {
    let owner = Owner::new();
    let boundary = AsyncBoundary::coherent();
    let trace = Rc::new(RefCell::new(Vec::new()));
    let _mount = boundary
        .attach(&owner.handle(), {
            let trace = trace.clone();
            move |_| {
                Ok(Box::new(Candidate {
                    scene: Rc::new(Cell::new(0)),
                    value: 1,
                    trace: trace.clone(),
                    valid: true,
                    apply_ok: false,
                }))
            }
        })
        .unwrap();
    assert_eq!(
        boundary.status(),
        BoundaryStatus::Faulted("renderer fault".into())
    );
    assert_eq!(*trace.borrow(), ["validate", "apply", "drop"]);
}

struct Lease(Rc<Cell<usize>>);
impl ReadLease for Lease {
    fn cancel(&self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn speculative_reads_cancel_on_changed_inputs_and_ignore_stale_notifications() {
    let owner = Owner::new();
    let boundary = AsyncBoundary::coherent();
    let input = signal(1);
    let cancelled = Rc::new(Cell::new(0));
    let ready = Rc::new(Cell::new(false));
    let notices = Rc::new(RefCell::new(Vec::<Box<dyn Fn()>>::new()));
    let evaluations = Rc::new(Cell::new(0));
    let scene = Rc::new(Cell::new(0));
    let _mount = boundary
        .attach(&owner.handle(), {
            let (input, cancelled, ready, notices, evaluations, scene) = (
                input.clone(),
                cancelled.clone(),
                ready.clone(),
                notices.clone(),
                evaluations.clone(),
                scene.clone(),
            );
            move |attempt| {
                evaluations.set(evaluations.get() + 1);
                let value = input.get();
                attempt.register(7, Rc::new(Lease(cancelled.clone())));
                notices.borrow_mut().push(Box::new(attempt.notifier()));
                if !ready.get() {
                    attempt.pending();
                }
                Ok(Box::new(Candidate {
                    scene: scene.clone(),
                    value,
                    trace: Rc::default(),
                    valid: true,
                    apply_ok: true,
                }))
            }
        })
        .unwrap();
    assert!(
        !owner.handle().is_active(),
        "read discovery can precede commit"
    );
    assert_eq!(boundary.status(), BoundaryStatus::Pending);
    assert_eq!(scene.get(), 0);
    input.set(2);
    assert_eq!(cancelled.get(), 1);
    assert_eq!(evaluations.get(), 2);
    let first = notices.borrow_mut().remove(0);
    first();
    assert_eq!(evaluations.get(), 2);
    ready.set(true);
    let current = notices.borrow_mut().remove(0);
    current();
    assert_eq!(scene.get(), 2);
    assert_eq!(boundary.status(), BoundaryStatus::Ready);
    owner.dispose();
    assert_eq!(cancelled.get(), 2);
    current();
    assert_eq!(evaluations.get(), 3);
}

#[test]
fn candidate_signal_reads_validate_collection_versions_without_publishing() {
    let collection = signal(vec![2]);
    let row = signal(1);
    let observed = Rc::new(RefCell::new(Vec::new()));
    let (read, trace) = (row.clone(), observed.clone());
    let _effect = effect(move || trace.borrow_mut().push(read.get()));
    let (items, inputs) = Versions::capture(|| collection.get());
    let (candidate, versions) = row.with_render_value(Rc::new(items[0]), inputs, || {
        let (value, versions) = Versions::capture(|| row.get());
        assert_eq!(row.get_untracked(), 2);
        assert_eq!(
            row.with_render_value(Rc::new(3), versions.clone(), || row.get()),
            3
        );
        assert_eq!(row.get(), 2);
        (value, versions)
    });
    assert_eq!(candidate, 2);
    assert_eq!(row.get(), 1);
    assert_eq!(*observed.borrow(), [1]);
    assert!(versions.is_current());
    collection.set(vec![4]);
    assert!(!versions.is_current());
    assert_eq!(*observed.borrow(), [1]);
}

#[test]
fn candidate_memos_restore_on_unwind_without_changing_the_committed_cache() {
    let row = signal(1);
    let computations = Rc::new(Cell::new(0));
    let projection = memo({
        let (row, computations) = (row.clone(), computations.clone());
        move || {
            computations.set(computations.get() + 1);
            let value = row.get();
            assert_ne!(value, 2, "candidate rejected");
            value * 10
        }
    });
    assert_eq!(projection.get(), 10);
    let failed = catch_unwind(AssertUnwindSafe(|| {
        row.with_render_value(Rc::new(2), Versions::default(), || {
            projection.get();
        });
    }));
    assert!(failed.is_err());
    assert_eq!(row.get(), 1);
    assert_eq!(projection.get(), 10);
    assert_eq!(computations.get(), 2, "committed cache was discarded");
    row.with_render_value(Rc::new(3), Versions::default(), || {
        assert_eq!(projection.get_untracked(), 30);
    });
    row.set(4);
    assert_eq!(
        projection.get(),
        40,
        "published updates still invalidate the memo"
    );
}

#[test]
fn nested_candidate_memos_track_actual_sources_without_replacing_committed_dependencies() {
    let collection = signal(true);
    let row = signal(false);
    let left = signal(1);
    let right = signal(2);
    let selected = memo({
        let (row, left, right) = (row.clone(), left.clone(), right.clone());
        move || if row.get() { right.get() } else { left.get() }
    });
    let projection = memo({
        let selected = selected.clone();
        move || selected.get() * 10
    });
    let observed = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let (projection, observed) = (projection.clone(), observed.clone());
        move || observed.borrow_mut().push(projection.get())
    });
    let (_, inputs) = Versions::capture(|| collection.get());
    let versions = row.with_render_value(Rc::new(true), inputs, || {
        let (value, versions) = Versions::capture(|| projection.get());
        assert_eq!(value, 20);
        let (_, expected) = Versions::capture(|| (collection.get(), right.get()));
        assert!(versions.same(&expected));
        row.with_render_value(Rc::new(false), Versions::default(), || {
            assert_eq!(projection.get(), 10);
        });
        assert_eq!(projection.get(), 20);
        versions
    });
    assert!(versions.is_current());
    assert_eq!(projection.get(), 10);
    right.set(3);
    assert!(!versions.is_current());
    assert_eq!(*observed.borrow(), [10]);
    left.set(4);
    assert_eq!(*observed.borrow(), [10, 40]);

    let (_, inputs) = Versions::capture(|| collection.get());
    let (_, versions) = row.with_render_value(Rc::new(true), inputs, || {
        Versions::capture(|| projection.get_untracked())
    });
    collection.set(false);
    assert!(!versions.is_current());
}

#[test]
fn candidate_memo_reads_subscribe_the_consumer_without_initializing_the_cache() {
    let collection = signal(2);
    let row = signal(1);
    let projection = memo({
        let row = row.clone();
        move || row.get() * 10
    });
    let observed = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let (collection, row, projection, observed) = (
            collection.clone(),
            row.clone(),
            projection.clone(),
            observed.clone(),
        );
        move || {
            let (value, versions) = Versions::capture(|| collection.get());
            row.with_render_value(Rc::new(value), versions, || {
                observed.borrow_mut().push(projection.get());
            });
        }
    });
    assert_eq!(
        projection.get(),
        10,
        "first candidate read filled the cache"
    );
    collection.set(3);
    assert_eq!(*observed.borrow(), [20, 30]);
    assert_eq!(projection.get(), 10);
    let untracked_reads = Rc::new(Cell::new(0));
    let _untracked = effect({
        let (row, projection, untracked_reads) =
            (row.clone(), projection.clone(), untracked_reads.clone());
        move || {
            row.with_render_value(Rc::new(4), Versions::default(), || {
                assert_eq!(projection.get_untracked(), 40);
            });
            untracked_reads.set(untracked_reads.get() + 1);
        }
    });
    row.set(2);
    assert_eq!(*observed.borrow(), [20, 30, 30]);
    assert_eq!(projection.get(), 20);
    assert_eq!(untracked_reads.get(), 1);
}

#[test]
fn version_validation_inside_an_override_refreshes_only_committed_memo_values() {
    let row = signal(1);
    let projection = memo({
        let row = row.clone();
        move || row.get() * 10
    });
    let (_, versions) = Versions::capture(|| projection.get());
    let observed = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let (projection, observed) = (projection.clone(), observed.clone());
        move || observed.borrow_mut().push(projection.get())
    });
    batch(|| {
        row.set(2);
        row.with_render_value(Rc::new(3), Versions::default(), || {
            assert!(!versions.is_current());
            assert_eq!(projection.get(), 30);
        });
        assert_eq!(projection.get(), 20);
        assert_eq!(*observed.borrow(), [10]);
    });
    assert_eq!(*observed.borrow(), [10, 20]);
}

#[test]
fn excluding_completion_versions_preserves_normal_reactive_dependencies() {
    let input = signal(1);
    let complete = signal(false);
    let calls = Rc::new(Cell::new(0));
    let captured = Rc::new(RefCell::new(Versions::default()));
    let _effect = effect({
        let (input, complete, calls, captured) = (
            input.clone(),
            complete.clone(),
            calls.clone(),
            captured.clone(),
        );
        move || {
            let (_, versions) = Versions::capture(|| {
                input.get();
                Versions::exclude(|| complete.get());
            });
            *captured.borrow_mut() = versions;
            calls.set(calls.get() + 1);
        }
    });
    let initial = captured.borrow().clone();
    complete.set(true);
    assert_eq!(calls.get(), 2);
    assert!(initial.is_current());
    assert!(initial.same(&captured.borrow()));
    input.set(2);
    assert!(!initial.is_current());
    assert!(!initial.same(&captured.borrow()));
}
