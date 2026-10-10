use fixture_components::{
    AsyncPanel, AsyncPanelInputs, Child, ChildInputs, Panel, PanelInputs, Routing, RoutingInputs,
    Row,
};
use fusor::{FromInputs, signal};
use memory_renderer::{Children, Component};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn main() {
    async_contract();
    routing_contract();
    let visible = signal(true);
    let rows = signal(vec![row(1, "one"), row(2, "two")]);
    let cleanups = Rc::new(Cell::new(0));
    let number = signal(3);
    let checked = signal(false);
    let observed = signal(0);
    let inputs = PanelInputs {
        title: "Shared".into(),
        visible: visible.clone(),
        rows: rows.clone(),
        cleanups: cleanups.clone(),
        number: number.clone(),
        checked: checked.clone(),
        observed: observed.clone(),
    };
    let scope = Panel::prepare(
        None,
        Box::new(|owner| {
            let calls = Rc::new(Cell::new(0));
            let seen = calls.clone();
            let _effect = fusor::effect(move || seen.set(seen.get() + 1));
            assert_eq!(
                calls.get(),
                1,
                "ordinary constructor effects run immediately"
            );
            Panel::from_inputs(inputs, owner).map_err(memory_renderer::error)
        }),
        Children::default(),
    )
    .unwrap();
    let root = scope.root();
    assert_eq!(root.attribute("class"), Some("static-panel"));
    assert!(root.text().contains("Static & complete"));
    assert!(root.text().contains("Shared:one"));
    assert!(root.text().contains("Shared:10"));
    assert_eq!(
        root.elements("p")
            .iter()
            .filter(|node| node.attribute("class") == Some("child-tail"))
            .map(|node| node.text())
            .collect::<Vec<_>>(),
        ["Shared tail", "one tail", "two tail"]
    );
    assert_eq!(root.find("projected").unwrap().text(), "Shared");
    let named = root.find("named-number").unwrap();
    assert_eq!(named.text(), "3");
    let payload = root.find("payload").unwrap();
    assert_eq!(payload.text(), "3");
    let increment = root.find("increment").unwrap();
    increment.dispatch("click");
    assert!(
        increment.text().contains("Shared: 0"),
        "unpublished owner blocks user callbacks"
    );
    scope.publish();
    increment.dispatch("click");
    assert!(increment.text().contains("Shared: 1"));

    let input = root.find("number").unwrap();
    input.edit("12");
    assert_eq!(number.get(), 12);
    assert_eq!(root.find("named-number").unwrap().id(), named.id());
    assert_eq!(named.text(), "12");
    assert_eq!(
        observed.get(),
        12,
        "bind listener precedes authored input listener"
    );
    assert_eq!(root.find("payload").unwrap().id(), payload.id());
    assert_eq!(
        payload.text(),
        "12",
        "matching branch payload updates without replacement"
    );
    input.edit("012");
    assert_eq!(number.get(), 12);
    assert_eq!(
        input.value(),
        "012",
        "equal parsed values preserve authored drafts"
    );
    input.edit("-");
    assert_eq!(number.get(), 12);
    assert_eq!(input.value(), "-", "invalid text draft survives");
    number.set(20);
    assert_eq!(input.value(), "20");
    let checkbox = root.find("checked").unwrap();
    checkbox.check(true);
    assert!(checked.get());
    checked.set(false);
    assert!(!checkbox.checked());

    let before = root.elements("li");
    before[0].elements("button")[0].dispatch("click");
    assert!(before[0].text().contains("one clicks 1"));
    rows.set(vec![row(2, "two"), row(1, "one")]);
    let after = root.elements("li");
    assert_eq!(after[0].id(), before[1].id());
    assert_eq!(after[1].id(), before[0].id());
    assert!(
        after[1].text().contains("one clicks 1"),
        "keyed component local state survives reorder"
    );

    let remove = root
        .elements("button")
        .into_iter()
        .find(|node| node.attribute("class") == Some("remove"))
        .unwrap();
    remove.dispatch("click");
    assert!(!visible.get());
    assert_eq!(
        cleanups.get(),
        1,
        "callback safely removes its own component"
    );
    remove.dispatch("click");
    assert_eq!(cleanups.get(), 1, "stale callback is inert");
    rows.set(vec![row(1, "one")]);
    assert_eq!(cleanups.get(), 2);
    scope.dispose();
    assert_eq!(cleanups.get(), 3);
    increment.dispatch("click");
    assert!(increment.text().contains("Shared: 1"));

    assert!(
        Child::prepare(
            None,
            Box::new(|owner| {
                Child::from_inputs(
                    ChildInputs {
                        label: String::new(),
                        visible: signal(true),
                        cleanups: Rc::new(Cell::new(0)),
                    },
                    owner,
                )
                .map_err(memory_renderer::error)
            }),
            Children::default()
        )
        .is_err(),
        "generated preparation propagates fallible constructor errors"
    );
    println!(
        "external backend: static structure, typed inputs, local state, events, bind, nested captures, keyed identity and cleanup passed"
    );
}

