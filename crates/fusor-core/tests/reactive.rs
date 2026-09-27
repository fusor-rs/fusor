use fusor::{batch, derived, effect, signal, untrack};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[test]
fn leaf_effect_notifications_keep_batch_disposal_and_observer_order() {
    let value = signal(0);
    let log = Rc::new(RefCell::new(Vec::new()));
    let observe = |id| {
        let value = value.clone();
        let log = log.clone();
        effect(move || log.borrow_mut().push((id, value.get())))
    };
    let first = observe(1);
    log.borrow_mut().clear();
    batch(|| {
        value.set(1);
        value.set(2);
    });
    assert_eq!(*log.borrow(), [(1, 2)]);
    let second = observe(2);
    log.borrow_mut().clear();
    value.set(3);
    assert_eq!(*log.borrow(), [(1, 3), (2, 3)]);
    drop(first);
    log.borrow_mut().clear();
    value.set(4);
    assert_eq!(*log.borrow(), [(2, 4)]);
    batch(|| {
        value.set(5);
        second.dispose();
        value.set(6);
    });
    assert_eq!(*log.borrow(), [(2, 4)]);

    let input = value.clone();
    let memo = fusor::memo(move || input.get() * 2);
    let output = Rc::new(Cell::new(0));
    let seen = output.clone();
    let _effect = effect(move || seen.set(memo.get()));
    value.set(7);
    assert_eq!(output.get(), 14);
}

#[test]
fn tracks_reads_and_skips_equal_writes() {
    let count = signal(1);
    let log = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let count = count.clone();
        let log = log.clone();
        move || log.borrow_mut().push(count.get())
    });
    count.set(1);
    count.set(2);
    assert_eq!(*log.borrow(), [1, 2]);
}

#[test]
fn conditional_reads_remove_old_dependencies() {
    let choose_left = signal(true);
    let left = signal(10);
    let right = signal(20);
    let log = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let (choose_left, left, right, log) = (
            choose_left.clone(),
            left.clone(),
            right.clone(),
            log.clone(),
        );
        move || {
            log.borrow_mut().push(if choose_left.get() {
                left.get()
            } else {
                right.get()
            })
        }
    });
    right.set(21);
    choose_left.set(false);
    left.set(11);
    right.set(22);
    assert_eq!(*log.borrow(), [10, 21, 22]);
}

#[test]
fn nested_batches_and_diamond_derivations_have_one_consistent_result() {
    let value = signal(1);
    let double = derived({
        let value = value.clone();
        move || value.get() * 2
    });
    let triple = derived({
        let value = value.clone();
        move || value.get() * 3
    });
    let log = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let log = log.clone();
        move || log.borrow_mut().push(double.get() + triple.get())
    });
    let result = batch(|| {
        value.set(2);
        batch(|| value.set(3));
        value.set(4);
        42
    });
    assert_eq!(result, 42);
    assert_eq!(*log.borrow(), [5, 20]);
}

#[test]
fn updates_release_the_mutable_borrow_before_notifying() {
    let values = signal(vec![1]);
    let total = Rc::new(Cell::new(0));
    let _effect = effect({
        let values = values.clone();
        let total = total.clone();
        move || total.set(values.with(|v| v.iter().sum::<i32>()))
    });
    values.update(|values| values.push(2));
    assert_eq!(total.get(), 3);
}

#[test]
fn untracked_reads_do_not_subscribe() {
    let tracked = signal(1);
    let ignored = signal(10);
    let log = Rc::new(RefCell::new(Vec::new()));
    let _effect = effect({
        let (tracked, ignored, log) = (tracked.clone(), ignored.clone(), log.clone());
        move || {
            log.borrow_mut().push((
                tracked.get(),
                untrack(|| ignored.get()),
                ignored.get_untracked(),
            ));
        }
    });
    ignored.set(20);
    tracked.set(2);
    assert_eq!(*log.borrow(), [(1, 10, 10), (2, 20, 20)]);
}

#[test]
fn disposal_cancels_pending_work_and_releases_captured_state() {
    let value = signal(0);
    let owner = Rc::new(());
    let weak = Rc::downgrade(&owner);
    let runs = Rc::new(Cell::new(0));
    let subscription = effect({
        let (value, runs) = (value.clone(), runs.clone());
        move || {
            let _ = &owner;
            value.get();
            runs.set(runs.get() + 1);
        }
    });
    batch(|| {
        value.set(1);
        drop(subscription);
    });
    value.set(2);
    assert_eq!(runs.get(), 1);
    assert!(weak.upgrade().is_none());
}

