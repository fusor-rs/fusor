use fusor::versions::Versions;
use fusor::{Memo, Signal, batch, derived, effect, memo, memo_with_eq, signal, untrack};
use std::{
    cell::{Cell, RefCell},
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};

#[test]
fn lazy_cache_is_shared_and_skips_equal_outputs() {
    let input = signal(1);
    let computations = Rc::new(Cell::new(0));
    let parity = memo({
        let (input, computations) = (input.clone(), computations.clone());
        move || {
            computations.set(computations.get() + 1);
            input.get() % 2
        }
    });
    input.set(3);
    assert_eq!(computations.get(), 0);
    let results = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let (parity, results) = (parity.clone(), results.clone());
        move || results.borrow_mut().push(parity.get())
    });
    assert_eq!(parity.clone().get(), 1);
    input.set(5);
    assert_eq!(computations.get(), 2);
    assert_eq!(*results.borrow(), [1]);
    input.set(6);
    assert_eq!(*results.borrow(), [1, 0]);
    assert_eq!(computations.get(), 3);
}

#[test]
fn diamonds_and_direct_reads_inside_nested_batches_are_consistent() {
    let input = signal(1);
    let double = memo({
        let input = input.clone();
        move || input.get() * 2
    });
    let triple = memo({
        let input = input.clone();
        move || input.get() * 3
    });
    let sum = memo({
        let (double, triple) = (double.clone(), triple.clone());
        move || double.get() + triple.get()
    });
    let results = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let (input, sum, results) = (input.clone(), sum.clone(), results.clone());
        move || results.borrow_mut().push((input.get(), sum.get()))
    });
    batch(|| {
        input.set(2);
        assert_eq!(sum.get(), 10);
        batch(|| {
            input.set(3);
            assert_eq!(double.get(), 6);
            assert_eq!(sum.get(), 15);
        });
        input.set(4);
    });
    assert_eq!(*results.borrow(), [(1, 5), (4, 20)]);
    assert_eq!(triple.get(), 12);
}

#[test]
fn equal_upstream_memos_skip_downstream_computation_and_dynamic_edges_are_removed() {
    let choose = signal(true);
    let left = signal(1);
    let right = signal(2);
    let selected = memo({
        let (choose, left, right) = (choose.clone(), left.clone(), right.clone());
        move || {
            if choose.get() {
                left.get()
            } else {
                right.get()
            }
        }
    });
    let parity = memo({
        let selected = selected.clone();
        move || selected.get() % 2
    });
    let runs = Rc::new(Cell::new(0));
    let output = memo({
        let (runs, parity) = (runs.clone(), parity.clone());
        move || {
            runs.set(runs.get() + 1);
            parity.get() * 10
        }
    });
    assert_eq!(output.get(), 10);
    left.set(3);
    assert_eq!(output.get(), 10);
    assert_eq!(runs.get(), 1);
    choose.set(false);
    assert_eq!(output.get(), 0);
    assert_eq!(runs.get(), 2);
    left.set(4);
    assert_eq!(output.get(), 0);
    assert_eq!(runs.get(), 2);
    right.set(5);
    assert_eq!(output.get(), 10);
    assert_eq!(runs.get(), 3);
}

#[test]
fn custom_equality_retains_the_previous_non_clone_value() {
    struct Payload {
        group: u32,
        detail: u32,
    }
    let input = signal(1);
    let value = memo_with_eq(
        {
            let input = input.clone();
            move || Payload {
                group: input.get() / 10,
                detail: input.get(),
            }
        },
        |a, b| a.group == b.group,
    );
    assert_eq!(value.with(|v| v.detail), 1);
    input.set(2);
    assert_eq!(value.clone().with(|v| v.detail), 1);
    input.set(10);
    assert_eq!(value.with(|v| v.detail), 10);
}