fn async_contract() {
    use fusor::coherence::{AsyncBoundary, BoundaryStatus};
    use fusor_async::AsyncValue;
    use fusor_test::{ControlledLoader, TestExecutor};
    let executor = Rc::new(TestExecutor::new());
    let values = ControlledLoader::<u32, String, String>::new();
    let row_values = ControlledLoader::<String, String, String>::new();
    let boundary = AsyncBoundary::coherent();
    let selection = signal(1);
    let rows = signal(vec![row(1, "one"), row(2, "two")]);
    let starts = Rc::new(Cell::new(0));
    let cleanups = Rc::new(Cell::new(0));
    let clicks = signal(0);
    let visible = signal(true);
    let scope = AsyncPanel::prepare(
        None,
        Box::new(|owner| {
            let key = selection.clone();
            let loader = values.clone();
            let read = AsyncValue::new(
                &owner,
                move || key.get(),
                move |key, cancel| loader.load(key, cancel),
                executor.spawner(),
            );
            AsyncPanel::from_inputs(
                AsyncPanelInputs {
                    prefix: "Async".into(),
                    selection: selection.clone(),
                    read,
                    boundary: boundary.clone(),
                    rows: rows.clone(),
                    starts: starts.clone(),
                    cleanups: cleanups.clone(),
                    clicks: clicks.clone(),
                    visible: visible.clone(),
                    make_read: {
                        let executor = executor.clone();
                        let loader = row_values.clone();
                        Rc::new(move |owner, row| {
                            let loader = loader.clone();
                            AsyncValue::new(
                                owner,
                                move || row.get().label,
                                move |key, cancel| loader.load(key, cancel),
                                executor.spawner(),
                            )
                        })
                    },
                },
                owner,
            )
            .map_err(memory_renderer::error)
        }),
        Children::default(),
    )
    .unwrap();
    scope.publish();
    let root = scope.root();
    executor.run_until_stalled();
    assert_eq!(boundary.status(), BoundaryStatus::Pending);
    assert_eq!(starts.get(), 0, "candidate effects wait for publication");
    assert_eq!(root.find("selection").unwrap().text(), "");
    values
        .next_request()
        .unwrap()
        .complete(Ok("first".into()))
        .unwrap();
    executor.run_until_stalled();
    let first_row = row_values.next_request().unwrap();
    let second_row = row_values.next_request().unwrap();
    assert_eq!((&*first_row.key, &*second_row.key), ("one", "two"));
    first_row.complete(Ok("ONE".into())).unwrap();
    executor.run_until_stalled();
    assert_eq!(boundary.status(), BoundaryStatus::Pending);
    assert_eq!(starts.get(), 0);
    second_row.complete(Ok("TWO".into())).unwrap();
    executor.run_until_stalled();
    assert_eq!(boundary.status(), BoundaryStatus::Ready);
    assert_eq!(
        starts.get(),
        2,
        "read completion reuses prepared constructors"
    );
    assert_eq!(
        root.find("nested-ready").unwrap().text(),
        "Async:first:first"
    );
    assert_eq!(root.find("async-children").unwrap().text(), "first");
    assert_eq!(root.find("async-named").unwrap().text(), "first");
    let before = root.elements("li");
    let click = root.find("async-click").unwrap();
    click.dispatch("click");
    assert_eq!(clicks.get(), 1);

    rows.set(vec![row(2, "changed"), row(1, "one")]);
    executor.run_until_stalled();
    let changed = row_values.next_request().unwrap();
    assert_eq!(
        changed.key, "changed",
        "cached row memo sees candidate input"
    );
    assert_eq!(boundary.status(), BoundaryStatus::Pending);
    assert_eq!(root.elements("li")[0].id(), before[0].id());
    click.dispatch("click");
    assert_eq!(clicks.get(), 1, "pending scene is not interactive");
    changed.complete(Ok("CHANGED".into())).unwrap();
    let retained = row_values.next_request().unwrap();
    assert_eq!(retained.key, "one");
    retained.complete(Ok("ONE".into())).unwrap();
    executor.run_until_stalled();
    assert_eq!(boundary.status(), BoundaryStatus::Ready);
    let after = root.elements("li");
    assert_eq!(
        (after[0].id(), after[1].id()),
        (before[1].id(), before[0].id())
    );
    assert!(
        after[0].text().contains("Async:0:changed"),
        "{}",
        after[0].text()
    );
    assert!(after[0].text().contains("CHANGED"));
    assert_eq!(starts.get(), 2);

    let previous = root.text();
    rows.set(vec![row(3, "reject")]);
    let BoundaryStatus::Error(error) = boundary.status() else {
        panic!("expected renderer rejection");
    };
    assert_eq!(error.kind(), fusor::coherence::ErrorKind::Renderer);
    assert_eq!(error.downcast_ref::<&str>(), Some(&"rejected async row"));
    assert_eq!(
        root.text(),
        previous,
        "failed preparation preserves the complete scene"
    );
    assert_eq!(starts.get(), 2);
    rows.set(vec![row(1, "one")]);
    executor.run_until_stalled();
    row_values
        .next_request()
        .unwrap()
        .complete(Ok("ONE".into()))
        .unwrap();
    executor.run_until_stalled();
    assert_eq!(boundary.status(), BoundaryStatus::Ready);
    assert_eq!(cleanups.get(), 1);

    selection.set(2);
    executor.run_until_stalled();
    let obsolete = values.next_request().unwrap();
    selection.set(3);
    executor.run_until_stalled();
    assert!(obsolete.is_cancelled());
    let _ = obsolete.complete(Ok("obsolete".into()));
    let newest = values.next_request().unwrap();
    assert_eq!(newest.key, 3);
    newest.complete(Err("offline".into())).unwrap();
    executor.run_until_stalled();
    let BoundaryStatus::Error(error) = boundary.status() else {
        panic!("expected rejected resource read");
    };
    assert_eq!(error.kind(), fusor::coherence::ErrorKind::Read);
    assert_eq!(
        error.downcast_ref::<String>().map(String::as_str),
        Some("offline")
    );
    assert_eq!(root.find("async-value").unwrap().text(), "first");
    boundary.retry();
    executor.run_until_stalled();
    values
        .next_request()
        .unwrap()
        .complete(Ok("third".into()))
        .unwrap();
    executor.run_until_stalled();
    assert_eq!(boundary.status(), BoundaryStatus::Ready);
    assert_eq!(root.find("selection").unwrap().text(), "3");
    assert_eq!(root.find("async-value").unwrap().text(), "third");
    let remove = root.find("async-remove").unwrap();
    remove.dispatch("click");
    assert!(!visible.get());
    assert!(root.find("async-remove").is_none());
    remove.dispatch("click");

    selection.set(4);
    executor.run_until_stalled();
    let pending = values.next_request().unwrap();
    scope.dispose();
    executor.run_until_stalled();
    assert!(pending.is_cancelled());
    assert_eq!(boundary.status(), BoundaryStatus::Disposed);
    assert_eq!(cleanups.get(), 2);
    click.dispatch("click");
    assert_eq!(clicks.get(), 1);
}