#[test]
fn effect_writes_are_queued_instead_of_recursively_borrowed() {
    let value = signal(0);
    let _effect = effect({
        let value = value.clone();
        move || {
            let next = value.get();
            if next < 5 {
                value.set(next + 1);
            }
        }
    });
    assert_eq!(value.get(), 5);
}

#[test]
fn multiple_subscribers_are_all_queued_before_the_first_runs() {
    let value = signal(0);
    let log = Rc::new(RefCell::new(Vec::new()));
    let _first = effect({
        let value = value.clone();
        move || {
            if value.get() == 1 {
                value.set(2);
            }
        }
    });
    let _second = effect({
        let value = value.clone();
        let log = log.clone();
        move || log.borrow_mut().push(value.get())
    });
    value.set(1);
    assert_eq!(*log.borrow(), [0, 2]);
}

#[test]
fn nested_effects_restore_parent_tracking() {
    let a = signal(0);
    let b = signal(0);
    let inner = Rc::new(RefCell::new(None));
    let runs = Rc::new(Cell::new(0));
    let _outer = effect({
        let (a, b, inner, runs) = (a.clone(), b.clone(), inner.clone(), runs.clone());
        move || {
            let a = a.clone();
            *inner.borrow_mut() = Some(effect(move || {
                a.get();
            }));
            b.get();
            runs.set(runs.get() + 1);
        }
    });
    a.set(1);
    assert_eq!(runs.get(), 1);
    b.set(1);
    assert_eq!(runs.get(), 2);
}

#[test]
fn panic_does_not_poison_scheduler_or_tracking() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let value = signal(0);
    let subscription = effect({
        let value = value.clone();
        move || {
            assert_ne!(value.get(), 1, "intentional");
        }
    });
    assert!(catch_unwind(AssertUnwindSafe(|| value.set(1))).is_err());
    drop(subscription);
    let seen = Rc::new(Cell::new(0));
    let _effect = effect({
        let value = value.clone();
        let seen = seen.clone();
        move || seen.set(value.get())
    });
    value.set(2);
    assert_eq!(seen.get(), 2);
    assert!(
        catch_unwind(AssertUnwindSafe(|| batch(|| {
            value.set(3);
            panic!("intentional");
        })))
        .is_err()
    );
    value.set(4);
    assert_eq!(seen.get(), 4);
}

#[test]
fn runaway_cycles_fail_clearly_and_leave_the_scheduler_usable() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let value = signal(0);
    let result = catch_unwind(AssertUnwindSafe(|| {
        effect({
            let value = value.clone();
            move || value.set(value.get() + 1)
        })
    }));
    assert!(result.is_err());
    let seen = Rc::new(Cell::new(0));
    let _effect = effect({
        let value = value.clone();
        let seen = seen.clone();
        move || seen.set(value.get())
    });
    value.set(42);
    assert_eq!(seen.get(), 42);
}

#[test]
fn signal_clone_does_not_require_the_value_to_be_clone() {
    struct NotClone(usize);
    let first = signal(NotClone(1));
    let second = first.clone();
    second.update(|value| value.0 = 2);
    assert_eq!(first.with(|value| value.0), 2);
}

#[test]
fn older_observers_joining_late_keep_notification_order_after_repeated_detachment() {
    let enabled = signal(false);
    let source = signal(0);
    let log = Rc::new(RefCell::new(Vec::new()));
    let _early = effect({
        let (enabled, source, log) = (enabled.clone(), source.clone(), log.clone());
        move || {
            if enabled.get() {
                source.get();
                log.borrow_mut().push("early");
            }
        }
    });
    let _late = effect({
        let (source, log) = (source.clone(), log.clone());
        move || {
            source.get();
            log.borrow_mut().push("late");
        }
    });
    for value in 1..10 {
        enabled.set(true);
        log.borrow_mut().clear();
        source.set(value);
        assert_eq!(*log.borrow(), ["early", "late"]);
        enabled.set(false);
        log.borrow_mut().clear();
        source.set(-value);
        assert_eq!(*log.borrow(), ["late"]);
    }
}