#[test]
fn untracked_reads_preserve_internal_dependencies_and_equality_does_not_track() {
    let input = signal(0);
    let ignored = signal(0);
    let runs = Rc::new(Cell::new(0));
    let value = memo_with_eq(
        {
            let input = input.clone();
            move || input.get()
        },
        {
            let ignored = ignored.clone();
            move |a, b| {
                ignored.get();
                a == b
            }
        },
    );
    let trigger = signal(0);
    let _effect = effect({
        let (value, runs, trigger) = (value.clone(), runs.clone(), trigger.clone());
        move || {
            trigger.get();
            value.get_untracked();
            runs.set(runs.get() + 1);
        }
    });
    input.set(1);
    assert_eq!(runs.get(), 1);
    assert_eq!(untrack(|| value.get()), 1);
    trigger.set(1);
    ignored.set(1);
    assert_eq!(runs.get(), 2);
    let tracked_runs = Rc::new(Cell::new(0));
    let _effect = effect({
        let (value, tracked_runs) = (value.clone(), tracked_runs.clone());
        move || {
            value.get();
            tracked_runs.set(tracked_runs.get() + 1);
        }
    });
    input.set(2);
    ignored.set(2);
    assert_eq!(tracked_runs.get(), 2);
}

#[test]
fn first_untracked_memo_read_captures_its_published_version() {
    let input = signal(1);
    let value = memo({
        let input = input.clone();
        move || input.get() % 2
    });
    let (initial, versions) = Versions::capture(|| value.get_untracked());
    assert_eq!(initial, 1);
    assert!(versions.is_current());
    input.set(3);
    assert!(
        versions.is_current(),
        "equal output changed the captured version"
    );
    let (_, equal) = Versions::capture(|| value.clone().get_untracked());
    assert!(versions.same(&equal));
    input.set(4);
    assert!(
        !versions.is_current(),
        "untracked read did not capture the memo"
    );
    let (changed, current) = Versions::capture(|| value.get_untracked());
    assert_eq!(changed, 0);
    assert!(current.is_current());
    assert!(!versions.same(&current));
}

#[test]
fn last_unread_clone_releases_compute_and_equality_captures() {
    let input = signal(0);
    let computations = Rc::new(Cell::new(0));
    let compute_capture = Rc::new(());
    let equal_capture = Rc::new(());
    let compute_weak = Rc::downgrade(&compute_capture);
    let equal_weak = Rc::downgrade(&equal_capture);
    let value = memo_with_eq(
        {
            let (input, computations) = (input.clone(), computations.clone());
            move || {
                let _ = &compute_capture;
                computations.set(computations.get() + 1);
                input.get()
            }
        },
        move |a, b| {
            let _ = &equal_capture;
            a == b
        },
    );
    let last = value.clone();
    drop(value);
    input.set(1);
    assert_eq!(computations.get(), 0);
    assert!(compute_weak.upgrade().is_some());
    assert!(equal_weak.upgrade().is_some());
    drop(last);
    assert!(compute_weak.upgrade().is_none());
    assert!(equal_weak.upgrade().is_none());
    input.set(2);
    assert_eq!(computations.get(), 0);
}

#[test]
fn last_handle_drop_releases_captures_and_pending_consumers_can_be_disposed() {
    let input = signal(0);
    let captured = Rc::new(());
    let weak = Rc::downgrade(&captured);
    let value = memo({
        let input = input.clone();
        move || {
            let _ = &captured;
            input.get()
        }
    });
    let runs = Rc::new(Cell::new(0));
    let subscription = effect({
        let (value, runs) = (value.clone(), runs.clone());
        move || {
            value.get();
            runs.set(runs.get() + 1);
        }
    });
    drop(value);
    batch(|| {
        input.set(1);
        drop(subscription);
    });
    assert!(weak.upgrade().is_none());
    input.set(2);
    assert_eq!(runs.get(), 1);
}

