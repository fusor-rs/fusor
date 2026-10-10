use fusor::{
    Owner, Signal, batch,
    coherence::{AsyncBoundary, BoundaryStatus, Error, ErrorKind, Publication},
    signal,
};
use fusor_async::{AsyncRead, AsyncValue};
use fusor_test::{ControlledLoader, TestExecutor};
use std::{cell::RefCell, error::Error as _, rc::Rc};

struct Publish {
    view: Rc<RefCell<Vec<String>>>,
    next: Vec<String>,
    after: Option<Box<dyn FnOnce()>>,
}
impl Publication for Publish {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn apply(&mut self) -> Result<(), Error> {
        *self.view.borrow_mut() = std::mem::take(&mut self.next);
        Ok(())
    }
    fn finish(mut self: Box<Self>) {
        if let Some(after) = self.after.take() {
            after();
        }
    }
}
type Loader = ControlledLoader<String, String, String>;
fn read(
    owner: &Owner,
    selected: &Signal<String>,
    loader: &Loader,
    executor: &TestExecutor,
) -> AsyncValue<String, String, String> {
    let selected = selected.clone();
    let loader = loader.clone();
    AsyncValue::new(
        &owner.handle(),
        move || selected.get(),
        move |key, context| loader.load(key, context),
        executor.spawner(),
    )
}

fn complete(loader: &Loader, executor: &TestExecutor, result: Result<String, String>) {
    loader.next_request().unwrap().complete(result).unwrap();
    executor.run_until_stalled();
}

fn publication(view: &Rc<RefCell<Vec<String>>>, next: Vec<String>) -> Box<dyn Publication> {
    Box::new(Publish {
        view: view.clone(),
        next,
        after: None,
    })
}

fn append_ready(
    read: &AsyncValue<String, String, String>,
    attempt: &fusor::coherence::Attempt,
    next: &mut Vec<String>,
) -> Result<bool, Error> {
    let AsyncRead::Ready(value) = read.read(attempt)? else {
        return Ok(false);
    };
    next.push((*value).clone());
    Ok(true)
}

#[test]
fn independent_reads_prepare_without_dom_activation_and_commit_together() {
    let owner = Owner::new();
    let executor = TestExecutor::new();
    let selected = signal("A".to_owned());
    let prices = Loader::new();
    let stocks = Loader::new();
    let price = read(&owner, &selected, &prices, &executor);
    let stock = read(&owner, &selected, &stocks, &executor);
    let view = Rc::new(RefCell::new(vec!["fallback".to_owned()]));
    let boundary = AsyncBoundary::coherent();
    let interactive = Rc::new(RefCell::new(Vec::new()));
    let _status_observer = fusor::effect({
        let boundary = boundary.clone();
        let interactive = interactive.clone();
        move || interactive.borrow_mut().push(boundary.is_interactive())
    });

    let _mount = boundary
        .attach(&owner.handle(), {
            let selected = selected.clone();
            let view = view.clone();
            move |attempt| {
                let mut next = vec![selected.get()];
                append_ready(&price, attempt, &mut next)?;
                append_ready(&stock, attempt, &mut next)?;
                Ok(publication(&view, next))
            }
        })
        .unwrap();
    executor.run_until_stalled();
    assert!(!owner.handle().is_active());
    complete(&prices, &executor, Ok("price-A".into()));
    assert_eq!(&*view.borrow(), &["fallback"]);
    complete(&stocks, &executor, Ok("stock-A".into()));
    assert_eq!(&*view.borrow(), &["A", "price-A", "stock-A"]);
    assert_eq!(boundary.status(), BoundaryStatus::Ready);
    assert_eq!(interactive.borrow().last(), Some(&true));

    selected.set("B".into());
    assert_eq!(interactive.borrow().last(), Some(&false));

    executor.run_until_stalled();
    let old_price = prices.next_request().unwrap();
    let old_stock = stocks.next_request().unwrap();
    selected.set("C".into());
    executor.run_until_stalled();
    assert!(old_price.is_cancelled() && old_stock.is_cancelled());
    for (request, value) in [(old_stock, "stock-B"), (old_price, "price-B")] {
        assert_eq!(request.complete(Ok(value.into())), Err(Ok(value.into())));
    }
    complete(&stocks, &executor, Ok("stock-C".into()));
    assert_eq!(&*view.borrow(), &["A", "price-A", "stock-A"]);
    complete(&prices, &executor, Ok("price-C".into()));
    assert_eq!(&*view.borrow(), &["C", "price-C", "stock-C"]);
    assert_eq!(prices.counts().started, 3);
    owner.dispose();
    assert_eq!(boundary.status(), BoundaryStatus::Disposed);
}

