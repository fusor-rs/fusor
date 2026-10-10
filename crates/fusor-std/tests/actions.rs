#![cfg(feature = "actions")]
use fusor::{Owner, effect};
use fusor_std::actions::{Action, AdmissionError, Outcome, Status};
use fusor_test::{ControlledLoader, TestExecutor};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[test]
fn direct_actions_reject_busy_and_effect_can_dispatch_on_completion() {
    let owner = Owner::new();
    let executor = TestExecutor::new();
    let backend = ControlledLoader::<Rc<String>, Outcome<(), ()>, ()>::new();
    let loader = backend.clone();
    let action = Action::new(
        &owner.handle(),
        move |command, context| {
            let future = loader.load(command, context);
            async move { future.await.unwrap() }
        },
        executor.spawner(),
    );
    assert_eq!(
        action.dispatch("first".into()).unwrap_err().reason,
        AdmissionError::NotActive
    );
    owner.commit();
    let first = action.dispatch("first".into()).unwrap();
    let busy = action.dispatch("second".into()).unwrap_err();
    assert_eq!(busy.reason, AdmissionError::Busy);
    assert_eq!(
        std::error::Error::source(&busy).and_then(|cause| cause.downcast_ref::<AdmissionError>()),
        Some(&AdmissionError::Busy)
    );
    assert_eq!(busy.command, "second");
    assert_eq!(action.state().submission.unwrap().id, first);
    let dispatched = Rc::new(Cell::new(false));
    let flag = dispatched.clone();
    let captured = action.clone();
    let _effect = effect(move || {
        if captured.state().status == Status::Accepted && !flag.replace(true) {
            captured.dispatch("next".into()).unwrap();
        }
    });
    executor.run_until_stalled();
    backend
        .next_request()
        .unwrap()
        .complete(Ok(Outcome::Accepted(())))
        .unwrap();
    executor.run_until_stalled();
    assert!(dispatched.get());
    assert!(action.pending());
    let next = backend.next_request().unwrap();
    assert_eq!(*next.key, "next");
    assert!(!next.is_cancelled());
    next.complete(Ok(Outcome::Unknown(()))).unwrap();
    executor.run_until_stalled();
    assert_eq!(
        action.dispatch("retry".into()).unwrap_err().reason,
        AdmissionError::Unresolved
    );
    action.reconcile().unwrap();
    assert_eq!(action.state().status, Status::Idle);
}

#[test]
fn disposing_from_pending_notification_never_starts_transport() {
    let owner = Rc::new(Owner::new());
    owner.commit();
    let executor = TestExecutor::new();
    let started = Rc::new(Cell::new(false));
    let flag = started.clone();
    let action = Action::new(
        &owner.handle(),
        move |_: Rc<()>, _| {
            flag.set(true);
            async { Outcome::<(), ()>::Accepted(()) }
        },
        executor.spawner(),
    );
    let captured = action.clone();
    let parent = owner.clone();
    let _effect = effect(move || {
        if captured.pending() {
            parent.dispose();
        }
    });
    action.dispatch(()).unwrap();
    executor.run_until_stalled();
    assert!(!started.get());
    assert_eq!(action.state().status, Status::Disposed);
}

#[test]
fn completion_releases_cancellation_and_disposal_cancels_pending_action() {
    let owner = Owner::new();
    owner.commit();
    let executor = TestExecutor::new();
    let backend = ControlledLoader::<Rc<()>, Outcome<(), ()>, ()>::new();
    let loader = backend.clone();
    let observed_token = Rc::new(RefCell::new(None));
    let captured_token = observed_token.clone();
    let action = Action::new(
        &owner.handle(),
        move |command, token| {
            *captured_token.borrow_mut() = Some(token.clone());
            let future = loader.load(command, token);
            async move { future.await.unwrap() }
        },
        executor.spawner(),
    );
    action.dispatch(()).unwrap();
    executor.run_until_stalled();
    backend
        .next_request()
        .unwrap()
        .complete(Ok(Outcome::Accepted(())))
        .unwrap();
    executor.run_until_stalled();
    assert_eq!(action.state().status, Status::Accepted);
    let completed_token = observed_token.borrow().as_ref().unwrap().clone();
    action.dispatch(()).unwrap();
    executor.run_until_stalled();
    let pending = backend.next_request().unwrap();
    action.dispose();
    assert_eq!(action.state().status, Status::Disposed);
    assert!(pending.is_cancelled());
    assert!(!completed_token.is_cancelled());
    assert_eq!(backend.counts().cancelled, 1);
    executor.run_until_stalled();
    assert!(matches!(
        pending.complete(Ok(Outcome::Accepted(()))),
        Err(Ok(Outcome::Accepted(())))
    ));
}