#[test]
fn panics_and_forbidden_writes_do_not_poison_the_graph() {
    let input = signal(0);
    let value = memo({
        let input = input.clone();
        move || {
            let n = input.get();
            assert_ne!(n, 1, "intentional");
            n
        }
    });
    let results = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let (value, results) = (value.clone(), results.clone());
        move || results.borrow_mut().push(value.get())
    });
    assert!(catch_unwind(AssertUnwindSafe(|| input.set(1))).is_err());
    input.set(2);
    assert_eq!(*results.borrow(), [0, 2]);
    let illegal = memo({
        let input = input.clone();
        move || {
            untrack(|| input.set(99));
            0
        }
    });
    assert!(catch_unwind(AssertUnwindSafe(|| illegal.get())).is_err());
    assert_eq!(input.get(), 2);
    let illegal = memo(|| {
        let _effect = effect(|| {});
        0
    });
    assert!(catch_unwind(AssertUnwindSafe(|| illegal.get())).is_err());
    input.set(3);
    assert_eq!(*results.borrow(), [0, 2, 3]);
}

#[test]
fn failed_recomputations_retry_without_another_signal_write() {
    let input = signal(0);
    let fail_compute = Rc::new(Cell::new(false));
    let fail_equal = Rc::new(Cell::new(false));
    let value = memo_with_eq(
        {
            let (input, fail) = (input.clone(), fail_compute.clone());
            move || {
                let value = input.get();
                assert!(!fail.replace(false), "compute failed once");
                value
            }
        },
        {
            let fail = fail_equal.clone();
            move |a, b| {
                assert!(!fail.replace(false), "equality failed once");
                a == b
            }
        },
    );
    assert_eq!(value.get(), 0);
    input.set(1);
    fail_compute.set(true);
    assert!(catch_unwind(AssertUnwindSafe(|| value.get())).is_err());
    assert_eq!(value.get(), 1);
    input.set(2);
    fail_equal.set(true);
    assert!(catch_unwind(AssertUnwindSafe(|| value.get())).is_err());
    assert_eq!(value.get(), 2);
}

#[test]
fn failed_first_read_retries_without_a_write_and_tracks_remaining_inputs() {
    let first = signal(1);
    let second = signal(2);
    let value = memo({
        let (first, second) = (first.clone(), second.clone());
        let fail = Cell::new(true);
        move || {
            let first = first.get();
            assert!(!fail.replace(false), "first read failed once");
            first + second.get()
        }
    });
    assert!(catch_unwind(AssertUnwindSafe(|| value.get())).is_err());
    assert_eq!(value.get(), 3);
    let results = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let results = results.clone();
        move || results.borrow_mut().push(value.get())
    });
    first.set(2);
    second.set(3);
    assert_eq!(*results.borrow(), [3, 4, 5]);
}

#[test]
fn destructors_of_discarded_equal_values_do_not_subscribe_the_caller() {
    struct Value {
        value: i32,
        unrelated: Signal<i32>,
    }
    impl Drop for Value {
        fn drop(&mut self) {
            self.unrelated.get();
        }
    }
    let input = signal(0);
    let unrelated = signal(0);
    let value = memo_with_eq(
        {
            let (input, unrelated) = (input.clone(), unrelated.clone());
            move || Value {
                value: input.get() / 10,
                unrelated: unrelated.clone(),
            }
        },
        |a, b| a.value == b.value,
    );
    let runs = Rc::new(Cell::new(0));
    let _effect = effect({
        let (input, runs) = (input.clone(), runs.clone());
        move || {
            input.get(); // Rerun even when the memo result stays equal.
            value.with(|_| ());
            runs.set(runs.get() + 1);
        }
    });
    input.set(1);
    assert_eq!(runs.get(), 2);
    unrelated.set(1);
    assert_eq!(runs.get(), 2);

    let cache = memo_with_eq(
        {
            let unrelated = unrelated.clone();
            move || Value {
                value: 0,
                unrelated: unrelated.clone(),
            }
        },
        |a, b| a.value == b.value,
    );
    cache.with_untracked(|_| ());
    let mut last_handle = Some(cache);
    let drops = Rc::new(Cell::new(0));
    let _dispose_cache = effect({
        let drops = drops.clone();
        move || {
            drop(last_handle.take());
            drops.set(drops.get() + 1);
        }
    });
    unrelated.set(2);
    assert_eq!(
        drops.get(),
        1,
        "final cache destruction must also stay untracked"
    );
    assert_eq!(runs.get(), 2);
}