#[test]
fn aba_restarts_but_retry_reuses_success_and_third_descendant_joins() {
    let owner = Owner::new();
    let executor = TestExecutor::new();
    let selected = signal("A".to_owned());
    let locale = signal("en".to_owned());
    let first = Loader::new();
    let second = Loader::new();
    let third = Loader::new();
    let first_read = read(&owner, &selected, &first, &executor);
    let second_read = read(&owner, &selected, &second, &executor);
    let dependent_read = read(&owner, &selected, &third, &executor);
    let boundary = AsyncBoundary::coherent();
    let view = Rc::new(RefCell::new(vec![]));
    let _mount = boundary
        .attach(&owner.handle(), {
            let view = view.clone();
            let locale = locale.clone();
            move |attempt| {
                let mut next = vec![locale.get()];
                append_ready(&first_read, attempt, &mut next)?;
                if append_ready(&second_read, attempt, &mut next)? {
                    append_ready(&dependent_read, attempt, &mut next)?;
                }
                Ok(publication(&view, next))
            }
        })
        .unwrap();
    executor.run_until_stalled();
    let stale_a = first.next_request().unwrap();
    let stale_b = second.next_request().unwrap();
    batch(|| {
        selected.set("B".into());
        selected.set("A".into());
    });
    executor.run_until_stalled();
    assert!(stale_a.is_cancelled() && stale_b.is_cancelled());
    complete(&first, &executor, Ok("a".into()));
    complete(&second, &executor, Err("unavailable".into()));
    let BoundaryStatus::Error(error) = boundary.status() else {
        panic!("expected read failure")
    };
    assert_eq!(error.kind(), ErrorKind::Read);
    assert_eq!(error.downcast_ref::<String>().unwrap(), "unavailable");
    assert_eq!(error.source().unwrap().to_string(), "unavailable");
    assert!(view.borrow().is_empty());
    boundary.retry();
    executor.run_until_stalled();
    assert_eq!(first.counts().started, 2);
    complete(&second, &executor, Ok("b".into()));
    assert!(view.borrow().is_empty());
    assert_eq!(third.counts().started, 1);
    complete(&third, &executor, Ok("c".into()));
    assert_eq!(&*view.borrow(), &["en", "a", "b", "c"]);
    locale.set("fr".into());
    executor.run_until_stalled();
    assert_eq!(&*view.borrow(), &["fr", "a", "b", "c"]);
    assert_eq!(first.counts().started, 2);
}

#[test]
fn own_status_cycles_fail_and_attach_is_exclusive() {
    let owner = Owner::new();
    let boundary = AsyncBoundary::coherent();
    let captured = boundary.clone();
    let _mount = boundary
        .attach(&owner.handle(), move |_| {
            captured.status();
            Ok(Box::new(Publish {
                view: Rc::new(RefCell::new(vec![])),
                next: vec![],
                after: None,
            }))
        })
        .unwrap();
    assert_eq!(
        boundary.status(),
        BoundaryStatus::Error("a coherent region cannot read its own boundary status".into())
    );
    assert_eq!(
        boundary.attach(&owner.handle(), |_| unreachable!()).err(),
        Some("an async boundary can only attach once".into())
    );
}

#[test]
fn activation_invalidation_cannot_publish_ready_for_obsolete_inputs() {
    let owner = Owner::new();
    let selected = signal(0);
    let boundary = AsyncBoundary::coherent();
    let view = Rc::new(RefCell::new(vec![]));
    let _mount = boundary
        .attach(&owner.handle(), {
            let selected = selected.clone();
            let view = view.clone();
            move |_| {
                let value = selected.get();
                let selected = selected.clone();
                Ok(Box::new(Publish {
                    view: view.clone(),
                    next: vec![value.to_string()],
                    after: Some(Box::new(move || {
                        if value == 0 {
                            selected.set(1);
                        }
                    })),
                }))
            }
        })
        .unwrap();
    assert_eq!(&*view.borrow(), &["1"]);
    assert_eq!(boundary.status(), BoundaryStatus::Ready);
}