#[test]
fn an_unread_previous_dependency_cannot_reschedule_its_running_effect() {
    use std::{cell::Cell, rc::Rc};
    let switch = fusor::signal(false);
    let previous = fusor::signal(0);
    let current = fusor::signal(0);
    let calls = Rc::new(Cell::new(0));
    let _effect = fusor::effect({
        let (switch, previous, current, calls) = (
            switch.clone(),
            previous.clone(),
            current.clone(),
            calls.clone(),
        );
        move || {
            calls.set(calls.get() + 1);
            if switch.get() {
                previous.update(|value| *value += 1);
                current.get();
            } else {
                previous.get();
            }
        }
    });
    switch.set(true);
    assert_eq!(calls.get(), 2);
    previous.set(99);
    assert_eq!(calls.get(), 2);
    current.set(1);
    assert_eq!(calls.get(), 3);
}

#[test]
fn a_panicked_effect_retains_only_dependencies_read_before_the_panic() {
    use std::{
        cell::Cell,
        panic::{AssertUnwindSafe, catch_unwind},
        rc::Rc,
    };
    let trigger = fusor::signal(false);
    let previous = fusor::signal(0);
    let next = fusor::signal(0);
    let panic_once = Rc::new(Cell::new(true));
    let calls = Rc::new(Cell::new(0));
    let _effect = fusor::effect({
        let (trigger, previous, next, panic_once, calls) = (
            trigger.clone(),
            previous.clone(),
            next.clone(),
            panic_once.clone(),
            calls.clone(),
        );
        move || {
            calls.set(calls.get() + 1);
            if trigger.get() {
                next.get();
                assert!(!panic_once.replace(false), "intentional partial collection");
            } else {
                previous.get();
            }
        }
    });
    assert!(catch_unwind(AssertUnwindSafe(|| trigger.set(true))).is_err());
    previous.set(1);
    assert_eq!(calls.get(), 2);
    next.set(1);
    assert_eq!(calls.get(), 3);
}

#[test]
fn wide_acyclic_flushes_are_not_feedback_cycles() {
    let source = signal(0);
    let calls = Rc::new(Cell::new(0));
    let _effects: Vec<_> = (0..20_001)
        .map(|_| {
            effect({
                let source = source.clone();
                let calls = calls.clone();
                move || {
                    source.get();
                    calls.set(calls.get() + 1);
                }
            })
        })
        .collect();
    calls.set(0);
    source.set(1);
    assert_eq!(calls.get(), 20_001);
    calls.set(0);
    source.set(2);
    assert_eq!(calls.get(), 20_001);
}

#[test]
fn long_acyclic_chains_are_not_feedback_cycles() {
    let signals: Vec<_> = (0..12_001).map(|_| signal(0)).collect();
    let _effects: Vec<_> = signals
        .windows(2)
        .map(|pair| {
            let from = pair[0].clone();
            let to = pair[1].clone();
            effect(move || to.set(from.get()))
        })
        .collect();
    signals[0].set(42);
    assert_eq!(signals.last().unwrap().get(), 42);
}

#[test]
fn replacement_and_equal_input_destructors_read_published_state() {
    struct Value(i32, Option<Box<dyn Fn()>>);
    impl PartialEq for Value {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }
    impl Drop for Value {
        fn drop(&mut self) {
            if let Some(read) = self.1.take() {
                read();
            }
        }
    }
    let value = signal(Value(0, None));
    let observed = Rc::new(Cell::new(0));
    let _watch = effect({
        let value = value.clone();
        let observed = observed.clone();
        move || observed.set(value.with(|value| value.0))
    });
    let drops = Rc::new(Cell::new(0));
    let on_drop = || {
        let value = value.clone();
        let observed = observed.clone();
        let drops = drops.clone();
        Box::new(move || {
            assert_eq!(value.with_untracked(|value| value.0), 2);
            assert_eq!(observed.get(), 2);
            drops.set(drops.get() + 1);
        }) as Box<dyn Fn()>
    };
    value.set(Value(1, Some(on_drop())));
    value.set(Value(2, None));
    value.set(Value(2, Some(on_drop())));
    assert_eq!(drops.get(), 2);
}