#[test]
fn a_cycle_on_the_first_read_reports_the_error_and_recovers() {
    let holder = Rc::new(RefCell::new(None::<Memo<i32>>));
    let recursive = signal(true);
    let value = memo({
        let (holder, recursive) = (holder.clone(), recursive.clone());
        move || {
            if recursive.get() {
                holder.borrow().as_ref().unwrap().get()
            } else {
                42
            }
        }
    });
    *holder.borrow_mut() = Some(value.clone());
    let error = catch_unwind(AssertUnwindSafe(|| value.get())).unwrap_err();
    let message = error
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| error.downcast_ref::<String>().map(String::as_str));
    assert_eq!(message, Some("reactive cycle: a memo depends on itself"));
    recursive.set(false);
    assert_eq!(value.get(), 42);
    holder.borrow_mut().take();
}

#[test]
fn cycles_fail_clearly_and_can_recover_after_a_branch_change() {
    let holder = Rc::new(RefCell::new(None::<Memo<i32>>));
    let recursive = signal(false);
    let value = memo({
        let (holder, recursive) = (holder.clone(), recursive.clone());
        move || {
            if recursive.get() {
                holder.borrow().as_ref().unwrap().get()
            } else {
                42
            }
        }
    });
    *holder.borrow_mut() = Some(value.clone());
    assert_eq!(value.get(), 42);
    recursive.set(true);
    assert!(catch_unwind(AssertUnwindSafe(|| value.get())).is_err());
    recursive.set(false);
    assert_eq!(value.get(), 42);
    holder.borrow_mut().take();
}

#[test]
fn effect_feedback_is_queued_and_mixed_derived_reads_stay_current() {
    let input = signal(0);
    let value = memo({
        let input = input.clone();
        move || input.get()
    });
    let double = derived({
        let value = value.clone();
        move || value.get() * 2
    });
    let _effect = effect({
        let input = input.clone();
        move || {
            let n = double.get();
            if n < 10 {
                input.set(n / 2 + 1);
            }
        }
    });
    assert_eq!(input.get(), 5);
    assert_eq!(value.get(), 5);
}

#[test]
fn randomized_branching_graphs_agree_with_full_recomputation() {
    // No graph-internal assertions: observable cached/effect results are checked
    // against a plain independent reference across thousands of input changes.
    for seed in 1..=16_u64 {
        let mut random = seed;
        let mut next = || {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            random
        };
        let inputs: Vec<_> = (0..8).map(|_| signal(0_i64)).collect();
        let mut specs = Vec::new();
        let mut nodes: Vec<Memo<i64>> = Vec::new();
        for i in 0..24 {
            let gate = (next() % 8) as usize;
            let a = (next() % (i + 8)) as usize;
            let b = (next() % (i + 8)) as usize;
            specs.push((gate, a, b));
            let sources = inputs.clone();
            let previous = nodes.clone();
            nodes.push(memo(move || {
                let selected = if sources[gate].get() % 2 == 0 { a } else { b };
                let value = if selected < 8 {
                    sources[selected].get()
                } else {
                    previous[selected - 8].get()
                };
                (value + 3) % 97
            }));
        }
        let observed = Rc::new(Cell::new(0));
        let _effect = effect({
            let (nodes, observed) = (nodes.clone(), observed.clone());
            move || observed.set(nodes.iter().map(Memo::get).sum::<i64>())
        });
        for _ in 0..200 {
            batch(|| {
                for _ in 0..3 {
                    let index = (next() % 8) as usize;
                    inputs[index].set((next() % 50) as i64);
                    let mut expected: Vec<_> = inputs.iter().map(|s| s.get_untracked()).collect();
                    for &(gate, a, b) in &specs {
                        let selected = if expected[gate] % 2 == 0 { a } else { b };
                        expected.push((expected[selected] + 3) % 97);
                    }
                    for (node, expected) in nodes.iter().zip(&expected[8..]) {
                        assert_eq!(node.get(), *expected, "seed {seed}");
                    }
                }
            });
            assert_eq!(observed.get(), nodes.iter().map(Memo::get).sum::<i64>());
        }
    }
}

