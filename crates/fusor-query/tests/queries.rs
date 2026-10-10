use fusor::{Owner, signal};
use fusor_query::{Freshness, QueryClient, QueryOptions, QueryState};
use fusor_test::{ControlledLoader, OwnerProbe, TestClock, TestExecutor};
use std::{num::NonZeroUsize, time::Duration};

fn options(capacity: usize) -> QueryOptions {
    QueryOptions {
        freshness: Freshness::For(Duration::from_secs(10)),
        retention: Duration::from_secs(30),
        capacity: NonZeroUsize::new(capacity).unwrap(),
    }
}
fn active(parent: &Owner) -> Owner {
    let child = Owner::child(&parent.handle());
    child.commit();
    child
}
type Loader = ControlledLoader<u32, String, &'static str>;
fn client(
    parent: &Owner,
    pool: &TestExecutor,
    clock: &TestClock,
    loader: &Loader,
    capacity: usize,
) -> QueryClient<u32, String, &'static str> {
    let loader = loader.clone();
    QueryClient::new(
        &parent.handle(),
        options(capacity),
        move |key, context| loader.load(key, context),
        pool.spawner(),
        clock.reader(),
    )
}

#[test]
fn shares_requests_and_one_view_leaving_does_not_cancel_another() {
    let app = Owner::new();
    app.commit();
    let pool = TestExecutor::new();
    let clock = TestClock::default();
    let load = Loader::new();
    let client = client(&app, &pool, &clock, &load, 3);
    let departing_view = active(&app);
    let surviving_view = active(&app);
    let departing_query = client.observe(&departing_view.handle(), || Some(1));
    let surviving_query = client.observe(&surviving_view.handle(), || Some(1));
    pool.run_until_stalled();
    assert_eq!(load.counts().started, 1);
    assert_eq!(client.info().observers, 2);
    departing_view.dispose();
    assert_eq!(load.counts().cancelled, 0);
    assert!(matches!(departing_query.get(), QueryState::Disposed));
    load.next_request()
        .unwrap()
        .complete(Ok("shared".into()))
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(&*surviving_query.get().data().unwrap().value, "shared");
    surviving_view.dispose();
    let returning_view = active(&app);
    let returning_query = client.observe(&returning_view.handle(), || Some(1));
    pool.run_until_stalled();
    assert_eq!(load.counts().started, 1);
    assert_eq!(&*returning_query.get().data().unwrap().value, "shared");
}

#[test]
fn commit_gates_loading_and_last_observer_cancels_queued_results() {
    let app = Owner::new();
    let pool = TestExecutor::new();
    let clock = TestClock::default();
    let load = Loader::new();
    let client = client(&app, &pool, &clock, &load, 2);
    let view = active(&app);
    let probe = OwnerProbe::new(&view.handle());
    let query = client.observe(&view.handle(), || Some(1));
    pool.run_until_stalled();
    assert_eq!(load.counts().started, 0);
    app.commit();
    pool.run_until_stalled();
    let request = load.next_request().unwrap();
    view.dispose();
    assert!(request.is_cancelled());
    request.complete(Ok("late".into())).unwrap();
    pool.run_until_stalled();
    assert_eq!(probe.cleanup_count(), 1);
    assert!(probe.is_disposed());
    assert!(matches!(query.get(), QueryState::Disposed));
    assert_eq!(load.counts().live, 0);
    let view = active(&app);
    let query = client.observe(&view.handle(), || Some(1));
    pool.run_until_stalled();
    assert_eq!(load.counts().started, 2);
    assert!(query.get().data().is_none());
}

#[test]
fn invalidation_supersedes_pending_work_and_revalidation_keeps_cached_data() {
    let app = Owner::new();
    app.commit();
    let pool = TestExecutor::new();
    let clock = TestClock::default();
    let load = Loader::new();
    let client = client(&app, &pool, &clock, &load, 2);
    let view = active(&app);
    let query = client.observe(&view.handle(), || Some(1));
    pool.run_until_stalled();
    load.next_request()
        .unwrap()
        .complete(Ok("first".into()))
        .unwrap();
    pool.run_until_stalled();
    client.invalidate(&1);
    pool.run_until_stalled();
    let old = load.next_request().unwrap();
    query.refresh();
    pool.run_until_stalled();
    let new = load.next_request().unwrap();
    assert!(old.is_cancelled());
    assert_eq!(old.complete(Ok("stale".into())), Err(Ok("stale".into())));
    pool.run_until_stalled();
    assert_eq!(&*query.get().data().unwrap().value, "first");
    new.complete(Ok("new".into())).unwrap();
    pool.run_until_stalled();
    assert_eq!(&*query.get().data().unwrap().value, "new");
    client.invalidate(&1);
    pool.run_until_stalled();
    load.next_request()
        .unwrap()
        .complete(Err("offline"))
        .unwrap();
    pool.run_until_stalled();
    assert!(matches!(
        query.get(),
        QueryState::Error { key: 1, error, .. } if *error == "offline"
    ));
    assert_eq!(&*query.get().data().unwrap().value, "new");
}