#[test]
fn abandoned_reads_cancel_and_impure_evaluation_does_not_publish() {
    let owner = Owner::new();
    let executor = TestExecutor::new();
    let selected = signal("A".to_owned());
    let loader = Loader::new();
    let value = read(&owner, &selected, &loader, &executor);
    let boundary = AsyncBoundary::coherent();
    let mount = boundary
        .attach(&owner.handle(), move |attempt| {
            value.read(attempt)?;
            Ok(Box::new(Publish {
                view: Rc::new(RefCell::new(vec![])),
                next: vec![],
                after: None,
            }))
        })
        .unwrap();
    executor.run_until_stalled();
    let pending = loader.next_request().unwrap();
    drop(mount);
    assert!(pending.is_cancelled());
    assert_eq!(boundary.status(), BoundaryStatus::Disposed);

    let boundary = AsyncBoundary::coherent();
    let view = Rc::new(RefCell::new(vec!["previous".to_owned()]));
    let input = signal(0);
    let _mount = boundary
        .attach(&owner.handle(), {
            let view = view.clone();
            move |_| {
                input.set(input.get() + 1);
                Ok(Box::new(Publish {
                    view: view.clone(),
                    next: vec!["invalid".into()],
                    after: None,
                }))
            }
        })
        .unwrap();
    assert_eq!(
        boundary.status(),
        BoundaryStatus::Error("coherent render evaluation must be pure: signal write".into())
    );
    assert_eq!(&*view.borrow(), &["previous"]);
}

#[test]
fn failed_dom_publication_faults_without_activating_work() {
    struct Failure;
    impl Publication for Failure {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn apply(&mut self) -> Result<(), Error> {
            Err("DOM removed".into())
        }
        fn finish(self: Box<Self>) {
            panic!("must not activate a failed publication");
        }
    }
    let owner = Owner::new();
    let boundary = AsyncBoundary::coherent();
    let _mount = boundary
        .attach(&owner.handle(), |_| Ok(Box::new(Failure)))
        .unwrap();
    assert_eq!(
        boundary.status(),
        BoundaryStatus::Faulted("DOM removed".into())
    );
}

#[test]
fn retained_read_can_remount_after_its_boundary_disposes_but_cannot_join_two_live_views() {
    let owner = Owner::new();
    let executor = TestExecutor::new();
    let key = signal("A".to_owned());
    let loader = Loader::new();
    let data = read(&owner, &key, &loader, &executor);
    let view = Rc::new(RefCell::new(Vec::new()));
    let attach = |boundary: &AsyncBoundary| {
        let data = data.clone();
        let view = view.clone();
        boundary
            .attach(&owner.handle(), move |attempt| {
                let next = match data.read(attempt)? {
                    AsyncRead::Ready(value) => vec![value.as_ref().clone()],
                    AsyncRead::Pending => Vec::new(),
                };
                Ok(publication(&view, next))
            })
            .unwrap()
    };
    let first = AsyncBoundary::coherent();
    let first_mount = attach(&first);
    executor.run_until_stalled();
    let old = loader.next_request().unwrap();
    let conflicting = AsyncBoundary::coherent();
    let conflict_mount = attach(&conflicting);
    assert_eq!(
        conflicting.status(),
        BoundaryStatus::Error(
            "one AsyncValue cannot participate in different live async boundaries".into()
        )
    );
    assert!(!old.is_cancelled());
    drop(conflict_mount);
    drop(first_mount);
    assert!(old.is_cancelled());
    let replacement = AsyncBoundary::coherent();
    let _replacement_mount = attach(&replacement);
    executor.run_until_stalled();
    let fresh = loader.next_request().unwrap();
    assert_eq!(
        old.complete(Ok("obsolete".into())),
        Err(Ok("obsolete".into()))
    );
    executor.run_until_stalled();
    assert!(view.borrow().is_empty());
    fresh.complete(Ok("fresh".into())).unwrap();
    executor.run_until_stalled();
    assert_eq!(&*view.borrow(), &["fresh"]);
    assert_eq!(replacement.status(), BoundaryStatus::Ready);
}
