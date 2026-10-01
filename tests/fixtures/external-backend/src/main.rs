use fixture_components::{Child, ChildInputs, Panel, PanelInputs, Row};
use fusor::{FromInputs, signal};
use memory_renderer::{Children, Component};
use std::{cell::Cell, rc::Rc};

fn main() {
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
        Box::new(|owner| Panel::from_inputs(inputs, owner).map_err(memory_renderer::error)),
        Children::default(),
    )
    .unwrap();
    let root = scope.root();
    assert_eq!(root.attribute("class"), Some("static-panel"));
    assert!(root.text().contains("Static & complete"));
    assert!(root.text().contains("Shared:one"));
    assert!(root.text().contains("Shared:10"));
    assert_eq!(root.find("projected").unwrap().text(), "Shared");
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