#[test]
fn changing_keys_detaches_and_freshness_and_retention_use_injected_time() {
    let app = Owner::new();
    app.commit();
    let pool = TestExecutor::new();
    let clock = TestClock::default();
    let load = Loader::new();
    let client = client(&app, &pool, &clock, &load, 2);
    let view = active(&app);
    let key = signal(Some(1));
    let query = client.observe(&view.handle(), {
        let key = key.clone();
        move || key.get()
    });
    pool.run_until_stalled();
    load.next_request()
        .unwrap()
        .complete(Ok("one".into()))
        .unwrap();
    pool.run_until_stalled();
    key.set(None);
    clock.advance(Duration::from_secs(9));
    key.set(Some(1));
    pool.run_until_stalled();
    assert_eq!(load.counts().started, 1);
    key.set(None);
    clock.advance(Duration::from_secs(1));
    key.set(Some(1));
    pool.run_until_stalled();
    assert_eq!(load.counts().started, 2);
    assert_eq!(&*query.get().data().unwrap().value, "one");
    load.next_request()
        .unwrap()
        .complete(Ok("new one".into()))
        .unwrap();
    pool.run_until_stalled();
    key.set(Some(2));
    pool.run_until_stalled();
    assert!(
        query.get().data().is_none(),
        "never mislabel another key's cached data"
    );
    load.next_request()
        .unwrap()
        .complete(Ok("two".into()))
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(client.info().entries, 2);
    clock.advance(Duration::from_secs(30));
    assert_eq!(client.collect(), 1);
    assert_eq!(client.info().entries, 1);
}

#[test]
fn capacity_is_bounded_even_for_active_keys_and_clients_have_distinct_identities() {
    let app = Owner::new();
    app.commit();
    let pool = TestExecutor::new();
    let clock = TestClock::default();
    let load = Loader::new();
    let one = client(&app, &pool, &clock, &load, 1);
    let other = client(&app, &pool, &clock, &load, 1);
    let departing_view = active(&app);
    let surviving_view = active(&app);
    let departing_query = one.observe(&departing_view.handle(), || Some(1));
    let surviving_query = one.observe(&surviving_view.handle(), || Some(2));
    let independent = other.observe(&surviving_view.handle(), || Some(1));
    pool.run_until_stalled();
    assert_eq!(load.counts().started, 2);
    assert!(matches!(
        surviving_query.get(),
        QueryState::Capacity { key: 2 }
    ));
    assert_eq!(one.info().entries, 1);
    departing_query.dispose();
    surviving_query.refresh();
    pool.run_until_stalled();
    assert_eq!(load.counts().started, 3);
    assert_eq!(one.info().entries, 1);
    assert!(independent.get().is_loading());
    one.dispose();
    assert!(matches!(surviving_query.get(), QueryState::Disposed));
    assert!(independent.get().is_loading());
    app.dispose();
    pool.run_until_stalled();
    assert_eq!(load.counts().live, 0);
}

#[test]
fn dropping_last_query_handle_releases_subscription_and_disposal_clears_identity() {
    let app = Owner::new();
    app.commit();
    let pool = TestExecutor::new();
    let clock = TestClock::default();
    let load = Loader::new();
    let client = client(&app, &pool, &clock, &load, 2);
    let view = active(&app);
    let query = client.observe(&view.handle(), || Some(1));
    let clone = query.clone();
    pool.run_until_stalled();
    drop(query);
    assert_eq!(client.info().observers, 1);
    drop(clone);
    assert_eq!(client.info().observers, 0);
    assert_eq!(load.counts().cancelled, 1);
    let query = client.observe(&view.handle(), || Some(1));
    pool.run_until_stalled();
    client.dispose();
    assert_eq!(client.info().entries, 0);
    assert!(matches!(query.get(), QueryState::Disposed));
    let another = client.observe(&view.handle(), || Some(1));
    assert!(matches!(another.get(), QueryState::Disposed));
}

#[test]
fn cancellation_can_dispose_the_view_during_a_key_switch() {
    use std::{cell::Cell, future::pending, rc::Rc};
    let app = Owner::new();
    app.commit();
    let view = Rc::new(active(&app));
    let pool = TestExecutor::new();
    let started = Rc::new(Cell::new(0));
    let client = QueryClient::new(
        &app.handle(),
        options(2),
        {
            let view = view.clone();
            let started = started.clone();
            move |_key: u32, request| {
                started.set(started.get() + 1);
                let view = view.clone();
                let cancellation = request.on_cancel(move || view.dispose());
                async move {
                    let _cancellation = cancellation;
                    pending::<Result<String, ()>>().await
                }
            }
        },
        pool.spawner(),
        TestClock::default().reader(),
    );
    let key = signal(Some(1));
    let query = client.observe(&view.handle(), {
        let key = key.clone();
        move || key.get()
    });
    pool.run_until_stalled();
    key.set(Some(2));
    pool.run_until_stalled();
    assert!(matches!(query.get(), QueryState::Disposed));
    assert_eq!(client.info().observers, 0);
    assert_eq!(
        started.get(),
        1,
        "no request can start after a reentrant disposal"
    );
}

#[test]
fn non_clone_payloads_are_shared_and_released_at_session_disposal() {
    use std::{cell::Cell, rc::Rc};
    struct Value(Rc<Cell<usize>>);
    impl Drop for Value {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    struct Error;
    let app = Owner::new();
    app.commit();
    let pool = TestExecutor::new();
    let loader = ControlledLoader::<u32, Value, Error>::new();
    let client = QueryClient::new(
        &app.handle(),
        options(2),
        {
            let loader = loader.clone();
            move |key, request| loader.load(key, request)
        },
        pool.spawner(),
        TestClock::default().reader(),
    );
    let one = client.observe(&app.handle(), || Some(1));
    let two = client.observe(&app.handle(), || Some(1));
    pool.run_until_stalled();
    let drops = Rc::new(Cell::new(0));
    assert!(
        loader
            .next_request()
            .unwrap()
            .complete(Ok(Value(drops.clone())))
            .is_ok()
    );
    pool.run_until_stalled();
    assert!(Rc::ptr_eq(
        &one.get().data().unwrap().value,
        &two.get().data().unwrap().value
    ));
    client.dispose();
    assert_eq!(drops.get(), 1);
    assert!(matches!(one.get(), QueryState::Disposed));
    assert!(matches!(two.get(), QueryState::Disposed));
}