fn routing_contract() {
    use fusor_router::{AppUrl, view::Navigation};
    type Nav = Navigation<memory_renderer::Scope>;
    let handle = Rc::new(RefCell::new(None::<Nav>));
    let cleanups = Rc::new(Cell::new(0));
    let inputs = RoutingInputs {
        title: "Routes".into(),
        visible: signal(true),
        cleanups: cleanups.clone(),
        go: {
            let handle = Rc::downgrade(&handle);
            Rc::new(move |path| {
                handle
                    .upgrade()
                    .unwrap()
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .navigate(AppUrl::parse(path).unwrap())
                    .unwrap();
            })
        },
    };
    let scope = Routing::prepare(
        None,
        Box::new(|owner| Routing::from_inputs(inputs, owner).map_err(memory_renderer::error)),
        Children::default(),
    )
    .unwrap();
    let navigation = Nav::from_owner(&scope.owner()).unwrap();
    *handle.borrow_mut() = Some(navigation.clone());
    scope.publish();
    let root = scope.root();
    assert!(root.find("home").is_some());
    let navigate = |path| navigation.navigate(AppUrl::parse(path).unwrap());
    navigate("/teams/alpha/members/1").unwrap();
    let team = root.find("team").unwrap();
    let member = root.find("member").unwrap();
    member.elements("button")[0].dispatch("click");
    navigate("/teams/alpha/members/1?q=1#details").unwrap();
    assert_eq!(root.find("team").unwrap().id(), team.id());
    assert_eq!(root.find("member").unwrap().id(), member.id());
    assert!(root.text().contains("1 clicks 1"));
    assert_eq!(cleanups.get(), 0);
    navigate("/teams/alpha/members/2").unwrap();
    assert_eq!(root.find("team").unwrap().id(), team.id());
    assert_ne!(root.find("member").unwrap().id(), member.id());
    assert_eq!(root.find("member-name").unwrap().text(), "Routes:alpha:2");
    assert_eq!(cleanups.get(), 1);
    let location = navigation.location().get();
    assert!(navigate("/broken").is_err());
    assert_eq!(navigation.location().get(), location);
    assert_eq!(root.find("member-name").unwrap().text(), "Routes:alpha:2");
    navigate("/teams/beta/members/2").unwrap();
    assert_ne!(root.find("team").unwrap().id(), team.id());
    assert_eq!(cleanups.get(), 3);
    let leave = root.find("leave").unwrap();
    leave.dispatch("click");
    assert!(root.find("home").is_some());
    assert_eq!(
        cleanups.get(),
        5,
        "navigation removes its own callback's view"
    );
    leave.dispatch("click");
    assert_eq!(cleanups.get(), 5);
    scope.dispose();
    assert!(navigate("/teams/alpha/members/1").is_err());
}

fn row(id: u32, label: &str) -> Row {
    Row {
        id,
        label: label.into(),
        children: vec![id * 10],
    }
}

#[test]
fn compiled_external_backend_contract() {
    main();
}