#[test]
fn reruns_that_repeat_their_reads_keep_every_dependency_live() {
    use std::{cell::Cell, rc::Rc};
    let (a, b) = (fusor::signal(0), fusor::signal(0));
    let calls = Rc::new(Cell::new(0));
    let _effect = fusor::effect({
        let (a, b, calls) = (a.clone(), b.clone(), calls.clone());
        move || {
            calls.set(calls.get() + 1);
            // The duplicate read is one dependency, in the same place each run.
            a.get();
            a.get();
            b.get();
        }
    });
    for round in 1..=3 {
        a.set(round);
        assert_eq!(calls.get(), round * 2);
        b.set(round);
        assert_eq!(calls.get(), round * 2 + 1);
    }
}

#[test]
fn reads_that_change_order_or_stop_rebuild_dependencies_exactly() {
    use std::{cell::Cell, rc::Rc};
    let mode = fusor::signal(0);
    let (a, b, c) = (fusor::signal(0), fusor::signal(0), fusor::signal(0));
    let calls = Rc::new(Cell::new(0));
    let _effect = fusor::effect({
        let (mode, a, b, c, calls) = (mode.clone(), a.clone(), b.clone(), c.clone(), calls.clone());
        move || {
            calls.set(calls.get() + 1);
            match mode.get() {
                0 => {
                    a.get();
                    b.get();
                    c.get();
                }
                1 => {
                    // Same prefix, then a different order with a repeat.
                    c.get();
                    a.get();
                    c.get();
                }
                _ => {}
            }
        }
    });
    assert_eq!(calls.get(), 1);
    mode.set(1);
    assert_eq!(calls.get(), 2);
    b.set(1);
    assert_eq!(calls.get(), 2, "b was not read again");
    for (signal, expected) in [(&a, 3), (&c, 4)] {
        signal.update(|value| *value += 1);
        assert_eq!(calls.get(), expected);
    }
    mode.set(2);
    assert_eq!(calls.get(), 5);
    for signal in [&a, &b, &c] {
        signal.update(|value| *value += 1);
    }
    assert_eq!(
        calls.get(),
        5,
        "a run that reads nothing else keeps only mode"
    );
    mode.set(0);
    assert_eq!(calls.get(), 6);
    b.set(10);
    assert_eq!(calls.get(), 7);
}

#[test]
fn a_panic_while_repeating_reads_releases_the_unread_rest() {
    use std::{
        cell::Cell,
        panic::{AssertUnwindSafe, catch_unwind},
        rc::Rc,
    };
    let (a, b, c) = (fusor::signal(0), fusor::signal(0), fusor::signal(0));
    let fail = Rc::new(Cell::new(false));
    let calls = Rc::new(Cell::new(0));
    let _effect = fusor::effect({
        let (a, b, c, fail, calls) = (a.clone(), b.clone(), c.clone(), fail.clone(), calls.clone());
        move || {
            calls.set(calls.get() + 1);
            a.get();
            assert!(!fail.replace(false), "intentional partial collection");
            b.get();
            c.get();
        }
    });
    fail.set(true);
    assert!(catch_unwind(AssertUnwindSafe(|| a.set(1))).is_err());
    assert_eq!(calls.get(), 2);
    b.set(1);
    c.set(1);
    assert_eq!(calls.get(), 2, "sources after the panic were released");
    a.set(2);
    assert_eq!(calls.get(), 3);
    b.set(2);
    assert_eq!(calls.get(), 4);
}

#[test]
fn a_wide_memo_that_rereads_its_inputs_in_order_stays_exact() {
    let inputs: Vec<_> = (0..1000u64).map(fusor::signal).collect();
    let sum = fusor::memo({
        let inputs = inputs.clone();
        move || inputs.iter().map(|input| input.get()).sum::<u64>()
    });
    let mut expected: u64 = (0..1000).sum();
    assert_eq!(sum.get(), expected);
    for round in 0..3 {
        fusor::batch(|| {
            for input in inputs.iter().skip(round).step_by(7) {
                input.update(|value| *value += 2);
                expected += 2;
            }
        });
        assert_eq!(sum.get(), expected);
    }
    inputs[999].set(0);
    expected -= 999 + 2 * (0..3).filter(|round| (999 - round) % 7 == 0).count() as u64;
    assert_eq!(sum.get(), expected);
}