#[test]
fn memo_equality_can_dispose_the_effect_that_is_validating_it() {
    let input = signal(0);
    let subscription = Rc::new(RefCell::new(None::<fusor::Effect>));
    let weak = Rc::downgrade(&subscription);
    let value = memo_with_eq(
        {
            let input = input.clone();
            move || input.get()
        },
        move |old, next| {
            if let Some(subscription) = weak.upgrade() {
                subscription.borrow().as_ref().unwrap().dispose();
            }
            old == next
        },
    );
    let calls = Rc::new(Cell::new(0));
    *subscription.borrow_mut() = Some(effect({
        let calls = calls.clone();
        move || {
            value.get();
            calls.set(calls.get() + 1);
        }
    }));
    input.set(1);
    let settled = calls.get();
    input.set(2);
    assert_eq!(calls.get(), settled);
}

#[test]
fn memo_equality_disposal_preserves_multi_dependency_validation_snapshots() {
    // Equality keeps the first source version unchanged, so validation continues
    // through the remaining snapshot after disposal unsubscribes the live graph.
    for count in [2, 3] {
        let input = signal(0);
        let subscription = Rc::new(RefCell::new(None::<fusor::Effect>));
        let weak = Rc::downgrade(&subscription);
        let value = memo_with_eq(
            {
                let input = input.clone();
                move || input.get() % 2
            },
            move |old, next| {
                weak.upgrade().unwrap().borrow().as_ref().unwrap().dispose();
                old == next
            },
        );
        let remaining: Vec<_> = (1..count).map(signal).collect();
        let calls = Rc::new(Cell::new(0));
        *subscription.borrow_mut() = Some(effect({
            let calls = calls.clone();
            let remaining = remaining.clone();
            move || {
                value.get();
                for source in &remaining {
                    source.get();
                }
                calls.set(calls.get() + 1);
            }
        }));
        input.set(2);
        assert_eq!(calls.get(), 1);
        for source in &remaining {
            source.set(100);
        }
        input.set(4);
        assert_eq!(calls.get(), 1);
    }
}

#[test]
fn widening_memo_notifications_preserve_breadth_first_effect_order() {
    let input = signal(1);
    let shared = memo({
        let input = input.clone();
        move || input.get() * 2
    });
    let deep = memo({
        let shared = shared.clone();
        move || shared.get() + 1
    });
    let sibling = memo({
        let shared = shared.clone();
        move || shared.get() + 2
    });
    let log = Rc::new(RefCell::new(Vec::new()));
    let _direct = effect({
        let log = log.clone();
        move || log.borrow_mut().push(("direct", shared.get()))
    });
    let _deep = effect({
        let log = log.clone();
        move || log.borrow_mut().push(("deep", deep.get()))
    });
    let _sibling = effect({
        let log = log.clone();
        move || log.borrow_mut().push(("sibling", sibling.get()))
    });
    log.borrow_mut().clear();
    input.set(3);
    assert_eq!(*log.borrow(), [("direct", 6), ("deep", 7), ("sibling", 8)]);
}

#[test]
fn memo_notifications_keep_creation_order_when_first_reads_are_reversed() {
    let input = signal(0);
    let first = memo({
        let input = input.clone();
        move || input.get() + 10
    });
    let second = memo({
        let input = input.clone();
        move || input.get() + 20
    });
    let log = Rc::new(RefCell::new(Vec::new()));
    let _second = effect({
        let log = log.clone();
        move || log.borrow_mut().push(("second", second.get()))
    });
    let _first = effect({
        let log = log.clone();
        move || log.borrow_mut().push(("first", first.get()))
    });
    log.borrow_mut().clear();
    input.set(1);
    assert_eq!(*log.borrow(), [("first", 11), ("second", 21)]);
}
